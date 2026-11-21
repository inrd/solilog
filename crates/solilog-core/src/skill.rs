//! The agent skill: a `SKILL.md`, compiled in, that tells coding agents
//! (Claude Code, Codex, Cursor, …) when and how to use the `solilog` command,
//! and its installation into their skills folders.
//!
//! This is installation support, not a domain operation: it touches no
//! database. It lives in core so it can be tested without gpui, against a
//! temporary home directory ([`SkillEnv`]).
//!
//! Three folders cover every supported agent: `~/.claude/skills/solilog`
//! (Claude Code), `~/.agents/skills/solilog` (the shared Agent Skills
//! folder, read by Codex, Cursor, OpenCode and pi), and
//! `~/.gemini/config/skills/solilog` (Antigravity). Each holds its own copy,
//! so any can be removed without breaking the others. An installed copy
//! carries a marker in its frontmatter (`managed-by: solilog`); a file
//! without it is never overwritten or removed.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, Result};
use serde::Serialize;

/// The skill's name: its folder name and the `name` in its frontmatter.
pub const SKILL_NAME: &str = "solilog";

/// The version written into the skill: the version of this binary.
pub const SKILL_VERSION: &str = env!("CARGO_PKG_VERSION");

const SKILL_TEMPLATE: &str = include_str!("../assets/skill/SKILL.md");

/// Frontmatter line that marks a `SKILL.md` as written by Solilog.
const MARKER: &str = "managed-by: solilog";

const SKILL_FILE: &str = "SKILL.md";

/// The skill's `SKILL.md`, as installed by this binary.
pub fn skill_md() -> String {
    SKILL_TEMPLATE.replace("@VERSION@", SKILL_VERSION)
}

/// A coding agent the skill can be installed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Antigravity,
    Claude,
    Codex,
    Cursor,
    Opencode,
    Pi,
}

impl Agent {
    pub const ALL: [Agent; 6] = [
        Agent::Antigravity,
        Agent::Claude,
        Agent::Codex,
        Agent::Cursor,
        Agent::Opencode,
        Agent::Pi,
    ];

    /// Stable identifier, used on the command line and in JSON.
    pub fn key(self) -> &'static str {
        match self {
            Agent::Antigravity => "antigravity",
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
            Agent::Opencode => "opencode",
            Agent::Pi => "pi",
        }
    }

    /// Where this agent finds the skill.
    pub fn location(self) -> SkillLocation {
        match self {
            Agent::Antigravity => SkillLocation::Antigravity,
            Agent::Claude => SkillLocation::Claude,
            Agent::Codex | Agent::Cursor | Agent::Opencode | Agent::Pi => SkillLocation::Agents,
        }
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for Agent {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Agent::ALL
            .into_iter()
            .find(|a| a.key().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| {
                let keys: Vec<_> = Agent::ALL.iter().map(|a| a.key()).collect();
                format!("unknown agent {s:?} (expected one of: {})", keys.join(", "))
            })
    }
}

/// A skills folder the skill is installed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLocation {
    /// `~/.claude/skills/`, for Claude Code.
    Claude,
    /// `~/.agents/skills/`, the shared Agent Skills folder.
    Agents,
    /// `~/.gemini/config/skills/`, for Antigravity.
    Antigravity,
}

impl SkillLocation {
    pub const ALL: [SkillLocation; 3] = [
        SkillLocation::Claude,
        SkillLocation::Agents,
        SkillLocation::Antigravity,
    ];

    /// The agents that get the skill from this folder.
    pub fn agents(self) -> Vec<Agent> {
        Agent::ALL
            .into_iter()
            .filter(|a| a.location() == self)
            .collect()
    }
}

/// Where agents keep their configuration: the home directory, plus the
/// overrides some agents support. Tests use a temporary home.
#[derive(Clone, Debug)]
pub struct SkillEnv {
    pub home: PathBuf,
    /// pi's agent directory (`$PI_CODING_AGENT_DIR`), if set.
    pub pi_agent_dir: Option<PathBuf>,
}

