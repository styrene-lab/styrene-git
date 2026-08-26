use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use ed25519_dalek::SigningKey;
use styrene_git_core::{
    CanonicalCbor, CoreError, Digest, GitObjectId, IdentityDocument, PublicKey, RefState,
    RefTarget, RefTransition, RepositoryId, SignerBinding, SignerSelection, StyreneIdentity,
    Visibility,
};
use styrene_git_protocol::{
    apply_transfer, export_transfer, ApplyOutcome, Prerequisite, ProtocolError,
    SignerAuthorization, StateWant, Transfer,
};
use styrene_git_store::{ObjectFormat, Repository, RepositoryStore, StoreError};

const MAX_PACK: u64 = 1024 * 1024;
static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "styrene-git-protocol-test-{}-{sequence}",
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

struct Fixture {
    _source_directory: TestDirectory,
    source: Repository,
    repository_id: RepositoryId,
    binding: SignerBinding,
    selection: SignerSelection,
    transition: RefTransition,
    state: RefState,
    head: GitObjectId,
}

impl Fixture {
    fn new() -> Self {
        let identity_key = SigningKey::from_bytes(&[41; 32]);
        let repository_key = SigningKey::from_bytes(&[42; 32]);
        let binding = SignerBinding::issue(
            &identity_key,
            PublicKey::new(repository_key.verifying_key().to_bytes()),
            0,
        )
        .expect("binding");
        let selection = binding.selection().expect("signer selection");
        let identity = IdentityDocument::new(
            "replication-fixture",
            "real Git graph",
            "main",
            Visibility::Public,
            vec![binding.identity()],
            1,
        )
        .expect("identity");
        let repository_id = identity.repository_id().expect("repository ID");
        let source_directory = TestDirectory::new();
        let source = RepositoryStore::new(&source_directory.0)
            .expect("source store")
            .create(repository_id, ObjectFormat::default())
            .expect("source repository");

        let tree = source.object_id("tree", &[]).expect("empty tree ID");
        let commit_bytes = format!(
            "tree {}\nauthor Styrene <styrene@example.com> 0 +0000\ncommitter Styrene <styrene@example.com> 0 +0000\n\ncarrier-neutral fixture\n",
            hex::encode(&tree.bytes)
        );
        let head = source
            .object_id("commit", commit_bytes.as_bytes())
            .expect("commit ID");
        let quarantine = source.begin_quarantine().expect("source quarantine");
        quarantine
            .write_verified_object("tree", &[], &tree)
            .expect("tree import");
        quarantine
            .write_verified_object("commit", commit_bytes.as_bytes(), &head)
            .expect("commit import");
        let transition = RefTransition::signed(
            repository_id,
            &binding,
            &repository_key,
            None,
            vec![RefTarget {
                name: "refs/heads/main".into(),
                target: head.clone(),
            }],
        )
        .expect("transition");
        let state = source
            .commit(&quarantine, &transition, &binding, &selection)
            .expect("source commit")
            .into_state();
        Self {
            _source_directory: source_directory,
            source,
            repository_id,
            binding,
            selection,
            transition,
            state,
            head,
        }
    }

    fn destination(&self) -> (TestDirectory, Repository) {
        let directory = TestDirectory::new();
        let repository = RepositoryStore::new(&directory.0)
            .expect("destination store")
            .create(self.repository_id, ObjectFormat::default())
            .expect("destination repository");
        (directory, repository)
    }

    fn transfer(&self) -> Transfer {
        export_transfer(
            &self.source,
            SignerAuthorization::new(&self.binding, &self.selection),
            &self.transition,
            None,
            std::slice::from_ref(&self.head),
            &[],
            MAX_PACK,
        )
        .expect("pack transfer")
    }
}

#[test]
fn replicates_a_real_git_graph_and_is_idempotent() {
    let fixture = Fixture::new();
    let transfer = fixture.transfer();
    let bytes = transfer.canonical_bytes().expect("canonical transfer");
    let decoded = Transfer::from_canonical_bytes(&bytes).expect("decoded transfer");
    let (_directory, destination) = fixture.destination();

    assert_eq!(
        apply_transfer(&destination, &decoded, &fixture.selection, MAX_PACK)
            .expect("first application"),
        ApplyOutcome::Applied(fixture.state.clone())
    );
    assert!(destination.has_object(&fixture.head).expect("head lookup"));
    assert_eq!(
        apply_transfer(&destination, &decoded, &fixture.selection, MAX_PACK)
            .expect("duplicate application"),
        ApplyOutcome::AlreadyPresent(fixture.state)
    );
}

