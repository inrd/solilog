//! Command-line interface. Designed for scripts and AI agents as much as
//! for humans:
//!
//! - every command accepts `--json` for stable, machine-readable output;
//! - write commands print the resulting object;
//! - errors go to stderr with a non-zero exit code (`--json`: as `{"error": …}`);
//! - issue ids are accepted as `SOL-12` or `12`;
//! - labels are `/`-separated paths (`acme/reverb`), matched case-insensitively.

//!
//! `args` holds the clap definitions (and so `--help`), `run` one function
//! per command group, `text` the human-readable output.

mod args;
mod run;
mod text;

use std::io::IsTerminal as _;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde_json::json;
use solilog_core::{Actor, SkillEnv};

pub use args::{Cli, Command};
use text::Output;

pub fn run(cli: Cli, command: Command) -> Result<()> {
    let out = Output { json: cli.json };
    let actor = match cli.actor {
        Some(actor) => actor,
        None => match std::env::var("SOLILOG_ACTOR") {
            Ok(raw) if !raw.trim().is_empty() => {
                Actor::parse(&raw).context("parsing SOLILOG_ACTOR")?
            }
            _ => Actor::cli(),
        },
    };
    let hint_skill = !cli.json && !matches!(command, Command::Skill(_));
    let mut opened = None;
    let result = run::command(&out, command, actor, &mut opened);
    // Changes made from the CLI (e.g. by agents) are backed up too.
    if let Some(db) = opened
        && result.is_ok()
        && db.has_written()
        && let Err(err) = db.auto_backup()
    {
        eprintln!("warning: automatic backup failed: {err:#}");
    }
    if hint_skill && result.is_ok() && std::io::stderr().is_terminal() {
        hint_outdated_skill();
    }
    result
}

const SKILL_HINT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const SKILL_HINT_FILE: &str = ".skill-hint-shown";

/// For people at a terminal (agents and scripts don't see it): a nudge to
/// update the agent skill after upgrading Solilog, at most once a day.
/// Set `SOLILOG_NO_SKILL_HINT=1` to silence it.
fn hint_outdated_skill() {
    if skill_hint_silenced() {
        return;
    }
    let Ok(env) = SkillEnv::from_env() else {
        return;
    };
    let Some(path) = solilog_core::outdated_skills(&env).into_iter().next() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = crate::ui::keymap::config_dir().map(|d| d.join(SKILL_HINT_FILE));
    if let Some(stamp) = &stamp
        && !skill_hint_due(stamp, now)
    {
        return;
    }
    eprintln!(
        "note: the Solilog skill for AI agents in {} is out of date; \
         run `solilog skill install` to update it",
        path.parent().unwrap_or(&path).display()
    );
    if let Some(stamp) = &stamp {
        record_skill_hint(stamp, now);
    }
}

fn skill_hint_silenced() -> bool {
    std::env::var_os("SOLILOG_NO_SKILL_HINT").is_some_and(|v| !v.is_empty() && v != "0")
}

fn skill_hint_due(stamp_path: &Path, now: u64) -> bool {
    let Ok(raw) = std::fs::read_to_string(stamp_path) else {
        return true;
    };
    let Ok(last) = raw.trim().parse::<u64>() else {
        return true;
    };
    now < last || now - last >= SKILL_HINT_INTERVAL.as_secs()
}

fn record_skill_hint(stamp_path: &Path, now: u64) {
    if let Some(dir) = stamp_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(stamp_path, format!("{now}\n"));
}

/// Prints an error the way the current output mode expects.
pub fn report_error(json: bool, err: &anyhow::Error) {
    if json {
        eprintln!("{}", json!({ "error": format!("{err:#}") }));
    } else {
        eprintln!("error: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::args::IssueCommand;
    use super::*;
    use clap::Parser as _;

    #[test]
    fn parse_issue_ls_compact() {
        let cli = Cli::try_parse_from(["solilog", "issue", "ls", "--compact", "--json"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Some(Command::Issue(IssueCommand::List {
                compact,
                archived,
                view,
                ..
            })) => {
                assert!(compact);
                assert!(!archived);
                assert!(view.is_none());
            }
            _ => panic!("expected IssueCommand::List"),
        }

        let cli = Cli::try_parse_from([
            "solilog",
            "issue",
            "ls",
            "--compact",
            "--archived",
            "--view",
            "Inbox",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Issue(IssueCommand::List {
                compact,
                archived,
                view,
                ..
            })) => {
                assert!(compact);
                assert!(archived);
                assert_eq!(view.as_deref(), Some("Inbox"));
            }
            _ => panic!("expected IssueCommand::List"),
        }
    }

    #[test]
    fn skill_hint_throttles_once_a_day() {
        let dir = std::env::temp_dir().join(format!("solilog-skill-hint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stamp = dir.join(SKILL_HINT_FILE);

        let t0 = 1_700_000_000;
        assert!(skill_hint_due(&stamp, t0));

        record_skill_hint(&stamp, t0);
        assert!(!skill_hint_due(&stamp, t0));
        assert!(!skill_hint_due(
            &stamp,
            t0 + SKILL_HINT_INTERVAL.as_secs() - 1
        ));
        assert!(skill_hint_due(&stamp, t0 + SKILL_HINT_INTERVAL.as_secs()));

        // A corrupt or future timestamp doesn't silence the hint forever.
        std::fs::write(&stamp, "not-a-timestamp\n").unwrap();
        assert!(skill_hint_due(&stamp, t0));
        record_skill_hint(&stamp, t0 + 100);
        assert!(skill_hint_due(&stamp, t0));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
