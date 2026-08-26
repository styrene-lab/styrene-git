use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use styrene_git_core::{
    derive_canonical_head, CanonicalCbor, CanonicalDecision, GitObjectId, IdentityDocument,
    PublicKey, RefState, RefTarget, RefTransition, RepositoryId, SignerBinding, SignerSelection,
    StyreneIdentity, Visibility,
};
use styrene_git_protocol::{
    apply_transfer, export_transfer, ApplyOutcome, SignerAuthorization, Transfer,
};
use styrene_git_store::{ObjectFormat, Repository, RepositoryStore};

use crate::wire::{Request, Response, MAX_CONTROL_BYTES};

const MAX_PACK_BYTES: u64 = 4 * 1024 * 1024;

struct RepositoryContext {
    repository: Repository,
    identity: IdentityDocument,
    bindings: BTreeMap<StyreneIdentity, SignerSelection>,
    canonical: Option<GitObjectId>,
}

struct OperatorState {
    name: String,
    incarnation: String,
    actor: Actor,
    store: RepositoryStore,
    repositories: BTreeMap<RepositoryId, RepositoryContext>,
}

struct Actor {
    repository_key: SigningKey,
    binding: SignerBinding,
}

impl Actor {
    fn seeded(seed: u8) -> Result<Self, String> {
        let identity_key = SigningKey::from_bytes(&[seed; 32]);
        let repository_key = SigningKey::from_bytes(&[seed.wrapping_add(100); 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            0,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            repository_key,
            binding,
        })
    }

    fn id(&self) -> StyreneIdentity {
        self.binding.identity()
    }
}

