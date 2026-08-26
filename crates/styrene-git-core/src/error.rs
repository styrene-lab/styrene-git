//! Typed failures at the repository trust boundary.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("CBOR encoding failed: {0}")]
    Encode(String),
    #[error("CBOR decoding failed: {0}")]
    Decode(String),
    #[error("signed input is not deterministic canonical CBOR")]
    NonCanonical,
    #[error("invalid repository identity: {0}")]
    InvalidIdentity(String),
    #[error("invalid repository identifier: {0}")]
    InvalidRepositoryId(String),
    #[error("invalid signer binding: {0}")]
    InvalidBinding(String),
    #[error("invalid identity transition: {0}")]
    InvalidIdentityTransition(String),
    #[error("invalid reference transition: {0}")]
    InvalidRefTransition(String),
    #[error("signature verification failed")]
    InvalidSignature,
}
