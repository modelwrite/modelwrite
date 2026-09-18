// SPDX-License-Identifier: AGPL-3.0-or-later
//! The "mw" command line: a hand-rolled argument parser and dispatcher that reaches the
//! repository service over HTTP/1.1 or opens a SQLite store directly for offline use.
//!
//! There is deliberately no CLI framework and no HTTP client dependency: arguments are
//! parsed here, the HTTP path is std::net::TcpStream, and a token is read from an
//! environment variable NAMED by a flag - never from the command line, because a token on
//! a command line is a token in the shell history and in the process list.

mod http;
mod offline;

use std::path::PathBuf;

use serde_json::Value;

/// How a command reaches a repository. Exactly one transport is chosen up front.
#[derive(Debug)]
enum Transport {
    /// Talk HTTP/1.1 to a running service. token_var, when present, names the
    /// environment variable that holds the bearer token (never the token itself).
    Server {
        url: String,
        token_var: Option<String>,
    },
    /// Open a local SQLite store directly; no service is required.
    Offline { db: PathBuf },
}

/// One parsed command. The parser produces exactly one of these or a clear error.
#[derive(Debug, Clone, PartialEq)]
enum Command {
    ProjectCreate {
        name: String,
    },
    ProjectList,
    Commit {
        project: String,
        branch: String,
        message: String,
        file: PathBuf,
        holder: Option<String>,
    },
    Log {
        project: String,
        branch: String,
    },
    Artifact {
        project: String,
        hash: String,
    },
    BranchList {
        project: String,
    },
    BranchCreate {
        project: String,
        name: String,
        from: String,
    },
    BranchDelete {
        project: String,
        name: String,
    },
    Merge {
        project: String,
        branch: String,
        other: String,
        message: String,
        holder: Option<String>,
    },
    Reset {
        project: String,
        branch: String,
        to: String,
        message: String,
        holder: Option<String>,
    },
    Gate {
        project: String,
        reference: String,
        candidate: String,
    },
    LockAcquire {
        project: String,
        branch: String,
        elements: Vec<String>,
        holder: String,
        ttl_seconds: i64,
    },
    LockRelease {
        project: String,
        holder: String,
        ids: Vec<String>,
    },
    LockList {
        project: String,
    },
    Audit {
        project: String,
        limit: i64,
    },
}

const USAGE: &str = r#"usage: mw [--server <url> [--token <ENV_VAR>] | --db <path>] <command>

  --server <url>   talk to a running service over plain http://
  --token <VAR>    read the bearer token from the environment variable named VAR
  --db <path>      open a local SQLite store directly (offline, no server)

commands:
  project create <name>
  project list
  commit <project> --branch <branch> --message <msg> --file <path> [--holder <name>]
  log <project> --branch <branch>
  artifact <project> --hash <hash>
  branch list <project>
  branch create <project> --name <name> --from <commit>
  branch delete <project> --name <name>
  merge <project> --branch <branch> --other <branch> --message <msg> [--holder <name>]
  reset <project> --branch <branch> --to <commit> --message <msg> [--holder <name>]
  gate <project> --reference <commit> --candidate <commit>
  lock acquire <project> --branch <branch> --elements <a,b> --holder <name> --ttl <seconds>
  lock release <project> --holder <name> --ids <a,b>
  lock list <project>
  audit <project> [--limit <n>]
"#;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        print!("{}", USAGE);
        std::process::exit(0);
    }
    match run(&args) {
        Ok(value) => {
            if !value.is_null() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
                );
            }
        }
        Err(message) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
    }
}

/// Parse arguments and execute the command against the chosen transport. Returns the
/// command's JSON output (or Value::Null for commands with no output).
fn run(args: &[String]) -> Result<Value, String> {
    let (transport, command) = parse(args)?;
    match transport {
        Transport::Offline { db } => offline::run(&db, command),
        Transport::Server { url, token_var } => {
            let token = match token_var {
                Some(var) => Some(std::env::var(&var).map_err(|_| {
                    format!(
                        "the environment variable {} named by --token is not set",
                        var
                    )
                })?),
                None => None,
            };
            http::run(&url, token.as_deref(), command)
        }
    }
}

