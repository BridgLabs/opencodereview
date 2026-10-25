use super::*;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EntryGroup {
    entry: Entry,
    additional_revisions: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CompactManifest {
    version: u32,
    policy_version: u32,
    scope: Scope,
    entry_groups: Vec<EntryGroup>,
    blobs: Vec<Blob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<Dependency>,
}

pub(super) fn encode(manifest: &Manifest, compact: bool) -> Result<Vec<u8>, Error> {
    if !compact {
        return serde_json::to_vec(manifest).map_err(|_| Error::Contract);
    }
    let mut groups = BTreeMap::new();
    for entry in &manifest.entries {
        let source = &entry.source;
        let key = (
            &source.path,
            &source.mode,
            &source.git_blob_id,
            source.bytes,
            &entry.content_hash,
            entry.exclusion,
        );
        let group = groups.entry(key).or_insert_with(|| EntryGroup {
            entry: entry.clone(),
            additional_revisions: Vec::new(),
        });
        if group.entry.source.revision != source.revision {
            group.additional_revisions.push(source.revision.clone());
        }
    }
    let compact = CompactManifest {
        version: manifest.version,
        policy_version: manifest.policy_version,
        scope: manifest.scope.clone(),
        entry_groups: groups.into_values().collect(),
        blobs: manifest.blobs.clone(),
        dependencies: manifest.dependencies.clone(),
    };
    serde_json::to_vec(&compact).map_err(|_| Error::Contract)
}

pub(super) fn decode(raw: &[u8]) -> Result<(Manifest, bool), Error> {
    validate_limits(0, raw.len() as u64, 0)?;
    let (manifest, compact) = match serde_json::from_slice::<Manifest>(raw) {
        Ok(manifest) => (manifest, false),
        Err(_) => {
            let compact: CompactManifest = serde_json::from_slice(raw).map_err(|_| Error::Contract)?;
            compact.scope.validate()?;
            let count = compact.entry_groups.iter().try_fold(0usize, |count, group| {
                count
                    .checked_add(1 + group.additional_revisions.len())
                    .ok_or(Error::Budget)
            })?;
            validate_limits(0, raw.len() as u64, count)?;
            let mut entries = Vec::with_capacity(count);
            for group in compact.entry_groups {
                group.entry.source.validate(&compact.scope)?;
                let mut previous = group.entry.source.revision.as_str();
                for revision in &group.additional_revisions {
                    if revision.as_str() <= previous || !compact.scope.revisions().contains(revision.as_str()) {
                        return Err(Error::Revision);
                    }
                    previous = revision;
                    let mut entry = group.entry.clone();
                    entry.source.revision = revision.clone();
                    entries.push(entry);
                }
                entries.push(group.entry);
            }
            entries.sort_by(|a, b| (&a.source.revision, &a.source.path).cmp(&(&b.source.revision, &b.source.path)));
            (
                Manifest {
                    version: compact.version,
                    policy_version: compact.policy_version,
                    scope: compact.scope,
                    entries,
                    blobs: compact.blobs,
                    dependencies: compact.dependencies,
                },
                true,
            )
        }
    };
    if encode(&manifest, compact)? != raw {
        return Err(Error::Contract);
    }
    Ok((manifest, compact))
}
