/// The SQLite schema's migrations, in order; `PRAGMA user_version` recorded how many had run. The
/// server no longer applies them (its schema is Postgres, see `pg/`), but `import_sqlite` brings an
/// older database up to the last version before copying it.
pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("gemini_accounts.sql"),
    include_str!("more_providers.sql"),
];