/// Parse the leading global flags, then the command, returning a typed transport and
/// command or a clear error.
fn parse(args: &[String]) -> Result<(Transport, Command), String> {
    let mut url: Option<String> = None;
    let mut token_var: Option<String> = None;
    let mut db: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--server" => url = Some(take_value(args, &mut i, "--server")?),
            "--token" => token_var = Some(take_value(args, &mut i, "--token")?),
            "--db" => db = Some(take_value(args, &mut i, "--db")?),
            _ => break,
        }
        i += 1;
    }

    let transport = match (url, db) {
        (Some(_), Some(_)) => {
            return Err("--server and --db are mutually exclusive; choose one".to_string())
        }
        (Some(url), None) => Transport::Server { url, token_var },
        (None, Some(db)) => {
            if token_var.is_some() {
                return Err("--token is only meaningful with --server".to_string());
            }
            Transport::Offline {
                db: PathBuf::from(db),
            }
        }
        (None, None) => {
            if token_var.is_some() {
                return Err("--token requires --server".to_string());
            }
            return Err("choose a transport: --server <url> or --db <path>".to_string());
        }
    };

    let rest = &args[i..];
    if rest.is_empty() {
        return Err("missing command; run mw --help for usage".to_string());
    }
    let command = parse_command(rest)?;
    Ok((transport, command))
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    let verb = args[0].as_str();
    let rest = &args[1..];
    match verb {
        "project" => parse_project(rest),
        "commit" => parse_commit(rest),
        "log" => parse_log(rest),
        "artifact" => parse_artifact(rest),
        "branch" => parse_branch(rest),
        "merge" => parse_merge(rest),
        "reset" => parse_reset(rest),
        "gate" => parse_gate(rest),
        "lock" => parse_lock(rest),
        "audit" => parse_audit(rest),
        other if other.starts_with("--") => {
            Err(format!("unknown flag {} before any command", other))
        }
        other => Err(format!("unknown command {}", other)),
    }
}

fn parse_project(args: &[String]) -> Result<Command, String> {
    match args.first().map(|s| s.as_str()) {
        Some("create") => {
            let name = one_positional(&args[1..], "project create")?;
            validate_name("project name", &name)?;
            Ok(Command::ProjectCreate { name })
        }
        Some("list") => {
            if args.len() > 1 {
                return Err("project list takes no arguments".to_string());
            }
            Ok(Command::ProjectList)
        }
        Some(other) if other.starts_with("--") => {
            Err(format!("unknown flag {} for project", other))
        }
        Some(other) => Err(format!("unknown project subcommand {}", other)),
        None => Err("project requires a subcommand: create or list".to_string()),
    }
}

fn parse_commit(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut branch: Option<String> = None;
    let mut message: Option<String> = None;
    let mut file: Option<String> = None;
    let mut holder: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--branch" => branch = Some(value(args, &mut i, "--branch")?),
            "--message" => message = Some(value(args, &mut i, "--message")?),
            "--file" => file = Some(value(args, &mut i, "--file")?),
            "--holder" => holder = Some(value(args, &mut i, "--holder")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for commit", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "commit")?;
    let branch = branch.ok_or("commit requires --branch")?;
    validate_name("branch name", &branch)?;
    let message = message.ok_or("commit requires --message")?;
    let file = file.ok_or("commit requires --file")?;
    Ok(Command::Commit {
        project,
        branch,
        message,
        file: PathBuf::from(file),
        holder,
    })
}

fn parse_log(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut branch: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--branch" => branch = Some(value(args, &mut i, "--branch")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for log", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "log")?;
    let branch = branch.ok_or("log requires --branch")?;
    validate_name("branch name", &branch)?;
    Ok(Command::Log { project, branch })
}

