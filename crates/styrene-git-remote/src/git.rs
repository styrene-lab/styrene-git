use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use styrene_git_core::GitObjectId;

const MAX_METADATA_BYTES: u64 = 4096;
const MAX_STDERR_BYTES: u64 = 16 * 1024;
const MAX_REVISION_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

impl GitObjectFormat {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }
}

pub trait GitPlumbing {
    fn object_format(&self) -> Result<GitObjectFormat, GitError>;

    fn resolve_revision(&self, revision: &str) -> Result<GitObjectId, GitError>;

    fn has_object(&self, object: &GitObjectId) -> Result<bool, GitError>;

    fn local_prerequisites(&self, max_count: usize) -> Result<Vec<GitObjectId>, GitError>;

    fn create_push_pack(
        &self,
        includes: &[GitObjectId],
        excludes: &[GitObjectId],
        max_revisions: usize,
        max_pack_bytes: u64,
    ) -> Result<Vec<u8>, GitError>;

    fn install_fetch_pack(&self, pack: &[u8], max_pack_bytes: u64) -> Result<(), GitError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCommand {
    git_dir: PathBuf,
}

impl GitCommand {
    pub fn new(git_dir: impl Into<PathBuf>) -> Self {
        Self {
            git_dir: git_dir.into(),
        }
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    fn run(
        &self,
        args: &[String],
        input: Option<&[u8]>,
        max_output_bytes: u64,
    ) -> Result<Vec<u8>, GitError> {
        let operation = args.join(" ");
        let mut command = Command::new("git");
        command
            .env("GIT_DIR", &self.git_dir)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|source| GitError::Io {
            operation: operation.clone(),
            source,
        })?;

        if let Some(bytes) = input {
            let mut stdin = child.stdin.take().ok_or_else(|| GitError::Command {
                operation: operation.clone(),
                stderr: "Git stdin was unavailable".into(),
            })?;
            if let Err(source) = stdin.write_all(bytes) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Io { operation, source });
            }
        }

        let stdout = child.stdout.take().ok_or_else(|| GitError::Command {
            operation: operation.clone(),
            stderr: "Git stdout was unavailable".into(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| GitError::Command {
            operation: operation.clone(),
            stderr: "Git stderr was unavailable".into(),
        })?;
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_STDERR_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });

        let mut output = Vec::new();
        let read_result = stdout
            .take(max_output_bytes.saturating_add(1))
            .read_to_end(&mut output);
        if read_result.is_err() || output.len() as u64 > max_output_bytes {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|source| GitError::Io {
            operation: operation.clone(),
            source,
        })?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| GitError::Command {
                operation: operation.clone(),
                stderr: "Git stderr reader stopped unexpectedly".into(),
            })?
            .map_err(|source| GitError::Io {
                operation: operation.clone(),
                source,
            })?;
        read_result.map_err(|source| GitError::Io {
            operation: operation.clone(),
            source,
        })?;
        if output.len() as u64 > max_output_bytes {
            return Err(GitError::OutputLimitExceeded {
                operation,
                limit: max_output_bytes,
            });
        }
        if stderr.len() as u64 > MAX_STDERR_BYTES {
            return Err(GitError::OutputLimitExceeded {
                operation,
                limit: MAX_STDERR_BYTES,
            });
        }
        if !status.success() {
            return Err(GitError::Command {
                operation,
                stderr: String::from_utf8_lossy(&stderr).trim().into(),
            });
        }
        Ok(output)
    }

    fn parse_object_id(
        &self,
        format: GitObjectFormat,
        bytes: &[u8],
        operation: &'static str,
    ) -> Result<GitObjectId, GitError> {
        let value = std::str::from_utf8(bytes)
            .map_err(|_| GitError::InvalidOutput(operation))?
            .trim();
        GitObjectId::from_hex(format.name(), value).map_err(|_| GitError::InvalidOutput(operation))
    }

    fn require_format(format: GitObjectFormat, object: &GitObjectId) -> Result<(), GitError> {
        object.validate().map_err(|_| GitError::InvalidObjectId)?;
        if object.algorithm == format.name() {
            Ok(())
        } else {
            Err(GitError::ObjectFormatMismatch)
        }
    }
}

impl GitPlumbing for GitCommand {
    fn object_format(&self) -> Result<GitObjectFormat, GitError> {
        let output = self.run(
            &["rev-parse".into(), "--show-object-format".into()],
            None,
            MAX_METADATA_BYTES,
        )?;
        match std::str::from_utf8(&output)
            .map_err(|_| GitError::InvalidOutput("rev-parse --show-object-format"))?
            .trim()
        {
            "sha1" => Ok(GitObjectFormat::Sha1),
            "sha256" => Ok(GitObjectFormat::Sha256),
            other => Err(GitError::UnsupportedObjectFormat(other.into())),
        }
    }

