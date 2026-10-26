//! A connection that answers to the slice of the rusqlite API the `db` modules use (`execute`,
//! `query_row`, `prepare` + `query`/`query_map`, `unchecked_transaction`, `changes`,
//! `is_autocommit`, `row.get`), so moving a module to Postgres changes its SQL, not its shape.
//!
//! Transactions nest: the outermost is `BEGIN`, inner ones are savepoints, and a guard that is
//! dropped without `commit` rolls back. A `Database` method can therefore open a transaction and
//! call other methods that open their own, as it did on SQLite.

use std::cell::{Cell, RefCell};
use std::fmt;

use postgres::error::SqlState;
use postgres::types::{FromSql, ToSql, Type};

use super::pool::Pooled;

pub type Param<'a> = &'a (dyn ToSql + Sync);

#[derive(Debug)]
pub enum Error {
    /// `query_row` found no row, as `rusqlite::Error::QueryReturnedNoRows`.
    QueryReturnedNoRows,
    /// A statement failed earlier in this transaction. Postgres had already aborted it, so the
    /// commit rolled back instead and nothing was saved.
    TransactionAborted,
    /// A column could not be read as the requested Rust type.
    Conversion(String),
    Postgres(postgres::Error),
    Pool(String),
}

impl Error {
    pub fn code(&self) -> Option<&SqlState> {
        match self {
            Error::Postgres(error) => error.code(),
            _ => None,
        }
    }

    pub fn is_unique_violation(&self) -> bool {
        self.code() == Some(&SqlState::UNIQUE_VIOLATION)
    }
}

/// Errors end up in HTTP responses through `to_string()`. A Postgres error's DETAIL can quote row
/// values, so the message here keeps only the SQLSTATE and the server's one-line message.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::QueryReturnedNoRows => f.write_str("Query returned no rows"),
            Error::TransactionAborted => {
                f.write_str("database transaction aborted by an earlier failed statement; nothing was saved")
            }
            Error::Pool(message) => write!(f, "database unavailable: {message}"),
            Error::Conversion(message) => write!(f, "database value: {message}"),
            Error::Postgres(error) => match error.as_db_error() {
                Some(db) => write!(f, "database error {}: {}", db.code().code(), db.message()),
                None if error.is_closed() => f.write_str("database connection closed"),
                // Client-side failures (a Rust type that does not fit the column, a lost connection)
                // describe types and transport, never row values.
                None => write!(f, "database error: {error}"),
            },
        }
    }
}

impl std::error::Error for Error {}

impl From<postgres::Error> for Error {
    fn from(error: postgres::Error) -> Self {
        Error::Postgres(error)
    }
}

/// `.optional()` on a single-row result, as `rusqlite::OptionalExtension`.
pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>, Error>;
}

impl<T> OptionalExtension<T> for Result<T, Error> {
    fn optional(self) -> Result<Option<T>, Error> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// Bind parameters in the forms the `db` modules already write: `params![..]`, `[]`, `[a, b]`,
/// and `&[..]`. A variable-length list binds as one array parameter (`= ANY($1)`).
pub trait Params {
    fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R;
}

impl Params for &[Param<'_>] {
    fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R {
        f(self)
    }
}

impl Params for [Param<'_>; 0] {
    fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R {
        f(&[])
    }
}

macro_rules! impl_params_for_arrays {
    ($($n:literal)*) => {$(
        impl<T: ToSql + Sync> Params for [T; $n] {
            fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R {
                let refs: Vec<Param<'_>> = self.iter().map(|value| value as Param<'_>).collect();
                f(&refs)
            }
        }
        impl<T: ToSql + Sync> Params for &[T; $n] {
            fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R {
                let refs: Vec<Param<'_>> = self.iter().map(|value| value as Param<'_>).collect();
                f(&refs)
            }
        }
    )*};
}

impl_params_for_arrays!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

/// `params![a, b]`, as rusqlite's macro: a slice of borrowed parameters.
#[macro_export]
macro_rules! pg_params {
    () => {
        &[] as &[&(dyn ::postgres::types::ToSql + Sync)]
    };
    ($($param:expr),+ $(,)?) => {
        &[$(&$param as &(dyn ::postgres::types::ToSql + Sync)),+] as &[&(dyn ::postgres::types::ToSql + Sync)]
    };
}

pub struct Row(postgres::Row);

impl Row {
    pub fn get<I: ColumnIndex, T: FromColumn>(&self, index: I) -> Result<T, Error> {
        let index = index.position(&self.0)?;
        T::from_column(&self.0, index)
    }
}

/// A column by position or by name, as rusqlite's `row.get` takes.
pub trait ColumnIndex {
    fn position(&self, row: &postgres::Row) -> Result<usize, Error>;
}

impl ColumnIndex for usize {
    fn position(&self, row: &postgres::Row) -> Result<usize, Error> {
        if *self < row.len() {
            Ok(*self)
        } else {
            Err(Error::Conversion(format!("no column {self}")))
        }
    }
}

