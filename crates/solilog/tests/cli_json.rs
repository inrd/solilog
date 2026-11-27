//! The CLI's `--json` output is a stable API: agents depend on its field
//! names and enum keys (see "CLI contract" in AGENTS.md). This test runs
//! the binary through a scenario on a temporary database and compares each
//! command's output with a snapshot in `tests/snapshots/cli_json/`.
//!
//! A renamed or removed field fails the test. So does an added one: adding
//! fields is allowed, but it should be deliberate. After checking the diff,
//! update the snapshots with
//!
//! ```sh
//! SOLILOG_UPDATE_SNAPSHOTS=1 cargo test -p solilog --test cli_json
//! ```
//!
//! Values that change from run to run (times, the temporary directory,
//! backup names) are replaced by placeholders before comparing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const ACTOR: &str = "claude:contract-test";
const UPDATE_ENV: &str = "SOLILOG_UPDATE_SNAPSHOTS";

/// Runs `solilog` on a database of its own, in a temporary directory.
struct Cli {
    dir: PathBuf,
    /// `dir` with symlinks resolved (`/var` → `/private/var` on macOS), as
    /// paths may be printed either way.
    canonical_dir: PathBuf,
    snapshots: Snapshots,
}

impl Cli {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("solilog-cli-json-{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(dir.join("code")).unwrap();
        let canonical_dir = dir.canonicalize().unwrap();
        Self {
            dir,
            canonical_dir,
            snapshots: Snapshots::new(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_solilog"));
        command
            .args(args)
            .current_dir(self.dir.join("code"))
            .env("SOLILOG_DB", self.dir.join("solilog.db"))
            .env("SOLILOG_CONFIG_DIR", self.dir.join("config"))
            .env("SOLILOG_NO_SKILL_HINT", "1")
            .env_remove("SOLILOG_ACTOR");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("running solilog")
    }

    /// Runs a command with `--json` as the test's agent, checks it
    /// succeeded, and returns its output.
    fn json(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.extend(["--json", "--actor", ACTOR]);
        let output = self.run(&all);
        assert!(
            output.status.success(),
            "solilog {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "solilog {} printed invalid JSON ({err}): {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    /// Runs a command (see [`Cli::json`]) and checks its output against
    /// the snapshot `name`. Returns the output, unscrubbed.
    fn check(&mut self, name: &str, args: &[&str]) -> Value {
        self.check_varying(name, args, &[])
    }

    /// [`Cli::check`], with the values of the `varying` fields (at any
    /// depth) replaced by a placeholder: the fields must be there, but
    /// their values change from run to run.
    fn check_varying(&mut self, name: &str, args: &[&str], varying: &[&str]) -> Value {
        let value = self.json(args);
        let scrubbed = self.scrub(value.clone(), None, varying);
        self.snapshots.check(name, &command_line(args), &scrubbed);
        value
    }

    /// Replaces what varies between runs with placeholders.
    fn scrub(&self, value: Value, key: Option<&str>, varying: &[&str]) -> Value {
        if key.is_some_and(|k| varying.contains(&k) || k == "app_version") {
            return "<varies>".into();
        }
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| {
                        let v = self.scrub(v, Some(&k), varying);
                        (k, v)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|v| self.scrub(v, key, varying))
                    .collect(),
            ),
            Value::Number(_) if key.is_some_and(is_time_key) => "<time>".into(),
            Value::String(s) if key.is_some_and(is_time_key) && !s.is_empty() => "<time>".into(),
            Value::String(s) => {
                let s = s
                    .replace(&*self.canonical_dir.to_string_lossy(), "<tmp>")
                    .replace(&*self.dir.to_string_lossy(), "<tmp>");
                Value::String(s)
            }
            other => other,
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn is_time_key(key: &str) -> bool {
    key.ends_with("_at") || key == "time"
}

/// `args` as typed in a shell (roughly): arguments with spaces quoted.
fn command_line(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                (*a).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The checked-in snapshots, and the ones this run produced.
struct Snapshots {
    dir: PathBuf,
    update: bool,
    seen: BTreeSet<String>,
    mismatches: Vec<String>,
}

impl Snapshots {
    fn new() -> Self {
        Self {
            dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/cli_json"),
            update: std::env::var_os(UPDATE_ENV).is_some_and(|v| !v.is_empty() && v != "0"),
            seen: BTreeSet::new(),
            mismatches: Vec::new(),
        }
    }

    fn check(&mut self, name: &str, command: &str, value: &Value) {
        assert!(
            self.seen.insert(name.to_owned()),
            "snapshot {name} taken twice"
        );
        let path = self.dir.join(format!("{name}.json"));
        let actual = format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": format!("solilog {command} --json"),
                "output": value,
            }))
            .unwrap()
        );
        let expected = std::fs::read_to_string(&path).ok();
        if expected.as_deref() == Some(&actual) {
            return;
        }
        if self.update {
            std::fs::create_dir_all(&self.dir).unwrap();
            std::fs::write(&path, actual).unwrap();
            return;
        }
        self.mismatches.push(match expected {
            None => format!("{name}: no snapshot yet; got\n{actual}"),
            Some(expected) => format!("{name}: {}", first_difference(&expected, &actual)),
        });
    }

    /// Fails if any output didn't match, or a snapshot is no longer
    /// produced (a command or step was removed).
    fn finish(&self) {
        let stale: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        name.strip_suffix(".json").map(str::to_owned)
                    })
                    .filter(|name| !self.seen.contains(name))
                    .collect()
            })
            .unwrap_or_default();
        if self.update {
            for name in &stale {
                std::fs::remove_file(self.dir.join(format!("{name}.json"))).unwrap();
            }
            return;
        }
        let mut problems = self.mismatches.clone();
        problems.extend(
            stale
                .iter()
                .map(|name| format!("{name}: no longer produced")),
        );
        assert!(
            problems.is_empty(),
            "the CLI's JSON output changed. If that's intended (adding fields is fine; \
             renaming or removing them breaks agents), update the snapshots with \
             `{UPDATE_ENV}=1 cargo test -p solilog --test cli_json`.\n\n{}",
            problems.join("\n\n")
        );
    }
}

