//! Postgres access: pools, a connection with the rusqlite-shaped API the `db` modules use, and
//! migrations.
//!
//! Nothing outside this module uses it yet: the `db` modules move over to it next, all at once.
#![allow(dead_code, unused_imports)]

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
