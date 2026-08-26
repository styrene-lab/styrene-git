//! Repository identity, signer binding, and governance transitions.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::Signer;
use minicbor::{Decode, Encode};

use crate::codec::domain_digest;
use crate::{
    CanonicalCbor, CoreError, Digest, PublicKey, RepositoryId, SignatureBytes, StyreneIdentity,
};

const REPOSITORY_ID_DOMAIN: &[u8] = b"styrene/git/repository-id/v1\0";
const SIGNER_BINDING_PURPOSE: &str = "styrene-repository-signing-v1";
const IDENTITY_ROOT_DOMAIN: &[u8] = b"styrene/git/identity-root/v1\0";
const IDENTITY_TRANSITION_DOMAIN: &[u8] = b"styrene/git/identity-transition/v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum Visibility {
    #[n(0)]
    Public,
    #[n(1)]
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct IdentityDocument {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    pub name: String,
    #[n(2)]
    pub description: String,
    #[n(3)]
    pub default_branch: String,
    #[n(4)]
    pub visibility: Visibility,
    #[n(5)]
    pub delegates: Vec<StyreneIdentity>,
    #[n(6)]
    pub threshold: u16,
}

impl IdentityDocument {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        default_branch: impl Into<String>,
        visibility: Visibility,
        mut delegates: Vec<StyreneIdentity>,
        threshold: u16,
    ) -> Result<Self, CoreError> {
        delegates.sort_unstable();
        let document = Self {
            version: 1,
            name: name.into(),
            description: description.into(),
            default_branch: default_branch.into(),
            visibility,
            delegates,
            threshold,
        };
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.version != 1 {
            return Err(CoreError::InvalidIdentity(
                "unsupported identity version".into(),
            ));
        }
        if self.name.is_empty() {
            return Err(CoreError::InvalidIdentity("name must not be empty".into()));
        }
        if !valid_branch_name(&self.default_branch) {
            return Err(CoreError::InvalidIdentity("invalid default branch".into()));
        }
        if self.delegates.is_empty() {
            return Err(CoreError::InvalidIdentity(
                "delegate set must not be empty".into(),
            ));
        }
        if self.threshold == 0 || usize::from(self.threshold) > self.delegates.len() {
            return Err(CoreError::InvalidIdentity(
                "threshold is outside delegate set".into(),
            ));
        }
        if !self.delegates.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(CoreError::InvalidIdentity(
                "delegates must be sorted and unique".into(),
            ));
        }
        Ok(())
    }

    pub fn repository_id(&self) -> Result<RepositoryId, CoreError> {
        self.validate()?;
        Ok(RepositoryId::new(domain_digest(
            REPOSITORY_ID_DOMAIN,
            &self.canonical_bytes()?,
        )))
    }
}

fn valid_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('.')
        && !branch.ends_with('.')
        && !branch.contains("..")
        && !branch.contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
        && !branch.ends_with('/')
        && !branch.ends_with(".lock")
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
struct SignerBindingPayload {
    #[n(0)]
    version: u8,
    #[n(1)]
    purpose: String,
    #[n(2)]
    identity: StyreneIdentity,
    #[n(3)]
    identity_key: PublicKey,
    #[n(4)]
    repository_key: PublicKey,
    #[n(5)]
    key_epoch: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct SignerBinding {
    #[n(0)]
    payload: SignerBindingPayload,
    #[n(1)]
    signature: SignatureBytes,
}

impl SignerBinding {
    pub fn issue(
        identity_key: &ed25519_dalek::SigningKey,
        repository_key: PublicKey,
        key_epoch: u32,
    ) -> Result<Self, CoreError> {
        let identity_key_public = PublicKey::new(identity_key.verifying_key().to_bytes());
        let payload = SignerBindingPayload {
            version: 1,
            purpose: SIGNER_BINDING_PURPOSE.into(),
            identity: StyreneIdentity::from_public_key(&identity_key_public),
            identity_key: identity_key_public,
            repository_key,
            key_epoch,
        };
        let signature = identity_key.sign(&payload.canonical_bytes()?);
        Ok(Self {
            payload,
            signature: SignatureBytes::new(signature.to_bytes()),
        })
    }