impl SkillEnv {
    /// The current user's: `$HOME` and the agents' environment variables.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            home: dirs::home_dir().context("no home directory")?,
            pi_agent_dir: std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from),
        })
    }

    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            pi_agent_dir: None,
        }
    }

    /// The skill's folder in a location (`~/.claude/skills/solilog`).
    pub fn skill_dir(&self, location: SkillLocation) -> PathBuf {
        let root = match location {
            SkillLocation::Claude => self.home.join(".claude"),
            SkillLocation::Agents => self.home.join(".agents"),
            SkillLocation::Antigravity => self.home.join(".gemini").join("config"),
        };
        root.join("skills").join(SKILL_NAME)
    }

    /// The folder whose presence shows that an agent is installed.
    pub fn agent_config_dir(&self, agent: Agent) -> PathBuf {
        match agent {
            Agent::Antigravity => self.home.join(".gemini").join("antigravity"),
            Agent::Claude => self.home.join(".claude"),
            Agent::Codex => self.home.join(".codex"),
            Agent::Cursor => self.home.join(".cursor"),
            Agent::Opencode => self.home.join(".config").join("opencode"),
            Agent::Pi => self
                .pi_agent_dir
                .clone()
                .unwrap_or_else(|| self.home.join(".pi").join("agent")),
        }
    }

    /// The agents whose configuration folder exists.
    pub fn detected_agents(&self) -> Vec<Agent> {
        Agent::ALL
            .into_iter()
            .filter(|&a| self.agent_config_dir(a).is_dir())
            .collect()
    }
}

/// The state of the skill in one location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    /// Not installed.
    Missing,
    /// Installed by Solilog and identical to this binary's skill.
    Current,
    /// Installed by Solilog, but different from this binary's skill.
    Outdated,
    /// Something Solilog did not write is in the way; it is left alone.
    Foreign,
}

impl SkillState {
    /// Stable identifier, as in JSON.
    pub fn key(self) -> &'static str {
        match self {
            SkillState::Missing => "missing",
            SkillState::Current => "current",
            SkillState::Outdated => "outdated",
            SkillState::Foreign => "foreign",
        }
    }
}

/// The skill in one location.
#[derive(Clone, Debug, Serialize)]
pub struct SkillStatus {
    pub location: SkillLocation,
    /// The `SKILL.md` file.
    pub path: PathBuf,
    /// The agents that read it.
    pub agents: Vec<Agent>,
    pub state: SkillState,
    /// The version in the installed skill's frontmatter, if it is Solilog's.
    pub installed_version: Option<String>,
}

/// What `install_skill` or `uninstall_skill` did in one location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillAction {
    Installed,
    Updated,
    Unchanged,
    Removed,
    /// Uninstall: there was nothing to remove.
    NotInstalled,
    /// Uninstall: not Solilog's, left alone.
    Foreign,
}

