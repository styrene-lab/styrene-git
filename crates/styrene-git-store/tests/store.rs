use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use ed25519_dalek::SigningKey;
use styrene_git_core::{
    GitObjectId, IdentityDocument, IdentityState, PublicKey, RefTarget, RefTransition,
    SignerBinding, SignerSelection, StyreneIdentity, Visibility,
};
use styrene_git_store::{CommitOutcome, ObjectFormat, Repository, RepositoryStore, StoreError};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "styrene-git-store-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory should be unique");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Actor {
    repository_key: SigningKey,
    binding: SignerBinding,
}

impl Actor {
    fn seeded(identity_seed: u8, repository_seed: u8) -> Self {
        let identity_key = SigningKey::from_bytes(&[identity_seed; 32]);
        let repository_key = SigningKey::from_bytes(&[repository_seed; 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            0,
        )
        .expect("fixed keys should create a binding");
        Self {
            repository_key,
            binding,
        }
    }

    fn id(&self) -> StyreneIdentity {
        self.binding.identity()
    }

    fn selection(&self) -> SignerSelection {
        self.binding
            .selection()
            .expect("fixture binding should produce a selection")
    }
}

fn repository(actors: &[&Actor]) -> (TestDirectory, RepositoryStore, Repository) {
    let directory = TestDirectory::new();
    let store = RepositoryStore::new(&directory.0).expect("store creation");
    let identity = IdentityDocument::new(
        "store-fixture",
        "bare Git integration",
        "main",
        Visibility::Public,
        actors.iter().map(|actor| actor.id()).collect(),
        1,
    )
    .expect("identity document");
    let id = IdentityState::initial(identity)
        .expect("initial identity")
        .repository;
    let repository = store
        .create(id, ObjectFormat::default())
        .expect("bare repository creation");
    (directory, store, repository)
}

fn publish_blob(
    repository: &Repository,
    actor: &Actor,
    previous: Option<&styrene_git_core::RefState>,
    name: &str,
    contents: &[u8],
) -> styrene_git_core::RefState {
    let object_id = repository
        .object_id("blob", contents)
        .expect("object ID calculation");
    let quarantine = repository.begin_quarantine().expect("quarantine");
    quarantine
        .write_verified_object("blob", contents, &object_id)
        .expect("verified object import");
    let transition = RefTransition::signed(
        repository.id(),
        &actor.binding,
        &actor.repository_key,
        previous,
        vec![RefTarget {
            name: name.into(),
            target: object_id,
        }],
    )
    .expect("signed transition");
    repository
        .commit(&quarantine, &transition, &actor.binding, &actor.selection())
        .expect("committed transition")
        .into_state()
}

#[test]
fn creates_reopens_and_commits_isolated_publisher_namespaces() {
    let alice = Actor::seeded(1, 11);
    let bob = Actor::seeded(2, 12);
    let (_directory, store, repository) = repository(&[&alice, &bob]);
    let alice_state = publish_blob(
        &repository,
        &alice,
        None,
        "refs/heads/main",
        b"shared contents",
    );
    let bob_state = publish_blob(
        &repository,
        &bob,
        None,
        "refs/heads/main",
        b"shared contents",
    );

    assert_eq!(alice_state.refs[0].target, bob_state.refs[0].target);
    assert_eq!(
        repository.publisher_refs(alice.id()).expect("Alice refs"),
        alice_state.refs
    );
    assert_eq!(
        repository
            .publisher_ref_target(alice.id(), "refs/heads/main")
            .expect("physical Alice ref"),
        Some(alice_state.refs[0].target.clone())
    );
    assert_eq!(
        repository.publisher_refs(bob.id()).expect("Bob refs"),
        bob_state.refs
    );
    let reopened = store.open(repository.id()).expect("repository lookup");
    assert_eq!(
        reopened.publisher_state(alice.id()).expect("stored state"),
        Some(alice_state)
    );
}

#[test]
fn complete_map_deletes_refs_without_touching_another_publisher() {
    let alice = Actor::seeded(3, 13);
    let bob = Actor::seeded(4, 14);
    let (_directory, _store, repository) = repository(&[&alice, &bob]);
    let alice_initial = publish_blob(&repository, &alice, None, "refs/heads/main", b"Alice main");
    let bob_initial = publish_blob(&repository, &bob, None, "refs/heads/main", b"Bob main");
    let alice_next = publish_blob(
        &repository,
        &alice,
        Some(&alice_initial),
        "refs/heads/topic",
        b"Alice topic",
    );

    assert_eq!(
        repository.publisher_refs(alice.id()).expect("Alice refs"),
        alice_next.refs
    );
    assert_eq!(
        repository
            .publisher_ref_target(alice.id(), "refs/heads/main")
            .expect("deleted physical ref"),
        None
    );
    assert_eq!(
        repository.publisher_refs(bob.id()).expect("Bob refs"),
        bob_initial.refs
    );
}

#[test]
fn rejects_wrong_or_missing_objects_without_changing_accepted_state() {
    let alice = Actor::seeded(5, 15);
    let (_directory, _store, repository) = repository(&[&alice]);
    let initial = publish_blob(&repository, &alice, None, "refs/heads/main", b"accepted");
    let quarantine = repository.begin_quarantine().expect("quarantine");
    let wrong_id = GitObjectId::sha256([9; 32]);
    assert!(matches!(
        quarantine.write_verified_object("blob", b"corrupt for declared ID", &wrong_id),
        Err(StoreError::ObjectIdMismatch)
    ));
    let transition = RefTransition::signed(
        repository.id(),
        &alice.binding,
        &alice.repository_key,
        Some(&initial),
        vec![RefTarget {
            name: "refs/heads/main".into(),
            target: wrong_id,
        }],
    )
    .expect("signed missing-object transition");
    assert!(matches!(
        repository.commit(&quarantine, &transition, &alice.binding, &alice.selection()),
        Err(StoreError::Git { .. })
    ));
    assert_eq!(
        repository
            .publisher_state(alice.id())
            .expect("accepted state"),
        Some(initial)
    );
}

#[test]
fn dropped_quarantine_never_changes_accepted_refs() {
    let alice = Actor::seeded(6, 16);
    let (_directory, _store, repository) = repository(&[&alice]);
    let object_id = repository
        .object_id("blob", b"interrupted")
        .expect("object ID");
    {
        let quarantine = repository.begin_quarantine().expect("quarantine");
        quarantine
            .write_verified_object("blob", b"interrupted", &object_id)
            .expect("quarantine write");
    }
    assert!(repository
        .publisher_refs(alice.id())
        .expect("publisher refs")
        .is_empty());
}

#[test]
fn concurrent_duplicate_commits_promote_objects_once_and_linearize() {
    let alice = Actor::seeded(21, 31);
    let (_directory, _store, repository) = repository(&[&alice]);
    let object = repository
        .object_id("blob", b"concurrent duplicate")
        .expect("object ID");
    let first_quarantine = repository.begin_quarantine().expect("first quarantine");
    first_quarantine
        .write_verified_object("blob", b"concurrent duplicate", &object)
        .expect("first object");
    let second_quarantine = repository.begin_quarantine().expect("second quarantine");
    second_quarantine
        .write_verified_object("blob", b"concurrent duplicate", &object)
        .expect("second object");
    let transition = RefTransition::signed(
        repository.id(),
        &alice.binding,
        &alice.repository_key,
        None,
        vec![RefTarget {
            name: "refs/heads/main".into(),
            target: object.clone(),
        }],
    )
    .expect("transition");
    let barrier = Arc::new(Barrier::new(3));

    let first_repository = repository.clone();
    let first_transition = transition.clone();
    let first_binding = alice.binding.clone();
    let first_selection = alice.selection();
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_repository.commit(
            &first_quarantine,
            &first_transition,
            &first_binding,
            &first_selection,
        )
    });

    let second_repository = repository.clone();
    let second_transition = transition.clone();
    let second_binding = alice.binding.clone();
    let second_selection = alice.selection();
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_repository.commit(
            &second_quarantine,
            &second_transition,
            &second_binding,
            &second_selection,
        )
    });

    barrier.wait();
    let outcomes = [
        first.join().expect("first thread").expect("first commit"),
        second
            .join()
            .expect("second thread")
            .expect("second commit"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, CommitOutcome::Applied(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, CommitOutcome::AlreadyPresent(_)))
            .count(),
        1
    );
    assert_eq!(outcomes[0].state(), outcomes[1].state());
    assert_eq!(
        repository.publisher_state(alice.id()).expect("final state"),
        Some(outcomes[0].state().clone())
    );
    assert!(repository.has_object(&object).expect("promoted object"));
    assert_eq!(
        repository
            .commit(
                &repository.begin_quarantine().expect("duplicate quarantine"),
                &transition,
                &alice.binding,
                &alice.selection(),
            )
            .expect("sequential duplicate"),
        CommitOutcome::AlreadyPresent(outcomes[0].state().clone())
    );
}