    pub fn verify(&self) -> Result<(), CoreError> {
        if self.payload.version != 1 || self.payload.purpose != SIGNER_BINDING_PURPOSE {
            return Err(CoreError::InvalidBinding(
                "unsupported binding domain".into(),
            ));
        }
        if StyreneIdentity::from_public_key(&self.payload.identity_key) != self.payload.identity {
            return Err(CoreError::InvalidBinding(
                "identity hash does not match key".into(),
            ));
        }
        self.payload
            .identity_key
            .verifying_key()?
            .verify_strict(
                &self.payload.canonical_bytes()?,
                &self.signature.signature(),
            )
            .map_err(|_| CoreError::InvalidSignature)
    }

    pub fn selection(&self) -> Result<SignerSelection, CoreError> {
        self.verify()?;
        Ok(SignerSelection {
            identity: self.identity(),
            repository_key: self.repository_key(),
            key_epoch: self.key_epoch(),
        })
    }

    pub fn verify_selected(&self, selected: &SignerSelection) -> Result<(), CoreError> {
        self.verify()?;
        if self.identity() != selected.identity
            || self.repository_key() != selected.repository_key
            || self.key_epoch() != selected.key_epoch
        {
            return Err(CoreError::InvalidBinding(
                "binding does not match prior-state signer selection".into(),
            ));
        }
        Ok(())
    }

    pub const fn identity(&self) -> StyreneIdentity {
        self.payload.identity
    }

    pub const fn repository_key(&self) -> PublicKey {
        self.payload.repository_key
    }

