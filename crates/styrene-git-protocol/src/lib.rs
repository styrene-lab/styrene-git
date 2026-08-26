//! Carrier-neutral summaries, transfer manifests, and replication application.

use minicbor::{Decode, Encode};
use styrene_git_core::{
    CanonicalCbor, Digest, GitObjectId, RefState, RefTransition, RepositoryId, SignerBinding,
    SignerSelection, StyreneIdentity,
};
use styrene_git_store::{CommitOutcome, Repository, StoreError};

const BINDING_DOMAIN: &[u8] = b"styrene/git/transfer-binding/v1\0";
const TRANSITION_DOMAIN: &[u8] = b"styrene/git/transfer-transition/v1\0";
const PAYLOAD_DOMAIN: &[u8] = b"styrene/git/transfer-payload/v1\0";

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct PublisherSummary {
    #[n(0)]
    pub publisher: StyreneIdentity,
    #[n(1)]
    pub transition: Digest,
    #[n(2)]
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct StateSummary {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    pub repository: RepositoryId,
    #[n(2)]
    pub publishers: Vec<PublisherSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct StateWant {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    pub repository: RepositoryId,
    #[n(2)]
    pub publisher: StyreneIdentity,
    #[n(3)]
    pub after: Option<Digest>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
#[cbor(array)]
pub enum Prerequisite {
    #[n(0)]
    PublisherTransition(#[n(0)] Digest),
    #[n(1)]
    Object(#[n(0)] GitObjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum PayloadKind {
    #[n(0)]
    GitPack,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct PayloadDescriptor {
    #[n(0)]
    pub kind: PayloadKind,
    #[n(1)]
    pub length: u64,
    #[n(2)]
    pub digest: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct TransferManifest {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    pub repository: RepositoryId,
    #[n(2)]
    pub publisher: StyreneIdentity,
    #[n(3)]
    pub prerequisites: Vec<Prerequisite>,
    #[n(4)]
    pub resulting_transition: Digest,
    #[n(5)]
    pub binding_digest: Digest,
    #[n(6)]
    pub transition_digest: Digest,
    #[n(7)]
    pub payload: PayloadDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Transfer {
    #[n(0)]
    pub manifest: TransferManifest,
    #[n(1)]
    pub binding: Vec<u8>,
    #[n(2)]
    pub transition: Vec<u8>,
    #[n(3)]
    pub payload: Vec<u8>,
}

impl Transfer {
    pub fn new(
        repository: RepositoryId,
        binding: &SignerBinding,
        transition: &RefTransition,
        prerequisite_transition: Option<Digest>,
        mut prerequisite_objects: Vec<GitObjectId>,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        prerequisite_objects.sort_unstable();
        prerequisite_objects.dedup();
        let mut prerequisites = Vec::new();
        if let Some(parent) = prerequisite_transition {
            prerequisites.push(Prerequisite::PublisherTransition(parent));
        }
        prerequisites.extend(prerequisite_objects.into_iter().map(Prerequisite::Object));
        let binding = binding.canonical_bytes()?;
        let transition_bytes = transition.canonical_bytes()?;
        let manifest = TransferManifest {
            version: 1,
            repository,
            publisher: binding_identity(&binding)?,
            prerequisites,
            resulting_transition: transition.transition_id()?,
            binding_digest: digest(BINDING_DOMAIN, &binding),
            transition_digest: digest(TRANSITION_DOMAIN, &transition_bytes),
            payload: PayloadDescriptor {
                kind: PayloadKind::GitPack,
                length: payload.len() as u64,
                digest: digest(PAYLOAD_DOMAIN, &payload),
            },
        };
        Ok(Self {
            manifest,
            binding,
            transition: transition_bytes,
            payload,
        })
    }

    pub fn validate(&self, max_payload_bytes: u64) -> Result<(), ProtocolError> {
        if self.manifest.version != 1 {
            return Err(ProtocolError::UnsupportedVersion(self.manifest.version));
        }
        if self.payload.len() as u64 > max_payload_bytes {
            return Err(ProtocolError::PayloadTooLarge {
                limit: max_payload_bytes,
            });
        }
        if self.manifest.payload.length != self.payload.len() as u64
            || self.manifest.binding_digest != digest(BINDING_DOMAIN, &self.binding)
            || self.manifest.transition_digest != digest(TRANSITION_DOMAIN, &self.transition)
            || self.manifest.payload.digest != digest(PAYLOAD_DOMAIN, &self.payload)
        {
            return Err(ProtocolError::IntegrityMismatch);
        }
        if !self
            .manifest
            .prerequisites
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(ProtocolError::NonCanonicalPrerequisites);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied(RefState),
    AlreadyPresent(RefState),
    MissingPrerequisites(Vec<Prerequisite>),
}

#[derive(Clone, Copy, Debug)]
pub struct SignerAuthorization<'a> {
    binding: &'a SignerBinding,
    selected: &'a SignerSelection,
}

impl<'a> SignerAuthorization<'a> {
    pub const fn new(binding: &'a SignerBinding, selected: &'a SignerSelection) -> Self {
        Self { binding, selected }
    }
}

pub fn export_transfer(
    repository: &Repository,
    signer: SignerAuthorization<'_>,
    transition: &RefTransition,
    previous: Option<&RefState>,
    wants: &[GitObjectId],
    prerequisite_objects: &[GitObjectId],
    max_payload_bytes: u64,
) -> Result<Transfer, ProtocolError> {
    transition.verify(repository.id(), signer.binding, signer.selected, previous)?;
    let payload = repository.export_pack(wants, prerequisite_objects, max_payload_bytes)?;
    Transfer::new(
        repository.id(),
        signer.binding,
        transition,
        previous.map(|state| state.transition),
        prerequisite_objects.to_vec(),
        payload,
    )
}

pub fn apply_transfer(
    repository: &Repository,
    transfer: &Transfer,
    selected: &SignerSelection,
    max_payload_bytes: u64,
) -> Result<ApplyOutcome, ProtocolError> {
    transfer.validate(max_payload_bytes)?;
    if transfer.manifest.repository != repository.id() {
        return Err(ProtocolError::RepositoryMismatch);
    }
    let binding = SignerBinding::from_canonical_bytes(&transfer.binding)?;
    let transition = RefTransition::from_canonical_bytes(&transfer.transition)?;
    binding.verify_selected(selected)?;
    if selected.identity() != transfer.manifest.publisher
        || transition.transition_id()? != transfer.manifest.resulting_transition
    {
        return Err(ProtocolError::IntegrityMismatch);
    }
    let current = repository.publisher_state(binding.identity())?;
    if let Some(current_state) = &current {
        if current_state.transition == transfer.manifest.resulting_transition {
            return Ok(ApplyOutcome::AlreadyPresent(current_state.clone()));
        }
    }

    let mut missing = Vec::new();
    for prerequisite in &transfer.manifest.prerequisites {
        let present = match prerequisite {
            Prerequisite::PublisherTransition(transition) => {
                current.as_ref().map(|state| state.transition) == Some(*transition)
            }
            Prerequisite::Object(object) => repository.has_object(object)?,
        };
        if !present {
            missing.push(prerequisite.clone());
        }
    }
    if !missing.is_empty() {
        return Ok(ApplyOutcome::MissingPrerequisites(missing));
    }
    let quarantine = repository.begin_quarantine()?;
    quarantine.import_pack(&transfer.payload, max_payload_bytes)?;
    match repository.commit(&quarantine, &transition, &binding, selected)? {
        CommitOutcome::Applied(state) => Ok(ApplyOutcome::Applied(state)),
        CommitOutcome::AlreadyPresent(state) => Ok(ApplyOutcome::AlreadyPresent(state)),
    }
}

fn binding_identity(bytes: &[u8]) -> Result<StyreneIdentity, ProtocolError> {
    Ok(SignerBinding::from_canonical_bytes(bytes)?.identity())
}

fn digest(domain: &[u8], bytes: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    Digest::new(*hasher.finalize().as_bytes())
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
    #[error("transfer payload exceeds the configured {limit}-byte limit")]
    PayloadTooLarge { limit: u64 },
    #[error("transfer integrity check failed")]
    IntegrityMismatch,
    #[error("transfer prerequisites must be sorted and unique")]
    NonCanonicalPrerequisites,
    #[error("transfer repository does not match the destination")]
    RepositoryMismatch,
    #[error(transparent)]
    Core(#[from] styrene_git_core::CoreError),
    #[error(transparent)]
    Store(#[from] StoreError),
}