fn parse_artifact(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut hash: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--hash" => hash = Some(value(args, &mut i, "--hash")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for artifact", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "artifact")?;
    let hash = hash.ok_or("artifact requires --hash")?;
    if hash.is_empty() {
        return Err("artifact --hash must not be empty".to_string());
    }
    Ok(Command::Artifact { project, hash })
}

fn parse_branch(args: &[String]) -> Result<Command, String> {
    match args.first().map(|s| s.as_str()) {
        Some("list") => {
            let project = one_project(&args[1..], "branch list")?;
            Ok(Command::BranchList { project })
        }
        Some("create") => {
            let mut positionals: Vec<String> = Vec::new();
            let mut name: Option<String> = None;
            let mut from: Option<String> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--name" => name = Some(value(args, &mut i, "--name")?),
                    "--from" => from = Some(value(args, &mut i, "--from")?),
                    other if other.starts_with("--") => {
                        return Err(format!("unknown flag {} for branch create", other))
                    }
                    other => positionals.push(other.to_string()),
                }
                i += 1;
            }
            let project = one_project(&positionals, "branch create")?;
            let name = name.ok_or("branch create requires --name")?;
            validate_name("branch name", &name)?;
            let from = from.ok_or("branch create requires --from")?;
            Ok(Command::BranchCreate {
                project,
                name,
                from,
            })
        }
        Some("delete") => {
            let mut positionals: Vec<String> = Vec::new();
            let mut name: Option<String> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--name" => name = Some(value(args, &mut i, "--name")?),
                    other if other.starts_with("--") => {
                        return Err(format!("unknown flag {} for branch delete", other))
                    }
                    other => positionals.push(other.to_string()),
                }
                i += 1;
            }
            let project = one_project(&positionals, "branch delete")?;
            let name = name.ok_or("branch delete requires --name")?;
            validate_name("branch name", &name)?;
            Ok(Command::BranchDelete { project, name })
        }
        Some(other) if other.starts_with("--") => Err(format!("unknown flag {} for branch", other)),
        Some(other) => Err(format!("unknown branch subcommand {}", other)),
        None => Err("branch requires a subcommand: list, create or delete".to_string()),
    }
}

