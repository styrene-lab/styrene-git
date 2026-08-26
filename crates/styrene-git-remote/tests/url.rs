use std::str::FromStr;

use styrene_git_core::{Digest, RepositoryId, StyreneIdentity};
use styrene_git_ipc::{RepositoryView, RequestBody};
use styrene_git_remote::{GitRemoteUrl, UrlError};

fn digest() -> String {
    Digest::new([7; 32]).base32()
}

fn canonical() -> String {
    format!("styrene:///git/v1/{}", digest())
}

#[test]
fn parses_canonical_and_publisher_views() {
    let parsed_canonical = GitRemoteUrl::from_str(&canonical()).expect("canonical URL");
    assert_eq!(parsed_canonical.to_string(), canonical());
    assert_eq!(parsed_canonical.view(), &RepositoryView::Canonical);
    assert_eq!(
        parsed_canonical.repository(),
        RepositoryId::new(Digest::new([7; 32]))
    );

    let publisher = StyreneIdentity::new([8; 16]);
    let value = format!("{}/publisher/{publisher}", canonical());
    let parsed = GitRemoteUrl::from_str(&value).expect("publisher URL");
    assert_eq!(parsed.view(), &RepositoryView::Publisher(publisher));
    assert_eq!(parsed.to_string(), value);
}

#[test]
fn accepts_only_canonical_non_authoritative_labels() {
    let value = format!("{}?label=nearby&label=trusted2", canonical());
    let parsed = GitRemoteUrl::from_str(&value).expect("labeled URL");
    assert_eq!(parsed.labels(), &["nearby", "trusted2"]);
    assert_eq!(parsed.to_string(), value);
    assert!(matches!(
        parsed.synchronization_request(),
        RequestBody::StartSynchronization { labels, .. }
            if labels == ["nearby", "trusted2"]
    ));

    for query in [
        "label=trusted&label=nearby",
        "label=nearby&label=nearby",
        "label=127.0.0.1",
        "label=lxmf:destination",
        "label=%6eearby",
    ] {
        assert!(GitRemoteUrl::from_str(&format!("{}?{query}", canonical())).is_err());
    }
}

#[test]
fn rejects_authorities_fragments_and_transport_parameters() {
    assert_eq!(
        GitRemoteUrl::from_str(&format!("styrene://{}/", digest())),
        Err(UrlError::AuthorityNotAllowed)
    );
    assert_eq!(
        GitRemoteUrl::from_str(&format!("{}#peer", canonical())),
        Err(UrlError::FragmentNotAllowed)
    );
    for query in ["carrier=lxmf", "peer=abc", "route=mesh", "address=host"] {
        assert_eq!(
            GitRemoteUrl::from_str(&format!("{}?{query}", canonical())),
            Err(UrlError::UnsupportedQueryKey)
        );
    }
}

#[test]
fn rejects_noncanonical_repository_and_publisher_paths() {
    assert_eq!(
        GitRemoteUrl::from_str("styrene:///git/v1/not-a-digest"),
        Err(UrlError::InvalidRepository)
    );
    assert_eq!(
        GitRemoteUrl::from_str(&format!("{}/publisher/not-an-identity", canonical())),
        Err(UrlError::InvalidPublisher)
    );
    assert_eq!(
        GitRemoteUrl::from_str(&format!("{}/publisher", canonical())),
        Err(UrlError::InvalidPath)
    );
}