impl ColumnIndex for &str {
    fn position(&self, row: &postgres::Row) -> Result<usize, Error> {
        row.columns()
            .iter()
            .position(|column| column.name() == *self)
            .ok_or_else(|| Error::Conversion(format!("no column {self}")))
    }
}

/// The Rust types a column is read as. Reads are as lenient as rusqlite's were, which the `db`
/// modules count on: an integer of any width or a boolean reads as `i64`, an integer reads as
/// `bool` (non-zero is true). Postgres types expressions strictly (`CASE … THEN 1` is `int4`,
/// `EXISTS` is `bool`), so without this every such column would fail at run time.
pub trait FromColumn: Sized {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error>;
}

fn typed<'a, T: FromSql<'a>>(row: &'a postgres::Row, index: usize) -> Result<Option<T>, Error> {
    row.try_get::<_, Option<T>>(index).map_err(Error::from)
}

fn integer(row: &postgres::Row, index: usize) -> Result<Option<i64>, Error> {
    match *row.columns()[index].type_() {
        Type::INT8 => typed::<i64>(row, index),
        Type::INT4 => Ok(typed::<i32>(row, index)?.map(i64::from)),
        Type::INT2 => Ok(typed::<i16>(row, index)?.map(i64::from)),
        Type::BOOL => Ok(typed::<bool>(row, index)?.map(i64::from)),
        ref other => Err(Error::Conversion(format!(
            "column {index} is {other}, not an integer (cast it to bigint in the query)"
        ))),
    }
}

fn required<T>(value: Option<T>, index: usize) -> Result<T, Error> {
    value.ok_or_else(|| Error::Conversion(format!("column {index} is NULL")))
}

impl FromColumn for Option<i64> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        integer(row, index)
    }
}

impl FromColumn for i64 {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(integer(row, index)?, index)
    }
}

impl FromColumn for Option<usize> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        integer(row, index)?
            .map(|value| usize::try_from(value).map_err(|_| Error::Conversion(format!("column {index} is negative"))))
            .transpose()
    }
}

impl FromColumn for usize {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(Option::<usize>::from_column(row, index)?, index)
    }
}

impl FromColumn for Option<u64> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        integer(row, index)?
            .map(|value| u64::try_from(value).map_err(|_| Error::Conversion(format!("column {index} is negative"))))
            .transpose()
    }
}

impl FromColumn for u64 {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(Option::<u64>::from_column(row, index)?, index)
    }
}

impl FromColumn for Option<bool> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        match *row.columns()[index].type_() {
            Type::BOOL => typed::<bool>(row, index),
            _ => Ok(integer(row, index)?.map(|value| value != 0)),
        }
    }
}

impl FromColumn for bool {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(Option::<bool>::from_column(row, index)?, index)
    }
}

impl FromColumn for Option<f64> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        match *row.columns()[index].type_() {
            Type::FLOAT8 => typed::<f64>(row, index),
            Type::FLOAT4 => Ok(typed::<f32>(row, index)?.map(f64::from)),
            _ => Ok(integer(row, index)?.map(|value| value as f64)),
        }
    }
}

impl FromColumn for f64 {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(Option::<f64>::from_column(row, index)?, index)
    }
}

impl FromColumn for Option<String> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        typed::<String>(row, index)
    }
}

impl FromColumn for String {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(typed::<String>(row, index)?, index)
    }
}

impl FromColumn for Option<Vec<u8>> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        typed::<Vec<u8>>(row, index)
    }
}

impl FromColumn for Vec<u8> {
    fn from_column(row: &postgres::Row, index: usize) -> Result<Self, Error> {
        required(typed::<Vec<u8>>(row, index)?, index)
    }
}

pub struct Conn {
    client: RefCell<Pooled>,
    depth: Cell<u32>,
    changes: Cell<u64>,
    /// The transaction depth at which a statement failed. Postgres aborts the transaction on any
    /// failed statement (SQLite does not); only rolling back that level or an outer one recovers.
    aborted_at: Cell<Option<u32>>,
}

impl Conn {
    pub(crate) fn new(client: Pooled) -> Self {
        Self {
            client: RefCell::new(client),
            depth: Cell::new(0),
            changes: Cell::new(0),
            aborted_at: Cell::new(None),
        }
    }