fn parse_merge(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut branch: Option<String> = None;
    let mut other: Option<String> = None;
    let mut message: Option<String> = None;
    let mut holder: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--branch" => branch = Some(value(args, &mut i, "--branch")?),
            "--other" => other = Some(value(args, &mut i, "--other")?),
            "--message" => message = Some(value(args, &mut i, "--message")?),
            "--holder" => holder = Some(value(args, &mut i, "--holder")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for merge", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "merge")?;
    let branch = branch.ok_or("merge requires --branch")?;
    validate_name("branch name", &branch)?;
    let other = other.ok_or("merge requires --other")?;
    validate_name("branch name", &other)?;
    let message = message.ok_or("merge requires --message")?;
    Ok(Command::Merge {
        project,
        branch,
        other,
        message,
        holder,
    })
}

fn parse_reset(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut branch: Option<String> = None;
    let mut to: Option<String> = None;
    let mut message: Option<String> = None;
    let mut holder: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--branch" => branch = Some(value(args, &mut i, "--branch")?),
            "--to" => to = Some(value(args, &mut i, "--to")?),
            "--message" => message = Some(value(args, &mut i, "--message")?),
            "--holder" => holder = Some(value(args, &mut i, "--holder")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for reset", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "reset")?;
    let branch = branch.ok_or("reset requires --branch")?;
    validate_name("branch name", &branch)?;
    let to = to.ok_or("reset requires --to")?;
    let message = message.ok_or("reset requires --message")?;
    Ok(Command::Reset {
        project,
        branch,
        to,
        message,
        holder,
    })
}

fn parse_gate(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reference: Option<String> = None;
    let mut candidate: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--reference" => reference = Some(value(args, &mut i, "--reference")?),
            "--candidate" => candidate = Some(value(args, &mut i, "--candidate")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for gate", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "gate")?;
    let reference = reference.ok_or("gate requires --reference")?;
    let candidate = candidate.ok_or("gate requires --candidate")?;
    Ok(Command::Gate {
        project,
        reference,
        candidate,
    })
}

fn parse_lock(args: &[String]) -> Result<Command, String> {
    match args.first().map(|s| s.as_str()) {
        Some("acquire") => {
            let mut positionals: Vec<String> = Vec::new();
            let mut branch: Option<String> = None;
            let mut elements: Option<Vec<String>> = None;
            let mut holder: Option<String> = None;
            let mut ttl: Option<i64> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--branch" => branch = Some(value(args, &mut i, "--branch")?),
                    "--elements" => {
                        elements = Some(split_list(
                            &value(args, &mut i, "--elements")?,
                            "--elements",
                        )?)
                    }
                    "--holder" => holder = Some(value(args, &mut i, "--holder")?),
                    "--ttl" => ttl = Some(parse_i64(&value(args, &mut i, "--ttl")?, "--ttl")?),
                    other if other.starts_with("--") => {
                        return Err(format!("unknown flag {} for lock acquire", other))
                    }
                    other => positionals.push(other.to_string()),
                }
                i += 1;
            }
            let project = one_project(&positionals, "lock acquire")?;
            let branch = branch.ok_or("lock acquire requires --branch")?;
            validate_name("branch name", &branch)?;
            let elements = elements.ok_or("lock acquire requires --elements")?;
            let holder = holder.ok_or("lock acquire requires --holder")?;
            if holder.is_empty() {
                return Err("lock acquire requires a non-empty --holder".to_string());
            }
            let ttl_seconds = ttl.ok_or("lock acquire requires --ttl")?;
            if !(30..=86400).contains(&ttl_seconds) {
                return Err("lock acquire --ttl must be between 30 and 86400 seconds".to_string());
            }
            Ok(Command::LockAcquire {
                project,
                branch,
                elements,
                holder,
                ttl_seconds,
            })
        }
        Some("release") => {
            let mut positionals: Vec<String> = Vec::new();
            let mut holder: Option<String> = None;
            let mut ids: Option<Vec<String>> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--holder" => holder = Some(value(args, &mut i, "--holder")?),
                    "--ids" => ids = Some(split_list(&value(args, &mut i, "--ids")?, "--ids")?),
                    other if other.starts_with("--") => {
                        return Err(format!("unknown flag {} for lock release", other))
                    }
                    other => positionals.push(other.to_string()),
                }
                i += 1;
            }
            let project = one_project(&positionals, "lock release")?;
            let holder = holder.ok_or("lock release requires --holder")?;
            if holder.is_empty() {
                return Err("lock release requires a non-empty --holder".to_string());
            }
            let ids = ids.ok_or("lock release requires --ids")?;
            Ok(Command::LockRelease {
                project,
                holder,
                ids,
            })
        }
        Some("list") => {
            let project = one_project(&args[1..], "lock list")?;
            Ok(Command::LockList { project })
        }
        Some(other) if other.starts_with("--") => Err(format!("unknown flag {} for lock", other)),
        Some(other) => Err(format!("unknown lock subcommand {}", other)),
        None => Err("lock requires a subcommand: acquire, release or list".to_string()),
    }
}

fn parse_audit(args: &[String]) -> Result<Command, String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut limit: Option<i64> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" => limit = Some(parse_i64(&value(args, &mut i, "--limit")?, "--limit")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {} for audit", other))
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }
    let project = one_project(&positionals, "audit")?;
    let limit = limit.unwrap_or(1000);
    if limit < 1 {
        return Err("audit --limit must be at least 1".to_string());
    }
    Ok(Command::Audit { project, limit })
}

/// The value that follows a flag, consumed from the argument list.
fn value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{} requires a value", flag))
}

/// The value that follows a leading global flag.
fn take_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    value(args, i, flag)
}

/// Exactly one positional argument, or a clear error.
fn one_positional(positionals: &[String], verb: &str) -> Result<String, String> {
    match positionals {
        [name] => Ok(name.clone()),
        [] => Err(format!("{} requires a project", verb)),
        _ => Err(format!(
            "{} takes exactly one project argument, got {}",
            verb,
            positionals.len()
        )),
    }
}