#[test]
fn identical_bytes_have_identical_results_across_test_carriers() {
    let fixture = Fixture::new();
    let bytes = fixture
        .transfer()
        .canonical_bytes()
        .expect("transfer bytes");
    let (_first_directory, first) = fixture.destination();
    let (_second_directory, second) = fixture.destination();
    let direct_stream = Transfer::from_canonical_bytes(&bytes).expect("direct transfer");
    let resource = Transfer::from_canonical_bytes(&bytes).expect("resource transfer");

    let first_result = apply_transfer(&first, &direct_stream, &fixture.selection, MAX_PACK)
        .expect("direct result");
    let second_result =
        apply_transfer(&second, &resource, &fixture.selection, MAX_PACK).expect("resource result");
    assert_eq!(first_result, second_result);
    assert_eq!(
        first
            .publisher_state(fixture.binding.identity())
            .expect("first state"),
        second
            .publisher_state(fixture.binding.identity())
            .expect("second state")
    );
}

#[test]
fn concurrent_duplicate_delivery_commits_once() {
    let fixture = Fixture::new();
    let transfer = fixture.transfer();
    let (_directory, destination) = fixture.destination();
    let barrier = Arc::new(Barrier::new(3));
    let first_repository = destination.clone();
    let first_transfer = transfer.clone();
    let first_selection = fixture.selection;
    let first_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        apply_transfer(
            &first_repository,
            &first_transfer,
            &first_selection,
            MAX_PACK,
        )
    });
    let second_repository = destination.clone();
    let second_transfer = transfer.clone();
    let second_selection = fixture.selection;
    let second_barrier = Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        apply_transfer(
            &second_repository,
            &second_transfer,
            &second_selection,
            MAX_PACK,
        )
    });
    barrier.wait();
    let outcomes = [
        first.join().expect("first thread").expect("first result"),
        second
            .join()
            .expect("second thread")
            .expect("second result"),
    ];

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ApplyOutcome::Applied(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ApplyOutcome::AlreadyPresent(_)))
            .count(),
        1
    );
    assert_eq!(
        destination
            .publisher_state(fixture.binding.identity())
            .expect("final state"),
        Some(fixture.state)
    );
}

#[test]
fn rejects_modified_and_oversized_payloads_before_state_changes() {
    let fixture = Fixture::new();
    let (directory, destination) = fixture.destination();
    let mut modified = fixture.transfer();
    modified.payload[0] ^= 0xff;
    assert!(matches!(
        apply_transfer(&destination, &modified, &fixture.selection, MAX_PACK),
        Err(ProtocolError::IntegrityMismatch)
    ));
    assert_eq!(
        destination
            .publisher_state(fixture.binding.identity())
            .expect("unchanged state"),
        None
    );
    drop(directory);

    let (_directory, destination) = fixture.destination();
    let transfer = fixture.transfer();
    assert!(matches!(
        apply_transfer(&destination, &transfer, &fixture.selection, 4),
        Err(ProtocolError::PayloadTooLarge { limit: 4 })
    ));
}

