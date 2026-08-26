use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use styrene_git_remote::{
    run_command_loop, AuthenticatedGitTransport, ClientConfig, GitCommand, GitIpcClient,
    GitRemoteUrl, RemoteSession, TransportError,
};

struct UnavailableTransport;

impl AuthenticatedGitTransport for UnavailableTransport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: u64,
    ) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::new(
            "authenticated styrened Git connector is unavailable",
        ))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-remote-styrene: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next().ok_or("missing remote name or URL")?;
    let second = arguments.next();
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }
    let url = second.unwrap_or(first);
    let url = url.to_str().ok_or("remote URL is not UTF-8")?;
    let url = GitRemoteUrl::from_str(url)?;
    let git_dir = std::env::var_os("GIT_DIR")
        .map(PathBuf::from)
        .ok_or("GIT_DIR is not set")?;
    let client = GitIpcClient::new(UnavailableTransport, ClientConfig::default())?;
    let mut session = RemoteSession::new(url, client, GitCommand::new(git_dir));
    run_command_loop(
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut session,
    )?;
    Ok(())
}
