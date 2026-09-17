// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = mcp::handle_request(&line);
        if response.is_empty() {
            continue;
        }
        if writeln!(out, "{}", response).is_err() {
            break;
        }
        let _ = out.flush();
    }
}
