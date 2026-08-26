use std::io::{BufRead, Write};

use styrene_git_core::GitObjectId;

use crate::{
    AuthenticatedGitTransport, FetchCommand, GitPlumbing, HelperError, PushCommand, RemoteSession,
};

const MAX_COMMAND_LINE_BYTES: usize = 4096;

pub fn run_command_loop<Transport, Git, Input, Output>(
    input: &mut Input,
    output: &mut Output,
    session: &mut RemoteSession<Transport, Git>,
) -> Result<(), CommandLoopError>
where
    Transport: AuthenticatedGitTransport,
    Git: GitPlumbing,
    Input: BufRead,
    Output: Write,
{
    let mut advertised = false;
    loop {
        let Some(line) = read_line(input)? else {
            return Ok(());
        };
        if line.is_empty() {
            return Ok(());
        }
        if !advertised && line != "capabilities" {
            return Err(CommandLoopError::CapabilitiesRequired);
        }
        match line.as_str() {
            "capabilities" => {
                if advertised {
                    return Err(CommandLoopError::DuplicateCapabilities);
                }
                output.write_all(b"fetch\npush\n\n")?;
                output.flush()?;
                advertised = true;
            }
            "list" => {
                let listing = session.list()?;
                for reference in listing.refs {
                    writeln!(output, "{} {}", reference.target.hex(), reference.name)?;
                }
                if let Some(head) = listing.head {
                    writeln!(output, "@{head} HEAD")?;
                }
                output.write_all(b"\n")?;
                output.flush()?;
            }
            "list for-push" => {
                let listing = session.list_for_push()?;
                for reference in listing.refs {
                    writeln!(output, "{} {}", reference.target.hex(), reference.name)?;
                }
                output.write_all(b"\n")?;
                output.flush()?;
            }
            _ if line.starts_with("fetch ") => {
                let format = session.git().object_format().map_err(HelperError::from)?;
                let mut commands = vec![parse_fetch(&line, format.name())?];
                loop {
                    let line = read_batch_line(input, "fetch")?;
                    if line.is_empty() {
                        break;
                    }
                    if !line.starts_with("fetch ") {
                        return Err(CommandLoopError::MalformedFetch);
                    }
                    commands.push(parse_fetch(&line, format.name())?);
                }
                session.fetch_batch(&commands)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
            _ if line.starts_with("push ") => {
                let mut commands = vec![parse_push(&line)?];
                loop {
                    let line = read_batch_line(input, "push")?;
                    if line.is_empty() {
                        break;
                    }
                    if !line.starts_with("push ") {
                        return Err(CommandLoopError::MalformedPush);
                    }
                    commands.push(parse_push(&line)?);
                }
                if let Err(error) = session.push_batch(&commands) {
                    for command in &commands {
                        writeln!(output, "error {} atomic push failed", command.destination)?;
                    }
                    output.write_all(b"\n")?;
                    output.flush()?;
                    return Err(error.into());
                }
                for command in commands {
                    writeln!(output, "ok {}", command.destination)?;
                }
                output.write_all(b"\n")?;
                output.flush()?;
            }
            _ => return Err(CommandLoopError::UnsupportedCommand),
        }
    }
}

fn parse_fetch(line: &str, algorithm: &str) -> Result<FetchCommand, CommandLoopError> {
    let fields = line
        .strip_prefix("fetch ")
        .ok_or(CommandLoopError::MalformedFetch)?;
    let (object, reference) = fields
        .split_once(' ')
        .ok_or(CommandLoopError::MalformedFetch)?;
    if object.is_empty() || reference.is_empty() || reference.contains(' ') {
        return Err(CommandLoopError::MalformedFetch);
    }
    let object =
        GitObjectId::from_hex(algorithm, object).map_err(|_| CommandLoopError::MalformedFetch)?;
    Ok(FetchCommand {
        object,
        reference: reference.into(),
    })
}

fn parse_push(line: &str) -> Result<PushCommand, CommandLoopError> {
    let fields = line
        .strip_prefix("push ")
        .ok_or(CommandLoopError::MalformedPush)?;
    if fields.is_empty() || fields.contains(' ') {
        return Err(CommandLoopError::MalformedPush);
    }
    let (force, fields) = fields
        .strip_prefix('+')
        .map_or((false, fields), |fields| (true, fields));
    let (source, destination) = fields
        .split_once(':')
        .ok_or(CommandLoopError::MalformedPush)?;
    if destination.is_empty() || destination.contains(':') {
        return Err(CommandLoopError::MalformedPush);
    }
    Ok(PushCommand {
        source: (!source.is_empty()).then(|| source.into()),
        destination: destination.into(),
        force,
    })
}

fn read_batch_line<Input: BufRead>(
    input: &mut Input,
    operation: &'static str,
) -> Result<String, CommandLoopError> {
    read_line(input)?.ok_or(CommandLoopError::UnexpectedEof(operation))
}

fn read_line<Input: BufRead>(input: &mut Input) -> Result<Option<String>, CommandLoopError> {
    let mut line = String::new();
    let mut limited = std::io::Read::take(input, (MAX_COMMAND_LINE_BYTES + 2) as u64);
    let bytes = limited.read_line(&mut line)?;
    if bytes == 0 {
        return Ok(None);
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    if line.len() > MAX_COMMAND_LINE_BYTES {
        return Err(CommandLoopError::LineTooLong);
    }
    Ok(Some(line))
}

#[derive(Debug, thiserror::Error)]
pub enum CommandLoopError {
    #[error("capabilities must be the first helper command")]
    CapabilitiesRequired,
    #[error("capabilities command was repeated")]
    DuplicateCapabilities,
    #[error("unsupported helper command")]
    UnsupportedCommand,
    #[error("malformed fetch command")]
    MalformedFetch,
    #[error("malformed push command")]
    MalformedPush,
    #[error("unexpected EOF in {0} batch")]
    UnexpectedEof(&'static str),
    #[error("helper command line exceeds {MAX_COMMAND_LINE_BYTES} bytes")]
    LineTooLong,
    #[error(transparent)]
    Helper(#[from] HelperError),
    #[error("helper protocol I/O failed: {0}")]
    Io(#[from] std::io::Error),
}
