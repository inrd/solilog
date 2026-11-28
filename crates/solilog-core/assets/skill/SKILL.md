---
name: solilog
description: >-
  Read and update the user's issues in Solilog, their local issue tracker,
  with the `solilog` command. Use whenever the user mentions an issue id like
  SOL-12, or their issues, backlog, tasks, todos or what to work on next, from
  any project; also to record progress, tick checklist items, file new issues
  or change an issue's status.
metadata:
  managed-by: solilog
  version: "@VERSION@"
---

# Solilog

Solilog is the user's offline, single-user issue tracker: a desktop app and
the `solilog` command over one local database. Use the command; add `--json`
for stable, machine-readable output. Issue ids look like `SOL-12`; commands
accept `SOL-12` or `12`.

This file covers the workflows. For commands and flags, run
`solilog --help` and `solilog <command> --help`.

## Working on an issue

1. Read it first: `solilog issue context SOL-12 --json` (the issue with its
   projects, dependencies, any `SOL-n` issues mentioned in the
   description, the issues that mention it (`mentioned_by`), and its
   `notes`; use `issue show` when you only want the
   issue). Read its `decisions` first: settled points that apply to the
   issue (its own, its projects' and its labels'); follow them, and don't
   reopen one without asking the user. Then the notes: they hold what
   earlier sessions tried, and steering from the user. If `waiting_on` isn't empty,
   issues it depends on are still open: tell the user before starting.
2. When you start: `solilog issue set-status SOL-12 in_progress`
   (`solilog statuses` lists the keys). Link the issue to where the work
   lives as soon as each exists: `solilog link add SOL-12 <branch>`, then
   `solilog link add SOL-12 <PR URL>` (and other pages worth keeping, with
   `-t "<title>"`). The next session starts from them.
3. Tick checklist items as you finish them: `solilog checklist check <id>`.
   Item ids are global numbers from `checklist[].id`, not positions: the
   third item of SOL-12 may be 41.
4. Log your work as notes: `solilog note add SOL-12 "<text>"` (`-` reads a
   longer note from stdin). Add one when you learn something the next
   session needs (what you tried and what failed, a decision and why,
   files or commits touched) and a short handoff before you stop: done,
   left, next step. Notes are the place for this, not the description:
   leave the description (the user's spec) alone unless asked to change it.
   Notes can't be edited; add a new one to correct one.
5. Keep the status true (`blocked` if you are stuck).
6. When the work is finished, don't set `done`: hand it to the user for
   review with a receipt, then tell them in your session and move on:
   `solilog issue request-review SOL-12 -m "<what was done>" --pass "<command>"
   --fail "<command: what failed>" --commit <sha> --file <path>`
   (each flag repeatable). List the checks you actually ran, failures
   included, and the commits and files you changed. The issue goes to
   `in_review`; the user approves it (`done`) or sends it back to `todo`
   with a note saying what to change: read the latest notes before picking
   it up again, and hand it off again when fixed. Approve or set `done`
   yourself only when the user tells you to, and ask before `cancelled`.

## The codebase you are in

Labels or projects can be mapped to the directories where their code
lives. In a repository, run `solilog where --json` first: it names the
label or project the current directory belongs to (`kind`, `name`), with
the `decisions` that apply there: read and follow them. Then
`solilog issue ls --here` lists its issues and `solilog issue create
--here` files new ones under it. If `where` fails, nothing is mapped
there: ask the user which label or project the repository belongs to and
offer to map it (`solilog label add-path <label> .` or `solilog project
add-path <project> .`) rather than guessing. Picking an issue from another
codebase, `solilog where --all` tells you where it lives.

To triage or suggest what to work on next, start from
`solilog issue ls --ready --compact --json` (or `solilog issue ls --compact --json`
for the full backlog): lightweight summaries without descriptions or full
checklists, then drill into the few issues you need with `issue context`.
`--ready` leaves out issues in review: they wait for the user, not for you.

## Decisions

When the user settles something that later sessions should know (an API
to keep, a library to avoid, a convention), record it where it applies:
`solilog decision add --issue SOL-12 "<decision>"`, `--label <label>` or
`--project <project>` for wider ones (a label's apply to its sublabels),
or `--here` for this codebase's label or project. Record what the user
decided, in a sentence or two, not your own choices or plans (those go in
notes). When the user changes their mind, `solilog decision replace <id>
"<new decision>"` or `solilog decision withdraw <id>`. Decisions you record
or withdraw wait for the user's review but apply at once; never confirm
them yourself (`decision review` is for the user).

## Searching and catching up

- Before filing an issue or starting on a bug, look for earlier work:
  `solilog search "<words>" --include-closed --json` searches every
  issue's title, description, checklist and notes, done and archived ones
  included, across all codebases. Past decisions, failed attempts and
  fixes are often there; read the hits that matter with `issue context`.
- When resuming after a while (a scheduled run, a new session on old
  work), see what changed instead of re-reading everything:
  `solilog issue activity --since <time> --json` (a Unix time, a date, or
  an age like `2h`; add `--by user` for the user's changes, `--here` for
  this codebase), or `solilog issue ls --since <time> --compact --json`
  for the issues that changed. Note the time (`date +%s`) when you start,
  to use next time.

## Writing

- Identify yourself on every write with `--actor <agent>:<task>` or
  `SOLILOG_ACTOR=<agent>:<task>` (e.g. `claude:fix-auth`, `codex:triage`).
  Unidentified CLI writes are recorded as `cli` and flagged for review.
- New issue: `solilog issue create "Title" -d - -l <label> -i "<item>"`
  (`-` reads text from stdin). Every write prints the resulting issue.
- Several changes to one issue go in one `solilog issue edit` (text,
  `--status`, labels, projects, dependencies; all or nothing) rather than
  one command each.
- Writing `SOL-n` in a description links to that issue.
- When an issue needs another finished first, record it:
  `solilog issue add-dependency SOL-12 SOL-7` (SOL-12 depends on SOL-7),
  rather than only saying so in the description. Nothing changes a status
  by itself.
- Labels are paths (`acme/reverb`); filtering by `acme` also matches its
  sublabels. Look at `solilog label ls` and `solilog project ls` before
  inventing new ones.
- Deleting an issue is permanent; prefer `issue archive`.
- Overwriting a title, description or checklist item keeps the old text:
  if you replaced or deleted something by mistake, find it with
  `solilog issue history SOL-12` and put it back with
  `solilog issue restore SOL-12 <revision>`. Tell the user when you do.

## Safety

- Backups are automatic (`solilog backup --help`).
- To experiment, point at a scratch database:
  `SOLILOG_DB=/tmp/scratch.db solilog …`.
