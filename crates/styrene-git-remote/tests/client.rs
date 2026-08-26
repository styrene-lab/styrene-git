use std::collections::VecDeque;

use styrene_git_core::{CanonicalCbor, Digest, RefState, RepositoryId, StyreneIdentity};
use styrene_git_ipc::{
    Capability, CapabilitySet, ErrorCode, Limits, OperationId, PushDisposition, PushRefListing,
    PushResult, RefListing, RepositoryView, Request, RequestBody, RequestId, Response,
    ResponseBody, StableError, IPC_VERSION,
};
use styrene_git_remote::{
    AuthenticatedGitTransport, ClientConfig, ClientError, GitIpcClient, TransportError,
};

enum Step {
    Body(ResponseBody),
    WrongId(ResponseBody),
    Version(u16, ResponseBody),
    Raw(Vec<u8>),
    Failure(&'static str),
}

#[derive(Default)]
struct ScriptedTransport {
    steps: VecDeque<Step>,
    requests: Vec<Request>,
    response_limits: Vec<u64>,
}

impl ScriptedTransport {
    fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            requests: Vec::new(),
            response_limits: Vec::new(),
        }
    }
}

impl AuthenticatedGitTransport for ScriptedTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: u64,
    ) -> Result<Vec<u8>, TransportError> {
        let request = Request::from_canonical_bytes(request)
            .map_err(|error| TransportError::new(error.to_string()))?;
        let id = request.id;
        self.requests.push(request);
        self.response_limits.push(max_response_bytes);
        match self
            .steps
            .pop_front()
            .ok_or_else(|| TransportError::new("missing scripted response"))?
        {
            Step::Body(body) => Response::new(id, body)
                .canonical_bytes()
                .map_err(|error| TransportError::new(error.to_string())),
            Step::WrongId(body) => Response::new(RequestId::new([0xff; 16]), body)
                .canonical_bytes()
                .map_err(|error| TransportError::new(error.to_string())),
            Step::Version(version, body) => {
                let mut response = Response::new(id, body);
                response.version = version;
                response
                    .canonical_bytes()
                    .map_err(|error| TransportError::new(error.to_string()))
            }
            Step::Raw(bytes) => Ok(bytes),
            Step::Failure(detail) => Err(TransportError::new(detail)),
        }
    }
}

fn repository(byte: u8) -> RepositoryId {
    RepositoryId::new(Digest::new([byte; 32]))
}

fn limits() -> Limits {
    Limits {
        max_request_bytes: 4096,
        max_pack_bytes: 16,
        max_ref_updates: 2,
        max_wants: 2,
        max_haves: 2,
        max_observations_per_page: 2,
    }
}

fn capabilities(granted: Vec<Capability>) -> ResponseBody {
    ResponseBody::Capabilities(CapabilitySet {
        protocol_version: IPC_VERSION,
        granted,
        limits: limits(),
    })
}

fn all_capabilities() -> Vec<Capability> {
    vec![
        Capability::ReadRepository,
        Capability::WriteOwnNamespace,
        Capability::StartSynchronization,
        Capability::ReadOperations,
        Capability::CancelOperations,
    ]
}

fn client(steps: impl IntoIterator<Item = Step>) -> GitIpcClient<ScriptedTransport> {
    GitIpcClient::new(
        ScriptedTransport::new(steps),
        ClientConfig {
            initial_request_id: 7,
            max_response_bytes: 8192,
        },
    )
    .expect("client")
}

#[test]
fn negotiates_then_sends_canonical_correlated_requests_with_fresh_ids() {
    let repository = repository(2);
    let listing = RefListing {
        repository,
        view: RepositoryView::Canonical,
        refs: Vec::new(),
        head: Some("refs/heads/main".into()),
    };
    let mut client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Body(ResponseBody::Refs(listing.clone())),
    ]);

    assert_eq!(
        client.negotiate().expect("capabilities").granted,
        all_capabilities()
    );
    assert_eq!(
        client
            .list_refs(repository, RepositoryView::Canonical)
            .expect("listing"),
        listing
    );
    let transport = client.into_transport();
    assert_eq!(transport.requests.len(), 2);
    assert_eq!(
        transport.requests[0].id,
        RequestId::new(7_u128.to_be_bytes())
    );
    assert_eq!(
        transport.requests[1].id,
        RequestId::new(8_u128.to_be_bytes())
    );
    assert!(matches!(
        transport.requests[0].body,
        RequestBody::GetCapabilities
    ));
    assert!(matches!(
        transport.requests[1].body,
        RequestBody::ListRefs { repository: actual, view: RepositoryView::Canonical }
            if actual == repository
    ));
    assert_eq!(transport.response_limits, [8192, 8192]);
}

#[test]
fn enforces_negotiated_capabilities_and_limits_before_writing() {
    let mut client = client([Step::Body(capabilities(vec![Capability::ReadRepository]))]);
    client.negotiate().expect("capabilities");

    assert!(matches!(
        client.list_push_refs(repository(2)),
        Err(ClientError::CapabilityDenied(Capability::WriteOwnNamespace))
    ));
    assert!(matches!(
        client.fetch(
            repository(2),
            RepositoryView::Canonical,
            vec![
                styrene_git_core::GitObjectId::sha256([1; 32]),
                styrene_git_core::GitObjectId::sha256([2; 32]),
                styrene_git_core::GitObjectId::sha256([3; 32]),
            ],
            Vec::new(),
        ),
        Err(ClientError::CountLimitExceeded {
            field: "wants",
            limit: 2
        })
    ));
    assert_eq!(client.transport().requests.len(), 1);
}

