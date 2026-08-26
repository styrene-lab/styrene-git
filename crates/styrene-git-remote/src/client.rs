use styrene_git_core::{CanonicalCbor, GitObjectId, RepositoryId};
use styrene_git_ipc::{
    Capability, CapabilitySet, ObservationPage, Operation, OperationId, OperationKind,
    PushRefListing, PushResult, RefListing, RefUpdate, RepositoryView, Request, RequestBody,
    RequestId, Response, ResponseBody, StableError, IPC_VERSION,
};

const BOOTSTRAP_MAX_REQUEST_BYTES: u64 = 4096;

pub trait AuthenticatedGitTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: u64,
    ) -> Result<Vec<u8>, TransportError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub initial_request_id: u128,
    pub max_response_bytes: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            initial_request_id: 1,
            max_response_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct GitIpcClient<Transport> {
    transport: Transport,
    next_request_id: Option<u128>,
    max_response_bytes: u64,
    capabilities: Option<CapabilitySet>,
    failed: bool,
}

impl<Transport: AuthenticatedGitTransport> GitIpcClient<Transport> {
    pub fn new(transport: Transport, config: ClientConfig) -> Result<Self, ClientError> {
        if config.max_response_bytes == 0 {
            return Err(ClientError::InvalidConfig);
        }
        Ok(Self {
            transport,
            next_request_id: Some(config.initial_request_id),
            max_response_bytes: config.max_response_bytes,
            capabilities: None,
            failed: false,
        })
    }

    pub fn capabilities(&self) -> Option<&CapabilitySet> {
        self.capabilities.as_ref()
    }

    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    pub fn into_transport(self) -> Transport {
        self.transport
    }