impl SkillAction {
    /// Stable identifier, as in JSON.
    pub fn key(self) -> &'static str {
        match self {
            SkillAction::Installed => "installed",
            SkillAction::Updated => "updated",
            SkillAction::Unchanged => "unchanged",
            SkillAction::Removed => "removed",
            SkillAction::NotInstalled => "not_installed",
            SkillAction::Foreign => "foreign",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillChange {
    pub location: SkillLocation,
    /// The `SKILL.md` file.
    pub path: PathBuf,
    /// The agents that read it.
    pub agents: Vec<Agent>,
    pub action: SkillAction,
    /// Uninstall: other files left in the skill's folder, which was kept.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<PathBuf>,
}

/// The locations the given agents read, in [`SkillLocation::ALL`] order.
fn locations_for(agents: &[Agent]) -> Vec<SkillLocation> {
    SkillLocation::ALL
        .into_iter()
        .filter(|l| agents.iter().any(|a| a.location() == *l))
        .collect()
}

/// The skill's state in the locations the given agents read (all locations
/// if `agents` is empty).
pub fn skill_status(env: &SkillEnv, agents: &[Agent]) -> Result<Vec<SkillStatus>> {
    let locations = if agents.is_empty() {
        SkillLocation::ALL.to_vec()
    } else {
        locations_for(agents)
    };
    locations.into_iter().map(|l| status_at(env, l)).collect()
}

/// The installed skills that differ from this binary's. Errors count as
/// "not outdated": this only feeds a hint.
pub fn outdated_skills(env: &SkillEnv) -> Vec<PathBuf> {
    SkillLocation::ALL
        .into_iter()
        .filter_map(|l| status_at(env, l).ok())
        .filter(|s| s.state == SkillState::Outdated)
        .map(|s| s.path)
        .collect()
}

/// Installs or updates the skill for the given agents (the detected ones if
/// `agents` is empty). All or nothing: if a location holds something Solilog
/// did not write, nothing is written.
pub fn install_skill(env: &SkillEnv, agents: &[Agent]) -> Result<Vec<SkillChange>> {
    let agents = if agents.is_empty() {
        let detected = env.detected_agents();
        anyhow::ensure!(
            !detected.is_empty(),
            "no supported agent found (looked for {}); choose one with --agent",
            Agent::ALL
                .iter()
                .map(|&a| env.agent_config_dir(a).display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        detected
    } else {
        agents.to_vec()
    };
    let statuses: Vec<_> = locations_for(&agents)
        .into_iter()
        .map(|l| status_at(env, l))
        .collect::<Result<_>>()?;
    let foreign: Vec<_> = statuses
        .iter()
        .filter(|s| s.state == SkillState::Foreign)
        .map(|s| s.path.parent().unwrap_or(&s.path).display().to_string())
        .collect();
    anyhow::ensure!(
        foreign.is_empty(),
        "not installing: {} {} not written by Solilog; move {} away and try again",
        foreign.join(", "),
        if foreign.len() == 1 { "was" } else { "were" },
        if foreign.len() == 1 { "it" } else { "them" },
    );
    let content = skill_md();
    let mut changes = Vec::new();
    for status in statuses {
        // Re-read: two locations can be the same folder (a symlink).
        let state = status_at(env, status.location)?.state;
        let action = match state {
            SkillState::Current => SkillAction::Unchanged,
            SkillState::Missing => SkillAction::Installed,
            SkillState::Outdated => SkillAction::Updated,
            SkillState::Foreign => unreachable!("checked above"),
        };
        if action != SkillAction::Unchanged {
            let dir = env.skill_dir(status.location);
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            fs::write(&status.path, &content)
                .with_context(|| format!("writing {}", status.path.display()))?;
        }
        changes.push(change(status, action));
    }
    Ok(changes)
}

/// Updates the skill where Solilog installed it from an older version, as
/// when a new version starts. Never installs it where it is missing, touches
/// a `SKILL.md` Solilog didn't write, or replaces a skill from the same or a
/// newer version (an older app, or a development build, leaves it alone).
/// Returns the locations it updated.
pub fn update_outdated_skills(env: &SkillEnv) -> Result<Vec<SkillChange>> {
    let content = skill_md();
    let mut changes = Vec::new();
    for location in SkillLocation::ALL {
        // Read each in turn: two locations can be the same folder (a symlink).
        let status = status_at(env, location)?;
        let older = status
            .installed_version
            .as_deref()
            .is_none_or(|v| version_older(v, SKILL_VERSION));
        if status.state == SkillState::Outdated && older {
            fs::write(&status.path, &content)
                .with_context(|| format!("writing {}", status.path.display()))?;
            changes.push(change(status, SkillAction::Updated));
        }
    }
    Ok(changes)
}

/// Removes the skill from the locations the given agents read (every
/// location if `agents` is empty). Only Solilog's own `SKILL.md` is removed,
/// then its folder if nothing else is in it; other files are left alone.
pub fn uninstall_skill(env: &SkillEnv, agents: &[Agent]) -> Result<Vec<SkillChange>> {
    let mut changes = Vec::new();
    for status in skill_status(env, agents)? {
        let dir = env.skill_dir(status.location);
        let mut kept = Vec::new();
        let action = match status.state {
            SkillState::Missing => SkillAction::NotInstalled,
            SkillState::Foreign => SkillAction::Foreign,
            SkillState::Current | SkillState::Outdated => {
                if is_symlink(&dir) {
                    // A link to Solilog's skill elsewhere: remove the link only.
                    remove_link(&dir)?;
                } else {
                    fs::remove_file(&status.path)
                        .with_context(|| format!("removing {}", status.path.display()))?;
                    kept = dir_entries(&dir)?;
                    if kept.is_empty() {
                        fs::remove_dir(&dir)
                            .with_context(|| format!("removing {}", dir.display()))?;
                    }
                }
                SkillAction::Removed
            }
        };
        changes.push(SkillChange {
            kept,
            ..change(status, action)
        });
    }
    Ok(changes)
}

/// Whether version `a` comes before `b` (`MAJOR.MINOR.PATCH`; a pre-release
/// suffix is ignored). A version that doesn't parse counts as older.
fn version_older(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        let core = v.trim().split(['-', '+']).next()?;
        core.split('.').map(|n| n.parse().ok()).collect()
    }
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a < b,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

fn change(status: SkillStatus, action: SkillAction) -> SkillChange {
    SkillChange {
        location: status.location,
        path: status.path,
        agents: status.agents,
        action,
        kept: Vec::new(),
    }
}

fn status_at(env: &SkillEnv, location: SkillLocation) -> Result<SkillStatus> {
    let dir = env.skill_dir(location);
    let path = dir.join(SKILL_FILE);
    let (state, installed_version) = read_state(&dir, &path)?;
    Ok(SkillStatus {
        location,
        path,
        agents: location.agents(),
        state,
        installed_version,
    })
}

fn read_state(dir: &Path, path: &Path) -> Result<(SkillState, Option<String>)> {
    match fs::symlink_metadata(dir) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok((SkillState::Missing, None)),
        Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
        Ok(_) if !dir.is_dir() => return Ok((SkillState::Foreign, None)),
        Ok(_) => {}
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            // An empty folder is free to use; one with other files is not ours.
            let state = if dir_entries(dir)?.is_empty() {
                SkillState::Missing
            } else {
                SkillState::Foreign
            };
            return Ok((state, None));
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let frontmatter = frontmatter(&text);
    if !frontmatter.iter().any(|l| l.trim() == MARKER) {
        return Ok((SkillState::Foreign, None));
    }
    let version = frontmatter.iter().find_map(|l| {
        let value = l.trim().strip_prefix("version:")?.trim();
        Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned())
    });
    let state = if text == skill_md() {
        SkillState::Current
    } else {
        SkillState::Outdated
    };
    Ok((state, version))
}

/// The lines between the opening and closing `---` (none if there is no
/// frontmatter).
fn frontmatter(text: &str) -> Vec<&str> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Vec::new();
    }
    lines.take_while(|l| l.trim_end() != "---").collect()
}

fn dir_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    entries.sort();
    Ok(entries)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn remove_link(path: &Path) -> Result<()> {
    // On Windows a link to a folder is removed like a folder.
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .with_context(|| format!("removing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ISSUE_PREFIX;

    /// A fresh temporary home directory, removed on drop.
    struct TempHome(PathBuf);

    impl TempHome {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("solilog-skill-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn env(&self) -> SkillEnv {
            SkillEnv::new(&self.0)
        }

        /// Makes an agent look installed.
        fn agent(&self, agent: Agent) -> &Self {
            fs::create_dir_all(self.env().agent_config_dir(agent)).unwrap();
            self
        }

        fn skill(&self, location: SkillLocation) -> PathBuf {
            self.env().skill_dir(location).join(SKILL_FILE)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn actions(changes: &[SkillChange]) -> Vec<(SkillLocation, SkillAction)> {
        changes.iter().map(|c| (c.location, c.action)).collect()
    }

    fn states(env: &SkillEnv) -> Vec<SkillState> {
        skill_status(env, &[])
            .unwrap()
            .into_iter()
            .map(|s| s.state)
            .collect()
    }

    #[test]
    fn skill_text_is_valid() {
        let text = skill_md();
        let front = frontmatter(&text);
        assert!(front.contains(&format!("name: {SKILL_NAME}").as_str()));
        assert!(front.iter().any(|l| l.trim() == MARKER));
        assert!(front.contains(&format!("  version: \"{SKILL_VERSION}\"").as_str()));
        assert!(!text.contains("@VERSION@"));
        // Agent Skills limit: 1024 characters.
        let description: usize = front
            .iter()
            .skip_while(|l| !l.starts_with("description:"))
            .skip(1)
            .take_while(|l| l.starts_with("  "))
            .map(|l| l.trim().len() + 1)
            .sum();
        assert!((100..=1024).contains(&description), "{description}");
    }

    #[test]
    fn skill_text_uses_the_issue_prefix() {
        let text = skill_md();
        assert!(!crate::issue_mentions(&text).is_empty());
        // Every `ABC-12` in the text uses the real prefix.
        for (i, _) in text.match_indices('-') {
            let letters = text[..i]
                .chars()
                .rev()
                .take_while(char::is_ascii_uppercase)
                .count();
            let digit = text[i + 1..].starts_with(|c: char| c.is_ascii_digit());
            if letters > 0 && digit {
                assert_eq!(&text[i - letters..i], ISSUE_PREFIX);
            }
        }
    }

    #[test]
    fn keys_match_json() {
        for state in [
            SkillState::Missing,
            SkillState::Current,
            SkillState::Outdated,
            SkillState::Foreign,
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), state.key());
        }
        for action in [
            SkillAction::Installed,
            SkillAction::Updated,
            SkillAction::Unchanged,
            SkillAction::Removed,
            SkillAction::NotInstalled,
            SkillAction::Foreign,
        ] {
            assert_eq!(serde_json::to_value(action).unwrap(), action.key());
        }
    }

    #[test]
    fn installs_for_detected_agents() {
        let home = TempHome::new("detected");
        let env = home.env();
        assert!(install_skill(&env, &[]).is_err(), "no agent installed");

        home.agent(Agent::Cursor);
        let changes = install_skill(&env, &[]).unwrap();
        assert_eq!(
            actions(&changes),
            [(SkillLocation::Agents, SkillAction::Installed)]
        );
        assert_eq!(
            fs::read_to_string(home.skill(SkillLocation::Agents)).unwrap(),
            skill_md()
        );
        assert!(!home.skill(SkillLocation::Claude).exists());

        home.agent(Agent::Claude);
        let changes = install_skill(&env, &[]).unwrap();
        assert_eq!(
            actions(&changes),
            [
                (SkillLocation::Claude, SkillAction::Installed),
                (SkillLocation::Agents, SkillAction::Unchanged),
            ]
        );
        assert_eq!(changes[1].agents, SkillLocation::Agents.agents());
        assert_eq!(
            states(&env),
            [
                SkillState::Current,
                SkillState::Current,
                SkillState::Missing
            ]
        );

        home.agent(Agent::Antigravity);
        let changes = install_skill(&env, &[]).unwrap();
        assert_eq!(
            actions(&changes),
            [
                (SkillLocation::Claude, SkillAction::Unchanged),
                (SkillLocation::Agents, SkillAction::Unchanged),
                (SkillLocation::Antigravity, SkillAction::Installed),
            ]
        );
        assert_eq!(
            fs::read_to_string(home.skill(SkillLocation::Antigravity)).unwrap(),
            skill_md()
        );
        assert_eq!(
            states(&env),
            [
                SkillState::Current,
                SkillState::Current,
                SkillState::Current
            ]
        );
    }

    #[test]
    fn detects_pi_in_its_configured_dir() {
        let home = TempHome::new("pi");
        let mut env = home.env();
        env.pi_agent_dir = Some(home.0.join("pi-config"));
        fs::create_dir_all(home.0.join(".pi").join("agent")).unwrap();
        assert!(env.detected_agents().is_empty());
        fs::create_dir_all(home.0.join("pi-config")).unwrap();
        assert_eq!(env.detected_agents(), [Agent::Pi]);
    }

    #[test]
    fn updates_an_outdated_skill() {
        let home = TempHome::new("update");
        let env = home.env();
        install_skill(&env, &[Agent::Codex]).unwrap();
        let path = home.skill(SkillLocation::Agents);
        let old = skill_md().replace(SKILL_VERSION, "0.0.1") + "\nOld text.\n";
        fs::write(&path, &old).unwrap();

        let status = &skill_status(&env, &[Agent::Codex]).unwrap()[0];
        assert_eq!(status.state, SkillState::Outdated);
        assert_eq!(status.installed_version.as_deref(), Some("0.0.1"));
        assert_eq!(outdated_skills(&env), std::slice::from_ref(&path));

        let changes = install_skill(&env, &[Agent::Codex]).unwrap();
        assert_eq!(
            actions(&changes),
            [(SkillLocation::Agents, SkillAction::Updated)]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), skill_md());
        assert!(outdated_skills(&env).is_empty());
    }

    #[test]
    fn updates_only_outdated_skills() {
        let home = TempHome::new("update-only");
        let env = home.env();
        home.agent(Agent::Claude).agent(Agent::Codex);
        assert!(
            update_outdated_skills(&env).unwrap().is_empty(),
            "nothing installed"
        );
        assert_eq!(
            states(&env),
            [
                SkillState::Missing,
                SkillState::Missing,
                SkillState::Missing
            ]
        );

        // Claude's is Solilog's but old; the shared folder's is someone else's.
        install_skill(&env, &[Agent::Claude]).unwrap();
        let claude = home.skill(SkillLocation::Claude);
        fs::write(&claude, old_skill()).unwrap();
        let foreign = home.skill(SkillLocation::Agents);
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        let mine = "---\nname: solilog\ndescription: mine\n---\n";
        fs::write(&foreign, mine).unwrap();

        let changes = update_outdated_skills(&env).unwrap();
        assert_eq!(
            actions(&changes),
            [(SkillLocation::Claude, SkillAction::Updated)]
        );
        assert_eq!(fs::read_to_string(&claude).unwrap(), skill_md());
        assert_eq!(fs::read_to_string(&foreign).unwrap(), mine);
        assert!(update_outdated_skills(&env).unwrap().is_empty(), "current");

        // A missing location stays missing.
        fs::remove_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&claude, old_skill()).unwrap();
        update_outdated_skills(&env).unwrap();
        assert_eq!(
            states(&env),
            [
                SkillState::Current,
                SkillState::Missing,
                SkillState::Missing
            ]
        );
    }

    #[test]
    fn leaves_a_skill_from_the_same_or_a_newer_version() {
        let home = TempHome::new("update-newer");
        let env = home.env();
        install_skill(&env, &[Agent::Claude]).unwrap();
        let path = home.skill(SkillLocation::Claude);
        for text in [
            // A development build with edited text.
            skill_md() + "\nEdited.\n",
            skill_md().replace(SKILL_VERSION, "999.0.0"),
        ] {
            fs::write(&path, &text).unwrap();
            assert_eq!(states(&env)[0], SkillState::Outdated);
            assert!(update_outdated_skills(&env).unwrap().is_empty());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn compares_versions() {
        assert!(version_older("0.1.0", "0.2.0"));
        assert!(version_older("0.9.0", "0.10.0"));
        assert!(version_older("garbage", "0.1.0"));
        assert!(!version_older("0.2.0", "0.2.0"));
        assert!(!version_older("1.0.0", "0.9.9"));
        assert!(!version_older("0.2.0-dev", "0.2.0"));
    }

    /// Solilog's skill as an older version installed it.
    fn old_skill() -> String {
        skill_md().replace(&format!("\"{SKILL_VERSION}\""), "\"0.0.1\"") + "\nOld text.\n"
    }

    #[cfg(unix)]
    #[test]
    fn updates_a_linked_skill_once() {
        let home = TempHome::new("update-link");
        let env = home.env();
        install_skill(&env, &[Agent::Codex]).unwrap();
        let link = env.skill_dir(SkillLocation::Claude);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(env.skill_dir(SkillLocation::Agents), &link).unwrap();
        fs::write(home.skill(SkillLocation::Agents), old_skill()).unwrap();

        let changes = update_outdated_skills(&env).unwrap();
        assert_eq!(
            actions(&changes),
            [(SkillLocation::Claude, SkillAction::Updated)]
        );
        assert_eq!(
            states(&env),
            [
                SkillState::Current,
                SkillState::Current,
                SkillState::Missing
            ]
        );
    }

    #[test]
    fn never_overwrites_a_foreign_skill() {
        let home = TempHome::new("foreign");
        let env = home.env();
        let foreign = home.skill(SkillLocation::Agents);
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&foreign, "---\nname: solilog\ndescription: mine\n---\n").unwrap();

        let err = install_skill(&env, &[Agent::Claude, Agent::Pi]).unwrap_err();
        assert!(err.to_string().contains("not written by Solilog"), "{err}");
        // All or nothing: Claude's copy was not written either.
        assert!(!home.skill(SkillLocation::Claude).exists());
        assert_eq!(
            states(&env),
            [
                SkillState::Missing,
                SkillState::Foreign,
                SkillState::Missing
            ]
        );

        // Uninstalling leaves it alone too.
        let changes = uninstall_skill(&env, &[]).unwrap();
        assert_eq!(
            actions(&changes),
            [
                (SkillLocation::Claude, SkillAction::NotInstalled),
                (SkillLocation::Agents, SkillAction::Foreign),
                (SkillLocation::Antigravity, SkillAction::NotInstalled),
            ]
        );
        assert!(foreign.exists());
    }

    #[test]
    fn a_folder_with_other_files_is_foreign() {
        let home = TempHome::new("other-files");
        let env = home.env();
        let dir = env.skill_dir(SkillLocation::Claude);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(states(&env)[0], SkillState::Missing, "empty folder");
        fs::write(dir.join("notes.md"), "mine").unwrap();
        assert_eq!(states(&env)[0], SkillState::Foreign);
        assert!(install_skill(&env, &[Agent::Claude]).is_err());
    }

    #[test]
    fn uninstalls_only_its_own_files() {
        let home = TempHome::new("uninstall");
        let env = home.env();
        install_skill(&env, &[Agent::Claude, Agent::Opencode]).unwrap();
        let extra = env.skill_dir(SkillLocation::Agents).join("notes.md");
        fs::write(&extra, "mine").unwrap();

        let changes = uninstall_skill(&env, &[Agent::Claude]).unwrap();
        assert_eq!(
            actions(&changes),
            [(SkillLocation::Claude, SkillAction::Removed)]
        );
        assert!(!env.skill_dir(SkillLocation::Claude).exists());
        assert!(home.skill(SkillLocation::Agents).exists());

        let changes = uninstall_skill(&env, &[]).unwrap();
        assert_eq!(
            actions(&changes),
            [
                (SkillLocation::Claude, SkillAction::NotInstalled),
                (SkillLocation::Agents, SkillAction::Removed),
                (SkillLocation::Antigravity, SkillAction::NotInstalled),
            ]
        );
        assert_eq!(changes[1].kept, std::slice::from_ref(&extra));
        assert!(extra.exists());
        assert!(!home.skill(SkillLocation::Agents).exists());
    }

    #[cfg(unix)]
    #[test]
    fn handles_a_linked_skill_folder() {
        let home = TempHome::new("link");
        let env = home.env();
        install_skill(&env, &[Agent::Codex]).unwrap();
        // As some skill managers do: ~/.claude/skills/x -> ~/.agents/skills/x.
        let link = env.skill_dir(SkillLocation::Claude);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(env.skill_dir(SkillLocation::Agents), &link).unwrap();

        let changes = install_skill(&env, &[Agent::Claude, Agent::Codex]).unwrap();
        assert_eq!(
            actions(&changes),
            [
                (SkillLocation::Claude, SkillAction::Unchanged),
                (SkillLocation::Agents, SkillAction::Unchanged),
            ]
        );

        uninstall_skill(&env, &[Agent::Claude]).unwrap();
        assert!(!is_symlink(&link));
        assert!(home.skill(SkillLocation::Agents).exists(), "target kept");
    }
}
