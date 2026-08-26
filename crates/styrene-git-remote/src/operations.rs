use std::collections::BTreeSet;

use styrene_git_core::GitObjectId;
use styrene_git_ipc::{PushRefListing, PushResult, RefListing, RefUpdate};

use crate::{
    AuthenticatedGitTransport, ClientError, GitError, GitIpcClient, GitPlumbing, GitRemoteUrl,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchCommand {
    pub object: GitObjectId,
    pub reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushCommand {
    pub source: Option<String>,
    pub destination: String,
    pub force: bool,
}

pub struct RemoteSession<Transport, Git> {
    url: GitRemoteUrl,
    client: GitIpcClient<Transport>,
    git: Git,
}

impl<Transport, Git> RemoteSession<Transport, Git>
where
    Transport: AuthenticatedGitTransport,
    Git: GitPlumbing,
{
    pub fn new(url: GitRemoteUrl, client: GitIpcClient<Transport>, git: Git) -> Self {
        Self { url, client, git }
    }

    pub fn url(&self) -> &GitRemoteUrl {
        &self.url
    }

    pub fn client(&self) -> &GitIpcClient<Transport> {
        &self.client
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    pub fn into_parts(self) -> (GitRemoteUrl, GitIpcClient<Transport>, Git) {
        (self.url, self.client, self.git)
    }

    pub fn list(&mut self) -> Result<RefListing, HelperError> {
        self.client.negotiate()?;
        let format = self.git.object_format()?;
        let listing = self
            .client
            .list_refs(self.url.repository(), self.url.view().clone())?;
        validate_listing(format, &listing.refs)?;
        if let Some(head) = &listing.head {
            validate_reference(head)?;
        }
        Ok(listing)
    }

    pub fn list_for_push(&mut self) -> Result<PushRefListing, HelperError> {
        self.client.negotiate()?;
        let format = self.git.object_format()?;
        let listing = self.client.list_push_refs(self.url.repository())?;
        validate_listing(format, &listing.refs)?;
        Ok(listing)
    }

    pub fn fetch_batch(&mut self, commands: &[FetchCommand]) -> Result<(), HelperError> {
        if commands.is_empty() {
            return Err(HelperError::EmptyBatch("fetch"));
        }
        let capabilities = self.client.negotiate()?;
        let format = self.git.object_format()?;
        for command in commands {
            validate_reference(&command.reference)?;
            validate_object(format, &command.object)?;
        }

        let wants = commands
            .iter()
            .map(|command| command.object.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if wants.len() > usize::from(capabilities.limits.max_wants) {
            return Err(HelperError::CommandLimitExceeded {
                operation: "fetch",
                limit: usize::from(capabilities.limits.max_wants),
            });
        }
        let haves = self
            .git
            .local_prerequisites(usize::from(capabilities.limits.max_haves))?;
        if haves.len() > usize::from(capabilities.limits.max_haves) {
            return Err(HelperError::GitResultLimitExceeded {
                field: "prerequisites",
                limit: usize::from(capabilities.limits.max_haves),
            });
        }
        for object in &haves {
            validate_object(format, object)?;
        }
        let haves = haves.into_iter().collect::<BTreeSet<_>>();
        let pack = self.client.fetch(
            self.url.repository(),
            self.url.view().clone(),
            wants.clone(),
            haves.into_iter().collect(),
        )?;
        if !pack.is_empty() {
            self.git
                .install_fetch_pack(&pack, capabilities.limits.max_pack_bytes)?;
        }
        for object in wants {
            if !self.git.has_object(&object)? {
                return Err(HelperError::WantedObjectMissing(object));
            }
        }
        Ok(())
    }

    pub fn push_batch(&mut self, commands: &[PushCommand]) -> Result<PushResult, HelperError> {
        if commands.is_empty() {
            return Err(HelperError::EmptyBatch("push"));
        }
        let capabilities = self.client.negotiate()?;
        if commands.len() > usize::from(capabilities.limits.max_ref_updates) {
            return Err(HelperError::CommandLimitExceeded {
                operation: "push",
                limit: usize::from(capabilities.limits.max_ref_updates),
            });
        }
        let format = self.git.object_format()?;
        let listing = self.client.list_push_refs(self.url.repository())?;
        validate_listing(format, &listing.refs)?;
        let current = listing
            .refs
            .iter()
            .map(|reference| (reference.name.as_str(), &reference.target))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut destinations = BTreeSet::new();
        let mut updates = Vec::with_capacity(commands.len());
        let mut includes = BTreeSet::new();
        for command in commands {
            validate_reference(&command.destination)?;
            if !destinations.insert(command.destination.as_str()) {
                return Err(HelperError::DuplicateDestination(
                    command.destination.clone(),
                ));
            }
            let new = command
                .source
                .as_deref()
                .map(|source| self.git.resolve_revision(source))
                .transpose()?;
            if let Some(object) = &new {
                validate_object(format, object)?;
                includes.insert(object.clone());
            }
            updates.push(RefUpdate {
                name: command.destination.clone(),
                expected: current
                    .get(command.destination.as_str())
                    .map(|object| (*object).clone()),
                new,
                force: command.force,
            });
        }
        let excludes = listing
            .refs
            .iter()
            .map(|reference| reference.target.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(usize::from(capabilities.limits.max_haves))
            .collect::<Vec<_>>();
        let includes = includes.into_iter().collect::<Vec<_>>();
        let max_revisions = usize::from(capabilities.limits.max_ref_updates)
            .checked_add(usize::from(capabilities.limits.max_haves))
            .ok_or(HelperError::RevisionLimitOverflow)?;
        let pack = self.git.create_push_pack(
            &includes,
            &excludes,
            max_revisions,
            capabilities.limits.max_pack_bytes,
        )?;
        self.client
            .push(self.url.repository(), updates, pack)
            .map_err(HelperError::from)
    }
}

fn validate_object(
    format: crate::GitObjectFormat,
    object: &GitObjectId,
) -> Result<(), HelperError> {
    object
        .validate()
        .map_err(|_| HelperError::InvalidObjectId)?;
    if object.algorithm == format.name() {
        Ok(())
    } else {
        Err(HelperError::ObjectFormatMismatch)
    }
}

fn validate_listing(
    format: crate::GitObjectFormat,
    refs: &[styrene_git_core::RefTarget],
) -> Result<(), HelperError> {
    if !refs.windows(2).all(|pair| pair[0].name < pair[1].name) {
        return Err(HelperError::InvalidRefListing);
    }
    for reference in refs {
        validate_reference(&reference.name)?;
        validate_object(format, &reference.target)?;
    }
    Ok(())
}

fn validate_reference(reference: &str) -> Result<(), HelperError> {
    let components_valid = reference.strip_prefix("refs/").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.split('/').all(|component| {
                !component.is_empty()
                    && !component.starts_with('.')
                    && !component.ends_with('.')
                    && !component.ends_with(".lock")
            })
    });
    let valid = reference.starts_with("refs/")
        && !reference.ends_with('/')
        && components_valid
        && !reference.contains("..")
        && !reference.contains("@{")
        && !reference.contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
        && !reference.bytes().any(|byte| byte < 0x20 || byte == 0x7f);
    if valid {
        Ok(())
    } else {
        Err(HelperError::InvalidReference(reference.into()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("{0} batch is empty")]
    EmptyBatch(&'static str),
    #[error("{operation} batch exceeds {limit} commands")]
    CommandLimitExceeded {
        operation: &'static str,
        limit: usize,
    },
    #[error("invalid Git reference: {0}")]
    InvalidReference(String),
    #[error("duplicate push destination: {0}")]
    DuplicateDestination(String),
    #[error("Git object identifier uses the wrong repository object format")]
    ObjectFormatMismatch,
    #[error("Git object identifier is invalid")]
    InvalidObjectId,
    #[error("Git plumbing returned more than {limit} {field}")]
    GitResultLimitExceeded { field: &'static str, limit: usize },
    #[error("daemon reference listing is not sorted and unique")]
    InvalidRefListing,
    #[error("fetched pack does not provide wanted object {0:?}")]
    WantedObjectMissing(GitObjectId),
    #[error("push pack revision limit cannot be represented")]
    RevisionLimitOverflow,
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Git(#[from] GitError),
}
