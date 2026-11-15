//! On start, brings the agent skill up to date where it was installed by an
//! older version (`solilog_core::update_outdated_skills`): a new app makes
//! the command line tool current at once (it links into the app), and this
//! does the same for the skill. No prompt: installing it was the user's
//! choice; missing and foreign skills are left alone.

use anyhow::Result;
use gpui::{Context, Task};
use solilog_core::{SKILL_VERSION, SkillChange, SkillEnv, update_outdated_skills};

use super::Workspace;

impl Workspace {
    pub(super) fn update_skills(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let updated = cx.background_executor().spawn(async { update() }).await;
            this.update(cx, |this, cx| {
                let updated = this.report(
                    updated.map_err(|err| err.context("couldn't update the agent skill")),
                    cx,
                );
                if let Some(Some(notice)) = updated {
                    this.notice = Some(notice);
                    cx.notify();
                }
            })
            .ok();
        })
    }
}

/// Updates the outdated skills; returns what to tell the user, if anything.
fn update() -> Result<Option<String>> {
    let env = SkillEnv::from_env()?;
    let changes = update_outdated_skills(&env)?;
    if changes.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Updated the agent skill to {SKILL_VERSION} for {}",
        agents(&changes, &env)
    )))
}

/// The agents that read the updated skills: the installed ones, or all of
/// them if none is (the skill was put there by hand).
fn agents(changes: &[SkillChange], env: &SkillEnv) -> String {
    let all: Vec<_> = changes.iter().flat_map(|c| c.agents.clone()).collect();
    let detected = env.detected_agents();
    let installed: Vec<_> = all.iter().filter(|a| detected.contains(a)).collect();
    let shown = if installed.is_empty() {
        all.iter().collect()
    } else {
        installed
    };
    shown.iter().map(|a| a.key()).collect::<Vec<_>>().join(", ")
}
