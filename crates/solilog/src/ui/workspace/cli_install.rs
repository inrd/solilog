//! `workspace::InstallCommandLineTool` (macOS): links the `solilog` command
//! into /usr/local/bin and installs the agent skill, for people who
//! downloaded the app rather than ran `make install`. The logic is in
//! `solilog_core`'s `cli_link.rs` and `skill.rs`; this runs it, asks for
//! administrator rights when needed, and reports what happened.

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use gpui::{Context, Window};
use solilog_core::{
    Agent, CLI_LINK_PATH, CliLinkState, NOT_A_LINK_EXIT, SkillAction, SkillChange, SkillEnv,
    admin_script, app_bundle_binary, applescript_string, cli_link_state, install_cli_link,
    install_skill,
};

use super::Workspace;
use crate::ui::actions::workspace::InstallCommandLineTool;

impl Workspace {
    pub(super) fn on_install_command_line_tool(
        &mut self,
        _: &InstallCommandLineTool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        self.notice = Some("Installing the command line tool…".into());
        cx.notify();
        // In the background: the password prompt blocks until answered.
        let done = cx.background_executor().spawn(async { install() });
        cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |this, cx| {
                this.notice = this.report(done, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Links the command, then installs the skill; returns what to tell the user.
fn install() -> Result<String> {
    let exe = std::env::current_exe().context("finding Solilog's binary")?;
    let target = app_bundle_binary(&exe)?;
    let link = Path::new(CLI_LINK_PATH);
    let linked = match cli_link_state(link, &target)? {
        CliLinkState::Current => format!("`solilog` was already installed ({CLI_LINK_PATH})"),
        CliLinkState::NotALink => bail!(
            "{CLI_LINK_PATH} exists and isn't a link to Solilog: remove it (or move it away) and try again"
        ),
        CliLinkState::Missing => {
            make_link(link, &target)?;
            format!("Installed `solilog` ({CLI_LINK_PATH})")
        }
        CliLinkState::Elsewhere(old) => {
            make_link(link, &target)?;
            format!(
                "Installed `solilog` ({CLI_LINK_PATH}, which pointed to {})",
                old.display()
            )
        }
    };
    let skill = match SkillEnv::from_env().and_then(|env| {
        let detected = env.detected_agents();
        if detected.is_empty() {
            Ok(None)
        } else {
            Ok(Some((install_skill(&env, &detected)?, detected)))
        }
    }) {
        Ok(Some((changes, detected))) => describe_skill(&changes, &detected),
        Ok(None) => "no coding agent found for the skill".into(),
        Err(err) => format!("skill not installed: {err:#}"),
    };
    Ok(format!("{linked}; {skill}"))
}

/// Makes the link, asking for administrator rights if the folder needs them.
fn make_link(link: &Path, target: &Path) -> Result<()> {
    match install_cli_link(link, target) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {}
        Err(err) => return Err(err).with_context(|| format!("linking {}", link.display())),
    }
    let script = format!(
        "do shell script {} with administrator privileges",
        applescript_string(&admin_script(link, target))
    );
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .context("running osascript")?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    // osascript reports the script's exit code as "(<code>)" at the end.
    if stderr.contains("(-128)") {
        bail!("Cancelled: the command line tool wasn't installed");
    }
    if stderr.trim_end().ends_with(&format!("({NOT_A_LINK_EXIT})")) {
        bail!("{} isn't a link; not replacing it", link.display());
    }
    bail!("couldn't link {}: {}", link.display(), stderr.trim())
}

/// "skill installed for claude, updated for codex, cursor".
fn describe_skill(changes: &[SkillChange], detected: &[Agent]) -> String {
    let mut parts = Vec::new();
    for (action, word) in [
        (SkillAction::Installed, "installed"),
        (SkillAction::Updated, "updated"),
        (SkillAction::Unchanged, "up to date"),
    ] {
        let agents: Vec<_> = changes
            .iter()
            .filter(|c| c.action == action)
            .flat_map(|c| {
                c.agents
                    .iter()
                    .filter(|a| detected.contains(a))
                    .map(|a| a.key())
            })
            .collect();
        if !agents.is_empty() {
            parts.push(format!("{word} for {}", agents.join(", ")));
        }
    }
    format!("skill {}", parts.join(", "))
}
