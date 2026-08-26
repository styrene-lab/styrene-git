use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::wire::{
    incarnation, request, wait_until_healthy, wait_until_restarted, Request, Response,
};

const ALICE_CONTROL: &str = "alice-control:7800";
const BOB_CONTROL: &str = "bob-control:7800";
const CAROL_CONTROL: &str = "carol-control:7800";
const ALICE_CARRIER: &str = "alice-carrier:7800";
const BOB_CARRIER: &str = "bob-carrier:7800";
const CAROL_CARRIER: &str = "carol-carrier:7800";

#[derive(Serialize)]
struct ScenarioArtifact {
    scenario: String,
    repository: String,
    identities: BTreeMap<String, String>,
    heads: BTreeMap<String, String>,
    final_states: BTreeMap<String, Response>,
    fsck: BTreeMap<String, String>,
    failure_recovery: BTreeMap<String, String>,
    restart_recovery: BTreeMap<String, String>,
    duplicate_outcome: String,
    status: String,
}

pub fn run_three_party(artifact_path: &Path) -> Result<(), String> {
    for address in [ALICE_CONTROL, BOB_CONTROL, CAROL_CONTROL] {
        wait_until_healthy(address, Duration::from_secs(60))?;
    }
    let actors: BTreeMap<String, (String, String)> = BTreeMap::from([
        ("alice".into(), identity(ALICE_CONTROL)?),
        ("bob".into(), identity(BOB_CONTROL)?),
        ("carol".into(), identity(CAROL_CONTROL)?),
    ]);
    let identities = actors
        .iter()
        .map(|(name, (identity, _))| (name.clone(), identity.clone()))
        .collect::<BTreeMap<_, _>>();
    let bindings = actors
        .values()
        .map(|(identity, binding)| (identity.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let delegates: Vec<_> = ["alice", "bob", "carol"]
        .iter()
        .map(|name| identities[*name].clone())
        .collect();
    let mut repository = None;
    for address in [ALICE_CONTROL, BOB_CONTROL, CAROL_CONTROL] {
        let response = request(
            address,
            &Request::Initialize {
                bindings: bindings.clone(),
                threshold: 2,
            },
        )?;
        let initialized = match response {
            Response::Initialized { repository } => repository,
            other => return Err(format!("unexpected initialize response: {other:?}")),
        };
        if let Some(expected) = &repository {
            if expected != &initialized {
                return Err("operators derived different repository identifiers".into());
            }
        } else {
            repository = Some(initialized);
        }
    }
    let repository = repository.ok_or_else(|| "repository was not initialized".to_owned())?;

    let alice_initial = publish_commit(ALICE_CONTROL, &repository, "initial", None)?;
    apply(BOB_CARRIER, &alice_initial.transfer, "applied")?;

    let bob = publish_commit(
        BOB_CONTROL,
        &repository,
        "bob descendant",
        Some(&alice_initial.head),
    )?;
    apply(ALICE_CARRIER, &bob.transfer, "applied")?;

    let carol_before_failures = state(CAROL_CONTROL, &repository, &delegates)?;
    rejected(
        CAROL_CARRIER,
        &corrupt_transfer(&bob.transfer)?,
        "integrity check failed",
    )?;
    assert_state_unchanged(
        "corrupt transfer",
        &carol_before_failures,
        &state(CAROL_CONTROL, &repository, &delegates)?,
    )?;
    rejected(
        CAROL_CARRIER,
        truncate_transfer(&bob.transfer)?,
        "CBOR decoding failed",
    )?;
    assert_state_unchanged(
        "truncated transfer",
        &carol_before_failures,
        &state(CAROL_CONTROL, &repository, &delegates)?,
    )?;
    let missing = apply_missing(CAROL_CARRIER, &bob.transfer)?;
    if missing != 1 {
        return Err(format!(
            "Carol reported {missing} missing prerequisites instead of one"
        ));
    }
    assert_state_unchanged(
        "missing prerequisite",
        &carol_before_failures,
        &state(CAROL_CONTROL, &repository, &delegates)?,
    )?;
    apply(CAROL_CARRIER, &alice_initial.transfer, "applied")?;
    apply(CAROL_CARRIER, &bob.transfer, "applied")?;

    let carol = publish_commit(
        CAROL_CONTROL,
        &repository,
        "carol divergent descendant",
        Some(&alice_initial.head),
    )?;
    apply(ALICE_CARRIER, &carol.transfer, "applied")?;
    apply(BOB_CARRIER, &carol.transfer, "applied")?;

    for address in [ALICE_CONTROL, BOB_CONTROL, CAROL_CONTROL] {
        let state = state(address, &repository, &delegates)?;
        if state.canonical.is_some() || state.decision != "no_agreement" {
            return Err(format!(
                "{address} reached canonical agreement before threshold"
            ));
        }
    }

    let alice_vote = publish_target(ALICE_CONTROL, &repository, &bob.head)?;
    apply(BOB_CARRIER, &alice_vote.transfer, "applied")?;
    apply(CAROL_CARRIER, &alice_vote.transfer, "applied")?;
    let duplicate = apply(ALICE_CARRIER, &bob.transfer, "already_present")?;

    let carol_before_restart = state(CAROL_CONTROL, &repository, &delegates)?;
    let prior_incarnation = incarnation(CAROL_CONTROL)?;
    restart(CAROL_CONTROL)?;
    wait_until_restarted(CAROL_CONTROL, &prior_incarnation, Duration::from_secs(30))?;
    match request(
        CAROL_CONTROL,
        &Request::Initialize {
            bindings,
            threshold: 2,
        },
    )? {
        Response::Initialized {
            repository: reopened,
        } if reopened == repository => {}
        other => return Err(format!("unexpected restart initialize response: {other:?}")),
    }
    let carol_after_restart = state(CAROL_CONTROL, &repository, &delegates)?;
    assert_state_unchanged(
        "operator process restart",
        &carol_before_restart,
        &carol_after_restart,
    )?;
    apply(CAROL_CARRIER, &alice_vote.transfer, "already_present")?;

    let mut final_states = BTreeMap::new();
    let mut fsck_results = BTreeMap::new();
    for (name, address) in [
        ("alice", ALICE_CONTROL),
        ("bob", BOB_CONTROL),
        ("carol", CAROL_CONTROL),
    ] {
        let state = state(address, &repository, &delegates)?;
        if state.canonical.as_deref() != Some(&bob.head) {
            return Err(format!(
                "{name} canonical head does not match Bob's descendant"
            ));
        }
        if state
            .publishers
            .get(&identities["carol"])
            .and_then(Clone::clone)
            .as_deref()
            != Some(&carol.head)
        {
            return Err(format!(
                "{name} did not retain Carol's divergent publisher head"
            ));
        }
        final_states.insert(
            name.into(),
            Response::State {
                canonical: state.canonical,
                decision: state.decision,
                publishers: state.publishers,
            },
        );
        fsck(address, &repository)?;
        fsck_results.insert(name.into(), "passed".into());
    }
    let heads = BTreeMap::from([
        ("initial".into(), alice_initial.head),
        ("bob".into(), bob.head),
        ("carol".into(), carol.head),
    ]);
    let artifact = ScenarioArtifact {
        scenario: "three-party-diverge-converge".into(),
        repository,
        identities,
        heads,
        final_states,
        fsck: fsck_results,
        failure_recovery: BTreeMap::from([
            (
                "corrupt_transfer".into(),
                "rejected_without_state_change".into(),
            ),
            (
                "truncated_transfer".into(),
                "rejected_without_state_change".into(),
            ),
            (
                "missing_prerequisite".into(),
                "reported_without_state_change".into(),
            ),
            ("valid_redelivery".into(), "applied".into()),
        ]),
        restart_recovery: BTreeMap::from([
            ("accepted_refs".into(), "reopened".into()),
            ("canonical_state".into(), "rederived_identically".into()),
            ("committed_redelivery".into(), "already_present".into()),
        ]),
        duplicate_outcome: duplicate,
        status: "passed".into(),
    };
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create artifact directory failed: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&artifact)
        .map_err(|error| format!("encode scenario artifact failed: {error}"))?;
    fs::write(artifact_path, bytes)
        .map_err(|error| format!("write scenario artifact failed: {error}"))?;
    println!("functional scenario passed: {}", artifact.scenario);
    Ok(())
}

struct Published {
    head: String,
    transfer: String,
}

#[derive(PartialEq, Eq)]
struct State {
    canonical: Option<String>,
    decision: String,
    publishers: BTreeMap<String, Option<String>>,
}

fn identity(address: &str) -> Result<(String, String), String> {
    match request(address, &Request::Identity)? {
        Response::Identity { identity, binding } => Ok((identity, binding)),
        other => Err(format!("unexpected identity response: {other:?}")),
    }
}

fn publish_commit(
    address: &str,
    repository: &str,
    message: &str,
    parent: Option<&str>,
) -> Result<Published, String> {
    published(request(
        address,
        &Request::PublishCommit {
            repository: repository.into(),
            message: message.into(),
            parent: parent.map(str::to_owned),
        },
    )?)
}

fn publish_target(address: &str, repository: &str, target: &str) -> Result<Published, String> {
    published(request(
        address,
        &Request::PublishTarget {
            repository: repository.into(),
            target: target.into(),
        },
    )?)
}

fn published(response: Response) -> Result<Published, String> {
    match response {
        Response::Published { head, transfer, .. } => Ok(Published { head, transfer }),
        other => Err(format!("unexpected publish response: {other:?}")),
    }
}

fn apply(address: &str, transfer: &str, expected: &str) -> Result<String, String> {
    match request(
        address,
        &Request::Apply {
            transfer: transfer.into(),
        },
    )? {
        Response::Applied { outcome, .. } if outcome == expected => Ok(outcome),
        other => Err(format!("unexpected apply response: {other:?}")),
    }
}

fn apply_missing(address: &str, transfer: &str) -> Result<usize, String> {
    match request(
        address,
        &Request::Apply {
            transfer: transfer.into(),
        },
    )? {
        Response::MissingPrerequisites { prerequisites } => Ok(prerequisites.len()),
        other => Err(format!(
            "expected missing prerequisites, received: {other:?}"
        )),
    }
}

fn rejected(address: &str, transfer: &str, expected: &str) -> Result<(), String> {
    match request(
        address,
        &Request::Apply {
            transfer: transfer.into(),
        },
    ) {
        Err(message) if message.contains(expected) => Ok(()),
        Err(message) => Err(format!(
            "transfer failed with unexpected error {message:?}; expected {expected:?}"
        )),
        Ok(response) => Err(format!("invalid transfer was accepted: {response:?}")),
    }
}

fn restart(address: &str) -> Result<(), String> {
    match request(address, &Request::Restart) {
        Err(_) => Ok(()),
        Ok(response) => Err(format!(
            "operator restart unexpectedly returned a response: {response:?}"
        )),
    }
}

fn corrupt_transfer(transfer: &str) -> Result<String, String> {
    let mut bytes = hex::decode(transfer).map_err(|error| format!("decode fixture: {error}"))?;
    let last = bytes
        .last_mut()
        .ok_or_else(|| "cannot corrupt an empty transfer".to_owned())?;
    *last ^= 1;
    Ok(hex::encode(bytes))
}

fn truncate_transfer(transfer: &str) -> Result<&str, String> {
    transfer
        .get(..transfer.len().saturating_sub(2))
        .filter(|truncated| !truncated.is_empty())
        .ok_or_else(|| "cannot truncate an empty transfer".to_owned())
}

fn assert_state_unchanged(label: &str, before: &State, after: &State) -> Result<(), String> {
    if before == after {
        Ok(())
    } else {
        Err(format!("accepted state changed after {label}"))
    }
}

fn state(address: &str, repository: &str, delegates: &[String]) -> Result<State, String> {
    match request(
        address,
        &Request::State {
            repository: repository.into(),
            delegates: delegates.to_vec(),
        },
    )? {
        Response::State {
            canonical,
            decision,
            publishers,
        } => Ok(State {
            canonical,
            decision,
            publishers,
        }),
        other => Err(format!("unexpected state response: {other:?}")),
    }
}

fn fsck(address: &str, repository: &str) -> Result<(), String> {
    match request(
        address,
        &Request::Fsck {
            repository: repository.into(),
        },
    )? {
        Response::Verified {
            repository: verified,
        } if verified == repository => Ok(()),
        other => Err(format!("unexpected fsck response: {other:?}")),
    }
}
