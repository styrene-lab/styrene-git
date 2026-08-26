//! Bare Git repositories with authenticated publisher namespaces and object quarantine.

mod error;
mod git;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use error::io_error;
pub use error::StoreError;
use styrene_git_core::{
    CanonicalCbor, GitObjectId, RefState, RefTarget, RefTransition, RepositoryId, SignerBinding,
    SignerSelection, StyreneIdentity,
};

static QUARANTINE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjectFormat {
    LegacySha1,
    #[default]
    Sha256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Applied(RefState),
    AlreadyPresent(RefState),
}

impl CommitOutcome {
    pub const fn state(&self) -> &RefState {
        match self {
            Self::Applied(state) | Self::AlreadyPresent(state) => state,
        }
    }

    pub fn into_state(self) -> RefState {
        match self {
            Self::Applied(state) | Self::AlreadyPresent(state) => state,
        }
    }
}

impl ObjectFormat {
    pub const fn name(self) -> &'static str {
        match self {
            Self::LegacySha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }

    const fn zero_id(self) -> &'static str {
        match self {
            Self::LegacySha1 => "0000000000000000000000000000000000000000",
            Self::Sha256 => "0000000000000000000000000000000000000000000000000000000000000000",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepositoryStore {
    root: PathBuf,
}

impl RepositoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(root.join("repositories")).map_err(|source| io_error(&root, source))?;
        Ok(Self { root })
    }

    pub fn create(&self, id: RepositoryId, format: ObjectFormat) -> Result<Repository, StoreError> {
        let path = self.repository_path(id);
        if path.exists() {
            return self.open(id);
        }
        let temporary = path.with_extension(format!(
            "git.creating-{}-{}",
            std::process::id(),
            QUARANTINE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(error) = git::run_with_path(
            &[
                "init",
                "--bare",
                &format!("--object-format={}", format.name()),
            ],
            &temporary,
            "init --bare",
        ) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        let metadata = temporary.join("styrene");
        if let Err(source) = fs::create_dir_all(&metadata) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(io_error(&metadata, source));
        }
        let metadata_path = metadata.join("repository-id");
        if let Err(source) = fs::write(&metadata_path, id.to_string()) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(io_error(metadata_path, source));
        }
        if let Err(source) = fs::rename(&temporary, &path) {
            let _ = fs::remove_dir_all(&temporary);
            if !path.exists() {
                return Err(io_error(&path, source));
            }
        }
        self.open(id)
    }

    pub fn open(&self, id: RepositoryId) -> Result<Repository, StoreError> {
        let path = self.repository_path(id);
        if !path.is_dir() {
            return Err(StoreError::RepositoryNotFound(id));
        }
        let metadata_path = path.join("styrene/repository-id");
        let metadata = fs::read_to_string(&metadata_path)
            .map_err(|source| io_error(&metadata_path, source))?;
        if metadata != id.to_string() {
            return Err(StoreError::RepositoryMismatch(id));
        }
        let output = git::run(&path, &["rev-parse", "--show-object-format"], None, None)?;
        let object_format = parse_utf8(&output, "rev-parse --show-object-format")?.trim();
        let format = match object_format {
            "sha1" => {
                eprintln!(
                    "WARNING: repository {id} uses legacy Git SHA-1 object IDs; create new repositories with SHA-256"
                );
                ObjectFormat::LegacySha1
            }
            "sha256" => ObjectFormat::Sha256,
            other => return Err(StoreError::UnsupportedObjectFormat(other.into())),
        };
        Ok(Repository { id, path, format })
    }

    fn repository_path(&self, id: RepositoryId) -> PathBuf {
        self.root
            .join("repositories")
            .join(format!("{}.git", id.digest().base32()))
    }
}

#[derive(Clone, Debug)]
pub struct Repository {
    id: RepositoryId,
    path: PathBuf,
    format: ObjectFormat,
}

impl Repository {
    pub const fn id(&self) -> RepositoryId {
        self.id
    }

    pub const fn object_format(&self) -> ObjectFormat {
        self.format
    }

