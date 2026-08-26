use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Cursor;
use std::process::{Command, Stdio};
use std::str::FromStr;

use styrene_git_core::{
    CanonicalCbor, Digest, GitObjectId, RefState, RefTarget, RepositoryId, StyreneIdentity,
};
use styrene_git_ipc::{
    Capability, CapabilitySet, ErrorCode, Limits, OperationId, PushDisposition, PushRefListing,
    PushResult, RefListing, Request, Response, ResponseBody, StableError, IPC_VERSION,
};
use styrene_git_remote::{
    run_command_loop, AuthenticatedGitTransport, ClientConfig, GitError, GitIpcClient,
    GitObjectFormat, GitPlumbing, GitRemoteUrl, RemoteSession, TransportError,
};

struct Transport {
    responses: VecDeque<ResponseBody>,
}

impl AuthenticatedGitTransport for Transport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: u64,
    ) -> Result<Vec<u8>, TransportError> {
        let request = Request::from_canonical_bytes(request)
            .map_err(|error| TransportError::new(error.to_string()))?;
        Response::new(
            request.id,
            self.responses
                .pop_front()
                .ok_or_else(|| TransportError::new("missing response"))?,
        )
        .canonical_bytes()
        .map_err(|error| TransportError::new(error.to_string()))
    }
}

struct Git {
    resolutions: BTreeMap<String, GitObjectId>,
    present: RefCell<BTreeSet<GitObjectId>>,
    installed: RefCell<usize>,
}

impl GitPlumbing for Git {
    fn object_format(&self) -> Result<GitObjectFormat, GitError> {
        Ok(GitObjectFormat::Sha256)
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
        Ok(Vec::new())
    }

    fn create_push_pack(
        &self,
        _includes: &[GitObjectId],
        _excludes: &[GitObjectId],
        _max_revisions: usize,
        _max_pack_bytes: u64,
    ) -> Result<Vec<u8>, GitError> {
        Ok(b"PUSH".to_vec())
    }

    fn install_fetch_pack(&self, _pack: &[u8], _max_pack_bytes: u64) -> Result<(), GitError> {
        *self.installed.borrow_mut() += 1;
        Ok(())
    }
}

fn repository() -> RepositoryId {
    RepositoryId::new(Digest::new([2; 32]))
}

fn object(byte: u8) -> GitObjectId {
    GitObjectId::sha256([byte; 32])
}

fn capabilities() -> ResponseBody {
    ResponseBody::Capabilities(CapabilitySet {
        protocol_version: IPC_VERSION,
        granted: vec![Capability::ReadRepository, Capability::WriteOwnNamespace],
        limits: Limits {
            max_request_bytes: 16 * 1024,
            max_pack_bytes: 1024,
            max_ref_updates: 4,
            max_wants: 4,
            max_haves: 4,
            max_observations_per_page: 4,
        },
    })
}

fn session(
    responses: impl IntoIterator<Item = ResponseBody>,
    git: Git,
) -> RemoteSession<Transport, Git> {
    let url = GitRemoteUrl::from_str(&format!(
        "styrene:///git/v1/{}",
        repository().digest().base32()
    ))
    .expect("URL");
    let client = GitIpcClient::new(
        Transport {
            responses: responses.into_iter().collect(),
        },
        ClientConfig::default(),
    )
    .expect("client");
    RemoteSession::new(url, client, git)
}

