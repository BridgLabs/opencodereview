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
use postgres::row::RowIndex;
use postgres::types::{FromSql, ToSql};

use super::pool::Pooled;

pub type Param<'a> = &'a (dyn ToSql + Sync);

#[derive(Debug)]
pub enum Error {
    /// `query_row` found no row, as `rusqlite::Error::QueryReturnedNoRows`.
    QueryReturnedNoRows,
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

    /// Serialization failures and deadlocks: the transaction lost a race and may be run again.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.code(),
            Some(code) if *code == SqlState::T_R_SERIALIZATION_FAILURE || *code == SqlState::T_R_DEADLOCK_DETECTED
        )
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
            Error::Pool(message) => write!(f, "database unavailable: {message}"),
            Error::Postgres(error) => match error.as_db_error() {
                Some(db) => write!(f, "database error {}: {}", db.code().code(), db.message()),
                None if error.is_closed() => f.write_str("database connection closed"),
                None => f.write_str("database error"),
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
/// `&[..]` and `params_from_iter(..)`.
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

pub struct ParamsFromIter<I>(I);

pub fn params_from_iter<I>(iter: I) -> ParamsFromIter<I>
where
    I: IntoIterator,
    I::Item: ToSql + Sync,
{
    ParamsFromIter(iter)
}

impl<I> Params for ParamsFromIter<I>
where
    I: IntoIterator,
    I::Item: ToSql + Sync,
{
    fn with_params<R>(self, f: impl FnOnce(&[Param<'_>]) -> R) -> R {
        let values: Vec<I::Item> = self.0.into_iter().collect();
        let refs: Vec<Param<'_>> = values.iter().map(|value| value as Param<'_>).collect();
        f(&refs)
    }
}

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
    pub fn get<I, T>(&self, index: I) -> Result<T, Error>
    where
        I: RowIndex + fmt::Display,
        T: for<'a> FromSql<'a>,
    {
        self.0.try_get(index).map_err(Error::from)
    }
}

pub struct Conn {
    client: RefCell<Pooled>,
    depth: Cell<u32>,
    changes: Cell<u64>,
}

impl Conn {
    pub(crate) fn new(client: Pooled) -> Self {
        Self {
            client: RefCell::new(client),
            depth: Cell::new(0),
            changes: Cell::new(0),
        }
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize, Error> {
        let changed = params.with_params(|params| self.client.borrow_mut().execute(sql, params))?;
        self.changes.set(changed);
        Ok(changed as usize)
    }

    /// Runs statements without parameters, separated by `;`.
    pub fn execute_batch(&self, sql: &str) -> Result<(), Error> {
        self.client.borrow_mut().batch_execute(sql).map_err(Error::from)
    }

    pub fn query_row<P, T, F>(&self, sql: &str, params: P, f: F) -> Result<T, Error>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T, Error>,
    {
        let row = params.with_params(|params| self.client.borrow_mut().query_opt(sql, params))?;
        match row {
            Some(row) => f(&Row(row)),
            None => Err(Error::QueryReturnedNoRows),
        }
    }

    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>, Error> {
        let statement = self.client.borrow_mut().prepare(sql)?;
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

    /// Runs `body` in a transaction, again from the start when it loses a serialization race or a
    /// deadlock. Only for the outermost transaction: a savepoint cannot be retried on its own.
    pub fn transaction_with_retry<T>(
        &self,
        mut body: impl FnMut(&Transaction<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        debug_assert!(self.is_autocommit(), "retry needs the outermost transaction");
        let mut attempt = 0;
        loop {
            let result = self.unchecked_transaction().and_then(|tx| {
                let value = body(&tx)?;
                tx.commit()?;
                Ok(value)
            });
            match result {
                Err(error) if error.is_retryable() && attempt < 2 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20 << attempt));
                }
                result => return result,
            }
        }
    }

    /// Low-level access for code that needs the native client (COPY, LISTEN). It must not be held
    /// across another call on this connection.
    pub fn with_client<T>(&self, f: impl FnOnce(&mut postgres::Client) -> T) -> T {
        f(&mut self.client.borrow_mut())
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
            self.conn.execute_batch("COMMIT")
        } else {
            self.conn.execute_batch(&format!("RELEASE SAVEPOINT sp{}", self.depth))
        }
    }

    pub fn rollback(mut self) -> Result<(), Error> {
        self.done = true;
        self.finish_rollback()
    }

    fn finish_rollback(&self) -> Result<(), Error> {
        self.conn.depth.set(self.depth);
        if self.depth == 0 {
            self.conn.execute_batch("ROLLBACK")
        } else {
            self.conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT sp{0}; RELEASE SAVEPOINT sp{0}",
                self.depth
            ))
        }
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
    pub fn execute<P: Params>(&mut self, params: P) -> Result<usize, Error> {
        let changed = params.with_params(|params| self.conn.client.borrow_mut().execute(&self.statement, params))?;
        self.conn.changes.set(changed);
        Ok(changed as usize)
    }

    pub fn query<P: Params>(&mut self, params: P) -> Result<Rows, Error> {
        let rows = params.with_params(|params| self.conn.client.borrow_mut().query(&self.statement, params))?;
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
        let row = params.with_params(|params| self.conn.client.borrow_mut().query_opt(&self.statement, params))?;
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
