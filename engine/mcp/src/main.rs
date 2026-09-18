// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::{BufRead, Write};

use mcp::repository::Repository;

fn main() {
    let repo = match Repository::from_env() {
        Ok(repo) => repo,
        Err(message) => {
            eprintln!("mw-mcp: {}", message);
            std::process::exit(2);
        }
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = mcp::handle_request_with(&line, &repo);
        if response.is_empty() {
            continue;
        }
        if writeln!(out, "{}", response).is_err() {
            break;
        }
        let _ = out.flush();
    }
}
