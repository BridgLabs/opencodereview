/// The SQLite schema's migrations, in order; `PRAGMA user_version` recorded how many had run. The
/// server no longer applies them (its schema is Postgres, see `pg/`), but the SQLite import brings
/// an older database up to the last version before copying it.
#[allow(dead_code)] // Read by the SQLite import, which lands after the switch.
pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("gemini_accounts.sql"),
    include_str!("more_providers.sql"),
];
