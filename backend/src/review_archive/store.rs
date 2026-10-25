use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use std::sync::Arc;

const UPLOAD_RESERVATION: u64 = MAX_BLOB_BYTES + MAX_MANIFEST_BYTES + 128 * 1024 * 1024;

pub const STORE_QUOTA: u64 = 4 * 1024 * 1024 * 1024;
struct Upload {
    descriptor: Descriptor,
    raw: Vec<u8>,
    manifest: Manifest,
}
pub struct ArchiveStore {
    root: PathBuf,
    conn: Connection,
    cache: RefCell<BTreeMap<String, Arc<Upload>>>,
}

impl ArchiveStore {
    pub fn open(root: &Path) -> Result<Self, Error> {
        if fs::symlink_metadata(root).is_ok_and(|meta| !meta.is_dir()) {
            return Err(Error::Path);
        }
        fs::create_dir_all(root).map_err(|_| Error::Preparation)?;
        let root = root.canonicalize().map_err(|_| Error::Preparation)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Preparation)?;
        }
        for area in ["pending", "sealed"] {
            let path = root.join(area);
            if fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_dir()) {
                return Err(Error::Path);
            }
            fs::create_dir_all(path).map_err(|_| Error::Preparation)?;
        }
        let database = root.join("index.sqlite");
        if fs::symlink_metadata(&database).is_ok_and(|meta| !meta.is_file()) {
            return Err(Error::Path);
        }
        let conn = Connection::open(database).map_err(|_| Error::Preparation)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| Error::Preparation)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS artifacts(id TEXT PRIMARY KEY,descriptor TEXT NOT NULL,bytes INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS uploads(owner TEXT PRIMARY KEY,id TEXT NOT NULL,descriptor TEXT NOT NULL,manifest BLOB NOT NULL,deadline INTEGER NOT NULL,reservation INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS receipts(owner TEXT PRIMARY KEY,id TEXT NOT NULL); CREATE TABLE IF NOT EXISTS deletions(id TEXT PRIMARY KEY,bytes INTEGER NOT NULL,phase TEXT NOT NULL,created_at INTEGER NOT NULL);").map_err(|_| Error::Preparation)?;
        let mut store = Self {
            root,
            conn,
            cache: RefCell::new(BTreeMap::new()),
        };
        store.recover_deletions()?;
        store.recover_unindexed()?;
        Ok(store)
    }

    fn recover_deletions(&mut self) -> Result<(), Error> {
        let ids: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id FROM deletions")
                .map_err(|_| Error::Preparation)?;
            let rows = stmt.query_map([], |row| row.get(0)).map_err(|_| Error::Preparation)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|_| Error::Preparation)?
        };
        for id in ids {
            let tx = self
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| Error::Preparation)?;
            Self::finish_deletion(&tx, &self.root, &id)?;
            tx.commit().map_err(|_| Error::Preparation)?;
        }
        Ok(())
    }

    // Call only while holding the index write transaction. A stale recovery/deleter
    // must not unlink a replacement deposited after another writer finished deletion.
    fn finish_deletion(tx: &Connection, root: &Path, id: &str) -> Result<(), Error> {
        let deleting: bool = tx
            .query_row("SELECT EXISTS(SELECT 1 FROM deletions WHERE id=?1)", [id], |row| {
                row.get(0)
            })
            .map_err(|_| Error::Preparation)?;
        if !deleting {
            return Ok(());
        }
        let directory = root.join("sealed");
        let path = directory.join(format!("{id}.archive"));
        if path.exists() {
            if !fs::symlink_metadata(&path).map_err(|_| Error::Preparation)?.is_file() {
                return Err(Error::Path);
            }
            fs::remove_file(&path).map_err(|_| Error::Preparation)?;
        }
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Preparation)?;
        tx.execute("DELETE FROM receipts WHERE id=?1", [id])
            .map_err(|_| Error::Preparation)?;
        tx.execute("DELETE FROM artifacts WHERE id=?1", [id])
            .map_err(|_| Error::Preparation)?;
        tx.execute("DELETE FROM deletions WHERE id=?1", [id])
            .map_err(|_| Error::Preparation)?;
        Ok(())
    }

    pub fn open_read(root: &Path) -> Result<Self, Error> {
        let meta = fs::symlink_metadata(root).map_err(|_| Error::Path)?;
        if !meta.is_dir() {
            return Err(Error::Path);
        }
        let root = root.canonicalize().map_err(|_| Error::Preparation)?;
        let database = root.join("index.sqlite");
        let db_meta = fs::symlink_metadata(&database).map_err(|_| Error::Path)?;
        if !db_meta.is_file() {
            return Err(Error::Path);
        }
        let conn = Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::Preparation)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| Error::Preparation)?;
        Ok(Self {
            root,
            conn,
            cache: RefCell::new(BTreeMap::new()),
        })
    }

    fn recover_unindexed(&mut self) -> Result<(), Error> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        let directory = self.root.join("sealed");
        for entry in fs::read_dir(&directory).map_err(|_| Error::Preparation)? {
            let entry = entry.map_err(|_| Error::Preparation)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let staging = name.strip_prefix(".publish-").filter(|id| oid(id, 64));
            let archive = name.strip_suffix(".archive").filter(|id| oid(id, 64));
            let Some(id) = staging.or(archive) else {
                continue;
            };
            if !fs::symlink_metadata(entry.path())
                .map_err(|_| Error::Preparation)?
                .is_file()
            {
                return Err(Error::Path);
            }
            if staging.is_some() {
                fs::remove_file(entry.path()).map_err(|_| Error::Preparation)?;
                continue;
            }
            let referenced: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1 UNION ALL SELECT 1 FROM receipts WHERE id=?1 UNION ALL SELECT 1 FROM uploads WHERE id=?1 UNION ALL SELECT 1 FROM deletions WHERE id=?1)", [id], |row| row.get(0)).map_err(|_| Error::Preparation)?;
            if !referenced {
                fs::remove_file(entry.path()).map_err(|_| Error::Preparation)?;
            }
        }
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Preparation)?;
        tx.commit().map_err(|_| Error::Preparation)
    }

    fn reserve(conn: &Connection, root: &Path, bytes: u64) -> Result<(), Error> {
        let recorded: u64 = conn.query_row("SELECT COALESCE((SELECT SUM(bytes) FROM artifacts),0)+COALESCE((SELECT SUM(reservation) FROM uploads),0)", [], |row| row.get(0)).map_err(|_| Error::Preparation)?;
        let mut pending = vec![root.to_path_buf()];
        let mut actual = 0u64;
        while let Some(path) = pending.pop() {
            let meta = fs::symlink_metadata(&path).map_err(|_| Error::Preparation)?;
            if meta.is_dir() {
                for entry in fs::read_dir(path).map_err(|_| Error::Preparation)? {
                    pending.push(entry.map_err(|_| Error::Preparation)?.path());
                }
            } else if !meta.is_file() {
                return Err(Error::Path);
            }
            #[cfg(unix)]
            let size = {
                use std::os::unix::fs::MetadataExt;
                meta.blocks().saturating_mul(512)
            };
            #[cfg(not(unix))]
            let size = meta.len().saturating_add(4096);
            actual = actual.checked_add(size).ok_or(Error::Budget)?;
            if actual > STORE_QUOTA {
                return Err(Error::Budget);
            }
        }
        if recorded
            .max(actual)
            .checked_add(bytes)
            .is_none_or(|total| total > STORE_QUOTA)
        {
            return Err(Error::Budget);
        }
        Ok(())
    }

    pub fn load(&self, descriptor: &Descriptor) -> Result<Archive, Error> {
        descriptor.validate()?;
        let raw: String = self
            .conn
            .query_row(
                "SELECT descriptor FROM artifacts WHERE id=?1",
                [&descriptor.id],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        if serde_json::from_str::<Descriptor>(&raw).map_err(|_| Error::Integrity)? != *descriptor {
            return Err(Error::Integrity);
        }
        Archive::read(
            &self.root.join("sealed").join(format!("{}.archive", descriptor.id)),
            descriptor,
        )
    }

    pub fn link(&self, owner: &str, descriptor: &Descriptor) -> Result<(), Error> {
        if owner.is_empty() || owner.len() > 256 {
            return Err(Error::Contract);
        }
        let tx = rusqlite::Transaction::new_unchecked(&self.conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        let is_deleting: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deletions WHERE id=?1)",
                [&descriptor.id],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        if is_deleting {
            return Err(Error::Integrity);
        }
        self.load(descriptor)?;
        let existing: Option<String> = self
            .conn
            .query_row("SELECT id FROM receipts WHERE owner=?1", [owner], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        if existing.as_ref().is_some_and(|id| *id != descriptor.id) {
            return Err(Error::Integrity);
        }
        self.conn
            .execute(
                "INSERT OR IGNORE INTO receipts(owner,id) VALUES(?1,?2)",
                params![owner, descriptor.id],
            )
            .map_err(|_| Error::Preparation)?;
        tx.commit().map_err(|_| Error::Preparation)
    }

    pub fn load_linked(&self, owner: &str) -> Result<Option<Archive>, Error> {
        let existing: Option<String> = self
            .conn
            .query_row("SELECT id FROM receipts WHERE owner=?1", [owner], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        let Some(id) = existing else {
            return Ok(None);
        };
        let raw: Option<String> = self
            .conn
            .query_row("SELECT descriptor FROM artifacts WHERE id=?1", [&id], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        let Some(raw) = raw else {
            return Err(Error::Integrity);
        };
        let descriptor: Descriptor = serde_json::from_str(&raw).map_err(|_| Error::Integrity)?;
        descriptor.validate()?;
        if descriptor.id != id {
            return Err(Error::Integrity);
        }
        Ok(Some(Archive::read(
            &self.root.join("sealed").join(format!("{}.archive", descriptor.id)),
            &descriptor,
        )?))
    }

    pub fn deposit(&mut self, archive: &Archive) -> Result<(), Error> {
        let descriptor = archive.descriptor();
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        Self::finish_deletion(&tx, &self.root, &descriptor.id)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1)",
                [&descriptor.id],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        let directory = self.root.join("sealed");
        let path = directory.join(format!("{}.archive", descriptor.id));
        if exists {
            Archive::read(&path, &descriptor)?;
            return Ok(());
        }
        for entry in fs::read_dir(&directory).map_err(|_| Error::Preparation)? {
            let entry = entry.map_err(|_| Error::Preparation)?;
            if entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(".publish-"))
                .is_some_and(|id| oid(id, 64))
            {
                if !fs::symlink_metadata(entry.path())
                    .map_err(|_| Error::Preparation)?
                    .is_file()
                {
                    return Err(Error::Path);
                }
                fs::remove_file(entry.path()).map_err(|_| Error::Preparation)?;
            }
        }
        let bytes = descriptor.blob_bytes + descriptor.manifest_bytes + 4096;
        Self::reserve(&tx, &self.root, if path.exists() { 0 } else { bytes })?;
        if path.exists() {
            Archive::read(&path, &descriptor)?;
        } else {
            let staging = directory.join(format!(".publish-{}", descriptor.id));
            let result = (|| {
                archive.write_new(&staging)?;
                fs::rename(&staging, &path).map_err(|_| Error::Preparation)?;
                File::open(&directory)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| Error::Preparation)
            })();
            if result.is_err() {
                let _ = fs::remove_file(staging);
            }
            result?;
        }
        tx.execute(
            "INSERT INTO artifacts(id,descriptor,bytes) VALUES(?1,?2,?3)",
            params![
                descriptor.id,
                serde_json::to_string(&descriptor).map_err(|_| Error::Contract)?,
                bytes
            ],
        )
        .map_err(|_| Error::Preparation)?;
        tx.commit().map_err(|_| Error::Preparation)
    }

    pub fn begin(&mut self, owner: &str, descriptor: &Descriptor, raw: &[u8], deadline: i64) -> Result<bool, Error> {
        if !oid(owner, 64) || deadline <= crate::sandbox::now() {
            return Err(Error::Contract);
        }
        let manifest = transfer_manifest(descriptor, raw)?;
        let receipt: Option<String> = self
            .conn
            .query_row("SELECT id FROM receipts WHERE owner=?1", [owner], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        if receipt.as_ref().is_some_and(|id| *id != descriptor.id) {
            return Err(Error::Integrity);
        }
        self.expire()?;
        if self.load(descriptor).is_ok() {
            self.link(owner, descriptor)?;
            return Ok(true);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        let existing: Option<(String, Vec<u8>)> = tx
            .query_row("SELECT id,manifest FROM uploads WHERE owner=?1", [owner], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(|_| Error::Preparation)?;
        if let Some((id, manifest)) = existing {
            if id != descriptor.id || manifest != raw {
                return Err(Error::Integrity);
            }
            tx.execute(
                "UPDATE uploads SET deadline=?2 WHERE owner=?1",
                params![owner, deadline],
            )
            .map_err(|_| Error::Preparation)?;
        } else {
            Self::reserve(&tx, &self.root, UPLOAD_RESERVATION)?;
            tx.execute(
                "INSERT INTO uploads(owner,id,descriptor,manifest,deadline,reservation) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    owner,
                    descriptor.id,
                    serde_json::to_string(descriptor).map_err(|_| Error::Contract)?,
                    raw,
                    deadline,
                    UPLOAD_RESERVATION
                ],
            )
            .map_err(|_| Error::Preparation)?;
        }
        tx.commit().map_err(|_| Error::Preparation)?;
        let directory = self.root.join("pending").join(owner);
        if fs::symlink_metadata(&directory).is_ok_and(|meta| !meta.is_dir()) {
            return Err(Error::Path);
        }
        fs::create_dir_all(directory).map_err(|_| Error::Preparation)?;
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= 2 {
            cache.clear();
        }
        cache.insert(
            owner.into(),
            Arc::new(Upload {
                descriptor: descriptor.clone(),
                raw: raw.to_vec(),
                manifest,
            }),
        );
        Ok(false)
    }

    pub fn renew(&self, owner: &str, deadline: i64) -> Result<(), Error> {
        self.conn
            .execute(
                "UPDATE uploads SET deadline=?2 WHERE owner=?1 AND deadline>=?3",
                params![owner, deadline, crate::sandbox::now()],
            )
            .map_err(|_| Error::Preparation)?;
        Ok(())
    }

    fn upload(&self, owner: &str) -> Result<Arc<Upload>, Error> {
        if !oid(owner, 64) {
            return Err(Error::Contract);
        }
        let id: String = self
            .conn
            .query_row(
                "SELECT id FROM uploads WHERE owner=?1 AND deadline>=?2",
                params![owner, crate::sandbox::now()],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        if let Some(cached) = self
            .cache
            .borrow()
            .get(owner)
            .filter(|cached| cached.descriptor.id == id)
        {
            return Ok(cached.clone());
        }
        let (descriptor, raw): (String, Vec<u8>) = self
            .conn
            .query_row(
                "SELECT descriptor,manifest FROM uploads WHERE owner=?1",
                [owner],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| Error::Preparation)?;
        let descriptor: Descriptor = serde_json::from_str(&descriptor).map_err(|_| Error::Contract)?;
        let manifest = transfer_manifest(&descriptor, &raw)?;
        let upload = Arc::new(Upload {
            descriptor,
            raw,
            manifest,
        });
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= 2 {
            cache.clear();
        }
        cache.insert(owner.into(), upload.clone());
        Ok(upload)
    }

    pub fn put(&mut self, owner: &str, digest: &str, bytes: &[u8]) -> Result<(), Error> {
        let upload = self.upload(owner)?;
        validate_blob(&upload.manifest, digest, bytes)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        let directory = self.root.join("pending").join(owner);
        let path = directory.join(digest);
        if path.exists() {
            let mut stored = Vec::new();
            open_archive(&path, false)?
                .take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut stored)
                .map_err(|_| Error::Preparation)?;
            if stored != bytes {
                return Err(Error::Integrity);
            }
        } else {
            let partial = directory.join(format!(".part-{digest}"));
            if partial.exists() {
                if !fs::symlink_metadata(&partial)
                    .map_err(|_| Error::Preparation)?
                    .is_file()
                {
                    return Err(Error::Path);
                }
                fs::remove_file(&partial).map_err(|_| Error::Preparation)?;
            }
            let mut file = open_archive(&partial, true)?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| Error::Preparation)?;
            fs::rename(partial, path).map_err(|_| Error::Preparation)?;
        }
        tx.commit().map_err(|_| Error::Preparation)
    }

    pub fn seal(&mut self, owner: &str) -> Result<Descriptor, Error> {
        if !oid(owner, 64) {
            return Err(Error::Contract);
        }
        let receipt: Option<String> = self
            .conn
            .query_row(
                "SELECT a.descriptor FROM receipts r JOIN artifacts a ON r.id=a.id WHERE r.owner=?1",
                [owner],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| Error::Preparation)?;
        if let Some(raw) = receipt {
            let descriptor = serde_json::from_str(&raw).map_err(|_| Error::Contract)?;
            self.load(&descriptor)?;
            self.abort(owner)?;
            return Ok(descriptor);
        }
        let upload = self.upload(owner)?;
        let mut incoming = Incoming::new(upload.descriptor.clone(), upload.raw.clone())?;
        for blob in &upload.manifest.blobs {
            let mut bytes = Vec::new();
            open_archive(&self.root.join("pending").join(owner).join(&blob.hash), false)?
                .take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Preparation)?;
            incoming.push(&blob.hash, &bytes)?;
        }
        let archive = incoming.finish()?;
        self.deposit(&archive)?;
        self.link(owner, &upload.descriptor)?;
        self.abort(owner)?;
        Ok(upload.descriptor.clone())
    }

    pub fn abort(&self, owner: &str) -> Result<(), Error> {
        if !oid(owner, 64) {
            return Err(Error::Contract);
        }
        let exists: bool = self
            .conn
            .query_row("SELECT EXISTS(SELECT 1 FROM uploads WHERE owner=?1)", [owner], |row| {
                row.get(0)
            })
            .map_err(|_| Error::Preparation)?;
        if !exists {
            return Ok(());
        }
        let path = self.root.join("pending").join(owner);
        if path.exists() {
            fs::remove_dir_all(path).map_err(|_| Error::Preparation)?;
        }
        self.conn
            .execute("DELETE FROM uploads WHERE owner=?1", [owner])
            .map_err(|_| Error::Preparation)?;
        self.cache.borrow_mut().remove(owner);
        Ok(())
    }

    pub fn expire(&self) -> Result<(), Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT owner FROM uploads WHERE deadline<?1 LIMIT 1000")
            .map_err(|_| Error::Preparation)?;
        let owners = stmt
            .query_map([crate::sandbox::now()], |row| row.get::<_, String>(0))
            .map_err(|_| Error::Preparation)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Preparation)?;
        for owner in owners {
            self.abort(&owner)?;
        }
        Ok(())
    }

    pub fn deposit_and_link(&mut self, archive: &Archive, owner: &str) -> Result<(), Error> {
        if owner.is_empty() || owner.len() > 256 {
            return Err(Error::Contract);
        }
        let descriptor = archive.descriptor();
        descriptor.validate()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        Self::finish_deletion(&tx, &self.root, &descriptor.id)?;
        let existing_receipt: Option<String> = tx
            .query_row("SELECT id FROM receipts WHERE owner=?1", [owner], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        if existing_receipt.as_ref().is_some_and(|id| *id != descriptor.id) {
            return Err(Error::Integrity);
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1)",
                [&descriptor.id],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        let directory = self.root.join("sealed");
        let path = directory.join(format!("{}.archive", descriptor.id));
        if !exists {
            for entry in fs::read_dir(&directory).map_err(|_| Error::Preparation)? {
                let entry = entry.map_err(|_| Error::Preparation)?;
                if entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_prefix(".publish-"))
                    .is_some_and(|id| oid(id, 64))
                {
                    if !fs::symlink_metadata(entry.path())
                        .map_err(|_| Error::Preparation)?
                        .is_file()
                    {
                        return Err(Error::Path);
                    }
                    fs::remove_file(entry.path()).map_err(|_| Error::Preparation)?;
                }
            }
            let bytes = descriptor.blob_bytes + descriptor.manifest_bytes + 4096;
            Self::reserve(&tx, &self.root, if path.exists() { 0 } else { bytes })?;
            if path.exists() {
                Archive::read(&path, &descriptor)?;
            } else {
                let staging = directory.join(format!(".publish-{}", descriptor.id));
                let result = (|| {
                    archive.write_new(&staging)?;
                    fs::rename(&staging, &path).map_err(|_| Error::Preparation)?;
                    File::open(&directory)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| Error::Preparation)
                })();
                if result.is_err() {
                    let _ = fs::remove_file(staging);
                }
                result?;
            }
            tx.execute(
                "INSERT INTO artifacts(id,descriptor,bytes) VALUES(?1,?2,?3)",
                params![
                    descriptor.id,
                    serde_json::to_string(&descriptor).map_err(|_| Error::Contract)?,
                    bytes
                ],
            )
            .map_err(|_| Error::Preparation)?;
        } else {
            Archive::read(&path, &descriptor)?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO receipts(owner,id) VALUES(?1,?2)",
            params![owner, descriptor.id],
        )
        .map_err(|_| Error::Preparation)?;
        tx.commit().map_err(|_| Error::Preparation)
    }

    pub fn delete_artifact(&mut self, id: &str, expected_receipts: &[String]) -> Result<Option<u64>, Error> {
        if !oid(id, 64) {
            return Err(Error::Contract);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        let current: BTreeSet<String> = {
            let mut stmt = tx
                .prepare("SELECT owner FROM receipts WHERE id=?1")
                .map_err(|_| Error::Preparation)?;
            let rows = stmt.query_map([id], |row| row.get(0)).map_err(|_| Error::Preparation)?;
            rows.collect::<Result<_, _>>().map_err(|_| Error::Preparation)?
        };
        let uploading: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM uploads WHERE id=?1 AND deadline>=?2)",
                params![id, crate::sandbox::now()],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        if current.is_empty() || current != expected_receipts.iter().cloned().collect() || uploading {
            return Ok(None);
        }
        let bytes: Option<u64> = tx
            .query_row("SELECT bytes FROM artifacts WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .map_err(|_| Error::Preparation)?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        tx.execute(
            "INSERT OR REPLACE INTO deletions(id, bytes, phase, created_at) VALUES(?1, ?2, 'reserved', ?3)",
            params![id, bytes, crate::sandbox::now()],
        )
        .map_err(|_| Error::Preparation)?;
        tx.commit().map_err(|_| Error::Preparation)?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::Preparation)?;
        Self::finish_deletion(&tx, &self.root, id)?;
        tx.commit().map_err(|_| Error::Preparation)?;
        Ok(Some(bytes))
    }

    #[cfg(test)]
    pub fn record_deletion_for_test(&mut self, id: &str, bytes: u64, created_at: i64) {
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO deletions(id, bytes, phase, created_at) VALUES(?1, ?2, 'reserved', ?3)",
            params![id, bytes, created_at],
        );
    }

    pub fn list_artifacts(&self) -> Result<Vec<(String, u64)>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, bytes FROM artifacts")
            .map_err(|_| Error::Preparation)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| Error::Preparation)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|_| Error::Preparation)
    }

    pub fn receipts_for_artifact(&self, id: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT owner FROM receipts WHERE id=?1")
            .map_err(|_| Error::Preparation)?;
        let rows = stmt.query_map([id], |row| row.get(0)).map_err(|_| Error::Preparation)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|_| Error::Preparation)
    }

    pub fn distinct_review_receipt_run_ids(&self) -> Result<Vec<i64>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT owner FROM receipts WHERE owner LIKE 'review:%'")
            .map_err(|_| Error::Preparation)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::Preparation)?;
        let mut ids = Vec::new();
        for row in rows.flatten() {
            if let Some(num) = row.strip_prefix("review:").and_then(|s| s.parse::<i64>().ok()) {
                ids.push(num);
            }
        }
        Ok(ids)
    }

    pub fn has_active_upload_for_artifact(&self, id: &str) -> Result<bool, Error> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM uploads WHERE id=?1 AND deadline>=?2)",
                params![id, crate::sandbox::now()],
                |row| row.get(0),
            )
            .map_err(|_| Error::Preparation)?;
        Ok(exists)
    }

    pub fn storage_usage(&self) -> Result<(u64, u64), Error> {
        let recorded: u64 = self.conn.query_row("SELECT COALESCE((SELECT SUM(bytes) FROM artifacts),0)+COALESCE((SELECT SUM(reservation) FROM uploads),0)", [], |row| row.get(0)).map_err(|_| Error::Preparation)?;
        let mut pending = vec![self.root.to_path_buf()];
        let mut actual = 0u64;
        while let Some(path) = pending.pop() {
            let meta = fs::symlink_metadata(&path).map_err(|_| Error::Preparation)?;
            if meta.is_dir() {
                for entry in fs::read_dir(path).map_err(|_| Error::Preparation)? {
                    pending.push(entry.map_err(|_| Error::Preparation)?.path());
                }
            } else if meta.is_file() {
                #[cfg(unix)]
                let size = {
                    use std::os::unix::fs::MetadataExt;
                    meta.blocks().saturating_mul(512)
                };
                #[cfg(not(unix))]
                let size = meta.len().saturating_add(4096);
                actual = actual.saturating_add(size);
            }
        }
        Ok((recorded, actual))
    }
}
