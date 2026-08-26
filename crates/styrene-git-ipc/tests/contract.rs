use std::fmt::Debug;

use styrene_git_core::{
    CanonicalCbor, Digest, GitObjectId, RefState, RefTarget, RepositoryId, StyreneIdentity,
};
use styrene_git_ipc::{
    Capability, CapabilitySet, ErrorCode, Limits, Observation, ObservationKind, ObservationPage,
    Operation, OperationId, OperationKind, OperationOutcome, OperationState, PushDisposition,
    PushRefListing, PushResult, RefListing, RefUpdate, RepositoryView, Request, RequestBody,
    RequestId, Response, ResponseBody, StableError, IPC_VERSION,
};

fn round_trip<T>(value: &T)
where
    T: CanonicalCbor + Debug + PartialEq,
{
    let bytes = value.canonical_bytes().expect("canonical encoding");
    assert_eq!(
        &T::from_canonical_bytes(&bytes).expect("canonical decoding"),
        value
    );
}

fn repository() -> RepositoryId {
    RepositoryId::new(Digest::new([2; 32]))
}

fn publisher() -> StyreneIdentity {
    StyreneIdentity::new([3; 16])
}

fn target(byte: u8) -> GitObjectId {
    GitObjectId::sha256([byte; 32])
}

fn limits() -> Limits {
    Limits {
        max_request_bytes: 1024,
        max_pack_bytes: 4096,
        max_ref_updates: 16,
        max_wants: 32,
        max_haves: 64,
        max_observations_per_page: 20,
    }
}

#[test]
fn request_contract_round_trips_every_operation() {
    let request_id = RequestId::new([1; 16]);
    let operation = OperationId::new([4; 16]);
    let view = RepositoryView::Publisher(publisher());
    let requests = [
        Request::new(request_id, RequestBody::GetCapabilities),
        Request::new(
            request_id,
            RequestBody::ListRefs {
                repository: repository(),
                view: view.clone(),
            },
        ),
        Request::new(
            request_id,
            RequestBody::Fetch {
                repository: repository(),
                view: view.clone(),
                wants: vec![target(5)],
                haves: vec![target(6)],
            },
        ),
        Request::new(
            request_id,
            RequestBody::Push {
                repository: repository(),
                updates: vec![RefUpdate {
                    name: "refs/heads/main".into(),
                    expected: Some(target(5)),
                    new: Some(target(6)),
                    force: false,
                }],
                pack: vec![7, 8],
            },
        ),
        Request::new(
            request_id,
            RequestBody::StartSynchronization {
                repository: repository(),
                view,
                labels: vec!["nearby".into(), "trusted".into()],
            },
        ),
        Request::new(request_id, RequestBody::GetOperation(operation)),
        Request::new(request_id, RequestBody::CancelOperation(operation)),
        Request::new(
            request_id,
            RequestBody::ListObservations {
                operation,
                after: Some(8),
                limit: 20,
            },
        ),
        Request::new(
            request_id,
            RequestBody::ListPushRefs {
                repository: repository(),
            },
        ),
    ];

    for request in requests {
        round_trip(&request);
    }
    assert_eq!(
        requests_golden(),
        "83015001010101010101010101010101010101820080"
    );
}

fn requests_golden() -> String {
    hex::encode(
        Request::new(RequestId::new([1; 16]), RequestBody::GetCapabilities)
            .canonical_bytes()
            .expect("golden request"),
    )
}

#[test]
fn listing_contract_has_stable_canonical_bytes() {
    let request_id = RequestId::new([1; 16]);
    let push_request = Request::new(
        request_id,
        RequestBody::ListPushRefs {
            repository: repository(),
        },
    );
    assert_eq!(
        hex::encode(push_request.canonical_bytes().expect("push-list request")),
        concat!(
            "83015001010101010101010101010101010101",
            "8208815820",
            "0202020202020202020202020202020202020202020202020202020202020202"
        )
    );

    let read_response = Response::new(
        request_id,
        ResponseBody::Refs(RefListing {
            repository: repository(),
            view: RepositoryView::Canonical,
            refs: Vec::new(),
            head: Some("refs/heads/main".into()),
        }),
    );
    assert_eq!(
        hex::encode(read_response.canonical_bytes().expect("read-list response")),
        concat!(
            "83015001010101010101010101010101010101",
            "820181845820",
            "0202020202020202020202020202020202020202020202020202020202020202",
            "820080806f726566732f68656164732f6d61696e"
        )
    );

    let push_response = Response::new(
        request_id,
        ResponseBody::PushRefs(PushRefListing {
            repository: repository(),
            refs: Vec::new(),
        }),
    );
    assert_eq!(
        hex::encode(push_response.canonical_bytes().expect("push-list response")),
        concat!(
            "83015001010101010101010101010101010101",
            "820781825820",
            "0202020202020202020202020202020202020202020202020202020202020202",
            "80"
        )
    );
}

