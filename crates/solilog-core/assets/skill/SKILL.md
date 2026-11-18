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

1. Read it first: `solilog issue show SOL-12 --json` (description, status,
   labels, projects, checklist, attachments with their file paths,
   dependencies). If `waiting_on` isn't empty, issues it depends on are
   still open: tell the user before starting.
2. When you start: `solilog issue set-status SOL-12 in_progress`
   (`solilog statuses` lists the keys).
3. Tick checklist items as you finish them: `solilog checklist check <id>`.
   Item ids are global numbers from `checklist[].id`, not positions: the
   third item of SOL-12 may be 41.
4. Keep the status true (`blocked` if you are stuck), but ask the user
   before setting `done` or `cancelled`.

To suggest what to work on next, start from `solilog issue ls --ready`:
open issues with nothing open to wait for.

## Writing

- New issue: `solilog issue create "Title" -d - -l <label> -i "<item>"`
  (`-` reads text from stdin). Every write prints the resulting issue.
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
