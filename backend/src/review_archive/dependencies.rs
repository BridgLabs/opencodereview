use super::*;

impl Dependency {
    pub fn scope(&self, host: &str) -> Scope {
        Scope {
            host: host.into(),
            repository: self.repository.clone(),
            head_sha: self.revision.clone(),
            base_sha: self.revision.clone(),
            merge_base_sha: self.revision.clone(),
            prior_sha: None,
        }
    }
}

impl Manifest {
    pub fn all_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .chain(self.dependencies.iter().flat_map(|dependency| &dependency.entries))
    }

    pub fn entry_count(&self) -> usize {
        self.all_entries().count()
    }

    pub(super) fn validate_sources(&self) -> Result<(), Error> {
        fn validate(scope: &Scope, entries: &[Entry]) -> Result<(), Error> {
            scope.validate()?;
            let mut previous = None;
            for entry in entries {
                entry.source.validate(scope)?;
                let key = (&entry.source.revision, &entry.source.path);
                if previous.is_some_and(|prior| prior >= key) {
                    return Err(Error::Contract);
                }
                previous = Some(key);
            }
            Ok(())
        }
        validate(&self.scope, &self.entries)?;
        if self.dependencies.len() > 16 {
            return Err(Error::Budget);
        }
        validate_limits(0, 0, self.entry_count())?;
        let mut previous = None;
        for dependency in &self.dependencies {
            let key = (&dependency.repository, &dependency.revision);
            if dependency.repository == self.scope.repository || previous.is_some_and(|prior| prior >= key) {
                return Err(Error::Contract);
            }
            previous = Some(key);
            validate(&dependency.scope(&self.scope.host), &dependency.entries)?;
            if dependency.provenance.len() > 512 {
                return Err(Error::Budget);
            }
            let mut previous_provenance: Option<&DependencyProvenance> = None;
            for prov in &dependency.provenance {
                if previous_provenance.is_some_and(|prior| prior >= prov) {
                    return Err(Error::Contract);
                }
                previous_provenance = Some(prov);
                if prov.repository != dependency.repository || prov.commit != dependency.revision {
                    return Err(Error::Contract);
                }
                let Some(entry) = dependency.entries.iter().find(|entry| entry.source.path == prov.path) else {
                    return Err(Error::Contract);
                };
                if entry.exclusion.is_some() || entry.content_hash.is_none() {
                    return Err(Error::Contract);
                }
                safe_path(&prov.path)?;
                if prov.url.is_empty() || prov.reference.is_empty() {
                    return Err(Error::Contract);
                }
                if !oid(&prov.commit, 40) && !oid(&prov.commit, 64) {
                    return Err(Error::Contract);
                }
                let expected_prefix = format!(
                    "https://{}/{}/blob/{}/{}",
                    self.scope.host, prov.repository, prov.reference, prov.path
                );
                let valid_url = prov.url == expected_prefix
                    || (prov.url.starts_with(&expected_prefix) && prov.url[expected_prefix.len()..].starts_with('#'));
                if !valid_url {
                    return Err(Error::Contract);
                }
                if prov.reference_kind == ReferenceKind::Commit && prov.reference != prov.commit {
                    return Err(Error::Contract);
                }
            }
        }
        Ok(())
    }
}

impl Archive {
    pub fn dependencies(&self) -> &[Dependency] {
        &self.manifest.dependencies
    }

    pub fn with_dependencies(self, dependencies: Vec<Archive>) -> Result<Self, Error> {
        self.with_dependencies_and_provenance(dependencies.into_iter().map(|dep| (dep, Vec::new())).collect())
    }

