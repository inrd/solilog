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
   description, and its `notes`; use `issue show` when you only want the
   issue). Read the notes: they hold what earlier sessions tried and
   decided, and steering from the user. If `waiting_on` isn't empty,
   issues it depends on are still open: tell the user before starting.
2. When you start: `solilog issue set-status SOL-12 in_progress`
   (`solilog statuses` lists the keys).
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

To triage or suggest what to work on next, start from
`solilog issue ls --ready --compact --json` (or `solilog issue ls --compact --json`
for the full backlog): lightweight summaries without descriptions or full
checklists, then drill into the few issues you need with `issue context`.
`--ready` leaves out issues in review: they wait for the user, not for you.

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
