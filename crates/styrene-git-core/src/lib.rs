//! Network-independent, self-certifying repository state for Styrene Git.

mod canonical;
mod codec;
mod error;
mod identity;
mod refs;
mod types;

pub use canonical::{derive_canonical_head, CanonicalDecision};
pub use codec::CanonicalCbor;
pub use error::CoreError;
pub use identity::{
    Approval, IdentityDocument, IdentityState, IdentityTransition, SignerBinding, SignerSelection,
    Visibility,
};
pub use refs::{GitObjectId, RefState, RefTarget, RefTransition};
pub use types::{Digest, PublicKey, RepositoryId, SignatureBytes, StyreneIdentity};
