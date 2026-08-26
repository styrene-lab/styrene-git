use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use styrene_git_core::GitObjectId;
use styrene_git_remote::{GitCommand, GitError, GitObjectFormat, GitPlumbing};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "styrene-git-remote-test-{}-{sequence}",
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

fn run_git(directory: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Styrene Test")
        .env("GIT_AUTHOR_EMAIL", "styrene@example.invalid")
        .env("GIT_COMMITTER_NAME", "Styrene Test")
        .env("GIT_COMMITTER_EMAIL", "styrene@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .expect("Git fixture command should start");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn initialize(format: &str, commit: bool) -> (TestDirectory, GitCommand) {
    let directory = TestDirectory::new();
    run_git(
        &directory.0,
        &["init", "--quiet", &format!("--object-format={format}")],
    );
    if commit {
        fs::write(directory.0.join("tracked.txt"), b"styrene git plumbing\n")
            .expect("fixture file");
        run_git(&directory.0, &["add", "tracked.txt"]);
        run_git(&directory.0, &["commit", "--quiet", "-m", "fixture"]);
    }
    let plumbing = GitCommand::new(directory.0.join(".git"));
    (directory, plumbing)
}

#[test]
fn discovers_formats_resolves_revisions_and_bounds_prerequisites() {
    let (sha256_directory, sha256) = initialize("sha256", true);
    assert_eq!(
        sha256.object_format().expect("SHA-256 object format"),
        GitObjectFormat::Sha256
    );
    let first_head = sha256.resolve_revision("HEAD").expect("SHA-256 HEAD");
    assert_eq!(first_head.algorithm, "sha256");
    fs::write(
        sha256_directory.0.join("tracked.txt"),
        b"second fixture revision\n",
    )
    .expect("updated fixture file");
    run_git(&sha256_directory.0, &["add", "tracked.txt"]);
    run_git(
        &sha256_directory.0,
        &["commit", "--quiet", "-m", "second fixture"],
    );
    let head = sha256
        .resolve_revision("HEAD")
        .expect("second SHA-256 HEAD");
    assert_eq!(
        sha256.local_prerequisites(1).expect("one prerequisite"),
        [head]
    );
    assert!(sha256
        .local_prerequisites(0)
        .expect("zero prerequisites")
        .is_empty());

    let (_sha1_directory, sha1) = initialize("sha1", true);
    assert_eq!(
        sha1.object_format().expect("SHA-1 object format"),
        GitObjectFormat::Sha1
    );
    assert_eq!(
        sha1.resolve_revision("HEAD").expect("SHA-1 HEAD").algorithm,
        "sha1"
    );
}

#[test]
fn creates_bounded_pack_and_installs_it_in_another_repository() {
    for format in ["sha256", "sha1"] {
        let (_source_directory, source) = initialize(format, true);
        let head = source.resolve_revision("HEAD").expect("source HEAD");
        let pack = source
            .create_push_pack(std::slice::from_ref(&head), &[], 1, 1024 * 1024)
            .expect("bounded pack");
        assert!(pack.starts_with(b"PACK"));

        let (destination_directory, destination) = initialize(format, false);
        destination
            .install_fetch_pack(&pack, 1024 * 1024)
            .expect("pack installation");
        run_git(
            &destination_directory.0,
            &["cat-file", "-e", &format!("{}^{{object}}", head.hex())],
        );
    }
}

#[test]
fn rejects_invalid_inputs_and_enforces_pack_limits() {
    let (_directory, plumbing) = initialize("sha256", true);
    let head = plumbing.resolve_revision("HEAD").expect("source HEAD");

    assert!(matches!(
        plumbing.resolve_revision("HEAD\n--help"),
        Err(GitError::InvalidRevision)
    ));
    assert!(matches!(
        plumbing.resolve_revision("refs/heads/missing"),
        Err(GitError::Command { .. })
    ));
    assert!(matches!(
        plumbing.create_push_pack(std::slice::from_ref(&head), &[], 0, 1024),
        Err(GitError::CountLimitExceeded { limit: 0 })
    ));
    assert!(matches!(
        plumbing.create_push_pack(&[GitObjectId::sha1([1; 20])], &[], 1, 1024),
        Err(GitError::ObjectFormatMismatch)
    ));
    assert!(matches!(
        plumbing.create_push_pack(std::slice::from_ref(&head), &[], 1, 3),
        Err(GitError::OutputLimitExceeded { limit: 3, .. })
    ));
    assert!(matches!(
        plumbing.install_fetch_pack(b"PACK", 3),
        Err(GitError::InputLimitExceeded { limit: 3 })
    ));
}