    pub fn negotiate(&mut self) -> Result<CapabilitySet, ClientError> {
        if let Some(capabilities) = &self.capabilities {
            return Ok(capabilities.clone());
        }
        let body = self.send(RequestBody::GetCapabilities, false)?;
        let ResponseBody::Capabilities(capabilities) = body else {
            return Err(ClientError::UnexpectedResponse);
        };
        if capabilities.protocol_version != IPC_VERSION
            || !capabilities
                .granted
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            self.failed = true;
            return Err(ClientError::InvalidCapabilities);
        }
        self.capabilities = Some(capabilities.clone());
        Ok(capabilities)
    }

    pub fn list_refs(
        &mut self,
        repository: RepositoryId,
        view: RepositoryView,
    ) -> Result<RefListing, ClientError> {
        match self.send(RequestBody::ListRefs { repository, view }, false)? {
            ResponseBody::Refs(listing) => Ok(listing),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn list_push_refs(
        &mut self,
        repository: RepositoryId,
    ) -> Result<PushRefListing, ClientError> {
        match self.send(RequestBody::ListPushRefs { repository }, false)? {
            ResponseBody::PushRefs(listing) => Ok(listing),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn fetch(
        &mut self,
        repository: RepositoryId,
        view: RepositoryView,
        wants: Vec<GitObjectId>,
        haves: Vec<GitObjectId>,
    ) -> Result<Vec<u8>, ClientError> {
        match self.send(
            RequestBody::Fetch {
                repository,
                view,
                wants,
                haves,
            },
            false,
        )? {
            ResponseBody::FetchPack(pack) => Ok(pack),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn push(
        &mut self,
        repository: RepositoryId,
        updates: Vec<RefUpdate>,
        pack: Vec<u8>,
    ) -> Result<PushResult, ClientError> {
        match self.send(
            RequestBody::Push {
                repository,
                updates,
                pack,
            },
            true,
        )? {
            ResponseBody::PushCommitted(result) => Ok(result),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn start_synchronization(
        &mut self,
        repository: RepositoryId,
        view: RepositoryView,
        labels: Vec<String>,
    ) -> Result<Operation, ClientError> {
        match self.send(
            RequestBody::StartSynchronization {
                repository,
                view,
                labels,
            },
            false,
        )? {
            ResponseBody::Operation(operation) => Ok(operation),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn get_operation(&mut self, operation: OperationId) -> Result<Operation, ClientError> {
        match self.send(RequestBody::GetOperation(operation), false)? {
            ResponseBody::Operation(operation) => Ok(operation),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn cancel_operation(&mut self, operation: OperationId) -> Result<Operation, ClientError> {
        match self.send(RequestBody::CancelOperation(operation), false)? {
            ResponseBody::Operation(operation) => Ok(operation),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub fn list_observations(
        &mut self,
        operation: OperationId,
        after: Option<u64>,
        limit: u16,
    ) -> Result<ObservationPage, ClientError> {
        match self.send(
            RequestBody::ListObservations {
                operation,
                after,
                limit,
            },
            false,
        )? {
            ResponseBody::Observations(observations) => Ok(observations),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    fn send(&mut self, body: RequestBody, mutating: bool) -> Result<ResponseBody, ClientError> {
        if self.failed {
            return Err(ClientError::SessionFailed);
        }
        self.validate_request(&body)?;
        let id = self.next_id()?;
        let request = Request::new(id, body);
        let request_bytes = request
            .canonical_bytes()
            .map_err(|error| ClientError::Encode(error.to_string()))?;
        let max_request_bytes = self
            .capabilities
            .as_ref()
            .map_or(BOOTSTRAP_MAX_REQUEST_BYTES, |capabilities| {
                capabilities.limits.max_request_bytes
            });
        if request_bytes.len() as u64 > max_request_bytes {
            return Err(ClientError::RequestTooLarge {
                limit: max_request_bytes,
            });
        }

        let response_bytes = match self
            .transport
            .exchange(&request_bytes, self.max_response_bytes)
        {
            Ok(response) => response,
            Err(error) => {
                self.failed = true;
                return Err(if mutating {
                    ClientError::PushIndeterminate {
                        source: Box::new(ClientError::Transport(error)),
                    }
                } else {
                    ClientError::Transport(error)
                });
            }
        };
        let result = self.validate_response(&request, response_bytes);
        match result {
            Ok(body) => Ok(body),
            Err(error @ ClientError::Remote(_)) => Err(error),
            Err(error) => {
                self.failed = true;
                Err(if mutating {
                    ClientError::PushIndeterminate {
                        source: Box::new(error),
                    }
                } else {
                    error
                })
            }
        }
    }

    fn next_id(&mut self) -> Result<RequestId, ClientError> {
        let value = self
            .next_request_id
            .ok_or(ClientError::RequestIdExhausted)?;
        self.next_request_id = value.checked_add(1);
        Ok(RequestId::new(value.to_be_bytes()))
    }

    fn validate_request(&self, body: &RequestBody) -> Result<(), ClientError> {
        if matches!(body, RequestBody::GetCapabilities) {
            return Ok(());
        }
        let capabilities = self
            .capabilities
            .as_ref()
            .ok_or(ClientError::NotNegotiated)?;
        let limits = capabilities.limits;
        match body {
            RequestBody::GetCapabilities => Ok(()),
            RequestBody::ListRefs { .. } => {
                require_capability(capabilities, Capability::ReadRepository)
            }
            RequestBody::ListPushRefs { .. } => {
                require_capability(capabilities, Capability::WriteOwnNamespace)
            }
            RequestBody::Fetch { wants, haves, .. } => {
                require_capability(capabilities, Capability::ReadRepository)?;
                require_count("wants", wants.len(), usize::from(limits.max_wants))?;
                require_count("haves", haves.len(), usize::from(limits.max_haves))
            }
            RequestBody::Push { updates, pack, .. } => {
                require_capability(capabilities, Capability::WriteOwnNamespace)?;
                require_count(
                    "reference updates",
                    updates.len(),
                    usize::from(limits.max_ref_updates),
                )?;
                require_bytes("pack", pack.len(), limits.max_pack_bytes)
            }
            RequestBody::StartSynchronization { .. } => {
                require_capability(capabilities, Capability::StartSynchronization)
            }
            RequestBody::GetOperation(_) | RequestBody::ListObservations { .. } => {
                require_capability(capabilities, Capability::ReadOperations)?;
                if let RequestBody::ListObservations { limit, .. } = body {
                    require_count(
                        "observations",
                        usize::from(*limit),
                        usize::from(limits.max_observations_per_page),
                    )?;
                }
                Ok(())
            }
            RequestBody::CancelOperation(_) => {
                require_capability(capabilities, Capability::CancelOperations)
            }
        }
    }

    fn validate_response(
        &self,
        request: &Request,
        bytes: Vec<u8>,
    ) -> Result<ResponseBody, ClientError> {
        if bytes.len() as u64 > self.max_response_bytes {
            return Err(ClientError::ResponseTooLarge {
                limit: self.max_response_bytes,
            });
        }
        let response = Response::from_canonical_bytes(&bytes)
            .map_err(|error| ClientError::Decode(error.to_string()))?;
        if response.version != IPC_VERSION {
            return Err(ClientError::UnsupportedVersion(response.version));
        }
        if response.id != request.id {
            return Err(ClientError::RequestIdMismatch);
        }
        if let ResponseBody::Error(error) = response.body {
            return Err(ClientError::Remote(error));
        }
        validate_response_context(&request.body, &response.body)?;
        if let Some(capabilities) = &self.capabilities {
            match &response.body {
                ResponseBody::FetchPack(pack) => {
                    require_bytes("pack", pack.len(), capabilities.limits.max_pack_bytes)?;
                }
                ResponseBody::Observations(page) => {
                    require_count(
                        "observations",
                        page.observations.len(),
                        usize::from(capabilities.limits.max_observations_per_page),
                    )?;
                }
                _ => {}
            }
        }
        Ok(response.body)
    }
}

fn require_capability(
    capabilities: &CapabilitySet,
    required: Capability,
) -> Result<(), ClientError> {
    if capabilities.granted.binary_search(&required).is_ok() {
        Ok(())
    } else {
        Err(ClientError::CapabilityDenied(required))
    }
}

fn require_count(field: &'static str, actual: usize, limit: usize) -> Result<(), ClientError> {
    if actual <= limit {
        Ok(())
    } else {
        Err(ClientError::CountLimitExceeded { field, limit })
    }
}

fn require_bytes(field: &'static str, actual: usize, limit: u64) -> Result<(), ClientError> {
    if actual as u64 <= limit {
        Ok(())
    } else {
        Err(ClientError::ByteLimitExceeded { field, limit })
    }
}

fn validate_response_context(
    request: &RequestBody,
    response: &ResponseBody,
) -> Result<(), ClientError> {
    let matches = match (request, response) {
        (RequestBody::GetCapabilities, ResponseBody::Capabilities(_)) => true,
        (RequestBody::ListRefs { repository, view }, ResponseBody::Refs(listing)) => {
            listing.repository == *repository && listing.view == *view
        }
        (RequestBody::ListPushRefs { repository }, ResponseBody::PushRefs(listing)) => {
            listing.repository == *repository
        }
        (RequestBody::Fetch { .. }, ResponseBody::FetchPack(_)) => true,
        (RequestBody::Push { repository, .. }, ResponseBody::PushCommitted(result)) => {
            result.state.repository == *repository
        }
        (
            RequestBody::StartSynchronization {
                repository, view, ..
            },
            ResponseBody::Operation(operation),
        ) => {
            operation.repository == *repository
                && operation.kind == OperationKind::Synchronize(view.clone())
        }
        (RequestBody::GetOperation(expected), ResponseBody::Operation(operation))
        | (RequestBody::CancelOperation(expected), ResponseBody::Operation(operation)) => {
            operation.id == *expected
        }
        (
            RequestBody::ListObservations { operation, .. },
            ResponseBody::Observations(observations),
        ) => observations.operation == *operation,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(ClientError::UnexpectedResponse)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct TransportError {
    detail: String,
}

impl TransportError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("IPC client configuration is invalid")]
    InvalidConfig,
    #[error("IPC request identifiers are exhausted")]
    RequestIdExhausted,
    #[error("IPC capabilities have not been negotiated")]
    NotNegotiated,
    #[error("IPC session has failed and cannot be reused")]
    SessionFailed,
    #[error("required IPC capability is not granted: {0:?}")]
    CapabilityDenied(Capability),
    #[error("IPC {field} count exceeds {limit}")]
    CountLimitExceeded { field: &'static str, limit: usize },
    #[error("IPC {field} exceeds {limit} bytes")]
    ByteLimitExceeded { field: &'static str, limit: u64 },
    #[error("IPC request exceeds {limit} bytes")]
    RequestTooLarge { limit: u64 },
    #[error("IPC response exceeds {limit} bytes")]
    ResponseTooLarge { limit: u64 },
    #[error("IPC request encoding failed: {0}")]
    Encode(String),
    #[error("IPC response decoding failed: {0}")]
    Decode(String),
    #[error("unsupported IPC response version {0}")]
    UnsupportedVersion(u16),
    #[error("IPC response request ID does not match")]
    RequestIdMismatch,
    #[error("IPC response does not match the request")]
    UnexpectedResponse,
    #[error("daemon rejected the IPC request: {0:?}")]
    Remote(StableError),
    #[error("authenticated IPC transport failed: {0}")]
    Transport(TransportError),
    #[error("push outcome is indeterminate: {source}")]
    PushIndeterminate {
        #[source]
        source: Box<ClientError>,
    },
    #[error("daemon returned invalid IPC capabilities")]
    InvalidCapabilities,
}