    pub fn with_dependencies_and_provenance(
        mut self,
        mut dependencies: Vec<(Archive, Vec<DependencyProvenance>)>,
    ) -> Result<Self, Error> {
        if !self.dependencies().is_empty() {
            return Err(Error::Contract);
        }
        dependencies.sort_by(|(a, _), (b, _)| {
            (&a.scope().repository, &a.scope().head_sha).cmp(&(&b.scope().repository, &b.scope().head_sha))
        });
        let mut known = self
            .manifest
            .blobs
            .iter()
            .map(|blob| blob.hash.clone())
            .collect::<BTreeSet<_>>();
        for (dependency, mut provenance) in dependencies {
            let scope = dependency.scope();
            if !dependency.dependencies().is_empty()
                || scope.host != self.scope().host
                || scope.repository == self.scope().repository
                || scope.revisions().len() != 1
                || scope.base_sha != scope.head_sha
            {
                return Err(Error::Contract);
            }
            validate_limits(
                self.data.len() as u64 + dependency.descriptor.blob_bytes,
                0,
                self.manifest.entry_count() + dependency.entries().len(),
            )?;
            for (digest, bytes) in dependency.transfer_blobs() {
                if known.insert(digest.into()) {
                    self.manifest.blobs.push(Blob {
                        hash: digest.into(),
                        offset: self.data.len() as u64,
                        bytes: bytes.len() as u64,
                    });
                    self.data.extend_from_slice(bytes);
                }
            }
            provenance.sort();
            provenance.dedup();
            self.manifest.dependencies.push(Dependency {
                repository: scope.repository.clone(),
                revision: scope.head_sha.clone(),
                entries: dependency.entries().to_vec(),
                provenance,
            });
            self.descriptor.expanded_bytes += dependency.descriptor.expanded_bytes;
        }
        let mut raw = manifest::encode(&self.manifest, false)?;
        if raw.len() as u64 > COMPACT_MANIFEST_BYTES {
            raw = manifest::encode(&self.manifest, true)?;
        }
        let descriptor = Descriptor {
            version: VERSION,
            id: hash(&raw),
            manifest_bytes: raw.len() as u64,
            blob_bytes: self.data.len() as u64,
            entry_count: self.manifest.entry_count(),
            expanded_bytes: self.descriptor.expanded_bytes,
        };
        Self::decode(&descriptor, &raw, self.data)
    }

    pub fn resolve_dependency(&self, repository: &str, revision: &str, path: &str) -> Result<&str, Error> {
        safe_path(path)?;
        let dependency = self
            .dependencies()
            .iter()
            .find(|dependency| dependency.repository == repository && dependency.revision == revision)
            .ok_or(Error::Revision)?;
        let entry = dependency
            .entries
            .iter()
            .find(|entry| entry.source.path == path)
            .ok_or(Error::Absent)?;
        if let Some(reason) = entry.exclusion {
            return Err(Error::Excluded(reason));
        }
        let blob = self
            .manifest
            .blobs
            .iter()
            .find(|blob| Some(&blob.hash) == entry.content_hash.as_ref())
            .ok_or(Error::Integrity)?;
        std::str::from_utf8(&self.data[blob.offset as usize..(blob.offset + blob.bytes) as usize])
            .map_err(|_| Error::Integrity)
    }

    pub(super) fn materialize_dependencies(
        &self,
        root: &Path,
        cancel: &AtomicBool,
        written: &mut BTreeMap<String, std::path::PathBuf>,
    ) -> Result<(), Error> {
        for dependency in self.dependencies() {
            for entry in dependency.entries.iter().filter(|entry| entry.exclusion.is_none()) {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                let target = root
                    .join("dependencies")
                    .join(&dependency.repository)
                    .join(&dependency.revision)
                    .join(&entry.source.path);
                self.write_source(
                    &target,
                    entry,
                    || self.resolve_dependency(&dependency.repository, &dependency.revision, &entry.source.path),
                    written,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn dependency_protocol(&self) -> String {
        if self.dependencies().is_empty() {
            return String::new();
        }
        let sources = self
            .dependencies()
            .iter()
            .map(|dependency| format!("- {} @ {}\n", dependency.repository, dependency.revision))
            .collect::<String>();
        format!("\nPinned supporting repository evidence is included in this snapshot's manifest.dependencies:\n{sources}Read only these supplied dependency files at /evidence/snapshot/dependencies/<owner>/<repo>/<revision>/<path>. References use source:\"repository:<owner>/<repo>\", revision:<exact dependency SHA>, path:<dependency-relative path>, line and exact quote. These are supporting evidence, not additional review scope: never add dependency files to reviewedFiles or anchor comments there. Findings still require current code evidence from the reviewed repository. A pinned git submodule is represented by its repository and exact gitlink commit; omit the submodule mount prefix when citing its files. Do not access other repositories, mutable branches or network. Missing/excluded essential dependency evidence remains missingEvidence.\n")
    }
}