pub fn run(name: String, seed: u8, listen: String, root: PathBuf) -> Result<(), String> {
    let state = Arc::new(Mutex::new(OperatorState {
        name,
        incarnation: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
            .as_nanos()
            .to_string(),
        actor: Actor::seeded(seed)?,
        store: RepositoryStore::new(root).map_err(|error| error.to_string())?,
        repositories: BTreeMap::new(),
    }));
    let listener = TcpListener::bind(&listen)
        .map_err(|error| format!("bind operator control at {listen} failed: {error}"))?;
    for stream in listener.incoming() {
        let state = Arc::clone(&state);
        match stream {
            Ok(stream) => {
                thread::spawn(move || serve(stream, state));
            }
            Err(error) => eprintln!("operator control accept failed: {error}"),
        }
    }
    Ok(())
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<OperatorState>>) {
    let response = read_request(&mut stream)
        .and_then(|request| {
            state
                .lock()
                .map_err(|_| "operator state lock was poisoned".to_owned())
                .and_then(|mut state| state.handle(request))
        })
        .unwrap_or_else(|message| Response::Error { message });
    match serde_json::to_vec(&response) {
        Ok(bytes) => {
            let _ = stream.write_all(&bytes);
        }
        Err(error) => {
            let _ = stream
                .write_all(format!("{{\"result\":\"error\",\"message\":\"{error}\"}}").as_bytes());
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut bytes = Vec::new();
    stream
        .take(MAX_CONTROL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read control request failed: {error}"))?;
    if bytes.len() as u64 > MAX_CONTROL_BYTES {
        return Err("control request exceeds limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid control request: {error}"))
}

impl OperatorState {
    fn handle(&mut self, request: Request) -> Result<Response, String> {
        match request {
            Request::Health => Ok(Response::Healthy {
                operator: self.name.clone(),
                incarnation: self.incarnation.clone(),
            }),
            Request::Identity => Ok(Response::Identity {
                identity: self.actor.id().to_string(),
                binding: hex::encode(
                    self.actor
                        .binding
                        .canonical_bytes()
                        .map_err(|error| error.to_string())?,
                ),
            }),
            Request::Initialize {
                bindings,
                threshold,
            } => self.initialize(bindings, threshold),
            Request::PublishCommit {
                repository,
                message,
                parent,
            } => self.publish_commit(&repository, &message, parent.as_deref()),
            Request::PublishTarget { repository, target } => {
                self.publish_target(&repository, &target)
            }
            Request::Apply { transfer } => self.apply(&transfer),
            Request::State {
                repository,
                delegates,
            } => self.state(&repository, delegates),
            Request::Fsck { repository } => self.fsck(&repository),
            Request::Restart => restart_process(),
        }
    }

    fn initialize(
        &mut self,
        bindings: BTreeMap<String, String>,
        threshold: u16,
    ) -> Result<Response, String> {
        let bindings = bindings
            .into_iter()
            .map(|(identity, binding)| {
                let identity =
                    StyreneIdentity::from_hex(&identity).map_err(|error| error.to_string())?;
                let bytes = hex::decode(binding)
                    .map_err(|error| format!("invalid signer binding hex: {error}"))?;
                let binding = SignerBinding::from_canonical_bytes(&bytes)
                    .map_err(|error| error.to_string())?;
                if binding.identity() != identity {
                    return Err("signer binding identity does not match selection".into());
                }
                Ok((
                    identity,
                    binding.selection().map_err(|error| error.to_string())?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let delegates = bindings.keys().copied().collect();
        let identity = IdentityDocument::new(
            "functional-fixture",
            "isolated multi-operator scenario",
            "main",
            Visibility::Public,
            delegates,
            threshold,
        )
        .map_err(|error| error.to_string())?;
        let repository_id = identity
            .repository_id()
            .map_err(|error| error.to_string())?;
        let repository = self
            .store
            .create(repository_id, ObjectFormat::default())
            .map_err(|error| error.to_string())?;
        self.repositories
            .entry(repository_id)
            .or_insert(RepositoryContext {
                repository,
                identity,
                bindings,
                canonical: None,
            });
        Ok(Response::Initialized {
            repository: repository_id.to_string(),
        })
    }

    fn publish_commit(
        &mut self,
        repository: &str,
        message: &str,
        parent: Option<&str>,
    ) -> Result<Response, String> {
        let repository_id = parse_repository(repository)?;
        let context = self
            .repositories
            .get_mut(&repository_id)
            .ok_or_else(|| "repository is not initialized".to_owned())?;
        let parent = parent
            .map(|value| GitObjectId::from_hex(context.repository.object_format().name(), value))
            .transpose()
            .map_err(|error| error.to_string())?;
        if let Some(parent) = &parent {
            if !context
                .repository
                .has_object(parent)
                .map_err(|error| error.to_string())?
            {
                return Err("commit parent is not present".into());
            }
        }
        let previous = context
            .repository
            .publisher_state(self.actor.id())
            .map_err(|error| error.to_string())?;
        let tree = context
            .repository
            .object_id("tree", &[])
            .map_err(|error| error.to_string())?;
        let sequence = previous.as_ref().map_or(0, |state| state.sequence + 1);
        let mut commit = format!("tree {}\n", tree.hex());
        if let Some(parent) = &parent {
            commit.push_str(&format!("parent {}\n", parent.hex()));
        }
        commit.push_str(&format!(
            "author {} <{}@example.invalid> {} +0000\ncommitter {} <{}@example.invalid> {} +0000\n\n{}\n",
            self.name,
            self.name,
            sequence + 1,
            self.name,
            self.name,
            sequence + 1,
            message
        ));
        let head = context
            .repository
            .object_id("commit", commit.as_bytes())
            .map_err(|error| error.to_string())?;
        let quarantine = context
            .repository
            .begin_quarantine()
            .map_err(|error| error.to_string())?;
        quarantine
            .write_verified_object("tree", &[], &tree)
            .map_err(|error| error.to_string())?;
        quarantine
            .write_verified_object("commit", commit.as_bytes(), &head)
            .map_err(|error| error.to_string())?;
        let transition =
            signed_transition(repository_id, &self.actor, previous.as_ref(), head.clone())?;
        let selected_binding = context
            .bindings
            .get(&self.actor.id())
            .ok_or_else(|| "publisher has no accepted signer binding".to_owned())?;
        let state = context
            .repository
            .commit(
                &quarantine,
                &transition,
                &self.actor.binding,
                selected_binding,
            )
            .map_err(|error| error.to_string())?
            .into_state();
        let prerequisites: Vec<_> = parent.into_iter().collect();
        let transfer = export_transfer(
            &context.repository,
            SignerAuthorization::new(&self.actor.binding, selected_binding),
            &transition,
            previous.as_ref(),
            std::slice::from_ref(&head),
            &prerequisites,
            MAX_PACK_BYTES,
        )
        .map_err(|error| error.to_string())?;
        Ok(Response::Published {
            head: head.hex(),
            sequence: state.sequence,
            transfer: encode_transfer(&transfer)?,
        })
    }

    fn publish_target(&mut self, repository: &str, target: &str) -> Result<Response, String> {
        let repository_id = parse_repository(repository)?;
        let context = self
            .repositories
            .get_mut(&repository_id)
            .ok_or_else(|| "repository is not initialized".to_owned())?;
        let target = GitObjectId::from_hex(context.repository.object_format().name(), target)
            .map_err(|error| error.to_string())?;
        if !context
            .repository
            .has_object(&target)
            .map_err(|error| error.to_string())?
        {
            return Err("published target is not present".into());
        }
        let previous = context
            .repository
            .publisher_state(self.actor.id())
            .map_err(|error| error.to_string())?;
        let transition = signed_transition(
            repository_id,
            &self.actor,
            previous.as_ref(),
            target.clone(),
        )?;
        let quarantine = context
            .repository
            .begin_quarantine()
            .map_err(|error| error.to_string())?;
        let selected_binding = context
            .bindings
            .get(&self.actor.id())
            .ok_or_else(|| "publisher has no accepted signer binding".to_owned())?;
        let state = context
            .repository
            .commit(
                &quarantine,
                &transition,
                &self.actor.binding,
                selected_binding,
            )
            .map_err(|error| error.to_string())?
            .into_state();
        let transfer = export_transfer(
            &context.repository,
            SignerAuthorization::new(&self.actor.binding, selected_binding),
            &transition,
            previous.as_ref(),
            std::slice::from_ref(&target),
            std::slice::from_ref(&target),
            MAX_PACK_BYTES,
        )
        .map_err(|error| error.to_string())?;
        Ok(Response::Published {
            head: target.hex(),
            sequence: state.sequence,
            transfer: encode_transfer(&transfer)?,
        })
    }

    fn apply(&mut self, transfer: &str) -> Result<Response, String> {
        let transfer = decode_transfer(transfer)?;
        let context = self
            .repositories
            .get_mut(&transfer.manifest.repository)
            .ok_or_else(|| "transfer repository is not initialized".to_owned())?;
        let selected_binding = context
            .bindings
            .get(&transfer.manifest.publisher)
            .ok_or_else(|| "publisher has no accepted signer binding".to_owned())?;
        let outcome = apply_transfer(
            &context.repository,
            &transfer,
            selected_binding,
            MAX_PACK_BYTES,
        )
        .map_err(|error| error.to_string())?;
        let (outcome, state) = match outcome {
            ApplyOutcome::Applied(state) => ("applied", state),
            ApplyOutcome::AlreadyPresent(state) => ("already_present", state),
            ApplyOutcome::MissingPrerequisites(missing) => {
                return Ok(Response::MissingPrerequisites {
                    prerequisites: missing
                        .into_iter()
                        .map(|prerequisite| format!("{prerequisite:?}"))
                        .collect(),
                });
            }
        };
        Ok(Response::Applied {
            outcome: outcome.into(),
            publisher: state.publisher.to_string(),
            sequence: state.sequence,
        })
    }

    fn state(&mut self, repository: &str, delegates: Vec<String>) -> Result<Response, String> {
        let repository_id = parse_repository(repository)?;
        let context = self
            .repositories
            .get_mut(&repository_id)
            .ok_or_else(|| "repository is not initialized".to_owned())?;
        let delegates = delegates
            .iter()
            .map(|identity| StyreneIdentity::from_hex(identity).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut states = BTreeMap::new();
        let mut publishers = BTreeMap::new();
        for delegate in delegates {
            let state = context
                .repository
                .publisher_state(delegate)
                .map_err(|error| error.to_string())?;
            let head = state
                .as_ref()
                .and_then(|state| state.target("refs/heads/main"))
                .map(GitObjectId::hex);
            publishers.insert(delegate.to_string(), head);
            if let Some(state) = state {
                states.insert(delegate, state);
            }
        }
        let decision = derive_canonical_head(
            &context.identity,
            &states,
            context.canonical.as_ref(),
            |ancestor, descendant| {
                context
                    .repository
                    .is_ancestor(ancestor, descendant)
                    .unwrap_or(false)
            },
        );
        let decision_name = match &decision {
            CanonicalDecision::Advance(head) => {
                context.canonical = Some(head.clone());
                "advance"
            }
            CanonicalDecision::Retain => "retain",
            CanonicalDecision::NoAgreement => "no_agreement",
            CanonicalDecision::Diverged(_) => "diverged",
        };
        Ok(Response::State {
            canonical: context.canonical.as_ref().map(GitObjectId::hex),
            decision: decision_name.into(),
            publishers,
        })
    }

    fn fsck(&self, repository: &str) -> Result<Response, String> {
        let repository_id = parse_repository(repository)?;
        let context = self
            .repositories
            .get(&repository_id)
            .ok_or_else(|| "repository is not initialized".to_owned())?;
        context
            .repository
            .fsck()
            .map_err(|error| error.to_string())?;
        Ok(Response::Verified {
            repository: repository.into(),
        })
    }
}

fn signed_transition(
    repository: RepositoryId,
    actor: &Actor,
    previous: Option<&RefState>,
    target: GitObjectId,
) -> Result<RefTransition, String> {
    RefTransition::signed(
        repository,
        &actor.binding,
        &actor.repository_key,
        previous,
        vec![RefTarget {
            name: "refs/heads/main".into(),
            target,
        }],
    )
    .map_err(|error| error.to_string())
}

fn parse_repository(value: &str) -> Result<RepositoryId, String> {
    RepositoryId::from_str(value).map_err(|error| error.to_string())
}

fn encode_transfer(transfer: &Transfer) -> Result<String, String> {
    transfer
        .canonical_bytes()
        .map(hex::encode)
        .map_err(|error| error.to_string())
}

fn decode_transfer(value: &str) -> Result<Transfer, String> {
    let bytes = hex::decode(value).map_err(|error| format!("invalid transfer hex: {error}"))?;
    Transfer::from_canonical_bytes(&bytes).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn restart_process() -> Result<Response, String> {
    use std::os::unix::process::CommandExt;

    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve operator executable failed: {error}"))?;
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(executable)
        .args(arguments)
        .exec();
    Err(format!("restart operator process failed: {error}"))
}

#[cfg(not(unix))]
fn restart_process() -> Result<Response, String> {
    Err("operator process restart is supported only on Unix harness hosts".into())
}
