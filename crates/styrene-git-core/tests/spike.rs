use std::{collections::BTreeMap, str::FromStr};

use ed25519_dalek::SigningKey;
use styrene_git_core::{
    derive_canonical_head, CanonicalCbor, CanonicalDecision, CoreError, Digest, GitObjectId,
    IdentityDocument, IdentityState, IdentityTransition, PublicKey, RefState, RefTarget,
    RefTransition, SignerBinding, SignerSelection, StyreneIdentity, Visibility,
};

struct Actor {
    identity_key: SigningKey,
    repository_key: SigningKey,
    binding: SignerBinding,
}

impl Actor {
    fn seeded(identity_seed: u8, repository_seed: u8) -> Self {
        Self::at_epoch(identity_seed, repository_seed, 0)
    }

    fn at_epoch(identity_seed: u8, repository_seed: u8, key_epoch: u32) -> Self {
        let identity_key = SigningKey::from_bytes(&[identity_seed; 32]);
        let repository_key = SigningKey::from_bytes(&[repository_seed; 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            key_epoch,
        )
        .expect("fixed test keys should produce a binding");
        Self {
            identity_key,
            repository_key,
            binding,
        }
    }

    fn id(&self) -> StyreneIdentity {
        StyreneIdentity::from_signing_key(&self.identity_key)
    }

    fn selection(&self) -> SignerSelection {
        self.binding
            .selection()
            .expect("fixture binding should produce a selection")
    }
}

#[test]
fn reference_operations_require_the_prior_state_signer_selection() {
    let stale = Actor::at_epoch(26, 36, 0);
    let current = Actor::at_epoch(26, 37, 1);
    let future = Actor::at_epoch(26, 38, 2);
    let substituted = Actor::at_epoch(26, 39, 1);
    let repository = project(&[&current], 1)
        .repository_id()
        .expect("repository ID");
    let transition = |actor: &Actor| {
        RefTransition::signed(
            repository,
            &actor.binding,
            &actor.repository_key,
            None,
            vec![main_ref(1)],
        )
        .expect("signed transition")
    };

    for rejected in [&stale, &future, &substituted] {
        assert!(matches!(
            transition(rejected).verify(
                repository,
                &rejected.binding,
                &current.selection(),
                None
            ),
            Err(CoreError::InvalidBinding(message))
                if message == "binding does not match prior-state signer selection"
        ));
    }

    let current_transition = transition(&current);
    assert!(matches!(
        current_transition.verify(repository, &stale.binding, &current.selection(), None),
        Err(CoreError::InvalidBinding(_))
    ));
    current_transition
        .verify(repository, &current.binding, &current.selection(), None)
        .expect("selected current binding should authorize the operation");

    let maximum = Actor::at_epoch(27, 40, u32::MAX);
    let maximum_repository = project(&[&maximum], 1)
        .repository_id()
        .expect("maximum-epoch repository ID");
    RefTransition::signed(
        maximum_repository,
        &maximum.binding,
        &maximum.repository_key,
        None,
        vec![main_ref(2)],
    )
    .expect("maximum-epoch transition")
    .verify(
        maximum_repository,
        &maximum.binding,
        &maximum.selection(),
        None,
    )
    .expect("maximum epoch must be compared without incrementing");
}

#[test]
fn identity_approvals_use_the_selected_current_binding() {
    let stale = Actor::at_epoch(28, 41, 0);
    let current = Actor::at_epoch(28, 42, 1);
    let initial = IdentityState::initial(project(&[&current], 1)).expect("initial state");
    let mut transition = IdentityTransition::proposed(&initial, initial.document.clone())
        .expect("transition proposal");
    transition
        .approve(&stale.binding, &stale.repository_key)
        .expect("historical approval");
    let selected = BTreeMap::from([(current.id(), current.selection())]);

    assert!(matches!(
        transition.verify(&initial, &selected),
        Err(CoreError::InvalidIdentityTransition(message)) if message == "signer epoch mismatch"
    ));

    let maximum = Actor::at_epoch(29, 43, u32::MAX);
    let maximum_initial =
        IdentityState::initial(project(&[&maximum], 1)).expect("maximum initial state");
    let mut maximum_transition =
        IdentityTransition::proposed(&maximum_initial, maximum_initial.document.clone())
            .expect("maximum epoch proposal");
    maximum_transition
        .approve(&maximum.binding, &maximum.repository_key)
        .expect("maximum-epoch approval");
    maximum_transition
        .verify(
            &maximum_initial,
            &BTreeMap::from([(maximum.id(), maximum.selection())]),
        )
        .expect("maximum approval epoch must not wrap");
}

#[test]
fn transition_sequences_reject_exhaustion_without_wraparound() {
    let alice = Actor::seeded(30, 44);
    let initial = IdentityState::initial(project(&[&alice], 1)).expect("initial state");
    let mut almost_exhausted = initial.clone();
    almost_exhausted.sequence = u64::MAX - 1;
    let mut identity_transition =
        IdentityTransition::proposed(&almost_exhausted, initial.document.clone())
            .expect("last identity sequence");
    identity_transition
        .approve(&alice.binding, &alice.repository_key)
        .expect("last identity approval");
    let mut exhausted_identity = almost_exhausted.clone();
    exhausted_identity.sequence = u64::MAX;
    assert!(matches!(
        IdentityTransition::proposed(&exhausted_identity, initial.document.clone()),
        Err(CoreError::InvalidIdentityTransition(message))
            if message == "identity transition sequence exhausted"
    ));
    assert!(matches!(
        identity_transition.verify(
            &exhausted_identity,
            &BTreeMap::from([(alice.id(), alice.selection())])
        ),
        Err(CoreError::InvalidIdentityTransition(message))
            if message == "identity transition sequence exhausted"
    ));

    let repository = initial.repository;
    let almost_exhausted_refs = RefState {
        repository,
        publisher: alice.id(),
        transition: Digest::new([7; 32]),
        sequence: u64::MAX - 1,
        refs: vec![main_ref(1)],
    };
    let reference_transition = RefTransition::signed(
        repository,
        &alice.binding,
        &alice.repository_key,
        Some(&almost_exhausted_refs),
        vec![main_ref(2)],
    )
    .expect("last reference sequence");
    let mut exhausted_refs = almost_exhausted_refs;
    exhausted_refs.sequence = u64::MAX;
    assert!(matches!(
        RefTransition::signed(
            repository,
            &alice.binding,
            &alice.repository_key,
            Some(&exhausted_refs),
            vec![main_ref(2)],
        ),
        Err(CoreError::InvalidRefTransition(message))
            if message == "reference transition sequence exhausted"
    ));
    assert!(matches!(
        reference_transition.verify(
            repository,
            &alice.binding,
            &alice.selection(),
            Some(&exhausted_refs),
        ),
        Err(CoreError::InvalidRefTransition(message))
            if message == "reference transition sequence exhausted"
    ));
}

fn project(delegates: &[&Actor], threshold: u16) -> IdentityDocument {
    IdentityDocument::new(
        "small-project",
        "container integration fixture",
        "main",
        Visibility::Public,
        delegates.iter().map(|actor| actor.id()).collect(),
        threshold,
    )
    .expect("fixture identity should be valid")
}

fn main_ref(byte: u8) -> RefTarget {
    RefTarget {
        name: "refs/heads/main".into(),
        target: GitObjectId::sha1([byte; 20]),
    }
}

#[test]
fn repository_identity_has_stable_golden_bytes_and_id() {
    let alice = Actor::seeded(1, 11);
    let document = project(&[&alice], 1);
    let bytes = document.canonical_bytes().expect("fixture should encode");

    assert_eq!(
        hex::encode(bytes),
        "87016d736d616c6c2d70726f6a656374781d636f6e7461696e657220696e746567726174696f6e2066697874757265646d61696e00815034750f98bd59fcfc946da45aaabe933b01"
    );
    assert_eq!(
        document
            .repository_id()
            .expect("fixture should hash")
            .to_string(),
        "styrene:git:v1:d5cf77ofrhjf3bfelthksbulv77grtm5jssotycl3atcmhjp4jmq"
    );
}

#[test]
fn textual_identifiers_round_trip_canonically() {
    let alice = Actor::seeded(1, 11);
    let identity = alice.id();
    assert_eq!(
        StyreneIdentity::from_hex(&identity.to_string()).expect("identity should parse"),
        identity
    );

    let repository = project(&[&alice], 1)
        .repository_id()
        .expect("fixture should hash");
    assert_eq!(
        styrene_git_core::RepositoryId::from_str(&repository.to_string())
            .expect("repository identifier should parse"),
        repository
    );

    let object = GitObjectId::sha256([7; 32]);
    assert_eq!(
        GitObjectId::from_hex("sha256", &object.hex()).expect("object ID should parse"),
        object
    );
}

#[test]
fn canonical_decoder_rejects_trailing_or_noncanonical_data() {
    let alice = Actor::seeded(2, 12);
    let document = project(&[&alice], 1);
    let mut bytes = document.canonical_bytes().expect("fixture should encode");
    bytes.push(0);

    assert_eq!(
        IdentityDocument::from_canonical_bytes(&bytes),
        Err(CoreError::NonCanonical)
    );

    let canonical = document.canonical_bytes().expect("fixture should encode");
    let mut long_form_integer = vec![canonical[0], 0x18, canonical[1]];
    long_form_integer.extend_from_slice(&canonical[2..]);
    assert_eq!(
        IdentityDocument::from_canonical_bytes(&long_form_integer),
        Err(CoreError::NonCanonical)
    );
}

#[test]
fn identity_document_rejects_invalid_delegate_policies() {
    let alice = Actor::seeded(21, 31);
    assert!(matches!(
        IdentityDocument::new(
            "bad-threshold",
            "fixture",
            "main",
            Visibility::Public,
            vec![alice.id()],
            2,
        ),
        Err(CoreError::InvalidIdentity(_))
    ));
    assert!(matches!(
        IdentityDocument::new(
            "duplicate",
            "fixture",
            "main",
            Visibility::Public,
            vec![alice.id(), alice.id()],
            1,
        ),
        Err(CoreError::InvalidIdentity(_))
    ));
}

#[test]
fn identity_change_uses_the_prior_delegate_policy() {
    let alice = Actor::seeded(3, 13);
    let bob = Actor::seeded(4, 14);
    let mallory = Actor::seeded(5, 15);
    let initial = IdentityState::initial(project(&[&alice, &bob], 2)).expect("initial state");
    let mut proposed = IdentityTransition::proposed(&initial, project(&[&alice, &mallory], 1))
        .expect("identity proposal");
    proposed
        .approve(&alice.binding, &alice.repository_key)
        .expect("Alice approval");
    proposed
        .approve(&mallory.binding, &mallory.repository_key)
        .expect("Mallory approval");

    let bindings = BTreeMap::from([
        (alice.id(), alice.selection()),
        (bob.id(), bob.selection()),
        (mallory.id(), mallory.selection()),
    ]);
    assert!(matches!(
        proposed.verify(&initial, &bindings),
        Err(CoreError::InvalidIdentityTransition(message)) if message == "delegate threshold not satisfied"
    ));

    proposed
        .approve(&bob.binding, &bob.repository_key)
        .expect("Bob approval");
    let accepted = proposed
        .verify(&initial, &bindings)
        .expect("current threshold is satisfied");
    assert_eq!(accepted.sequence, 1);
    assert_eq!(
        accepted.document.delegates,
        project(&[&alice, &mallory], 1).delegates
    );
}

#[test]
fn identity_transition_rejects_replay_and_cross_repository_use() {
    let alice = Actor::seeded(22, 32);
    let bindings = BTreeMap::from([(alice.id(), alice.selection())]);
    let initial = IdentityState::initial(project(&[&alice], 1)).expect("initial state");
    let mut transition = IdentityTransition::proposed(
        &initial,
        IdentityDocument::new(
            "renamed",
            "updated metadata",
            "main",
            Visibility::Public,
            vec![alice.id()],
            1,
        )
        .expect("updated document"),
    )
    .expect("identity proposal");
    transition
        .approve(&alice.binding, &alice.repository_key)
        .expect("approval");
    let accepted = transition
        .verify(&initial, &bindings)
        .expect("valid transition");
    assert_eq!(accepted.repository, initial.repository);

    assert!(matches!(
        transition.verify(&accepted, &bindings),
        Err(CoreError::InvalidIdentityTransition(_))
    ));

    let other = IdentityState::initial(
        IdentityDocument::new(
            "other",
            "other repository",
            "main",
            Visibility::Public,
            vec![alice.id()],
            1,
        )
        .expect("other document"),
    )
    .expect("other state");
    assert!(matches!(
        transition.verify(&other, &bindings),
        Err(CoreError::InvalidIdentityTransition(_))
    ));
}

#[test]
fn reference_transition_rejects_replay_and_cross_repository_use() {
    let alice = Actor::seeded(6, 16);
    let repository = project(&[&alice], 1)
        .repository_id()
        .expect("repository id");
    let first = RefTransition::signed(
        repository,
        &alice.binding,
        &alice.repository_key,
        None,
        vec![main_ref(1)],
    )
    .expect("first transition");
    let first_state = first
        .verify(repository, &alice.binding, &alice.selection(), None)
        .expect("first verification");
    let second = RefTransition::signed(
        repository,
        &alice.binding,
        &alice.repository_key,
        Some(&first_state),
        vec![main_ref(2)],
    )
    .expect("second transition");
    let second_state = second
        .verify(
            repository,
            &alice.binding,
            &alice.selection(),
            Some(&first_state),
        )
        .expect("second verification");

    assert!(matches!(
        first.verify(
            repository,
            &alice.binding,
            &alice.selection(),
            Some(&second_state)
        ),
        Err(CoreError::InvalidRefTransition(_))
    ));

    let other_repository = IdentityDocument::new(
        "other",
        "other",
        "main",
        Visibility::Public,
        vec![alice.id()],
        1,
    )
    .expect("other identity")
    .repository_id()
    .expect("other repository id");
    assert!(matches!(
        second.verify(
            other_repository,
            &alice.binding,
            &alice.selection(),
            Some(&first_state)
        ),
        Err(CoreError::InvalidRefTransition(_))
    ));
}

#[test]
fn canonical_head_requires_delegate_threshold_and_refuses_rewind() {
    let alice = Actor::seeded(7, 17);
    let bob = Actor::seeded(8, 18);
    let carol = Actor::seeded(9, 19);
    let document = project(&[&alice, &bob, &carol], 2);
    let repository = document.repository_id().expect("repository id");
    let agreed = GitObjectId::sha1([2; 20]);
    let old = GitObjectId::sha1([1; 20]);

    let state = |actor: &Actor, target: GitObjectId| RefState {
        repository,
        publisher: actor.id(),
        transition: styrene_git_core::Digest::new([0; 32]),
        sequence: 0,
        refs: vec![RefTarget {
            name: "refs/heads/main".into(),
            target,
        }],
    };
    let states = BTreeMap::from([
        (alice.id(), state(&alice, agreed.clone())),
        (bob.id(), state(&bob, agreed.clone())),
        (carol.id(), state(&carol, old.clone())),
    ]);

    assert_eq!(
        derive_canonical_head(&document, &states, Some(&old), |previous, candidate| {
            previous == &old && candidate == &agreed
        }),
        CanonicalDecision::Advance(agreed.clone())
    );
    assert_eq!(
        derive_canonical_head(&document, &states, Some(&old), |_previous, _candidate| {
            false
        }),
        CanonicalDecision::Diverged(agreed)
    );
}

#[test]
fn canonical_head_ignores_non_delegates_and_duplicate_publisher_entries() {
    let alice = Actor::seeded(23, 33);
    let bob = Actor::seeded(24, 34);
    let outsider = Actor::seeded(25, 35);
    let document = project(&[&alice, &bob], 2);
    let target = GitObjectId::sha1([9; 20]);
    let competing = GitObjectId::sha1([10; 20]);
    let state = |actor: &Actor, head: GitObjectId| RefState {
        repository: document.repository_id().expect("repository id"),
        publisher: actor.id(),
        transition: styrene_git_core::Digest::new([0; 32]),
        sequence: 0,
        refs: vec![RefTarget {
            name: "refs/heads/main".into(),
            target: head,
        }],
    };
    let disagreement = BTreeMap::from([
        (alice.id(), state(&alice, target.clone())),
        (bob.id(), state(&bob, competing)),
        (outsider.id(), state(&outsider, target.clone())),
    ]);
    assert_eq!(
        derive_canonical_head(&document, &disagreement, None, |_, _| true),
        CanonicalDecision::NoAgreement
    );

    let mut duplicate = BTreeMap::new();
    duplicate.insert(alice.id(), state(&alice, target.clone()));
    duplicate.insert(alice.id(), state(&alice, target));
    assert_eq!(
        derive_canonical_head(&document, &duplicate, None, |_, _| true),
        CanonicalDecision::NoAgreement
    );
}

#[test]
fn three_small_project_shapes_have_distinct_repository_ids() {
    let alice = Actor::seeded(10, 20);
    let projects = [
        ("cli", "main"),
        ("library", "master"),
        ("firmware", "stable"),
    ];
    let ids: std::collections::BTreeSet<_> = projects
        .into_iter()
        .map(|(name, branch)| {
            IdentityDocument::new(
                name,
                "integration project",
                branch,
                Visibility::Public,
                vec![alice.id()],
                1,
            )
            .expect("project fixture")
            .repository_id()
            .expect("project id")
        })
        .collect();
    assert_eq!(ids.len(), 3);
}
