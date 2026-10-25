/// Ordered schema migrations; `PRAGMA user_version` records how many have run.
/// Append new migrations to the end and never edit an applied one.
pub(crate) const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("gemini_accounts.sql"),
    include_str!("more_providers.sql"),
];
