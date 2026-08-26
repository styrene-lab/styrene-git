use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::{error::io_error, StoreError};

pub(crate) fn run(
    git_dir: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    object_directory: Option<&Path>,
) -> Result<Vec<u8>, StoreError> {
    let mut command = Command::new("git");
    command.env("GIT_DIR", git_dir).args(args);
    if let Some(directory) = object_directory {
        command
            .env("GIT_OBJECT_DIRECTORY", directory)
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", git_dir.join("objects"));
    }
    execute(command, args.join(" "), input).map(|output| output.stdout)
}

pub(crate) fn run_with_path(
    args: &[&str],
    path: &Path,
    operation: &str,
) -> Result<Vec<u8>, StoreError> {
    let mut command = Command::new("git");
    command.args(args).arg(path);
    execute(command, operation.into(), None).map(|output| output.stdout)
}

pub(crate) fn run_bounded(
    git_dir: &Path,
    args: &[&str],
    input: &[u8],
    max_output: u64,
) -> Result<Vec<u8>, StoreError> {
    let mut command = Command::new("git");
    command
        .env("GIT_DIR", git_dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|source| io_error("git", source))?;
    child
        .stdin
        .take()
        .ok_or_else(|| StoreError::Git {
            operation: args.join(" "),
            stderr: "Git stdin was unavailable".into(),
        })?
        .write_all(input)
        .map_err(|source| io_error("git stdin", source))?;
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| StoreError::Git {
            operation: args.join(" "),
            stderr: "Git stdout was unavailable".into(),
        })?
        .take(max_output.saturating_add(1))
        .read_to_end(&mut output)
        .map_err(|source| io_error("git stdout", source))?;
    if output.len() as u64 > max_output {
        let _ = child.kill();
        let _ = child.wait();
        return Err(StoreError::PackTooLarge { limit: max_output });
    }
    let result = child
        .wait_with_output()
        .map_err(|source| io_error("git process", source))?;
    if result.status.success() {
        Ok(output)
    } else {
        Err(StoreError::Git {
            operation: args.join(" "),
            stderr: String::from_utf8_lossy(&result.stderr).trim().into(),
        })
    }
}

fn execute(
    mut command: Command,
    operation: String,
    input: Option<&[u8]>,
) -> Result<Output, StoreError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|source| io_error("git", source))?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| StoreError::Git {
                operation: operation.clone(),
                stderr: "Git stdin was unavailable".into(),
            })?
            .write_all(bytes)
            .map_err(|source| io_error("git stdin", source))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|source| io_error("git process", source))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(StoreError::Git {
            operation,
            stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}
