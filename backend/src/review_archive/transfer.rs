use super::*;

pub struct Incoming {
    descriptor: Descriptor,
    raw: Vec<u8>,
    manifest: Manifest,
    next: usize,
    data: Vec<u8>,
}

pub fn transfer_manifest(descriptor: &Descriptor, raw: &[u8]) -> Result<Manifest, Error> {
    descriptor.validate()?;
    if raw.len() as u64 != descriptor.manifest_bytes || hash(raw) != descriptor.id {
        return Err(Error::Integrity);
    }
    let (manifest, _) = manifest::decode(raw)?;
    if manifest.version != VERSION
        || manifest.policy_version != POLICY_VERSION
        || manifest.entry_count() != descriptor.entry_count
        || manifest.blobs.len() > descriptor.entry_count
    {
        return Err(Error::Contract);
    }
    manifest.validate_sources()?;
    let mut offset = 0;
    let mut blobs = BTreeMap::new();
    for blob in &manifest.blobs {
        if !oid(&blob.hash, 64)
            || blob.offset != offset
            || blob.bytes > MAX_FILE_BYTES
            || blobs.insert(blob.hash.as_str(), blob.bytes).is_some()
        {
            return Err(Error::Integrity);
        }
        offset = offset.checked_add(blob.bytes).ok_or(Error::Budget)?;
    }
    if offset != descriptor.blob_bytes {
        return Err(Error::Integrity);
    }
    let mut expanded = 0u64;
    let mut used = BTreeSet::new();
    for entry in manifest.all_entries() {
        match (
            entry.source.exclusion()?,
            entry.exclusion,
            entry.content_hash.as_deref(),
        ) {
            (Some(expected), Some(actual), None) if expected == actual => {}
            (None, Some(Exclusion::Binary | Exclusion::NonUtf8), None) if entry.source.bytes > 0 => {}
            (None, Some(Exclusion::Budget), None) => {}
            (None, None, Some(digest)) if blobs.get(digest) == Some(&entry.source.bytes) => {
                expanded = expanded.checked_add(entry.source.bytes).ok_or(Error::Budget)?;
                used.insert(digest);
            }
            _ => return Err(Error::Contract),
        }
    }
    if expanded != descriptor.expanded_bytes || used.len() != blobs.len() {
        return Err(Error::Integrity);
    }
    Ok(manifest)
}

pub fn validate_blob(manifest: &Manifest, digest: &str, bytes: &[u8]) -> Result<(), Error> {
    let blob = manifest
        .blobs
        .iter()
        .find(|blob| blob.hash == digest)
        .ok_or(Error::Integrity)?;
    if bytes.len() as u64 != blob.bytes
        || bytes.len() as u64 > MAX_FILE_BYTES
        || hash(bytes) != digest
        || bytes.contains(&0)
        || std::str::from_utf8(bytes).is_err()
    {
        return Err(Error::Integrity);
    }
    Ok(())
}

impl Incoming {
    pub fn new(descriptor: Descriptor, raw: Vec<u8>) -> Result<Self, Error> {
        let manifest = transfer_manifest(&descriptor, &raw)?;
        Ok(Self {
            descriptor,
            raw,
            manifest,
            next: 0,
            data: Vec::new(),
        })
    }

    pub fn push(&mut self, digest: &str, bytes: &[u8]) -> Result<(), Error> {
        validate_blob(&self.manifest, digest, bytes)?;
        if let Some(blob) = self.manifest.blobs[..self.next].iter().find(|blob| blob.hash == digest) {
            return if &self.data[blob.offset as usize..(blob.offset + blob.bytes) as usize] == bytes {
                Ok(())
            } else {
                Err(Error::Integrity)
            };
        }
        if self
            .manifest
            .blobs
            .get(self.next)
            .is_none_or(|blob| blob.hash != digest)
        {
            return Err(Error::Integrity);
        }
        self.data.extend_from_slice(bytes);
        self.next += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<Archive, Error> {
        if self.next != self.manifest.blobs.len() {
            return Err(Error::Integrity);
        }
        Archive::decode(&self.descriptor, &self.raw, self.data)
    }
}