#[test]
fn rejects_cross_publisher_and_stale_transitions_without_partial_state() {
    let alice = Actor::seeded(8, 18);
    let bob = Actor::seeded(9, 19);
    let (_directory, _store, repository) = repository(&[&alice, &bob]);
    let initial = publish_blob(&repository, &alice, None, "refs/heads/main", b"initial");
    let object = repository
        .object_id("blob", b"next")
        .expect("next object ID");
    let quarantine = repository.begin_quarantine().expect("quarantine");
    quarantine
        .write_verified_object("blob", b"next", &object)
        .expect("next object");
    let transition = RefTransition::signed(
        repository.id(),
        &alice.binding,
        &alice.repository_key,
        Some(&initial),
        vec![RefTarget {
            name: "refs/heads/main".into(),
            target: object,
        }],
    )
    .expect("transition");
    assert!(repository
        .commit(&quarantine, &transition, &bob.binding, &bob.selection())
        .is_err());
    let accepted = repository
        .commit(&quarantine, &transition, &alice.binding, &alice.selection())
        .expect("Alice commit")
        .into_state();
    assert_eq!(
        repository
            .commit(&quarantine, &transition, &alice.binding, &alice.selection())
            .expect("duplicate Alice commit"),
        CommitOutcome::AlreadyPresent(accepted.clone())
    );
    assert_eq!(
        repository.publisher_state(alice.id()).expect("Alice state"),
        Some(accepted)
    );
    assert_eq!(
        repository.publisher_state(bob.id()).expect("Bob state"),
        None
    );
}

