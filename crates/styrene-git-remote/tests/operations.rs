use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::str::FromStr;

use styrene_git_core::{
    CanonicalCbor, Digest, GitObjectId, RefState, RefTarget, RepositoryId, StyreneIdentity,
};
use styrene_git_ipc::{
    Capability, CapabilitySet, Limits, OperationId, PushDisposition, PushRefListing, PushResult,
    Request, RequestBody, Response, ResponseBody, IPC_VERSION,
};
use styrene_git_remote::{
    AuthenticatedGitTransport, ClientConfig, FetchCommand, GitError, GitIpcClient, GitObjectFormat,
    GitPlumbing, GitRemoteUrl, HelperError, PushCommand, RemoteSession, TransportError,
};

struct RecordingTransport {
    responses: VecDeque<ResponseBody>,
    requests: Vec<Request>,
}

impl RecordingTransport {
    fn new(responses: impl IntoIterator<Item = ResponseBody>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            requests: Vec::new(),
        }
    }
}

impl AuthenticatedGitTransport for RecordingTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: u64,
    ) -> Result<Vec<u8>, TransportError> {
        let request = Request::from_canonical_bytes(request)
            .map_err(|error| TransportError::new(error.to_string()))?;
        let id = request.id;
        self.requests.push(request);
        Response::new(
            id,
            self.responses
                .pop_front()
                .ok_or_else(|| TransportError::new("missing response"))?,
        )
        .canonical_bytes()
        .map_err(|error| TransportError::new(error.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PackCall {
    includes: Vec<GitObjectId>,
    excludes: Vec<GitObjectId>,
    max_revisions: usize,
    max_pack_bytes: u64,
}

struct FakeGit {
    format: GitObjectFormat,
    prerequisites: Vec<GitObjectId>,
    resolutions: BTreeMap<String, GitObjectId>,
    present: RefCell<BTreeSet<GitObjectId>>,
    objects_after_install: BTreeSet<GitObjectId>,
    installed: RefCell<Vec<Vec<u8>>>,
    pack_calls: RefCell<Vec<PackCall>>,
    push_pack: Vec<u8>,
}

impl FakeGit {
    fn sha256() -> Self {
        Self {
            format: GitObjectFormat::Sha256,
            prerequisites: Vec::new(),
            resolutions: BTreeMap::new(),
            present: RefCell::new(BTreeSet::new()),
            objects_after_install: BTreeSet::new(),
            installed: RefCell::new(Vec::new()),
            pack_calls: RefCell::new(Vec::new()),
            push_pack: Vec::new(),
        }
    }
}

impl GitPlumbing for FakeGit {
    fn object_format(&self) -> Result<GitObjectFormat, GitError> {
        Ok(self.format)
    }

    fn resolve_revision(&self, revision: &str) -> Result<GitObjectId, GitError> {
        self.resolutions
            .get(revision)
            .cloned()
            .ok_or(GitError::InvalidRevision)
    }

    fn has_object(&self, object: &GitObjectId) -> Result<bool, GitError> {
        Ok(self.present.borrow().contains(object))
    }

    fn local_prerequisites(&self, _max_count: usize) -> Result<Vec<GitObjectId>, GitError> {
        Ok(self.prerequisites.clone())
    }

    fn create_push_pack(
        &self,
        includes: &[GitObjectId],
        excludes: &[GitObjectId],
        max_revisions: usize,
        max_pack_bytes: u64,
    ) -> Result<Vec<u8>, GitError> {
        self.pack_calls.borrow_mut().push(PackCall {
            includes: includes.to_vec(),
            excludes: excludes.to_vec(),
            max_revisions,
            max_pack_bytes,
        });
        Ok(self.push_pack.clone())
    }

    fn install_fetch_pack(&self, pack: &[u8], _max_pack_bytes: u64) -> Result<(), GitError> {
        self.installed.borrow_mut().push(pack.to_vec());
        self.present
            .borrow_mut()
            .extend(self.objects_after_install.iter().cloned());
        Ok(())
    }
}

fn object(byte: u8) -> GitObjectId {
    GitObjectId::sha256([byte; 32])
}

fn repository() -> RepositoryId {
    RepositoryId::new(Digest::new([2; 32]))
}

fn url(publisher: Option<StyreneIdentity>) -> GitRemoteUrl {
    let base = format!("styrene:///git/v1/{}", repository().digest().base32());
    GitRemoteUrl::from_str(&publisher.map_or(base.clone(), |publisher| {
        format!("{base}/publisher/{publisher}")
    }))
    .expect("URL")
}

fn limits() -> Limits {
    Limits {
        max_request_bytes: 16 * 1024,
        max_pack_bytes: 1024,
        max_ref_updates: 2,
        max_wants: 4,
        max_haves: 4,
        max_observations_per_page: 4,
    }
}

fn capabilities() -> ResponseBody {
    ResponseBody::Capabilities(CapabilitySet {
        protocol_version: IPC_VERSION,
        granted: vec![Capability::ReadRepository, Capability::WriteOwnNamespace],
        limits: limits(),
    })
}

fn client(responses: impl IntoIterator<Item = ResponseBody>) -> GitIpcClient<RecordingTransport> {
    GitIpcClient::new(
        RecordingTransport::new(responses),
        ClientConfig {
            initial_request_id: 1,
            max_response_bytes: 16 * 1024,
        },
    )
    .expect("client")
}

#[test]
fn fetch_batch_deduplicates_wants_bounds_haves_and_installs_once() {
    let first = object(1);
    let second = object(2);
    let have = object(3);
    let mut git = FakeGit::sha256();
    git.prerequisites = vec![have.clone(), have.clone()];
    git.objects_after_install = BTreeSet::from([first.clone(), second.clone()]);
    let mut session = RemoteSession::new(
        url(None),
        client([
            capabilities(),
            ResponseBody::FetchPack(vec![0x50, 0x41, 0x43, 0x4b]),
        ]),
        git,
    );
    session
        .fetch_batch(&[
            FetchCommand {
                object: second.clone(),
                reference: "refs/tags/v1".into(),
            },
            FetchCommand {
                object: first.clone(),
                reference: "refs/heads/main".into(),
            },
            FetchCommand {
                object: second.clone(),
                reference: "refs/remotes/duplicate".into(),
            },
        ])
        .expect("fetch batch");

    let (_, client, git) = session.into_parts();
    let transport = client.into_transport();
    assert_eq!(transport.requests.len(), 2);
    assert!(matches!(
        &transport.requests[1].body,
        RequestBody::Fetch {
            repository: actual,
            view: styrene_git_ipc::RepositoryView::Canonical,
            wants,
            haves,
        } if *actual == repository()
            && wants == &vec![first, second]
            && haves == &vec![have]
    ));
    assert_eq!(git.installed.into_inner(), [b"PACK".to_vec()]);
}

#[test]
fn push_batch_refreshes_own_refs_and_sends_one_atomic_update() {
    let publisher_view = StyreneIdentity::new([9; 16]);
    let old_main = object(1);
    let old_tag = object(2);
    let new_main = object(3);
    let listing = PushRefListing {
        repository: repository(),
        refs: vec![
            RefTarget {
                name: "refs/heads/main".into(),
                target: old_main.clone(),
            },
            RefTarget {
                name: "refs/tags/v1".into(),
                target: old_tag.clone(),
            },
        ],
    };
    let result = PushResult {
        disposition: PushDisposition::Applied,
        state: RefState {
            repository: repository(),
            publisher: StyreneIdentity::new([8; 16]),
            transition: Digest::new([7; 32]),
            sequence: 4,
            refs: vec![
                RefTarget {
                    name: "refs/heads/main".into(),
                    target: new_main.clone(),
                },
                RefTarget {
                    name: "refs/heads/other".into(),
                    target: object(4),
                },
                RefTarget {
                    name: "refs/tags/other".into(),
                    target: object(5),
                },
            ],
        },
        publication: OperationId::new([6; 16]),
    };
    let mut git = FakeGit::sha256();
    git.resolutions
        .insert("refs/heads/local".into(), new_main.clone());
    git.push_pack = b"PACK".to_vec();
    let mut session = RemoteSession::new(
        url(Some(publisher_view)),
        client([
            capabilities(),
            ResponseBody::PushRefs(listing),
            ResponseBody::PushCommitted(result.clone()),
        ]),
        git,
    );
    assert_eq!(
        session
            .push_batch(&[
                PushCommand {
                    source: Some("refs/heads/local".into()),
                    destination: "refs/heads/main".into(),
                    force: true,
                },
                PushCommand {
                    source: None,
                    destination: "refs/tags/v1".into(),
                    force: false,
                },
            ])
            .expect("push batch"),
        result
    );

    let (_, client, git) = session.into_parts();
    let transport = client.into_transport();
    assert_eq!(transport.requests.len(), 3);
    assert!(matches!(
        transport.requests[1].body,
        RequestBody::ListPushRefs { repository: actual } if actual == repository()
    ));
    let RequestBody::Push {
        repository: actual,
        updates,
        pack,
    } = &transport.requests[2].body
    else {
        panic!("third request must be push");
    };
    assert_eq!(*actual, repository());
    assert_eq!(pack, b"PACK");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].expected, Some(old_main.clone()));
    assert_eq!(updates[0].new, Some(new_main.clone()));
    assert!(updates[0].force);
    assert_eq!(updates[1].expected, Some(old_tag.clone()));
    assert_eq!(updates[1].new, None);
    assert!(!updates[1].force);
    assert_eq!(
        git.pack_calls.into_inner(),
        [PackCall {
            includes: vec![new_main],
            excludes: vec![old_main, old_tag],
            max_revisions: 6,
            max_pack_bytes: 1024,
        }]
    );
}

#[test]
fn mapping_rejects_missing_wants_and_duplicate_push_destinations() {
    let wanted = object(1);
    let mut fetch = RemoteSession::new(
        url(None),
        client([capabilities(), ResponseBody::FetchPack(b"PACK".to_vec())]),
        FakeGit::sha256(),
    );
    assert!(matches!(
        fetch.fetch_batch(&[FetchCommand {
            object: wanted.clone(),
            reference: "refs/heads/main".into(),
        }]),
        Err(HelperError::WantedObjectMissing(object)) if object == wanted
    ));

    let listing = PushRefListing {
        repository: repository(),
        refs: Vec::new(),
    };
    let mut git = FakeGit::sha256();
    git.resolutions.insert("HEAD".into(), object(2));
    let mut push = RemoteSession::new(
        url(None),
        client([capabilities(), ResponseBody::PushRefs(listing)]),
        git,
    );
    assert!(matches!(
        push.push_batch(&[
            PushCommand {
                source: Some("HEAD".into()),
                destination: "refs/heads/main".into(),
                force: false,
            },
            PushCommand {
                source: None,
                destination: "refs/heads/main".into(),
                force: true,
            },
        ]),
        Err(HelperError::DuplicateDestination(destination))
            if destination == "refs/heads/main"
    ));
    assert_eq!(push.client().transport().requests.len(), 2);
}

#[test]
fn fetch_limits_unique_wants_and_rejects_invalid_objects_before_fetching() {
    let wanted = object(1);
    let mut duplicates = RemoteSession::new(
        url(None),
        client([capabilities(), ResponseBody::FetchPack(b"PACK".to_vec())]),
        {
            let mut git = FakeGit::sha256();
            git.objects_after_install = BTreeSet::from([wanted.clone()]);
            git
        },
    );
    let commands = (0..5)
        .map(|index| FetchCommand {
            object: wanted.clone(),
            reference: format!("refs/heads/duplicate-{index}"),
        })
        .collect::<Vec<_>>();
    duplicates
        .fetch_batch(&commands)
        .expect("duplicates count as one wanted object");
    assert_eq!(duplicates.client().transport().requests.len(), 2);

    let mut malformed = RemoteSession::new(url(None), client([capabilities()]), FakeGit::sha256());
    assert!(matches!(
        malformed.fetch_batch(&[FetchCommand {
            object: GitObjectId {
                algorithm: "sha256".into(),
                bytes: vec![0; 31],
            },
            reference: "refs/heads/main".into(),
        }]),
        Err(HelperError::InvalidObjectId)
    ));
    assert_eq!(malformed.client().transport().requests.len(), 1);
}

#[test]
fn mapping_rejects_malformed_reference_components() {
    let mut session = RemoteSession::new(url(None), client([capabilities()]), FakeGit::sha256());
    assert!(matches!(
        session.fetch_batch(&[FetchCommand {
            object: object(1),
            reference: "refs/heads/.hidden".into(),
        }]),
        Err(HelperError::InvalidReference(reference)) if reference == "refs/heads/.hidden"
    ));
    assert_eq!(session.client().transport().requests.len(), 1);
}