    pub const fn key_epoch(&self) -> u32 {
        self.payload.key_epoch
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignerSelection {
    identity: StyreneIdentity,
    repository_key: PublicKey,
    key_epoch: u32,
}

impl SignerSelection {
    pub const fn identity(self) -> StyreneIdentity {
        self.identity
    }

    pub const fn repository_key(self) -> PublicKey {
        self.repository_key
    }

    pub const fn key_epoch(self) -> u32 {
        self.key_epoch
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
struct IdentityTransitionPayload {
    #[n(0)]
    version: u8,
    #[n(1)]
    repository: RepositoryId,
    #[n(2)]
    parent: Digest,
    #[n(3)]
    sequence: u64,
    #[n(4)]
    document: IdentityDocument,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Approval {
    #[n(0)]
    pub identity: StyreneIdentity,
    #[n(1)]
    pub key_epoch: u32,
    #[n(2)]
    pub signature: SignatureBytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct IdentityTransition {
    #[n(0)]
    payload: IdentityTransitionPayload,
    #[n(1)]
    approvals: Vec<Approval>,
}

impl IdentityTransition {
    pub fn proposed(
        previous: &IdentityState,
        document: IdentityDocument,
    ) -> Result<Self, CoreError> {
        let sequence = previous.sequence.checked_add(1).ok_or_else(|| {
            CoreError::InvalidIdentityTransition("identity transition sequence exhausted".into())
        })?;
        Ok(Self {
            payload: IdentityTransitionPayload {
                version: 1,
                repository: previous.repository,
                parent: previous.transition,
                sequence,
                document,
            },
            approvals: Vec::new(),
        })
    }

    pub fn approve(
        &mut self,
        binding: &SignerBinding,
        repository_key: &ed25519_dalek::SigningKey,
    ) -> Result<(), CoreError> {
        if repository_key.verifying_key().to_bytes() != *binding.repository_key().as_bytes() {
            return Err(CoreError::InvalidBinding(
                "repository private key does not match binding".into(),
            ));
        }
        let signature = repository_key.sign(&self.payload.canonical_bytes()?);
        self.approvals
            .retain(|approval| approval.identity != binding.identity());
        self.approvals.push(Approval {
            identity: binding.identity(),
            key_epoch: binding.key_epoch(),
            signature: SignatureBytes::new(signature.to_bytes()),
        });
        self.approvals.sort_by_key(|approval| approval.identity);
        Ok(())
    }

    pub fn verify(
        &self,
        previous: &IdentityState,
        selections: &BTreeMap<StyreneIdentity, SignerSelection>,
    ) -> Result<IdentityState, CoreError> {
        let expected_sequence = previous.sequence.checked_add(1).ok_or_else(|| {
            CoreError::InvalidIdentityTransition("identity transition sequence exhausted".into())
        })?;
        if self.payload.version != 1
            || self.payload.repository != previous.repository
            || self.payload.parent != previous.transition
            || self.payload.sequence != expected_sequence
        {
            return Err(CoreError::InvalidIdentityTransition(
                "repository, parent, or sequence mismatch".into(),
            ));
        }
        self.payload.document.validate()?;
        if !self
            .approvals
            .windows(2)
            .all(|pair| pair[0].identity < pair[1].identity)
        {
            return Err(CoreError::InvalidIdentityTransition(
                "approvals must be sorted and unique".into(),
            ));
        }

        let current_delegates: BTreeSet<_> = previous.document.delegates.iter().copied().collect();
        let signing_bytes = self.payload.canonical_bytes()?;
        let mut valid = 0_u16;
        for approval in &self.approvals {
            if !current_delegates.contains(&approval.identity) {
                continue;
            }
            let selection = selections.get(&approval.identity).ok_or_else(|| {
                CoreError::InvalidIdentityTransition("missing signer selection".into())
            })?;
            if selection.identity() != approval.identity {
                return Err(CoreError::InvalidIdentityTransition(
                    "signer identity mismatch".into(),
                ));
            }
            if selection.key_epoch() != approval.key_epoch {
                return Err(CoreError::InvalidIdentityTransition(
                    "signer epoch mismatch".into(),
                ));
            }
            selection
                .repository_key()
                .verifying_key()?
                .verify_strict(&signing_bytes, &approval.signature.signature())
                .map_err(|_| CoreError::InvalidSignature)?;
            valid += 1;
        }
        if valid < previous.document.threshold {
            return Err(CoreError::InvalidIdentityTransition(
                "delegate threshold not satisfied".into(),
            ));
        }

        let transition = domain_digest(IDENTITY_TRANSITION_DOMAIN, &self.canonical_bytes()?);
        Ok(IdentityState {
            repository: previous.repository,
            transition,
            sequence: self.payload.sequence,
            document: self.payload.document.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityState {
    pub repository: RepositoryId,
    pub transition: Digest,
    pub sequence: u64,
    pub document: IdentityDocument,
}

impl IdentityState {
    pub fn initial(document: IdentityDocument) -> Result<Self, CoreError> {
        document.validate()?;
        let bytes = document.canonical_bytes()?;
        Ok(Self {
            repository: document.repository_id()?,
            transition: domain_digest(IDENTITY_ROOT_DOMAIN, &bytes),
            sequence: 0,
            document,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_rejects_reordered_approvals() {
        let alice_identity_key = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
        let alice_repository_key = ed25519_dalek::SigningKey::from_bytes(&[2; 32]);
        let alice = SignerBinding::issue(
            &alice_identity_key,
            PublicKey::new(alice_repository_key.verifying_key().to_bytes()),
            0,
        )
        .expect("Alice binding");
        let bob_identity_key = ed25519_dalek::SigningKey::from_bytes(&[3; 32]);
        let bob_repository_key = ed25519_dalek::SigningKey::from_bytes(&[4; 32]);
        let bob = SignerBinding::issue(
            &bob_identity_key,
            PublicKey::new(bob_repository_key.verifying_key().to_bytes()),
            0,
        )
        .expect("Bob binding");
        let document = IdentityDocument::new(
            "project",
            "fixture",
            "main",
            Visibility::Public,
            vec![alice.identity(), bob.identity()],
            2,
        )
        .expect("document");
        let previous = IdentityState::initial(document.clone()).expect("initial state");
        let mut transition =
            IdentityTransition::proposed(&previous, document).expect("transition proposal");
        transition
            .approve(&alice, &alice_repository_key)
            .expect("Alice approval");
        transition
            .approve(&bob, &bob_repository_key)
            .expect("Bob approval");
        transition.approvals.reverse();

        let selections = BTreeMap::from([
            (
                alice.identity(),
                alice.selection().expect("Alice selection"),
            ),
            (bob.identity(), bob.selection().expect("Bob selection")),
        ]);
        assert!(matches!(
            transition.verify(&previous, &selections),
            Err(CoreError::InvalidIdentityTransition(message))
                if message == "approvals must be sorted and unique"
        ));
    }
}
