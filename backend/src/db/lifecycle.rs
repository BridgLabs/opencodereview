use super::*;

impl Database {
    /// Setting defaults the schema does not seed. Runs once at startup, after migrations; every
    /// write keeps a value already there, so instances starting together agree.
    pub(super) fn ensure_defaults(&self) -> Result<(), String> {
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
        self.ensure_setting_default("review_archive_retention_days", "0")?;
        self.ensure_setting_default("installation_id", &generate_installation_id()?)?;
        Ok(())
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