#[test]
fn response_operation_and_observation_contracts_round_trip() {
    let request_id = RequestId::new([1; 16]);
    let operation_id = OperationId::new([4; 16]);
    let state = RefState {
        repository: repository(),
        publisher: publisher(),
        transition: Digest::new([9; 32]),
        sequence: 1,
        refs: vec![RefTarget {
            name: "refs/heads/main".into(),
            target: target(6),
        }],
    };
    let outcome = OperationOutcome::Published(state.transition);
    let operation = Operation {
        id: operation_id,
        repository: repository(),
        kind: OperationKind::Publish {
            publisher: publisher(),
            transition: state.transition,
        },
        state: OperationState::Succeeded(outcome.clone()),
        last_observation: 2,
    };
    let observations = ObservationPage {
        operation: operation_id,
        observations: vec![
            Observation {
                operation: operation_id,
                sequence: 1,
                kind: ObservationKind::Started(1),
            },
            Observation {
                operation: operation_id,
                sequence: 2,
                kind: ObservationKind::Succeeded(outcome),
            },
        ],
        next_after: Some(2),
    };
    let responses = [
        Response::new(
            request_id,
            ResponseBody::Capabilities(CapabilitySet {
                protocol_version: IPC_VERSION,
                granted: vec![Capability::ReadRepository],
                limits: limits(),
            }),
        ),
        Response::new(
            request_id,
            ResponseBody::Refs(RefListing {
                repository: repository(),
                view: RepositoryView::Canonical,
                refs: state.refs.clone(),
                head: Some("refs/heads/main".into()),
            }),
        ),
        Response::new(
            request_id,
            ResponseBody::PushRefs(PushRefListing {
                repository: repository(),
                refs: state.refs.clone(),
            }),
        ),
        Response::new(request_id, ResponseBody::FetchPack(vec![1, 2, 3])),
        Response::new(
            request_id,
            ResponseBody::PushCommitted(PushResult {
                disposition: PushDisposition::Applied,
                state,
                publication: operation_id,
            }),
        ),
        Response::new(request_id, ResponseBody::Operation(operation)),
        Response::new(request_id, ResponseBody::Observations(observations)),
        Response::new(
            request_id,
            ResponseBody::Error(StableError {
                code: ErrorCode::Unauthorized,
                detail: None,
            }),
        ),
    ];

    for response in responses {
        round_trip(&response);
    }
}

#[test]
fn stable_errors_preserve_retry_semantics_and_fixed_ids_reject_wrong_lengths() {
    assert!(ErrorCode::LocalNotAvailable.is_retryable());
    assert!(ErrorCode::Unavailable.is_retryable());
    assert!(!ErrorCode::RepositoryNotFound.is_retryable());
    assert!(!ErrorCode::Unauthorized.is_retryable());
    assert!(!ErrorCode::SignerUnavailable.is_retryable());
    assert!(!ErrorCode::Conflict.is_retryable());

    assert!(RequestId::from_canonical_bytes(&[0x41, 0]).is_err());
    assert!(OperationId::from_canonical_bytes(&[0x41, 0]).is_err());
}

#[test]
fn terminal_operation_states_and_observation_variants_are_stable_values() {
    let error = StableError {
        code: ErrorCode::LocalNotAvailable,
        detail: Some("verified repository state is not local".into()),
    };
    for state in [
        OperationState::Queued,
        OperationState::Running,
        OperationState::Succeeded(OperationOutcome::Synchronized {
            applied: 2,
            already_present: 1,
        }),
        OperationState::Failed(error.clone()),
        OperationState::Cancelled,
    ] {
        round_trip(&state);
    }
    for kind in [
        ObservationKind::Queued,
        ObservationKind::Started(1),
        ObservationKind::Progress {
            completed_bytes: 10,
            total_bytes: Some(20),
        },
        ObservationKind::RepositoryCommitted {
            publisher: publisher(),
            transition: Digest::new([9; 32]),
            sequence: 3,
        },
        ObservationKind::RetryScheduled(2),
        ObservationKind::Succeeded(OperationOutcome::Published(Digest::new([9; 32]))),
        ObservationKind::Failed(error),
        ObservationKind::Cancelled,
    ] {
        round_trip(&kind);
    }
}
