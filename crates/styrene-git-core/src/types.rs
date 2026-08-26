//! Fixed-size identifiers and cryptographic values.

use std::{fmt, str::FromStr};

use data_encoding::BASE32_NOPAD;
use minicbor::{Decode, Decoder, Encode, Encoder};
use sha2::{Digest as _, Sha256};

use crate::CoreError;

macro_rules! fixed_bytes {
    ($name:ident, $size:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; $size]);

        impl $name {
            pub const fn new(bytes: [u8; $size]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }
        }

        impl<C> Encode<C> for $name {
            fn encode<W: minicbor::encode::Write>(
                &self,
                encoder: &mut Encoder<W>,
                _context: &mut C,
            ) -> Result<(), minicbor::encode::Error<W::Error>> {
                encoder.bytes(&self.0)?;
                Ok(())
            }
        }

        impl<'bytes, C> Decode<'bytes, C> for $name {
            fn decode(
                decoder: &mut Decoder<'bytes>,
                _context: &mut C,
            ) -> Result<Self, minicbor::decode::Error> {
                let bytes = decoder.bytes()?;
                let fixed: [u8; $size] = bytes
                    .try_into()
                    .map_err(|_| minicbor::decode::Error::message("invalid fixed byte length"))?;
                Ok(Self(fixed))
            }
        }
    };
}

fixed_bytes!(Digest, 32);
fixed_bytes!(PublicKey, 32);
fixed_bytes!(SignatureBytes, 64);
fixed_bytes!(StyreneIdentity, 16);

impl Digest {
    pub fn base32(&self) -> String {
        BASE32_NOPAD.encode(&self.0).to_ascii_lowercase()
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.base32())
    }
}

impl PublicKey {
    pub fn verifying_key(&self) -> Result<ed25519_dalek::VerifyingKey, CoreError> {
        ed25519_dalek::VerifyingKey::from_bytes(&self.0).map_err(|_| CoreError::InvalidSignature)
    }
}

impl SignatureBytes {
    pub fn signature(&self) -> ed25519_dalek::Signature {
        ed25519_dalek::Signature::from_bytes(&self.0)
    }
}

impl StyreneIdentity {
    pub fn from_signing_key(key: &ed25519_dalek::SigningKey) -> Self {
        Self::from_public_key(&PublicKey::new(key.verifying_key().to_bytes()))
    }

    pub fn from_public_key(key: &PublicKey) -> Self {
        let digest = Sha256::digest(key.as_bytes());
        let mut identity = [0_u8; 16];
        identity.copy_from_slice(&digest[..16]);
        Self(identity)
    }

    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(value: &str) -> Result<Self, CoreError> {
        if value.len() != 32 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(CoreError::InvalidIdentity(
                "identity must be 32 lowercase hexadecimal characters".into(),
            ));
        }
        let bytes = hex::decode(value)
            .map_err(|_| CoreError::InvalidIdentity("identity is not hexadecimal".into()))?;
        let fixed = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidIdentity("identity has the wrong length".into()))?;
        Ok(Self(fixed))
    }
}

impl fmt::Display for StyreneIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.hex())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
#[cbor(transparent)]
pub struct RepositoryId(#[n(0)] Digest);

impl RepositoryId {
    pub const fn new(digest: Digest) -> Self {
        Self(digest)
    }

    pub const fn digest(&self) -> &Digest {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "styrene:git:v1:{}", self.0)
    }
}

impl FromStr for RepositoryId {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let encoded = value.strip_prefix("styrene:git:v1:").ok_or_else(|| {
            CoreError::InvalidRepositoryId("unsupported repository identifier prefix".into())
        })?;
        if encoded.len() != 52 || encoded.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(CoreError::InvalidRepositoryId(
                "digest must be canonical lowercase base32".into(),
            ));
        }
        let bytes = BASE32_NOPAD
            .decode(encoded.to_ascii_uppercase().as_bytes())
            .map_err(|_| CoreError::InvalidRepositoryId("digest is not valid base32".into()))?;
        let digest = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidRepositoryId("digest has the wrong length".into()))?;
        Ok(Self(Digest::new(digest)))
    }
}
