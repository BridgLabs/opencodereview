//! Postgres access: pools, a connection with the rusqlite-shaped API the `db` modules use, and
//! migrations.

mod conn;
mod migrate;
mod pool;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) use crate::pg_params as params;
pub use conn::{Conn, Error, OptionalExtension, Row};
pub use migrate::migrate;
pub use pool::{DbPool, DbSettings};
