// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;
use std::process::ExitCode;

use okf::types::OkfRoot;

fn main() -> ExitCode {
    let mut reference: Option<PathBuf> = None;
    let mut candidate: Option<PathBuf> = None;
    let mut evidence: Option<PathBuf> = None;
    let mut json_output = false;
    let mut strict_coverage = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--reference" => reference = args.next().map(PathBuf::from),
            "--candidate" => candidate = args.next().map(PathBuf::from),
            "--evidence" => evidence = args.next().map(PathBuf::from),
            "--json" => json_output = true,
            "--strict-coverage" => strict_coverage = true,
            _ => {
                eprintln!("unknown argument: {}", a);
                return ExitCode::from(2);
            }
        }
    }
    let (Some(reference), Some(candidate)) = (reference, candidate) else {
        eprintln!("usage: mw-gate --reference FILE --candidate FILE [--evidence FILE] [--json] [--strict-coverage]");
        return ExitCode::from(2);
    };
    let read = |p: &PathBuf| -> anyhow::Result<OkfRoot> {
        let text = std::fs::read_to_string(p)?;
        Ok(serde_json::from_str(&text)?)
    };
    let (reference, candidate) = match (read(&reference), read(&candidate)) {
        (Ok(r), Ok(c)) => (r, c),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("failed to read input: {:#}", e);
            return ExitCode::from(2);
        }
    };
    let outcome = gate::run(&reference, &candidate, strict_coverage);
    if let Some(path) = &evidence {
        if let Err(e) = gate::write_evidence(path, &outcome.evidence) {
            eprintln!("failed to write evidence: {:#}", e);
            return ExitCode::from(2);
        }
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&outcome.evidence).expect("evidence serializes")
        );
    } else if outcome.passed {
        println!("GATE PASS");
    } else {
        println!("GATE FAIL");
        for f in &outcome.failures {
            println!("- {}", f);
        }
    }
    if outcome.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
