use super::*;

impl Database {
    pub fn open_at(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(&path).map_err(|error| error.to_string())?;
        Self::configure_connection(&conn, true)?;
        let db = Self { conn, path };
        db.init()?;
        Ok(db)
    }

    pub fn open_read_at(path: PathBuf) -> Result<Self, String> {
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
            .map_err(|error| error.to_string())?;
        Self::configure_connection(&conn, false)?;
        Ok(Self { conn, path })
    }

    pub(super) fn configure_connection(conn: &Connection, writable: bool) -> Result<(), String> {
        conn.busy_timeout(Duration::from_secs(30))
            .map_err(|error| error.to_string())?;
        if writable {
            let journal_mode: String = conn
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if !journal_mode.eq_ignore_ascii_case("wal") {
                conn.pragma_update(None, "journal_mode", "WAL")
                    .map_err(|error| error.to_string())?;
            }
            conn.pragma_update(None, "synchronous", "NORMAL")
                .map_err(|error| error.to_string())?;
        }
        conn.pragma_update(None, "busy_timeout", 30000)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn path(&self) -> String {
        self.path.to_string_lossy().to_string()
    }

    pub(super) fn init(&self) -> Result<(), String> {
        self.migrate()?;
        self.ensure_setting_default("theme", "system")?;
        self.ensure_setting_default("font_size", "14")?;
        self.ensure_setting_default("sidebar_collapsed", "false")?;
        self.ensure_setting_default("mission_workspace_configured", "false")?;
        self.ensure_setting_default("mission_index_done", "false")?;
        self.ensure_setting_default("poll_interval_seconds", &DEFAULT_POLL_INTERVAL_SECONDS.to_string())?;
        self.ensure_setting_default("auto_skip_keywords", "[]")?;
        match self.setting("index_strategy")? {
            Some(value) if value == "graph" => {}
            Some(_) => self.save_setting_value("index_strategy", "graph")?,
            None => self.ensure_setting_default("index_strategy", "graph")?,
        }
        self.ensure_setting_default("review_result_mode", "result_comment")?;
        self.ensure_setting_default("review_max_files", "100")?;
        self.ensure_setting_default("review_archive_retention_days", "1")?;
        if self.setting("installation_id")?.is_none() {
            self.save_setting_value("installation_id", &generate_installation_id()?)?;
        }
        Ok(())
    }

    pub(super) fn migrate(&self) -> Result<(), String> {
        let current: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if current > MIGRATIONS.len() as i64 {
            return Err(format!(
                "database schema {current} is newer than this application supports ({})",
                MIGRATIONS.len()
            ));
        }
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            let target = (index as i64) + 1;
            if current >= target {
                continue;
            }
            let batch = format!("BEGIN;\n{}\nPRAGMA user_version = {};\nCOMMIT;", migration, target);
            self.conn
                .execute_batch(&batch)
                .map_err(|error| format!("migration {} failed: {}", target, error))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn user_version(&self) -> i64 {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version")
    }
}

pub(crate) fn generate_installation_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    let mut file = std::fs::File::open("/dev/urandom").map_err(|error| error.to_string())?;
    use std::io::Read as _;
    file.read_exact(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex_encode(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