#[test]
fn adversarial_decode_pack_and_interruption_corpus_preserves_state() {
    let fixture = Fixture::new();
    let canonical = fixture
        .transfer()
        .canonical_bytes()
        .expect("canonical transfer");

    for cut in 0..canonical.len() {
        assert!(
            Transfer::from_canonical_bytes(&canonical[..cut]).is_err(),
            "truncated transfer at byte {cut} must fail"
        );
    }
    let mut trailing = canonical.clone();
    trailing.push(0);
    assert!(Transfer::from_canonical_bytes(&trailing).is_err());
    assert_eq!(canonical[0], 0x84, "fixture must use a canonical array");
    let mut long_form_array = vec![0x98, 0x04];
    long_form_array.extend_from_slice(&canonical[1..]);
    assert!(Transfer::from_canonical_bytes(&long_form_array).is_err());

    let valid_pack = fixture.transfer().payload;
    let mut modified_pack = valid_pack.clone();
    let middle = modified_pack.len() / 2;
    modified_pack[middle] ^= 0x80;
    let corrupt_packs = [
        Vec::new(),
        b"PACK".to_vec(),
        valid_pack[..valid_pack.len() / 2].to_vec(),
        modified_pack,
    ];
    for pack in corrupt_packs {
        let transfer = Transfer::new(
            fixture.repository_id,
            &fixture.binding,
            &fixture.transition,
            None,
            Vec::new(),
            pack,
        )
        .expect("integrity-consistent corrupt transfer");
        let (_directory, destination) = fixture.destination();
        assert!(apply_transfer(&destination, &transfer, &fixture.selection, MAX_PACK).is_err());
        assert_eq!(
            destination
                .publisher_state(fixture.binding.identity())
                .expect("unchanged state"),
            None
        );
        assert!(!destination.has_object(&fixture.head).expect("head lookup"));
        destination.fsck().expect("repository remains valid");
    }
}

#[test]
fn reports_missing_prerequisites_without_importing_the_pack() {
    let fixture = Fixture::new();
    let (_directory, destination) = fixture.destination();
    let base = fixture.transfer();
    let missing = GitObjectId::sha256([7; 32]);
    let transfer = Transfer::new(
        fixture.repository_id,
        &fixture.binding,
        &fixture.transition,
        Some(Digest::new([8; 32])),
        vec![missing.clone()],
        base.payload,
    )
    .expect("transfer with prerequisites");

    assert_eq!(
        apply_transfer(&destination, &transfer, &fixture.selection, MAX_PACK)
            .expect("prerequisite result"),
        ApplyOutcome::MissingPrerequisites(vec![
            Prerequisite::PublisherTransition(Digest::new([8; 32])),
            Prerequisite::Object(missing),
        ])
    );
    assert!(!destination.has_object(&fixture.head).expect("head lookup"));
}

#[test]
fn carrier_identity_cannot_replace_repository_authority() {
    let fixture = Fixture::new();
    let (_directory, destination) = fixture.destination();
    let mut transfer = fixture.transfer();
    transfer.manifest.publisher = StyreneIdentity::new([99; 16]);

    assert!(matches!(
        apply_transfer(&destination, &transfer, &fixture.selection, MAX_PACK),
        Err(ProtocolError::IntegrityMismatch)
    ));
    assert_eq!(
        destination
            .publisher_state(fixture.binding.identity())
            .expect("state"),
        None
    );
}

#[test]
fn stale_binding_is_rejected_before_pack_import() {
    let fixture = Fixture::new();
    let identity_key = SigningKey::from_bytes(&[41; 32]);
    let current_repository_key = SigningKey::from_bytes(&[43; 32]);
    let current_binding = SignerBinding::issue(
        &identity_key,
        PublicKey::new(current_repository_key.verifying_key().to_bytes()),
        1,
    )
    .expect("current binding");
    let transfer = fixture.transfer();
    let (_directory, destination) = fixture.destination();

    assert!(matches!(
        apply_transfer(
            &destination,
            &transfer,
            &current_binding.selection().expect("current selection"),
            MAX_PACK
        ),
        Err(ProtocolError::Core(CoreError::InvalidBinding(message)))
            if message == "binding does not match prior-state signer selection"
    ));
    assert_eq!(
        destination
            .publisher_state(fixture.binding.identity())
            .expect("publisher state"),
        None
    );
    assert!(!destination.has_object(&fixture.head).expect("head lookup"));
}

#[test]
fn pack_export_enforces_the_output_limit() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture
            .source
            .export_pack(std::slice::from_ref(&fixture.head), &[], 4),
        Err(StoreError::PackTooLarge { limit: 4 })
    ));
}

#[test]
fn state_want_has_stable_golden_bytes() {
    let want = StateWant {
        version: 1,
        repository: RepositoryId::new(Digest::new([1; 32])),
        publisher: StyreneIdentity::new([2; 16]),
        after: Some(Digest::new([3; 32])),
    };
    assert_eq!(
        hex::encode(want.canonical_bytes().expect("want bytes")),
        "840158200101010101010101010101010101010101010101010101010101010101010101500202020202020202020202020202020258200303030303030303030303030303030303030303030303030303030303030303"
    );
}