    pub fn begin_quarantine(&self) -> Result<Quarantine, StoreError> {
        let root = self.path.join("styrene/quarantine");
        fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;
        for _ in 0..100 {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let sequence = QUARANTINE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("{}-{timestamp}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let objects = path.join("objects");
                    fs::create_dir(&objects).map_err(|source| io_error(&objects, source))?;
                    return Ok(Quarantine {
                        repository: self.clone(),
                        path,
                        objects,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(io_error(path, source)),
            }
        }
        Err(io_error(
            root,
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique quarantine",
            ),
        ))
    }

    pub fn object_id(&self, kind: &str, bytes: &[u8]) -> Result<GitObjectId, StoreError> {
        validate_object_kind(kind)?;
        let output = git::run(
            &self.path,
            &["hash-object", "--stdin", "-t", kind],
            Some(bytes),
            None,
        )?;
        parse_object_id(self.format, &output)
    }

    pub fn export_pack(
        &self,
        wants: &[GitObjectId],
        prerequisites: &[GitObjectId],
        max_bytes: u64,
    ) -> Result<Vec<u8>, StoreError> {
        let mut revisions = String::new();
        for want in wants {
            self.require_object_format(want)?;
            revisions.push_str(&object_id_hex(want));
            revisions.push('\n');
        }
        for prerequisite in prerequisites {
            self.require_object_format(prerequisite)?;
            revisions.push('^');
            revisions.push_str(&object_id_hex(prerequisite));
            revisions.push('\n');
        }
        git::run_bounded(
            &self.path,
            &["pack-objects", "--stdout", "--revs"],
            revisions.as_bytes(),
            max_bytes,
        )
    }

    pub fn publisher_state(
        &self,
        publisher: StyreneIdentity,
    ) -> Result<Option<RefState>, StoreError> {
        self.publisher_state_with_object(publisher)
            .map(|state| state.map(|(_, state)| state))
    }

    fn publisher_state_with_object(
        &self,
        publisher: StyreneIdentity,
    ) -> Result<Option<(String, RefState)>, StoreError> {
        let state_ref = state_ref(publisher);
        let Some(object_id) = self.resolve_ref(&state_ref)? else {
            return Ok(None);
        };
        let bytes = git::run(&self.path, &["cat-file", "blob", &object_id], None, None)?;
        let state = RefState::from_canonical_bytes(&bytes)?;
        if state.repository != self.id || state.publisher != publisher {
            return Err(StoreError::InvalidPublisherState);
        }
        Ok(Some((object_id, state)))
    }

    pub fn publisher_refs(&self, publisher: StyreneIdentity) -> Result<Vec<RefTarget>, StoreError> {
        Ok(self
            .publisher_state(publisher)?
            .map(|state| state.refs)
            .unwrap_or_default())
    }

    pub fn publisher_ref_target(
        &self,
        publisher: StyreneIdentity,
        reference: &str,
    ) -> Result<Option<GitObjectId>, StoreError> {
        if !reference.starts_with("refs/") {
            return Err(StoreError::InvalidPublisherState);
        }
        self.resolve_ref(&namespace_ref(publisher, reference))?
            .map(|object| parse_object_id(self.format, object.as_bytes()))
            .transpose()
    }

    pub fn has_object(&self, object: &GitObjectId) -> Result<bool, StoreError> {
        self.require_object_format(object)?;
        let object = format!("{}^{{object}}", object_id_hex(object));
        match git::run(&self.path, &["cat-file", "-e", &object], None, None) {
            Ok(_) => Ok(true),
            Err(StoreError::Git { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn is_ancestor(
        &self,
        ancestor: &GitObjectId,
        descendant: &GitObjectId,
    ) -> Result<bool, StoreError> {
        self.require_object_format(ancestor)?;
        self.require_object_format(descendant)?;
        match git::run(
            &self.path,
            &[
                "merge-base",
                "--is-ancestor",
                &object_id_hex(ancestor),
                &object_id_hex(descendant),
            ],
            None,
            None,
        ) {
            Ok(_) => Ok(true),
            Err(StoreError::Git { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn fsck(&self) -> Result<(), StoreError> {
        git::run(&self.path, &["fsck", "--strict"], None, None).map(|_| ())
    }

    pub fn commit(
        &self,
        quarantine: &Quarantine,
        transition: &RefTransition,
        binding: &SignerBinding,
        selected: &SignerSelection,
    ) -> Result<CommitOutcome, StoreError> {
        if quarantine.repository.path != self.path {
            return Err(StoreError::RepositoryMismatch(self.id));
        }
        binding.verify_selected(selected)?;
        let transition_id = transition.transition_id()?;
        let previous = self.publisher_state_with_object(selected.identity())?;
        if let Some((_, state)) = &previous {
            if state.transition == transition_id {
                return Ok(CommitOutcome::AlreadyPresent(state.clone()));
            }
        }
        let previous_state = previous.as_ref().map(|(_, state)| state);
        let next = transition.verify(self.id, binding, selected, previous_state)?;
        quarantine.verify_targets(next.refs.iter().map(|reference| &reference.target))?;
        promote_objects(&quarantine.objects, &self.path.join("objects"))?;

        let state_object = self.write_blob(&next.canonical_bytes()?)?;
        let transition_object = self.write_blob(&transition.canonical_bytes()?)?;
        let update = self.update_publisher_refs(
            selected.identity(),
            previous.as_ref(),
            &next,
            &state_object,
            &transition_object,
        );
        match update {
            Ok(()) => Ok(CommitOutcome::Applied(next)),
            Err(error) => {
                let current = self.publisher_state(selected.identity())?;
                if let Some(state) = current {
                    if state.transition == transition_id {
                        return Ok(CommitOutcome::AlreadyPresent(state));
                    }
                }
                Err(error)
            }
        }
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<String, StoreError> {
        let output = git::run(
            &self.path,
            &["hash-object", "-w", "--stdin", "-t", "blob"],
            Some(bytes),
            None,
        )?;
        Ok(parse_utf8(&output, "hash-object")?.trim().into())
    }

    fn update_publisher_refs(
        &self,
        publisher: StyreneIdentity,
        previous: Option<&(String, RefState)>,
        next: &RefState,
        state_object: &str,
        transition_object: &str,
    ) -> Result<(), StoreError> {
        let old_refs: BTreeMap<_, _> = previous
            .into_iter()
            .flat_map(|(_, state)| &state.refs)
            .map(|reference| (reference.name.as_str(), reference.target.clone()))
            .collect();
        let new_refs: BTreeMap<_, _> = next
            .refs
            .iter()
            .map(|reference| (reference.name.as_str(), reference.target.clone()))
            .collect();
        let mut transaction = String::from("start\n");
        for (name, old_target) in &old_refs {
            if !new_refs.contains_key(name) {
                transaction.push_str(&format!(
                    "delete {} {}\n",
                    namespace_ref(publisher, name),
                    object_id_hex(old_target)
                ));
            }
        }
        for (name, target) in &new_refs {
            let old = old_refs
                .get(name)
                .map_or(self.format.zero_id().into(), object_id_hex);
            transaction.push_str(&format!(
                "update {} {} {}\n",
                namespace_ref(publisher, name),
                object_id_hex(target),
                old
            ));
        }
        let current_state = previous
            .map(|(object, _)| object.clone())
            .unwrap_or_else(|| self.format.zero_id().into());
        transaction.push_str(&format!(
            "update {} {} {}\n",
            state_ref(publisher),
            state_object,
            current_state
        ));
        transaction.push_str(&format!(
            "create {} {}\nprepare\ncommit\n",
            transition_ref(publisher, next),
            transition_object
        ));
        git::run(
            &self.path,
            &["update-ref", "--stdin"],
            Some(transaction.as_bytes()),
            None,
        )?;
        Ok(())
    }

    fn resolve_ref(&self, reference: &str) -> Result<Option<String>, StoreError> {
        match git::run(
            &self.path,
            &["show-ref", "--verify", "--hash", reference],
            None,
            None,
        ) {
            Ok(output) => Ok(Some(parse_utf8(&output, "show-ref")?.trim().to_owned())),
            Err(StoreError::Git { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn require_object_format(&self, object: &GitObjectId) -> Result<(), StoreError> {
        object.validate()?;
        if object.algorithm == self.format.name() {
            Ok(())
        } else {
            Err(StoreError::ObjectIdMismatch)
        }
    }
}

#[derive(Debug)]
pub struct Quarantine {
    repository: Repository,
    path: PathBuf,
    objects: PathBuf,
}

impl Quarantine {
    pub fn import_pack(&self, pack: &[u8], max_bytes: u64) -> Result<(), StoreError> {
        if pack.len() as u64 > max_bytes {
            return Err(StoreError::PackTooLarge { limit: max_bytes });
        }
        git::run(
            &self.repository.path,
            &["index-pack", "--stdin", "--fix-thin"],
            Some(pack),
            Some(&self.objects),
        )?;
        Ok(())
    }

    pub fn write_verified_object(
        &self,
        kind: &str,
        bytes: &[u8],
        expected: &GitObjectId,
    ) -> Result<(), StoreError> {
        validate_object_kind(kind)?;
        expected.validate()?;
        if expected.algorithm != self.repository.format.name() {
            return Err(StoreError::ObjectIdMismatch);
        }
        let output = git::run(
            &self.repository.path,
            &["hash-object", "-w", "--stdin", "-t", kind],
            Some(bytes),
            Some(&self.objects),
        )?;
        if parse_object_id(self.repository.format, &output)? != *expected {
            return Err(StoreError::ObjectIdMismatch);
        }
        Ok(())
    }

    fn verify_targets<'object>(
        &self,
        targets: impl Iterator<Item = &'object GitObjectId>,
    ) -> Result<(), StoreError> {
        let mut roots = String::new();
        for target in targets {
            target.validate()?;
            if target.algorithm != self.repository.format.name() {
                return Err(StoreError::ObjectIdMismatch);
            }
            let object = format!("{}^{{object}}", object_id_hex(target));
            git::run(
                &self.repository.path,
                &["cat-file", "-e", &object],
                None,
                Some(&self.objects),
            )?;
            roots.push_str(&object_id_hex(target));
            roots.push('\n');
        }
        git::run(
            &self.repository.path,
            &["rev-list", "--objects", "--stdin", "--missing=error"],
            Some(roots.as_bytes()),
            Some(&self.objects),
        )?;
        git::run(
            &self.repository.path,
            &["fsck", "--strict", "--connectivity-only", "--no-dangling"],
            None,
            Some(&self.objects),
        )?;
        Ok(())
    }
}

impl Drop for Quarantine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn validate_object_kind(kind: &str) -> Result<(), StoreError> {
    if matches!(kind, "blob" | "tree" | "commit" | "tag") {
        Ok(())
    } else {
        Err(StoreError::InvalidObjectKind(kind.into()))
    }
}

fn parse_object_id(format: ObjectFormat, output: &[u8]) -> Result<GitObjectId, StoreError> {
    let text = parse_utf8(output, "object ID")?.trim();
    let bytes = hex::decode(text).map_err(|_| StoreError::InvalidObjectId(text.into()))?;
    match format {
        ObjectFormat::LegacySha1 => bytes
            .try_into()
            .map(GitObjectId::sha1)
            .map_err(|_| StoreError::InvalidObjectId(text.into())),
        ObjectFormat::Sha256 => bytes
            .try_into()
            .map(GitObjectId::sha256)
            .map_err(|_| StoreError::InvalidObjectId(text.into())),
    }
}

fn parse_utf8<'bytes>(bytes: &'bytes [u8], operation: &str) -> Result<&'bytes str, StoreError> {
    std::str::from_utf8(bytes).map_err(|_| StoreError::InvalidGitOutput(operation.into()))
}

fn object_id_hex(object: &GitObjectId) -> String {
    hex::encode(&object.bytes)
}

fn namespace_ref(publisher: StyreneIdentity, reference: &str) -> String {
    format!("refs/namespaces/{publisher}/{reference}")
}

fn state_ref(publisher: StyreneIdentity) -> String {
    format!("refs/styrene/publishers/{publisher}/state")
}

fn transition_ref(publisher: StyreneIdentity, state: &RefState) -> String {
    format!(
        "refs/styrene/publishers/{publisher}/transitions/{}",
        state.transition.base32()
    )
}

fn promote_objects(source: &Path, destination: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(source).map_err(|error| io_error(source, error))? {
        let entry = entry.map_err(|error| io_error(source, error))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| io_error(&source_path, error))?
            .is_dir()
        {
            fs::create_dir_all(&destination_path)
                .map_err(|error| io_error(&destination_path, error))?;
            promote_objects(&source_path, &destination_path)?;
        } else if !destination_path.exists() {
            let file =
                fs::File::open(&source_path).map_err(|error| io_error(&source_path, error))?;
            file.sync_all()
                .map_err(|error| io_error(&source_path, error))?;
            drop(file);
            match fs::hard_link(&source_path, &destination_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(&destination_path, error)),
            }
        }
    }
    Ok(())
}
