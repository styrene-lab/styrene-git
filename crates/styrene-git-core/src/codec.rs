//! Deterministic CBOR encoding used by every hashed or signed value.

use minicbor::{Decode, Encode};

use crate::CoreError;

pub trait CanonicalCbor: for<'bytes> Decode<'bytes, ()> + Encode<()> + Sized {
    fn canonical_bytes(&self) -> Result<Vec<u8>, CoreError> {
        minicbor::to_vec(self).map_err(|error| CoreError::Encode(error.to_string()))
    }

    fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut decoder = minicbor::Decoder::new(bytes);
        let value = decoder
            .decode::<Self>()
            .map_err(|error| CoreError::Decode(error.to_string()))?;
        if decoder.position() != bytes.len() {
            return Err(CoreError::NonCanonical);
        }
        if value.canonical_bytes()? != bytes {
            return Err(CoreError::NonCanonical);
        }
        Ok(value)
    }
}

impl<T> CanonicalCbor for T where T: for<'bytes> Decode<'bytes, ()> + Encode<()> {}

pub(crate) fn domain_digest(domain: &[u8], value: &[u8]) -> crate::Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(value);
    crate::Digest::new(*hasher.finalize().as_bytes())
}