    fn resolve_revision(&self, revision: &str) -> Result<GitObjectId, GitError> {
        if revision.is_empty()
            || revision.len() > MAX_REVISION_BYTES
            || revision.contains(['\0', '\n', '\r'])
        {
            return Err(GitError::InvalidRevision);
        }
        let format = self.object_format()?;
        let output = self.run(
            &[
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{revision}^{{object}}"),
            ],
            None,
            MAX_METADATA_BYTES,
        )?;
        self.parse_object_id(format, &output, "rev-parse --verify")
    }

    fn has_object(&self, object: &GitObjectId) -> Result<bool, GitError> {
        let format = self.object_format()?;
        Self::require_format(format, object)?;
        let revision = format!("{}^{{object}}", object.hex());
        match self.run(
            &[
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                revision,
            ],
            None,
            MAX_METADATA_BYTES,
        ) {
            Ok(output) => {
                Ok(self.parse_object_id(format, &output, "rev-parse --verify")? == *object)
            }
            Err(GitError::Command { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn local_prerequisites(&self, max_count: usize) -> Result<Vec<GitObjectId>, GitError> {
        if max_count == 0 {
            return Ok(Vec::new());
        }
        let format = self.object_format()?;
        let bytes_per_line = match format {
            GitObjectFormat::Sha1 => 41_u64,
            GitObjectFormat::Sha256 => 65_u64,
        };
        let max_output = (max_count as u64)
            .checked_mul(bytes_per_line)
            .ok_or(GitError::InvalidLimit)?;
        let output = self.run(
            &[
                "rev-list".into(),
                format!("--max-count={max_count}"),
                "--all".into(),
            ],
            None,
            max_output,
        )?;
        let text =
            std::str::from_utf8(&output).map_err(|_| GitError::InvalidOutput("rev-list --all"))?;
        let mut objects = text
            .lines()
            .map(|line| {
                GitObjectId::from_hex(format.name(), line)
                    .map_err(|_| GitError::InvalidOutput("rev-list --all"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        objects.sort_unstable();
        objects.dedup();
        Ok(objects)
    }

    fn create_push_pack(
        &self,
        includes: &[GitObjectId],
        excludes: &[GitObjectId],
        max_revisions: usize,
        max_pack_bytes: u64,
    ) -> Result<Vec<u8>, GitError> {
        let revision_count = includes
            .len()
            .checked_add(excludes.len())
            .ok_or(GitError::InvalidLimit)?;
        if revision_count > max_revisions {
            return Err(GitError::CountLimitExceeded {
                limit: max_revisions,
            });
        }
        if includes.is_empty() {
            return Ok(Vec::new());
        }
        let format = self.object_format()?;
        let mut revisions = String::new();
        for object in includes {
            Self::require_format(format, object)?;
            revisions.push_str(&object.hex());
            revisions.push('\n');
        }
        for object in excludes {
            Self::require_format(format, object)?;
            revisions.push('^');
            revisions.push_str(&object.hex());
            revisions.push('\n');
        }
        self.run(
            &["pack-objects".into(), "--stdout".into(), "--revs".into()],
            Some(revisions.as_bytes()),
            max_pack_bytes,
        )
    }

    fn install_fetch_pack(&self, pack: &[u8], max_pack_bytes: u64) -> Result<(), GitError> {
        if pack.len() as u64 > max_pack_bytes {
            return Err(GitError::InputLimitExceeded {
                limit: max_pack_bytes,
            });
        }
        self.run(
            &["index-pack".into(), "--stdin".into(), "--fix-thin".into()],
            Some(pack),
            MAX_METADATA_BYTES,
        )?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git {operation} failed: {stderr}")]
    Command { operation: String, stderr: String },
    #[error("Git {operation} I/O failed: {source}")]
    Io {
        operation: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Git {operation} output exceeded {limit} bytes")]
    OutputLimitExceeded { operation: String, limit: u64 },
    #[error("Git input exceeded {limit} bytes")]
    InputLimitExceeded { limit: u64 },
    #[error("Git object count exceeded {limit}")]
    CountLimitExceeded { limit: usize },
    #[error("Git produced invalid output for {0}")]
    InvalidOutput(&'static str),
    #[error("Git revision is invalid")]
    InvalidRevision,
    #[error("Git limit cannot be represented")]
    InvalidLimit,
    #[error("Git object identifier is invalid")]
    InvalidObjectId,
    #[error("Git object identifier uses the wrong object format")]
    ObjectFormatMismatch,
    #[error("unsupported Git object format: {0}")]
    UnsupportedObjectFormat(String),
}
