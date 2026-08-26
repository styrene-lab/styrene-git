//! Network-independent request and observation types for local Git daemon IPC.

use minicbor::{Decode, Decoder, Encode, Encoder};
use styrene_git_core::{Digest, GitObjectId, RefState, RefTarget, RepositoryId, StyreneIdentity};

pub const IPC_VERSION: u16 = 1;

macro_rules! fixed_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn new(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
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
                Ok(Self(bytes.try_into().map_err(|_| {
                    minicbor::decode::Error::message("invalid IPC identifier length")
                })?))
            }
        }
    };
}

fixed_id!(RequestId);
fixed_id!(OperationId);

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Request {
    #[n(0)]
    pub version: u16,
    #[n(1)]
    pub id: RequestId,
    #[n(2)]
    pub body: RequestBody,
}

impl Request {
    pub const fn new(id: RequestId, body: RequestBody) -> Self {
        Self {
            version: IPC_VERSION,
            id,
            body,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Response {
    #[n(0)]
    pub version: u16,
    #[n(1)]
    pub id: RequestId,
    #[n(2)]
    pub body: ResponseBody,
}

impl Response {
    pub const fn new(id: RequestId, body: ResponseBody) -> Self {
        Self {
            version: IPC_VERSION,
            id,
            body,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum RepositoryView {
    #[n(0)]
    Canonical,
    #[n(1)]
    Publisher(#[n(0)] StyreneIdentity),
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum RequestBody {
    #[n(0)]
    GetCapabilities,
    #[n(1)]
    ListRefs {
        #[n(0)]
        repository: RepositoryId,
        #[n(1)]
        view: RepositoryView,
    },
    #[n(2)]
    Fetch {
        #[n(0)]
        repository: RepositoryId,
        #[n(1)]
        view: RepositoryView,
        #[n(2)]
        wants: Vec<GitObjectId>,
        #[n(3)]
        haves: Vec<GitObjectId>,
    },
    #[n(3)]
    Push {
        #[n(0)]
        repository: RepositoryId,
        #[n(1)]
        updates: Vec<RefUpdate>,
        #[n(2)]
        pack: Vec<u8>,
    },
    #[n(4)]
    StartSynchronization {
        #[n(0)]
        repository: RepositoryId,
        #[n(1)]
        view: RepositoryView,
        #[n(2)]
        labels: Vec<String>,
    },
    #[n(5)]
    GetOperation(#[n(0)] OperationId),
    #[n(6)]
    CancelOperation(#[n(0)] OperationId),
    #[n(7)]
    ListObservations {
        #[n(0)]
        operation: OperationId,
        #[n(1)]
        after: Option<u64>,
        #[n(2)]
        limit: u16,
    },
    #[n(8)]
    ListPushRefs {
        #[n(0)]
        repository: RepositoryId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RefUpdate {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub expected: Option<GitObjectId>,
    #[n(2)]
    pub new: Option<GitObjectId>,
    #[n(3)]
    pub force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum ResponseBody {
    #[n(0)]
    Capabilities(#[n(0)] CapabilitySet),
    #[n(1)]
    Refs(#[n(0)] RefListing),
    #[n(2)]
    FetchPack(#[n(0)] Vec<u8>),
    #[n(3)]
    PushCommitted(#[n(0)] PushResult),
    #[n(4)]
    Operation(#[n(0)] Operation),
    #[n(5)]
    Observations(#[n(0)] ObservationPage),
    #[n(6)]
    Error(#[n(0)] StableError),
    #[n(7)]
    PushRefs(#[n(0)] PushRefListing),
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct RefListing {
    #[n(0)]
    pub repository: RepositoryId,
    #[n(1)]
    pub view: RepositoryView,
    #[n(2)]
    pub refs: Vec<RefTarget>,
    #[n(3)]
    pub head: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct PushRefListing {
    #[n(0)]
    pub repository: RepositoryId,
    #[n(1)]
    pub refs: Vec<RefTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct PushResult {
    #[n(0)]
    pub disposition: PushDisposition,
    #[n(1)]
    pub state: RefState,
    #[n(2)]
    pub publication: OperationId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum PushDisposition {
    #[n(0)]
    Applied,
    #[n(1)]
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
#[cbor(index_only)]
pub enum Capability {
    #[n(0)]
    ReadRepository,
    #[n(1)]
    WriteOwnNamespace,
    #[n(2)]
    StartSynchronization,
    #[n(3)]
    ReadOperations,
    #[n(4)]
    CancelOperations,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct CapabilitySet {
    #[n(0)]
    pub protocol_version: u16,
    #[n(1)]
    pub granted: Vec<Capability>,
    #[n(2)]
    pub limits: Limits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Limits {
    #[n(0)]
    pub max_request_bytes: u64,
    #[n(1)]
    pub max_pack_bytes: u64,
    #[n(2)]
    pub max_ref_updates: u16,
    #[n(3)]
    pub max_wants: u16,
    #[n(4)]
    pub max_haves: u16,
    #[n(5)]
    pub max_observations_per_page: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct StableError {
    #[n(0)]
    pub code: ErrorCode,
    #[n(1)]
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(index_only)]
pub enum ErrorCode {
    #[n(0)]
    InvalidRequest,
    #[n(1)]
    UnsupportedVersion,
    #[n(2)]
    InvalidIdentifier,
    #[n(3)]
    LimitExceeded,
    #[n(4)]
    RepositoryNotFound,
    #[n(5)]
    LocalNotAvailable,
    #[n(6)]
    Unauthorized,
    #[n(7)]
    SignerUnavailable,
    #[n(8)]
    Conflict,
    #[n(9)]
    InvalidRepositoryState,
    #[n(10)]
    InvalidGitData,
    #[n(11)]
    OperationNotFound,
    #[n(12)]
    OperationNotCancellable,
    #[n(13)]
    Unavailable,
    #[n(14)]
    Internal,
}

impl ErrorCode {
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::LocalNotAvailable | Self::Unavailable)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Operation {
    #[n(0)]
    pub id: OperationId,
    #[n(1)]
    pub repository: RepositoryId,
    #[n(2)]
    pub kind: OperationKind,
    #[n(3)]
    pub state: OperationState,
    #[n(4)]
    pub last_observation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum OperationKind {
    #[n(0)]
    Synchronize(#[n(0)] RepositoryView),
    #[n(1)]
    Publish {
        #[n(0)]
        publisher: StyreneIdentity,
        #[n(1)]
        transition: Digest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum OperationState {
    #[n(0)]
    Queued,
    #[n(1)]
    Running,
    #[n(2)]
    Succeeded(#[n(0)] OperationOutcome),
    #[n(3)]
    Failed(#[n(0)] StableError),
    #[n(4)]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum OperationOutcome {
    #[n(0)]
    Synchronized {
        #[n(0)]
        applied: u32,
        #[n(1)]
        already_present: u32,
    },
    #[n(1)]
    Published(#[n(0)] Digest),
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct Observation {
    #[n(0)]
    pub operation: OperationId,
    #[n(1)]
    pub sequence: u64,
    #[n(2)]
    pub kind: ObservationKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub enum ObservationKind {
    #[n(0)]
    Queued,
    #[n(1)]
    Started(#[n(0)] u32),
    #[n(2)]
    Progress {
        #[n(0)]
        completed_bytes: u64,
        #[n(1)]
        total_bytes: Option<u64>,
    },
    #[n(3)]
    RepositoryCommitted {
        #[n(0)]
        publisher: StyreneIdentity,
        #[n(1)]
        transition: Digest,
        #[n(2)]
        sequence: u64,
    },
    #[n(4)]
    RetryScheduled(#[n(0)] u32),
    #[n(5)]
    Succeeded(#[n(0)] OperationOutcome),
    #[n(6)]
    Failed(#[n(0)] StableError),
    #[n(7)]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(array)]
pub struct ObservationPage {
    #[n(0)]
    pub operation: OperationId,
    #[n(1)]
    pub observations: Vec<Observation>,
    #[n(2)]
    pub next_after: Option<u64>,
}