fn first_difference(expected: &str, actual: &str) -> String {
    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    let mut line = 1;
    loop {
        match (expected_lines.next(), actual_lines.next()) {
            (Some(e), Some(a)) if e == a => line += 1,
            (e, a) => {
                return format!(
                    "first difference at line {line}\n  expected: {}\n  actual:   {}",
                    e.unwrap_or("<end>"),
                    a.unwrap_or("<end>")
                );
            }
        }
    }
}

/// Reads an issue id (`"SOL-1"`) from an issue's JSON.
fn id(issue: &Value) -> String {
    issue["id"].as_str().expect("issue id").to_owned()
}

#[test]
fn json_output_matches_snapshots() {
    let mut cli = Cli::new("snapshots");

    // Labels and projects.
    cli.check("label-create", &["label", "create", "acme/reverb"]);
    cli.check("label-color", &["label", "color", "acme", "blue"]);
    cli.check("label-add-path", &["label", "add-path", "acme/reverb", "."]);
    cli.check(
        "project-create",
        &[
            "project",
            "create",
            "Reverb v1",
            "-l",
            "acme/reverb",
            "-d",
            "Ship it",
        ],
    );
    cli.check(
        "project-create-planned",
        &["project", "create", "Later", "-s", "planned"],
    );
    cli.check(
        "project-update",
        &["project", "update", "Later", "-d", "Some day"],
    );

    // Issues.
    let dep = cli.check(
        "issue-create",
        &[
            "issue",
            "create",
            "Fix the tail",
            "-d",
            "It clicks.",
            "-l",
            "bug",
            "-p",
            "Reverb v1",
            "-i",
            "Reproduce",
            "-i",
            "Fix",
        ],
    );
    let dep = id(&dep);
    let issue = cli.check(
        "issue-create-dependent",
        &[
            "issue",
            "create",
            "Release",
            "-s",
            "backlog",
            "--depends-on",
            &dep,
            "-d",
            &format!("After {dep}."),
        ],
    );
    let issue = id(&issue);
    cli.check(
        "issue-edit",
        &[
            "issue",
            "edit",
            &dep,
            "--title",
            "Fix the reverb tail",
            "--status",
            "in_progress",
        ],
    );
    cli.check("issue-set-status", &["issue", "set-status", &issue, "todo"]);
    cli.check(
        "issue-add-label",
        &["issue", "add-label", &issue, "release"],
    );

    // Checklists.
    let added = cli.check("checklist-add", &["checklist", "add", &dep, "Add a test"]);
    let first_item = added["checklist"][0]["id"]
        .as_i64()
        .expect("checklist item id")
        .to_string();
    cli.check("checklist-check", &["checklist", "check", &first_item]);

    // Notes, decisions, review.
    cli.check(
        "note-add",
        &["note", "add", &dep, "Tried a longer window: no change."],
    );
    cli.check("note-list", &["note", "list", &dep]);
    let decision = cli.check(
        "decision-add",
        &["decision", "add", "--label", "acme", "Keep the v1 API"],
    );
    let decision = decision["id"].as_i64().expect("decision id").to_string();
    cli.check(
        "decision-replace",
        &[
            "decision",
            "replace",
            &decision,
            "Keep the v1 API until 2.0",
        ],
    );
    cli.check(
        "decision-list",
        &["decision", "list", "--label", "acme", "--all"],
    );
    cli.check("decision-unreviewed", &["decision", "unreviewed"]);
    cli.check(
        "issue-request-review",
        &[
            "issue",
            "request-review",
            &dep,
            "-m",
            "Fixed.",
            "--pass",
            "cargo test",
            "--fail",
            "cargo clippy: 1 lint",
            "--commit",
            "abc1234",
            "--file",
            "src/tail.rs",
        ],
    );

    // Attachments.
    // Relative to the current directory, so the command line is the same
    // on every run.
    std::fs::write(cli.dir.join("code/crash.log"), "boom\n").unwrap();
    cli.check("attachment-add", &["attachment", "add", &dep, "crash.log"]);
    cli.check("attachment-list", &["attachment", "list", &dep]);

    // Views.
    cli.check(
        "view-save",
        &[
            "view",
            "save",
            "Acme bugs",
            "--open",
            "-l",
            "acme",
            "-l",
            "bug",
        ],
    );
    cli.check("view-show", &["view", "show", "Acme bugs"]);
    cli.check("view-list", &["view", "list"]);

    // Reading.
    cli.check("issue-show", &["issue", "show", &dep]);
    cli.check("issue-list", &["issue", "list"]);
    cli.check("issue-list-compact", &["issue", "list", "--compact"]);
    cli.check(
        "issue-list-ready",
        &["issue", "list", "--ready", "--compact"],
    );
    cli.check("issue-context", &["issue", "context", &issue]);
    cli.check("issue-context-in-review", &["issue", "context", &dep]);
    cli.check("issue-history", &["issue", "history", &dep]);
    cli.check("issue-activity", &["issue", "activity", &dep]);
    cli.check("search", &["search", "reverb", "--include-closed"]);
    cli.check("label-list", &["label", "list"]);
    cli.check("label-show", &["label", "show", "acme/reverb"]);
    cli.check("project-list", &["project", "list", "--all"]);
    cli.check("project-show", &["project", "show", "Reverb v1"]);
    cli.check("where", &["where"]);
    cli.check("statuses", &["statuses"]);

    // Backups (a file name with the time and database id, a zip's size).
    cli.check_varying(
        "backup-create",
        &["backup", "create"],
        &["name", "path", "size", "database_id"],
    );
    cli.check("backup-dir", &["backup", "dir"]);

    // Approval and the issue's end.
    cli.check("issue-approve", &["issue", "approve", &dep]);
    cli.check("issue-review-all", &["issue", "review", "--all"]);
    cli.check("issue-archive", &["issue", "archive", &issue]);
    cli.check("issue-list-archived", &["issue", "list", "--archived"]);
    cli.check("issue-delete", &["issue", "delete", &issue]);

    cli.snapshots.finish();
}

/// Failures: non-zero exit, nothing on stdout, and with `--json` a single
/// `{"error": "<message>"}` object on stderr.
#[test]
fn errors() {
    let cli = Cli::new("errors");

    let output = cli.run(&["issue", "show", "SOL-999", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).expect("JSON error on stderr");
    let fields: Vec<&String> = error.as_object().expect("an object").keys().collect();
    assert_eq!(fields, ["error"]);
    assert!(error["error"].as_str().is_some_and(|m| m.contains("999")));

    let output = cli.run(&["issue", "show", "SOL-999"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: "));

    // Usage errors come from clap.
    let output = cli.run(&["issue", "show", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let output = cli.run(&["issue", "set-status", "1", "nonsense", "--json"]);
    assert_eq!(output.status.code(), Some(2));
}
