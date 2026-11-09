# session-helper

A Claude Code mod that keeps a plain-text copy of every session, groups recent sessions into bodies of work, and tells each new conversation where to find them. Claude reads past work with `rg`, `sed` and `tail` instead of parsing raw jsonl.

## Hooks

| Hook | Fires | What the mod does |
| --- | --- | --- |
| `session.start` | A session begins | Sets the `$CC_*` env vars. Interactive sessions also register `/areas` and run a refresh in the background: sync all transcripts, regroup if the last grouping is over an hour old, rank. |
| `prompt.submit` | Each prompt you send | Follows the session id. After a `/clear`, repoints `$CC_*`, sets `$CC_PREV_SESSION` and records the lineage. |
| `prompt.context` | Once per conversation, when the first message's context blocks are built; again after a compaction or `/clear` | Adds `bodiesOfWork`, plus `previousSession` after a `/clear`. Interactive sessions only. |
| `turn.complete` | A main-loop turn ends (not a subagent's) | Syncs this session's text copy and refs. |
| `session.compact` | `/compact` or auto-compaction | Nothing beyond noting it, so `previousSession` isn't sent again; the engine compacts as usual. |
| `session.end` | A session ends, including by `/clear` | Remembers a cleared session's id for the next conversation. |
| `command.run` | `/areas` | Runs the refresh with a forced regroup. |

Headless runs (`-p`, SDK scripts) get the env vars and transcript syncs only. The desktop app and VS Code count as interactive.

## What Claude sees

- **`bodiesOfWork`** context block, one line per body of work: `- On-call week 12%: handoffs, tickets, runbooks moved into the repo`, plus a pointer to `$CC_VIEWS/areas.md`.
- **`previousSession`** context block, after a `/clear`: the cleared session's id, length and body of work, its last three asks by line, and its top references.
- **Env vars** in every Bash call: `$CC_HOME`, `$CC_SESSIONS`, `$CC_INDEX`, `$CC_VIEWS`, `$CC_SESSION`, and `$CC_PREV_SESSION` after a `/clear`.
- **`/areas`** slash command.

Nothing goes into CLAUDE.md or the system prompt; the section you paste in (see Install) is what tells Claude when to read the files.

## What it runs

| Command | Called from | Does |
| --- | --- | --- |
| `cc-text sync [ID …]` | refresh (all), `turn.complete` (this session) | Converts new transcript lines to text from a saved offset, and records references and per-day attention. |
| `cc-text areas-input` → Sonnet | refresh, at most hourly; `/areas` | One `$.model.complete` call that groups the last 21 days of interactive sessions into 4–8 bodies of work, keeping earlier names where they fit. |
| `cc-text areas` | refresh | Ranks bodies of work by typing, active time and output (4-day half-life) and folds those under 4% into Misc. No model call. |
| `cc-text previous ID` | `prompt.context` after a `/clear` | Prints what the `previousSession` block shows. |
| `cc-text link CHILD PARENT` | `prompt.submit` after a `/clear` | Records the lineage. |

`cc-text` is Python 3 with no dependencies; run it with no arguments for usage.

## Files

Under `$CC_HOME` (default `~/.claude/session-helper`):

| Path | Env var | Holds |
| --- | --- | --- |
| `sessions/<id>.txt` | `$CC_SESSION` (this one), `$CC_SESSIONS` (the folder) | One copy per session. Each line starts with its role (`you:`, `claude:`, `tool:`, `result:`, `note:`); line numbers never change. |
| `sessions/<id>.refs.tsv` | | Every Slack thread, PR, Linear issue, Notion page, Granola meeting, artifact and file the session used: line, compaction, op, kind, key, title. Keys are normalized, so `rg TRA-2448 $CC_SESSIONS/*.refs.tsv` finds every session that touched one. |
| `sessions/<id>.meta` | | Sync offset and per-day attention. |
| `index.tsv` | `$CC_INDEX` | One row per session, newest first. |
| `lineage.tsv` | | child, parent; one row per `/clear`. |
| `views/areas.md` | `$CC_VIEWS` | Bodies of work ranked, each with its sessions newest first. `areas.json` (Sonnet's grouping) and `areas.tsv` sit beside it; `views/` is safe to delete. |

## Install

1. Load the mod in every session, in `~/.claude/settings.json`:

   ```json
   "env": { "CLAUDE_CODE_PLUGIN_DIRS": "/path/to/this/repo/session-helper" }
   ```

2. Tell Claude when to use it. The mod only puts the files in place, and a section like this in `~/.claude/CLAUDE.md` is what makes Claude reach for them:

   ```markdown
   # Efficiently navigating session history

   The `session-helper` mod keeps plain-text copies of every session.
   Use these to navigate session history and gather context, not `~/.claude/projects/*.jsonl`.

   Where things are:
   - `$CC_INDEX`: one row per session, with a header.
   - `$CC_VIEWS/areas.md`: recent bodies of work, ranked by attention, sessions newest first.
   - `$CC_SESSION`: this session's transcript.
   - `$CC_SESSIONS/<id>.txt`: any session's transcript.
   - `$CC_SESSIONS/<id>.refs.tsv`: the Slack threads, PRs, Linear issues, Notion pages, meetings, artifacts and files a session used.
   - `$CC_PREV_SESSION`: the session a `/clear` just ended.

   When to use them:
   - After a compaction, check details in `$CC_SESSION` before trusting the summary: names, ids, numbers, decisions, my wording.
   - After `/clear`, if my ask continues the old session, start with `tail -n 200 "$CC_PREV_SESSION"`.
   - When a task continues earlier work, search that area's sessions before asking me for context.
   - Skip all of this for self-contained tasks.

   How to read them:
   - Every line starts with its role: `you:`, `claude:`, `tool:`, `result:`, `note:`.
   - My asks start `you: [MM-DD HH:MM]`. `rg -n '^you: \[' "$CC_SESSION"` lists them.
   - Line numbers never change.
   - Tool calls and results are cut to one line. Use the raw jsonl only for full tool output.
   - refs.tsv rows: line, compaction, op, kind, key, title.
   - Keys: Slack `channel/ts`, `owner/repo#N`, Linear id, Notion page id, Granola meeting id, artifact id, file path.
   - To find where something came up: `rg <key or title> $CC_SESSIONS/*.refs.tsv`, then read around that line in the `.txt`.
   ```

The first interactive session syncs every existing transcript, which takes a while on a large history; after that each sync is incremental.

## Known gaps

- Edits made through Bash (`sed -i`, here-docs, scripts) count as reads, not edits. Only Edit, Write and NotebookEdit register as edits.
- A new session isn't in any body of work until the next regroup, up to an hour later.
- Repo-less `gh pr view 248` folds into the full PR key only when the session also has that PR's URL; otherwise it shows as `#248`.
