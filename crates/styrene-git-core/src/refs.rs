//! Signed, replay-resistant publisher reference transitions.

use ed25519_dalek::Signer;
use minicbor::{Decode, Encode};

use crate::codec::domain_digest;
use crate::{
    CanonicalCbor, CoreError, Digest, RepositoryId, SignatureBytes, SignerBinding, SignerSelection,
    StyreneIdentity,
};

const REF_TRANSITION_DOMAIN: &[u8] = b"styrene/git/ref-transition/v1\0";
const REF_TRANSITION_SIGNATURE_DOMAIN: &[u8] = b"styrene/git/ref-transition-signature/v1\0";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
#[cbor(array)]
pub struct GitObjectId {
    #[n(0)]
    pub algorithm: String,
    #[n(1)]
    pub bytes: Vec<u8>,
}

impl GitObjectId {
    pub fn sha1(bytes: [u8; 20]) -> Self {
        Self {
            algorithm: "sha1".into(),
            bytes: bytes.to_vec(),
        }
    }

    pub fn sha256(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: "sha256".into(),
            bytes: bytes.to_vec(),
        }
    }

    pub fn from_hex(algorithm: &str, value: &str) -> Result<Self, CoreError> {
        let bytes = hex::decode(value).map_err(|_| {
            CoreError::InvalidRefTransition("Git object identifier is not hexadecimal".into())
        })?;
        match algorithm {
            "sha1" => bytes
                .try_into()
                .map(Self::sha1)
                .map_err(|_| CoreError::InvalidRefTransition("invalid SHA-1 length".into())),
            "sha256" => bytes
                .try_into()
                .map(Self::sha256)
                .map_err(|_| CoreError::InvalidRefTransition("invalid SHA-256 length".into())),
            _ => Err(CoreError::InvalidRefTransition(
                "unsupported Git object algorithm".into(),
            )),
        }
    }

    pub fn hex(&self) -> String {
        hex::encode(&self.bytes)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        let valid = matches!(
            (self.algorithm.as_str(), self.bytes.len()),
            ("sha1", 20) | ("sha256", 32)
        );
        if valid {
            Ok(())
        } else {
            Err(CoreError::InvalidRefTransition(
                "invalid Git object identifier".into(),
            ))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RefTarget {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub target: GitObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
struct RefTransitionPayload {
    #[n(0)]
    version: u8,
    #[n(1)]
    repository: RepositoryId,
    #[n(2)]
    publisher: StyreneIdentity,
    #[n(3)]
    key_epoch: u32,
    #[n(4)]
    parent: Option<Digest>,
    #[n(5)]
    sequence: u64,
    #[n(6)]
    refs: Vec<RefTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RefTransition {
    #[n(0)]
    payload: RefTransitionPayload,
    #[n(1)]
    signature: SignatureBytes,
}

impl RefTransition {
    pub fn transition_id(&self) -> Result<Digest, CoreError> {
        Ok(domain_digest(
            REF_TRANSITION_DOMAIN,
            &self.canonical_bytes()?,
        ))
    }

    pub fn signed(
        repository: RepositoryId,
        binding: &SignerBinding,
        repository_key: &ed25519_dalek::SigningKey,
        previous: Option<&RefState>,
        mut refs: Vec<RefTarget>,
    ) -> Result<Self, CoreError> {
        binding.verify()?;
        if repository_key.verifying_key().to_bytes() != *binding.repository_key().as_bytes() {
            return Err(CoreError::InvalidBinding(
                "repository private key does not match binding".into(),
            ));
        }
        refs.sort_by(|left, right| left.name.cmp(&right.name));
        validate_refs(&refs)?;
        let (parent, sequence) = match previous {
            Some(state) => (
                Some(state.transition),
                state.sequence.checked_add(1).ok_or_else(|| {
                    CoreError::InvalidRefTransition(
                        "reference transition sequence exhausted".into(),
                    )
                })?,
            ),
            None => (None, 0),
        };
        let payload = RefTransitionPayload {
            version: 1,
            repository,
            publisher: binding.identity(),
            key_epoch: binding.key_epoch(),
            parent,
            sequence,
            refs,
        };
        let signature = repository_key.sign(&signature_frame(&payload)?);
        Ok(Self {
            payload,
            signature: SignatureBytes::new(signature.to_bytes()),
        })
    }

    pub fn verify(
        &self,
        repository: RepositoryId,
        binding: &SignerBinding,
        selected: &SignerSelection,
        previous: Option<&RefState>,
    ) -> Result<RefState, CoreError> {
        binding.verify_selected(selected)?;
        let (expected_parent, expected_sequence) = match previous {
            Some(state) => (
                Some(state.transition),
                state.sequence.checked_add(1).ok_or_else(|| {
                    CoreError::InvalidRefTransition(
                        "reference transition sequence exhausted".into(),
                    )
                })?,
            ),
            None => (None, 0),
        };
        if self.payload.version != 1
            || self.payload.repository != repository
            || self.payload.publisher != selected.identity()
            || self.payload.key_epoch != selected.key_epoch()
            || self.payload.parent != expected_parent
            || self.payload.sequence != expected_sequence
        {
            return Err(CoreError::InvalidRefTransition(
                "repository, publisher, parent, sequence, or epoch mismatch".into(),
            ));
        }
        validate_refs(&self.payload.refs)?;
        selected
            .repository_key()
            .verifying_key()?
            .verify_strict(
                &signature_frame(&self.payload)?,
                &self.signature.signature(),
            )
            .map_err(|_| CoreError::InvalidSignature)?;
        Ok(RefState {
            repository,
            publisher: self.payload.publisher,
            transition: self.transition_id()?,
            sequence: self.payload.sequence,
            refs: self.payload.refs.clone(),
        })
    }
}

fn signature_frame(payload: &RefTransitionPayload) -> Result<Vec<u8>, CoreError> {
    let payload = payload.canonical_bytes()?;
    let mut frame = Vec::with_capacity(REF_TRANSITION_SIGNATURE_DOMAIN.len() + payload.len());
    frame.extend_from_slice(REF_TRANSITION_SIGNATURE_DOMAIN);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn validate_refs(refs: &[RefTarget]) -> Result<(), CoreError> {
    if !refs.windows(2).all(|pair| pair[0].name < pair[1].name) {
        return Err(CoreError::InvalidRefTransition(
            "references must be sorted and unique".into(),
        ));
    }
    for reference in refs {
        if !reference.name.starts_with("refs/")
            || reference.name.contains("..")
            || reference
                .name
                .contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
        {
            return Err(CoreError::InvalidRefTransition(
                "invalid reference name".into(),
            ));
        }
        reference.target.validate()?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RefState {
    #[n(0)]
    pub repository: RepositoryId,
    #[n(1)]
    pub publisher: StyreneIdentity,
    #[n(2)]
    pub transition: Digest,
    #[n(3)]
    pub sequence: u64,
    #[n(4)]
    pub refs: Vec<RefTarget>,
}

impl RefState {
    pub fn target(&self, name: &str) -> Option<&GitObjectId> {
        self.refs
            .binary_search_by(|reference| reference.name.as_str().cmp(name))
            .ok()
            .map(|index| &self.refs[index].target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Digest, PublicKey};

    fn fixture() -> (
        RepositoryId,
        SignerBinding,
        SignerSelection,
        ed25519_dalek::SigningKey,
    ) {
        let identity_key = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
        let repository_key = ed25519_dalek::SigningKey::from_bytes(&[2; 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            3,
        )
        .expect("binding");
        let selected = binding.selection().expect("selection");
        (
            RepositoryId::new(Digest::new([4; 32])),
            binding,
            selected,
            repository_key,
        )
    }

    #[test]
    fn reference_signature_frame_has_stable_bytes() {
        let (repository, binding, _, repository_key) = fixture();
        let transition = RefTransition::signed(
            repository,
            &binding,
            &repository_key,
            None,
            vec![RefTarget {
                name: "refs/heads/main".into(),
                target: GitObjectId::sha256([5; 32]),
            }],
        )
        .expect("transition");
        assert_eq!(
            hex::encode(signature_frame(&transition.payload).expect("signature frame")),
            concat!(
                "73747972656e652f6769742f7265662d7472616e736974696f6e2d7369676e61747572652f763100",
                "870158200404040404040404040404040404040404040404040404040404040404040404",
                "5034750f98bd59fcfc946da45aaabe933b03f60081826f726566732f68656164732f6d61696e",
                "826673686132353698200505050505050505050505050505050505050505050505050505050505050505"
            )
        );
    }

    #[test]
    fn reference_verification_rejects_another_domain_signature() {
        let (repository, binding, selected, repository_key) = fixture();
        let mut transition = RefTransition::signed(
            repository,
            &binding,
            &repository_key,
            None,
            vec![RefTarget {
                name: "refs/heads/main".into(),
                target: GitObjectId::sha256([5; 32]),
            }],
        )
        .expect("transition");
        let mut wrong_frame = b"styrene/git/identity-transition-approval/v1\0".to_vec();
        wrong_frame.extend_from_slice(
            &transition
                .payload
                .canonical_bytes()
                .expect("cross-domain payload"),
        );
        transition.signature = SignatureBytes::new(repository_key.sign(&wrong_frame).to_bytes());
        assert_eq!(
            transition.verify(repository, &binding, &selected, None),
            Err(CoreError::InvalidSignature)
        );
    }
}
