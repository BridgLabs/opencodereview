use super::*;

/// The database the server works against. Cheap to clone and safe to share between threads; open a
/// `Database` from it for each unit of work and drop it before slow work (git, GitHub, models).
#[derive(Clone, Debug)]
pub struct Db {
    path: PathBuf,
}

impl Db {
    pub fn sqlite(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn open(&self) -> Result<Database, String> {
        Database::open_at(self.path.clone())
    }

    /// A connection that cannot write.
    pub fn open_read(&self) -> Result<Database, String> {
        Database::open_read_at(self.path.clone())
    }

    /// Names this database, for coalescing background work per database and for health output.
    pub fn id(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Database {
    /// The database this connection came from, for handing work to another thread.
    pub fn handle(&self) -> Db {
        Db::sqlite(self.path.clone())
    }
}
