//! Parsing and remote-helper behavior for carrier-neutral Styrene Git URLs.

mod client;
mod command_loop;
mod git;
mod operations;

use std::fmt;
use std::str::FromStr;

use styrene_git_core::{RepositoryId, StyreneIdentity};
use styrene_git_ipc::{RepositoryView, RequestBody};

pub use client::{
    AuthenticatedGitTransport, ClientConfig, ClientError, GitIpcClient, TransportError,
};
pub use command_loop::{run_command_loop, CommandLoopError};
pub use git::{GitCommand, GitError, GitObjectFormat, GitPlumbing};
pub use operations::{FetchCommand, HelperError, PushCommand, RemoteSession};

const URL_PREFIX: &str = "styrene:///git/v1/";
pub const MAX_ROUTING_LABELS: usize = 16;
pub const MAX_ROUTING_LABEL_BYTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRemoteUrl {
    repository: RepositoryId,
    view: RepositoryView,
    labels: Vec<String>,
}

impl GitRemoteUrl {
    pub const fn repository(&self) -> RepositoryId {
        self.repository
    }

    pub const fn view(&self) -> &RepositoryView {
        &self.view
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn synchronization_request(&self) -> RequestBody {
        RequestBody::StartSynchronization {
            repository: self.repository,
            view: self.view.clone(),
            labels: self.labels.clone(),
        }
    }
}

impl FromStr for GitRemoteUrl {
    type Err = UrlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.contains('#') {
            return Err(UrlError::FragmentNotAllowed);
        }
        if value.starts_with("styrene://") && !value.starts_with("styrene:///") {
            return Err(UrlError::AuthorityNotAllowed);
        }
        let value = value
            .strip_prefix(URL_PREFIX)
            .ok_or(UrlError::InvalidSchemeOrPath)?;
        let (path, query) = value
            .split_once('?')
            .map_or((value, None), |(path, query)| (path, Some(query)));
        let segments: Vec<_> = path.split('/').collect();
        let (digest, view) = match segments.as_slice() {
            [digest] if !digest.is_empty() => (*digest, RepositoryView::Canonical),
            [digest, "publisher", publisher] if !digest.is_empty() && !publisher.is_empty() => {
                let publisher =
                    StyreneIdentity::from_hex(publisher).map_err(|_| UrlError::InvalidPublisher)?;
                (*digest, RepositoryView::Publisher(publisher))
            }
            _ => return Err(UrlError::InvalidPath),
        };
        let repository = RepositoryId::from_str(&format!("styrene:git:v1:{digest}"))
            .map_err(|_| UrlError::InvalidRepository)?;
        let labels = parse_labels(query)?;
        Ok(Self {
            repository,
            view,
            labels,
        })
    }
}

impl fmt::Display for GitRemoteUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{URL_PREFIX}{}",
            self.repository.digest().base32()
        )?;
        if let RepositoryView::Publisher(publisher) = self.view {
            write!(formatter, "/publisher/{publisher}")?;
        }
        for (index, label) in self.labels.iter().enumerate() {
            write!(
                formatter,
                "{}label={label}",
                if index == 0 { '?' } else { '&' }
            )?;
        }
        Ok(())
    }
}

fn parse_labels(query: Option<&str>) -> Result<Vec<String>, UrlError> {
    let Some(query) = query else {
        return Ok(Vec::new());
    };
    if query.is_empty() {
        return Err(UrlError::InvalidQuery);
    }
    let labels = query
        .split('&')
        .map(|field| {
            let label = field
                .strip_prefix("label=")
                .ok_or(UrlError::UnsupportedQueryKey)?;
            if !valid_label(label) {
                return Err(UrlError::InvalidLabel);
            }
            Ok(label.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if labels.len() > MAX_ROUTING_LABELS {
        return Err(UrlError::TooManyLabels);
    }
    if !labels.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(UrlError::LabelsNotSortedUnique);
    }
    Ok(labels)
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_ROUTING_LABEL_BYTES
        && label
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("Styrene Git URL must use the authority-free styrene:///git/v1/ path")]
    InvalidSchemeOrPath,
    #[error("Styrene Git URL must not contain an authority")]
    AuthorityNotAllowed,
    #[error("Styrene Git URL path is invalid")]
    InvalidPath,
    #[error("repository digest is invalid")]
    InvalidRepository,
    #[error("publisher identity is invalid")]
    InvalidPublisher,
    #[error("URL fragments are not allowed")]
    FragmentNotAllowed,
    #[error("URL query is invalid")]
    InvalidQuery,
    #[error("only routing label query values are supported")]
    UnsupportedQueryKey,
    #[error("routing label is invalid")]
    InvalidLabel,
    #[error("routing labels must be sorted and unique")]
    LabelsNotSortedUnique,
    #[error("too many routing labels")]
    TooManyLabels,
}