#[test]
fn enforces_encoded_request_pack_and_response_limits() {
    let mut tiny_request_limits = limits();
    tiny_request_limits.max_request_bytes = 1;
    let mut request_client = client([Step::Body(ResponseBody::Capabilities(CapabilitySet {
        protocol_version: IPC_VERSION,
        granted: all_capabilities(),
        limits: tiny_request_limits,
    }))]);
    request_client.negotiate().expect("request capabilities");
    assert!(matches!(
        request_client.list_refs(repository(2), RepositoryView::Canonical),
        Err(ClientError::RequestTooLarge { limit: 1 })
    ));
    assert_eq!(request_client.transport().requests.len(), 1);

    let mut pack_client = client([Step::Body(capabilities(all_capabilities()))]);
    pack_client.negotiate().expect("pack capabilities");
    assert!(matches!(
        pack_client.push(repository(2), Vec::new(), vec![0; 17]),
        Err(ClientError::ByteLimitExceeded {
            field: "pack",
            limit: 16
        })
    ));
    assert_eq!(pack_client.transport().requests.len(), 1);

    let mut response_client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Body(ResponseBody::FetchPack(vec![0; 17])),
    ]);
    response_client.negotiate().expect("response capabilities");
    assert!(matches!(
        response_client.fetch(
            repository(2),
            RepositoryView::Canonical,
            Vec::new(),
            Vec::new()
        ),
        Err(ClientError::ByteLimitExceeded {
            field: "pack",
            limit: 16
        })
    ));
    assert!(matches!(
        response_client.list_refs(repository(2), RepositoryView::Canonical),
        Err(ClientError::SessionFailed)
    ));
}

#[test]
fn mismatched_response_context_fails_and_poisoned_session_is_not_reused() {
    let requested = repository(2);
    let mut client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Body(ResponseBody::Refs(RefListing {
            repository: repository(3),
            view: RepositoryView::Canonical,
            refs: Vec::new(),
            head: None,
        })),
    ]);
    client.negotiate().expect("capabilities");
    assert!(matches!(
        client.list_refs(requested, RepositoryView::Canonical),
        Err(ClientError::UnexpectedResponse)
    ));
    assert!(matches!(
        client.list_refs(requested, RepositoryView::Canonical),
        Err(ClientError::SessionFailed)
    ));
    assert_eq!(client.transport().requests.len(), 2);
}

#[test]
fn response_id_version_encoding_and_size_fail_closed() {
    let cases = [
        Step::WrongId(capabilities(all_capabilities())),
        Step::Version(IPC_VERSION + 1, capabilities(all_capabilities())),
        Step::Raw(vec![0xff]),
        Step::Raw(vec![0; 8193]),
    ];
    for step in cases {
        let mut client = client([step]);
        assert!(client.negotiate().is_err());
        assert!(matches!(
            client.negotiate(),
            Err(ClientError::SessionFailed)
        ));
        assert_eq!(client.transport().requests.len(), 1);
    }
}

#[test]
fn push_transport_failure_is_indeterminate_and_is_never_retried() {
    let mut client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Failure("EOF after request write"),
    ]);
    client.negotiate().expect("capabilities");
    assert!(matches!(
        client.push(repository(2), Vec::new(), Vec::new()),
        Err(ClientError::PushIndeterminate { source })
            if matches!(*source, ClientError::Transport(_))
    ));
    assert!(matches!(
        client.push(repository(2), Vec::new(), Vec::new()),
        Err(ClientError::SessionFailed)
    ));
    assert_eq!(client.transport().requests.len(), 2);
}

#[test]
fn validated_remote_push_rejection_is_not_indeterminate() {
    let repository = repository(2);
    let remote_error = StableError {
        code: ErrorCode::Conflict,
        detail: Some("stale destination".into()),
    };
    let push_refs = PushRefListing {
        repository,
        refs: Vec::new(),
    };
    let mut client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Body(ResponseBody::Error(remote_error.clone())),
        Step::Body(ResponseBody::PushRefs(push_refs.clone())),
    ]);
    client.negotiate().expect("capabilities");
    assert!(matches!(
        client.push(repository, Vec::new(), Vec::new()),
        Err(ClientError::Remote(error)) if error == remote_error
    ));
    assert_eq!(
        client
            .list_push_refs(repository)
            .expect("session remains valid"),
        push_refs
    );
    assert_eq!(client.transport().requests.len(), 3);
}

#[test]
fn push_response_mismatch_is_indeterminate() {
    let requested = repository(2);
    let result = PushResult {
        disposition: PushDisposition::Applied,
        state: RefState {
            repository: repository(3),
            publisher: StyreneIdentity::new([4; 16]),
            transition: Digest::new([5; 32]),
            sequence: 0,
            refs: Vec::new(),
        },
        publication: OperationId::new([6; 16]),
    };
    let mut client = client([
        Step::Body(capabilities(all_capabilities())),
        Step::Body(ResponseBody::PushCommitted(result)),
    ]);
    client.negotiate().expect("capabilities");
    assert!(matches!(
        client.push(requested, Vec::new(), Vec::new()),
        Err(ClientError::PushIndeterminate { source })
            if matches!(*source, ClientError::UnexpectedResponse)
    ));
}
