use super::*;

const BATCH_BYTES: u64 = 4 * 1024 * 1024;

fn git(repo: &Path, args: &[&str], cancel: &AtomicBool, input: &[u8]) -> Result<Vec<u8>, Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    crate::indexer::git_frozen_with_input(repo, args, cancel, input).map_err(|_| {
        if cancel.load(Ordering::Relaxed) {
            Error::Cancelled
        } else {
            Error::Preparation
        }
    })
}

fn tree(
    repo: &Path,
    revision: &str,
    paths: Option<&BTreeSet<String>>,
    cancel: &AtomicBool,
) -> Result<Vec<SourceEntry>, Error> {
    let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-l", "-z", revision, "--"];
    if let Some(paths) = paths {
        args.extend(paths.iter().map(String::as_str));
    }
    let raw = git(repo, &args, cancel, &[])?;
    let mut entries = Vec::new();
    for record in raw.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let tab = record.iter().position(|byte| *byte == b'\t').ok_or(Error::Integrity)?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| Error::Integrity)?
            .split_whitespace()
            .collect::<Vec<_>>();
        if metadata.len() != 4 {
            return Err(Error::Integrity);
        }
        let bytes = match (metadata[0], metadata[1], metadata[3]) {
            ("160000", "commit", "-") => 0,
            ("100644" | "100755" | "120000", "blob", "-" | "BAD") => return Err(Error::Preparation),
            ("100644" | "100755" | "120000", "blob", size) => size.parse().map_err(|_| Error::Integrity)?,
            _ => return Err(Error::Integrity),
        };
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| Error::Path)?
            .to_string();
        safe_path(&path)?;
        entries.push(SourceEntry {
            path,
            revision: revision.into(),
            mode: metadata[0].into(),
            git_blob_id: metadata[2].into(),
            bytes,
        });
        validate_limits(0, 0, entries.len())?;
    }
    Ok(entries)
}

fn load_batch(
    repo: &Path,
    expected: &BTreeMap<String, u64>,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let input = expected.keys().map(|sha| format!("{sha}\n")).collect::<String>();
    let raw = git(repo, &["cat-file", "--batch"], cancel, input.as_bytes())?;
    let mut cursor = 0;
    let mut contents = BTreeMap::new();
    for (sha, size) in expected {
        let newline = raw[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(Error::Integrity)?
            + cursor;
        let header = std::str::from_utf8(&raw[cursor..newline])
            .map_err(|_| Error::Integrity)?
            .split_whitespace()
            .collect::<Vec<_>>();
        if header.len() == 2 && header[1] == "missing" {
            return Err(Error::Preparation);
        }
        if header.len() != 3 || header[0] != sha || header[1] != "blob" || header[2].parse::<u64>().ok() != Some(*size)
        {
            return Err(Error::Integrity);
        }
        cursor = newline + 1;
        let end = cursor
            .checked_add(*size as usize)
            .filter(|end| *end < raw.len())
            .ok_or(Error::Integrity)?;
        if raw[end] != b'\n' {
            return Err(Error::Integrity);
        }
        contents.insert(sha.clone(), raw[cursor..end].to_vec());
        cursor = end + 1;
    }
    if cursor != raw.len() {
        return Err(Error::Integrity);
    }
    Ok(contents)
}

pub fn hydrate(repo: &Path, scope: &Scope, cancel: &AtomicBool) -> Result<(), Error> {
    scope.validate()?;
    for _ in 0..4 {
        let revisions = scope.revisions();
        let mut args = vec![
            "rev-list",
            "--objects",
            "--no-object-names",
            "--missing=print",
            "--no-walk",
        ];
        args.extend(revisions.iter().copied());
        args.push(&scope.base_sha);
        let inventory = git(repo, &args, cancel, &[])?;
        let text = std::str::from_utf8(&inventory).map_err(|_| Error::Integrity)?;
        let missing = text
            .lines()
            .filter_map(|line| line.strip_prefix('?'))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        if missing.len() > MAX_ENTRIES || missing.iter().any(|sha| !oid(sha, scope.head_sha.len())) {
            return Err(Error::Budget);
        }
        for batch in missing.chunks(128) {
            let mut fetch = vec![
                "-c",
                "fetch.negotiationAlgorithm=noop",
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--recurse-submodules=no",
                "--no-auto-maintenance",
                "origin",
            ];
            fetch.extend(batch.iter().copied());
            crate::indexer::git(repo, &fetch, Some(cancel)).map_err(|_| {
                if cancel.load(Ordering::Relaxed) {
                    Error::Cancelled
                } else {
                    Error::Preparation
                }
            })?;
        }
    }
    Err(Error::Preparation)
}

pub fn capture(repo: &Path, scope: Scope, cancel: &AtomicBool) -> Result<Archive, Error> {
    capture_selected(repo, scope, None, cancel)
}

pub fn capture_selected(
    repo: &Path,
    scope: Scope,
    paths: Option<&BTreeSet<String>>,
    cancel: &AtomicBool,
) -> Result<Archive, Error> {
    scope.validate()?;
    for revision in scope.revisions().into_iter().chain([scope.base_sha.as_str()]) {
        let resolved = git(
            repo,
            &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
            cancel,
            &[],
        )?;
        if std::str::from_utf8(&resolved).map_err(|_| Error::Integrity)?.trim() != revision {
            return Err(Error::Revision);
        }
    }
    let mut sources = Vec::new();
    for revision in scope.revisions() {
        sources.extend(
            tree(repo, revision, paths, cancel)?
                .into_iter()
                .filter(|entry| paths.is_none_or(|paths| paths.contains(&entry.path))),
        );
        validate_limits(0, 0, sources.len())?;
    }
    sources.sort_by(|a, b| (&a.revision, &a.path).cmp(&(&b.revision, &b.path)));
    let mut batches: Vec<BTreeMap<String, u64>> = Vec::new();
    let mut memberships = BTreeMap::new();
    let mut batch_bytes = 0;
    for source in &sources {
        source.validate(&scope)?;
        if source.exclusion()?.is_some() {
            continue;
        }
        let additional = if batches
            .last()
            .is_some_and(|batch| batch.contains_key(&source.git_blob_id))
        {
            0
        } else {
            source.bytes
        };
        if batches.is_empty() || batch_bytes + additional > BATCH_BYTES {
            batches.push(BTreeMap::new());
            batch_bytes = 0;
        }
        let index = batches.len() - 1;
        let batch = &mut batches[index];
        if let Some(size) = batch.insert(source.git_blob_id.clone(), source.bytes) {
            if size != source.bytes {
                return Err(Error::Integrity);
            }
        } else {
            batch_bytes += source.bytes;
        }
        memberships.insert((source.revision.clone(), source.path.clone()), index);
    }
    let mut loaded = None;
    let mut contents = BTreeMap::new();
    Archive::build(scope, sources, cancel, |entry| {
        let index = *memberships
            .get(&(entry.revision.clone(), entry.path.clone()))
            .ok_or(Error::Integrity)?;
        if loaded != Some(index) {
            contents = load_batch(repo, &batches[index], cancel)?;
            loaded = Some(index);
        }
        contents.get(&entry.git_blob_id).cloned().ok_or(Error::Integrity)
    })
}
