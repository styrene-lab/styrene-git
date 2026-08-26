use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const MAX_CONTROL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Health,
    Identity,
    Initialize {
        bindings: BTreeMap<String, String>,
        threshold: u16,
    },
    PublishCommit {
        repository: String,
        message: String,
        parent: Option<String>,
    },
    PublishTarget {
        repository: String,
        target: String,
    },
    Apply {
        transfer: String,
    },
    State {
        repository: String,
        delegates: Vec<String>,
    },
    Fsck {
        repository: String,
    },
    Restart,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Healthy {
        operator: String,
        incarnation: String,
    },
    Identity {
        identity: String,
        binding: String,
    },
    Initialized {
        repository: String,
    },
    Published {
        head: String,
        sequence: u64,
        transfer: String,
    },
    Applied {
        outcome: String,
        publisher: String,
        sequence: u64,
    },
    MissingPrerequisites {
        prerequisites: Vec<String>,
    },
    State {
        canonical: Option<String>,
        decision: String,
        publishers: BTreeMap<String, Option<String>>,
    },
    Verified {
        repository: String,
    },
    Error {
        message: String,
    },
}

pub fn request(address: &str, request: &Request) -> Result<Response, String> {
    let mut stream = TcpStream::connect(address)
        .map_err(|error| format!("connect to {address} failed: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set timeout failed: {error}"))?;
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("encode control request failed: {error}"))?;
    stream
        .write_all(&bytes)
        .map_err(|error| format!("write control request failed: {error}"))?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|error| format!("finish control request failed: {error}"))?;
    let mut response = Vec::new();
    stream
        .take(MAX_CONTROL_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|error| format!("read control response failed: {error}"))?;
    if response.len() as u64 > MAX_CONTROL_BYTES {
        return Err("control response exceeds limit".into());
    }
    let response: Response = serde_json::from_slice(&response)
        .map_err(|error| format!("decode control response failed: {error}"))?;
    match response {
        Response::Error { message } => Err(message),
        response => Ok(response),
    }
}

pub fn wait_until_healthy(address: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(
            request(address, &Request::Health),
            Ok(Response::Healthy { .. })
        ) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("operator {address} did not become healthy"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn incarnation(address: &str) -> Result<String, String> {
    match request(address, &Request::Health)? {
        Response::Healthy { incarnation, .. } => Ok(incarnation),
        response => Err(format!("unexpected health response: {response:?}")),
    }
}

pub fn wait_until_restarted(
    address: &str,
    previous: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if incarnation(address).is_ok_and(|incarnation| incarnation != previous) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("operator {address} did not restart"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