#[test]
fn command_loop_handles_capabilities_lists_fetch_and_atomic_push() {
    let fetched = object(1);
    let old = object(2);
    let pushed = object(3);
    let read_listing = RefListing {
        repository: repository(),
        view: styrene_git_ipc::RepositoryView::Canonical,
        refs: vec![RefTarget {
            name: "refs/heads/main".into(),
            target: fetched.clone(),
        }],
        head: Some("refs/heads/main".into()),
    };
    let push_listing = PushRefListing {
        repository: repository(),
        refs: vec![RefTarget {
            name: "refs/heads/main".into(),
            target: old,
        }],
    };
    let result = PushResult {
        disposition: PushDisposition::Applied,
        state: RefState {
            repository: repository(),
            publisher: StyreneIdentity::new([4; 16]),
            transition: Digest::new([5; 32]),
            sequence: 1,
            refs: Vec::new(),
        },
        publication: OperationId::new([6; 16]),
    };
    let git = Git {
        resolutions: BTreeMap::from([("HEAD".into(), pushed)]),
        present: RefCell::new(BTreeSet::from([fetched.clone()])),
        installed: RefCell::new(0),
    };
    let mut session = session(
        [
            capabilities(),
            ResponseBody::Refs(read_listing),
            ResponseBody::PushRefs(push_listing.clone()),
            ResponseBody::FetchPack(b"PACK".to_vec()),
            ResponseBody::PushRefs(push_listing),
            ResponseBody::PushCommitted(result),
        ],
        git,
    );
    let input = format!(
        "capabilities\nlist\nlist for-push\nfetch {} refs/heads/main\n\npush +HEAD:refs/heads/main\npush :refs/tags/old\n\n\n",
        fetched.hex()
    );
    let mut output = Vec::new();
    run_command_loop(&mut Cursor::new(input), &mut output, &mut session).expect("command loop");
    let expected = format!(
        "fetch\npush\n\n{} refs/heads/main\n@refs/heads/main HEAD\n\n{} refs/heads/main\n\n\nok refs/heads/main\nok refs/tags/old\n\n",
        fetched.hex(),
        object(2).hex()
    );
    assert_eq!(String::from_utf8(output).expect("UTF-8"), expected);
    let (_, _, git) = session.into_parts();
    assert_eq!(git.installed.into_inner(), 1);
}

#[test]
fn command_loop_rejects_malformed_input_and_incomplete_batches() {
    for (input, expected) in [
        ("list\n", "capabilities must be the first helper command"),
        (
            "capabilities\nfetch bad refs/heads/main\n",
            "malformed fetch command",
        ),
        (
            "capabilities\npush HEAD:refs/heads/main\n",
            "unexpected EOF in push batch",
        ),
        (
            "capabilities\noption verbosity 1\n",
            "unsupported helper command",
        ),
    ] {
        let git = Git {
            resolutions: BTreeMap::new(),
            present: RefCell::new(BTreeSet::new()),
            installed: RefCell::new(0),
        };
        let mut session = session([], git);
        let error = run_command_loop(&mut Cursor::new(input), &mut Vec::new(), &mut session)
            .expect_err("input must fail");
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn atomic_push_rejection_reports_every_destination_without_diagnostic_text() {
    let pushed = object(3);
    let git = Git {
        resolutions: BTreeMap::from([
            ("HEAD".into(), pushed.clone()),
            ("refs/heads/next".into(), pushed),
        ]),
        present: RefCell::new(BTreeSet::new()),
        installed: RefCell::new(0),
    };
    let mut session = session(
        [
            capabilities(),
            ResponseBody::PushRefs(PushRefListing {
                repository: repository(),
                refs: Vec::new(),
            }),
            ResponseBody::Error(StableError {
                code: ErrorCode::Conflict,
                detail: Some("private diagnostic\nnot protocol output".into()),
            }),
        ],
        git,
    );
    let mut output = Vec::new();
    let error = run_command_loop(
        &mut Cursor::new(
            "capabilities\npush HEAD:refs/heads/main\npush refs/heads/next:refs/heads/next\n\n",
        ),
        &mut output,
        &mut session,
    )
    .expect_err("push must fail");
    assert!(error.to_string().contains("private diagnostic"));
    assert_eq!(
        output,
        b"fetch\npush\n\nerror refs/heads/main atomic push failed\nerror refs/heads/next atomic push failed\n\n"
    );
}

#[test]
fn installable_binary_advertises_only_implemented_commands() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_git-remote-styrene"))
        .args([
            "origin",
            &format!("styrene:///git/v1/{}", repository().digest().base32()),
        ])
        .env("GIT_DIR", ".git")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start helper");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"capabilities\n\n")
        .expect("write commands");
    let output = child.wait_with_output().expect("helper output");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"fetch\npush\n\n");
}