#[test]
fn rejects_an_object_graph_with_missing_reachable_objects() {
    let alice = Actor::seeded(10, 20);
    let (_directory, _store, repository) = repository(&[&alice]);
    let malformed_commit = format!(
        "tree {}\nauthor Test <test@example.com> 0 +0000\ncommitter Test <test@example.com> 0 +0000\n\nmissing tree\n",
        "0".repeat(64)
    );
    let object = repository
        .object_id("commit", malformed_commit.as_bytes())
        .expect("commit object ID");
    let quarantine = repository.begin_quarantine().expect("quarantine");
    quarantine
        .write_verified_object("commit", malformed_commit.as_bytes(), &object)
        .expect("commit import");
    let transition = RefTransition::signed(
        repository.id(),
        &alice.binding,
        &alice.repository_key,
        None,
        vec![RefTarget {
            name: "refs/heads/main".into(),
            target: object,
        }],
    )
    .expect("transition");

    assert!(matches!(
        repository.commit(&quarantine, &transition, &alice.binding, &alice.selection()),
        Err(StoreError::Git { .. })
    ));
    assert_eq!(repository.publisher_state(alice.id()).expect("state"), None);
}

#[test]
fn defaults_to_sha256_bare_repositories() {
    let alice = Actor::seeded(7, 17);
    let directory = TestDirectory::new();
    let store = RepositoryStore::new(&directory.0).expect("store");
    let identity = IdentityDocument::new(
        "sha256",
        "fixture",
        "main",
        Visibility::Public,
        vec![alice.id()],
        1,
    )
    .expect("identity");
    let id = identity.repository_id().expect("repository ID");
    let repository = store
        .create(id, ObjectFormat::default())
        .expect("SHA-256 repository");
    assert_eq!(ObjectFormat::default(), ObjectFormat::Sha256);
    let state = publish_blob(
        &repository,
        &alice,
        None,
        "refs/heads/main",
        b"sha256 object",
    );
    assert_eq!(state.refs[0].target.algorithm, "sha256");
}

#[test]
fn opens_sha1_only_as_explicit_legacy_compatibility() {
    let alice = Actor::seeded(11, 21);
    let directory = TestDirectory::new();
    let store = RepositoryStore::new(&directory.0).expect("store");
    let identity = IdentityDocument::new(
        "legacy-sha1",
        "compatibility fixture",
        "main",
        Visibility::Public,
        vec![alice.id()],
        1,
    )
    .expect("identity");
    let id = identity.repository_id().expect("repository ID");
    let repository = store
        .create(id, ObjectFormat::LegacySha1)
        .expect("legacy SHA-1 repository");
    let state = publish_blob(
        &repository,
        &alice,
        None,
        "refs/heads/main",
        b"legacy object",
    );
    assert_eq!(repository.object_format(), ObjectFormat::LegacySha1);
    assert_eq!(state.refs[0].target.algorithm, "sha1");
}
