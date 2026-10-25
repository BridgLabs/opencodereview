use super::*;
use std::fs;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Stage {
    pub files: Vec<String>,
    pub before: String,
    pub snapshot_id: String,
    pub diff_hash: String,
    pub context_hashes: BTreeMap<String, String>,
}

impl Stage {
    pub(crate) fn hashes(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        files
            .iter()
            .filter(|(path, _)| path.starts_with("context/") && path.as_str() != "context/STAGE.json")
            .map(|(path, content)| (path.clone(), hash(content.as_bytes())))
            .collect()
    }

    pub(crate) fn file_hash(path: &Path) -> Result<String, Error> {
        let parent = path
            .parent()
            .ok_or(Error::Path)?
            .canonicalize()
            .map_err(|_| Error::Preparation)?;
        let mut file = open_archive(&parent.join(path.file_name().ok_or(Error::Path)?), false)?;
        if !file.metadata().map_err(|_| Error::Preparation)?.is_file() {
            return Err(Error::Path);
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 65_536];
        loop {
            let count = file.read(&mut buffer).map_err(|_| Error::Preparation)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    fn capture_context(context: &Path) -> Result<BTreeMap<String, String>, Error> {
        let mut hashes = BTreeMap::new();
        let mut pending = vec![context.to_path_buf()];
        while let Some(path) = pending.pop() {
            let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Preparation)?;
            if metadata.is_dir() {
                for entry in fs::read_dir(path).map_err(|_| Error::Preparation)? {
                    pending.push(entry.map_err(|_| Error::Preparation)?.path());
                }
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(context)
                    .map_err(|_| Error::Path)?
                    .to_str()
                    .ok_or(Error::Path)?;
                if relative == "STAGE.json" {
                    continue;
                }
                hashes.insert(format!("context/{relative}"), Self::file_hash(&path)?);
            } else {
                return Err(Error::Path);
            }
        }
        Ok(hashes)
    }
}

impl Archive {
    pub fn prepare_stage(&self, context: &Path, files: &[String], diff: &str, before: &str) -> Result<(), Error> {
        self.validate_assigned(files, diff, before)?;
        if !context.join("PR.md").is_file() || !context.join("skill/SKILL.md").is_file() {
            return Err(Error::Preparation);
        }
        for path in files {
            if let Ok(content) = self.resolve(&self.scope().head_sha, path) {
                let output = context.join("changed").join(path);
                fs::create_dir_all(output.parent().ok_or(Error::Path)?).map_err(|_| Error::Preparation)?;
                if fs::symlink_metadata(&output).is_ok_and(|meta| !meta.is_file()) {
                    return Err(Error::Path);
                }
                fs::write(output, content).map_err(|_| Error::Preparation)?;
            }
        }
        for area in ["changed", "related", "global"] {
            let root = context.join(area);
            let mut pending = if root.exists() { vec![root.clone()] } else { vec![] };
            while let Some(path) = pending.pop() {
                let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Preparation)?;
                if metadata.is_dir() {
                    for entry in fs::read_dir(&path).map_err(|_| Error::Preparation)? {
                        pending.push(entry.map_err(|_| Error::Preparation)?.path());
                    }
                } else if metadata.is_file() {
                    let relative = path
                        .strip_prefix(&root)
                        .map_err(|_| Error::Path)?
                        .to_str()
                        .ok_or(Error::Path)?;
                    match self.resolve(&self.scope().head_sha, relative) {
                        Ok(content) => {
                            if fs::read(&path).map_err(|_| Error::Preparation)? != content.as_bytes() {
                                return Err(Error::Integrity);
                            }
                        }
                        Err(Error::Excluded(_) | Error::Absent) if !files.iter().any(|file| file == relative) => {
                            fs::remove_file(&path).map_err(|_| Error::Preparation)?;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    return Err(Error::Path);
                }
            }
        }
        fs::write(context.join("PR_DIFF.patch"), diff).map_err(|_| Error::Preparation)?;
        fs::write(
            context.join("FROZEN.json"),
            serde_json::to_vec(&self.descriptor()).map_err(|_| Error::Contract)?,
        )
        .map_err(|_| Error::Preparation)?;
        let stage = Stage {
            files: files.to_vec(),
            before: before.into(),
            snapshot_id: self.descriptor().id,
            diff_hash: hash(diff.as_bytes()),
            context_hashes: Stage::capture_context(context)?,
        };
        fs::write(
            context.join("STAGE.json"),
            serde_json::to_vec(&stage).map_err(|_| Error::Contract)?,
        )
        .map_err(|_| Error::Preparation)?;
        Ok(())
    }

    pub fn validate_stage_files(&self, files: &BTreeMap<String, String>) -> Result<(), Error> {
        let descriptor: Descriptor = serde_json::from_str(files.get("context/FROZEN.json").ok_or(Error::Preparation)?)
            .map_err(|_| Error::Contract)?;
        if descriptor != self.descriptor() {
            return Err(Error::Integrity);
        }
        let stage: Stage = serde_json::from_str(files.get("context/STAGE.json").ok_or(Error::Preparation)?)
            .map_err(|_| Error::Contract)?;
        let diff = files.get("context/PR_DIFF.patch").ok_or(Error::Preparation)?;
        if stage.snapshot_id != descriptor.id
            || hash(diff.as_bytes()) != stage.diff_hash
            || stage.files.len() > 1000
            || stage.files.iter().collect::<BTreeSet<_>>().len() != stage.files.len()
            || !files.contains_key("context/PR.md")
            || !files.contains_key("context/skill/SKILL.md")
        {
            return Err(Error::Integrity);
        }
        if stage.context_hashes != Stage::hashes(files) {
            return Err(Error::Integrity);
        }
        self.validate_assigned(&stage.files, diff, &stage.before)?;
        for path in &stage.files {
            if let Ok(content) = self.resolve(&self.scope().head_sha, path) {
                if files.get(&format!("context/changed/{path}")).map(String::as_str) != Some(content) {
                    return Err(Error::Integrity);
                }
            }
        }
        for (path, content) in files {
            for area in ["changed", "related", "global"] {
                if let Some(path) = path.strip_prefix(&format!("context/{area}/")) {
                    if self.resolve(&self.scope().head_sha, path)? != content {
                        return Err(Error::Integrity);
                    }
                }
            }
        }
        Ok(())
    }
}