    /// Every statement goes through here, so a failure inside a transaction is remembered.
    fn run<T>(&self, f: impl FnOnce(&mut postgres::Client) -> Result<T, postgres::Error>) -> Result<T, Error> {
        let result = f(&mut self.client.borrow_mut());
        if result.is_err() && self.depth.get() > 0 && self.aborted_at.get().is_none() {
            self.aborted_at.set(Some(self.depth.get()));
        }
        result.map_err(Error::from)
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize, Error> {
        let changed = params.with_params(|params| self.run(|client| client.execute(sql, params)))?;
        self.changes.set(changed);
        Ok(changed as usize)
    }

    /// Runs statements without parameters, separated by `;`.
    pub fn execute_batch(&self, sql: &str) -> Result<(), Error> {
        self.run(|client| client.batch_execute(sql))
    }

    pub fn query_row<P, T, F>(&self, sql: &str, params: P, f: F) -> Result<T, Error>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T, Error>,
    {
        let row = params.with_params(|params| self.run(|client| client.query_opt(sql, params)))?;
        match row {
            Some(row) => f(&Row(row)),
            None => Err(Error::QueryReturnedNoRows),
        }
    }

    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>, Error> {
        let statement = self.run(|client| client.prepare(sql))?;
        Ok(Statement { conn: self, statement })
    }

    /// Rows changed by the last `execute`, as `rusqlite::Connection::changes`.
    pub fn changes(&self) -> u64 {
        self.changes.get()
    }

    /// No transaction is open on this connection.
    pub fn is_autocommit(&self) -> bool {
        self.depth.get() == 0
    }

    /// Opens a transaction, or a savepoint inside the one already open. Named after the rusqlite
    /// method it replaces: it borrows `&self`, so the `Database` methods called inside it run in it.
    pub fn unchecked_transaction(&self) -> Result<Transaction<'_>, Error> {
        let depth = self.depth.get();
        if depth == 0 {
            self.execute_batch("BEGIN")?;
        } else {
            self.execute_batch(&format!("SAVEPOINT sp{depth}"))?;
        }
        self.depth.set(depth + 1);
        Ok(Transaction {
            conn: self,
            depth,
            done: false,
        })
    }

    /// The id the last insert on this session drew from an identity column, as rusqlite's
    /// `last_insert_rowid`. Tests only: production code reads ids with `RETURNING`.
    #[cfg(test)]
    pub fn last_insert_rowid(&self) -> i64 {
        self.query_row("SELECT lastval()", [], |row| row.get(0))
            .expect("lastval")
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // A transaction left open would go back to the pool with it; end it here.
        if self.depth.get() > 0 {
            let _ = self.client.get_mut().batch_execute("ROLLBACK");
        }
    }
}

pub struct Transaction<'c> {
    conn: &'c Conn,
    depth: u32,
    done: bool,
}

impl Transaction<'_> {
    pub fn commit(mut self) -> Result<(), Error> {
        self.done = true;
        self.conn.depth.set(self.depth);
        if self.depth == 0 {
            // On an aborted transaction Postgres answers COMMIT with a rollback and no error.
            if self.conn.aborted_at.take().is_some() {
                self.conn.execute_batch("ROLLBACK")?;
                return Err(Error::TransactionAborted);
            }
            self.conn.execute_batch("COMMIT")
        } else {
            self.conn.execute_batch(&format!("RELEASE SAVEPOINT sp{}", self.depth))
        }
    }

    fn finish_rollback(&self) -> Result<(), Error> {
        self.conn.depth.set(self.depth);
        if self.depth == 0 {
            self.conn.execute_batch("ROLLBACK")?;
        } else {
            self.conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT sp{0}; RELEASE SAVEPOINT sp{0}",
                self.depth
            ))?;
        }
        // Rolling back this level undoes a failure inside it, not one in an enclosing level.
        if self.conn.aborted_at.get().is_some_and(|level| level > self.depth) {
            self.conn.aborted_at.set(None);
        }
        Ok(())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.finish_rollback();
        }
    }
}

impl std::ops::Deref for Transaction<'_> {
    type Target = Conn;

    fn deref(&self) -> &Conn {
        self.conn
    }
}

pub struct Statement<'c> {
    conn: &'c Conn,
    statement: postgres::Statement,
}

impl Statement<'_> {
    pub fn query<P: Params>(&mut self, params: P) -> Result<Rows, Error> {
        let rows = params.with_params(|params| self.conn.run(|client| client.query(&self.statement, params)))?;
        Ok(Rows {
            rows: rows.into_iter().map(Row).collect(),
            next: 0,
        })
    }

    pub fn query_map<P, T, F>(&mut self, params: P, f: F) -> Result<MappedRows<F>, Error>
    where
        P: Params,
        F: FnMut(&Row) -> Result<T, Error>,
    {
        let rows = self.query(params)?;
        Ok(MappedRows {
            rows: rows.rows.into_iter(),
            f,
        })
    }

    pub fn query_row<P, T, F>(&mut self, params: P, f: F) -> Result<T, Error>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T, Error>,
    {
        let row = params.with_params(|params| self.conn.run(|client| client.query_opt(&self.statement, params)))?;
        match row {
            Some(row) => f(&Row(row)),
            None => Err(Error::QueryReturnedNoRows),
        }
    }
}

/// Rows already fetched; `next()` walks them as `rusqlite::Rows::next` does.
pub struct Rows {
    rows: Vec<Row>,
    next: usize,
}

impl Rows {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<&Row>, Error> {
        let row = self.rows.get(self.next);
        self.next += 1;
        Ok(row)
    }
}

pub struct MappedRows<F> {
    rows: std::vec::IntoIter<Row>,
    f: F,
}

impl<T, F> Iterator for MappedRows<F>
where
    F: FnMut(&Row) -> Result<T, Error>,
{
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|row| (self.f)(&row))
    }
}