/// Exactly one positional project argument, validated as a project name.
fn one_project(positionals: &[String], verb: &str) -> Result<String, String> {
    let name = one_positional(positionals, verb)?;
    validate_name("project name", &name)?;
    Ok(name)
}

/// Reuse the service's own name rules so an offline name and an HTTP name obey the same
/// charset, and so a project or branch name can never break the URL the CLI builds.
fn validate_name(kind: &str, name: &str) -> Result<(), String> {
    server::api::validate_name(kind, name).map_err(|e| e.message)
}

fn split_list(raw: &str, flag: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(format!("{} requires at least one value", flag));
    }
    Ok(items)
}

fn parse_i64(raw: &str, flag: &str) -> Result<i64, String> {
    raw.parse::<i64>()
        .map_err(|_| format!("{} must be an integer, got {}", flag, raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &[&str]) -> Result<Command, String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (_, command) = parse(&args)?;
        Ok(command)
    }

    fn db_args(args: &[&'static str]) -> Vec<&'static str> {
        let mut out = vec!["--db", "test.db"];
        out.extend_from_slice(args);
        out
    }

    #[test]
    fn project_create_parses() {
        let c = cmd(&db_args(&["project", "create", "coffee"])).unwrap();
        assert_eq!(
            c,
            Command::ProjectCreate {
                name: "coffee".to_string()
            }
        );
    }

    #[test]
    fn project_list_parses() {
        let c = cmd(&db_args(&["project", "list"])).unwrap();
        assert_eq!(c, Command::ProjectList);
    }

    #[test]
    fn commit_parses_with_and_without_holder() {
        let c = cmd(&db_args(&[
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "first commit",
            "--file",
            "model.json",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::Commit {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                message: "first commit".to_string(),
                file: PathBuf::from("model.json"),
                holder: None,
            }
        );

        let c = cmd(&db_args(&[
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "m",
            "--file",
            "model.json",
            "--holder",
            "alex",
        ]))
        .unwrap();
        assert!(matches!(
            c,
            Command::Commit {
                holder: Some(ref h),
                ..
            } if h == "alex"
        ));
    }

    #[test]
    fn log_parses() {
        let c = cmd(&db_args(&["log", "coffee", "--branch", "main"])).unwrap();
        assert_eq!(
            c,
            Command::Log {
                project: "coffee".to_string(),
                branch: "main".to_string(),
            }
        );
    }

    #[test]
    fn artifact_parses() {
        let c = cmd(&db_args(&["artifact", "coffee", "--hash", "abc123"])).unwrap();
        assert_eq!(
            c,
            Command::Artifact {
                project: "coffee".to_string(),
                hash: "abc123".to_string(),
            }
        );
    }

    #[test]
    fn branch_list_parses() {
        let c = cmd(&db_args(&["branch", "list", "coffee"])).unwrap();
        assert_eq!(
            c,
            Command::BranchList {
                project: "coffee".to_string()
            }
        );
    }

    #[test]
    fn branch_create_parses() {
        let c = cmd(&db_args(&[
            "branch", "create", "coffee", "--name", "review", "--from", "abc123",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::BranchCreate {
                project: "coffee".to_string(),
                name: "review".to_string(),
                from: "abc123".to_string(),
            }
        );
    }

    #[test]
    fn branch_delete_parses() {
        let c = cmd(&db_args(&[
            "branch", "delete", "coffee", "--name", "review",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::BranchDelete {
                project: "coffee".to_string(),
                name: "review".to_string(),
            }
        );
    }

    #[test]
    fn merge_parses() {
        let c = cmd(&db_args(&[
            "merge",
            "coffee",
            "--branch",
            "main",
            "--other",
            "feature",
            "--message",
            "merge feature",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::Merge {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                other: "feature".to_string(),
                message: "merge feature".to_string(),
                holder: None,
            }
        );
    }

    #[test]
    fn reset_parses() {
        let c = cmd(&db_args(&[
            "reset",
            "coffee",
            "--branch",
            "main",
            "--to",
            "abc123",
            "--message",
            "revert",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::Reset {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                to: "abc123".to_string(),
                message: "revert".to_string(),
                holder: None,
            }
        );
    }

    #[test]
    fn gate_parses() {
        let c = cmd(&db_args(&[
            "gate",
            "coffee",
            "--reference",
            "a",
            "--candidate",
            "b",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::Gate {
                project: "coffee".to_string(),
                reference: "a".to_string(),
                candidate: "b".to_string(),
            }
        );
    }

    #[test]
    fn lock_acquire_parses() {
        let c = cmd(&db_args(&[
            "lock",
            "acquire",
            "coffee",
            "--branch",
            "main",
            "--elements",
            "a,b",
            "--holder",
            "alex",
            "--ttl",
            "300",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::LockAcquire {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                elements: vec!["a".to_string(), "b".to_string()],
                holder: "alex".to_string(),
                ttl_seconds: 300,
            }
        );
    }

    #[test]
    fn lock_release_parses() {
        let c = cmd(&db_args(&[
            "lock", "release", "coffee", "--holder", "alex", "--ids", "x,y",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Command::LockRelease {
                project: "coffee".to_string(),
                holder: "alex".to_string(),
                ids: vec!["x".to_string(), "y".to_string()],
            }
        );
    }

    #[test]
    fn lock_list_parses() {
        let c = cmd(&db_args(&["lock", "list", "coffee"])).unwrap();
        assert_eq!(
            c,
            Command::LockList {
                project: "coffee".to_string()
            }
        );
    }

    #[test]
    fn audit_parses() {
        let c = cmd(&db_args(&["audit", "coffee", "--limit", "10"])).unwrap();
        assert_eq!(
            c,
            Command::Audit {
                project: "coffee".to_string(),
                limit: 10,
            }
        );
    }

    #[test]
    fn server_and_token_parse() {
        let args: Vec<String> = [
            "--server",
            "http://localhost:8080",
            "--token",
            "MW_TOKEN",
            "project",
            "list",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (transport, _) = parse(&args).unwrap();
        match transport {
            Transport::Server { url, token_var } => {
                assert_eq!(url, "http://localhost:8080");
                assert_eq!(token_var, Some("MW_TOKEN".to_string()));
            }
            Transport::Offline { .. } => panic!("expected server transport"),
        }
    }

    #[test]
    fn unknown_flag_is_a_clear_error() {
        let args: Vec<String> = [
            "--db", "test.db", "commit", "coffee", "--branch", "main", "--bogus",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("unknown flag"), "error was: {}", err);
        assert!(err.contains("--bogus"), "error was: {}", err);
    }

    #[test]
    fn unknown_command_is_a_clear_error() {
        let args: Vec<String> = ["--db", "test.db", "frobnicate"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("unknown command"), "error was: {}", err);
        assert!(err.contains("frobnicate"), "error was: {}", err);
    }

    #[test]
    fn server_and_db_are_mutually_exclusive() {
        let args: Vec<String> = ["--server", "http://x", "--db", "test.db"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("mutually exclusive"), "error was: {}", err);
    }

    #[test]
    fn token_requires_server() {
        let args: Vec<String> = ["--db", "test.db", "--token", "MW_TOKEN"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("--server"), "error was: {}", err);
    }

    #[test]
    fn missing_transport_is_a_clear_error() {
        let args: Vec<String> = ["project", "list"].iter().map(|s| s.to_string()).collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("--server"), "error was: {}", err);
        assert!(err.contains("--db"), "error was: {}", err);
    }

    #[test]
    fn a_missing_flag_value_is_a_clear_error() {
        let args: Vec<String> = ["--db", "test.db", "commit", "coffee", "--branch"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let err = parse(&args).unwrap_err();
        assert!(err.contains("requires a value"), "error was: {}", err);
    }
}
