mod operator;
mod scenario;
mod wire;

use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("styrene-git-lab: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("operator") if args.len() == 6 => operator::run(
            args[2].clone(),
            args[3]
                .parse()
                .map_err(|error| format!("invalid operator seed: {error}"))?,
            args[4].clone(),
            PathBuf::from(&args[5]),
        ),
        Some("scenario") if args.len() == 3 => {
            scenario::run_three_party(&PathBuf::from(&args[2]))
        }
        _ => Err(
            "usage: styrene-git-lab operator <name> <seed> <listen> <state-root> | scenario <artifact-path>"
                .into(),
        ),
    }
}
