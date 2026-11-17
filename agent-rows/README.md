# agent-rows

A Claude Code mod that draws the subagent rows under the prompt. The stock rows leave out which model and effort each agent runs on; these show them:

```
◯ Explore H5.5 lo · Reading src/parse.rs                         3m 12s · ↓ 12.3k tokens
◯ fixer (claude) ⎇ wt1 hi · ✓ Patched the parser                           ↓ 48.1k tokens
```

- **Name and type.** `general-purpose` is left out.
- **⎇ worktree.** Shown when the agent works in a directory of its own.
- **Model.** Shown only when it differs from the session's.
- **Effort.** What the agent's last request actually sent, so a model's cap or lack of effort support shows. Before the first request, the effort the agent was asked to run at.
- **Status.** ✓ done, ✗ failed (red) or stopped (yellow). Elapsed time ticks while the agent runs.

It replaces the `subagentStatusLine` setting, which Claude Code removed in 2.1.323 in favour of mods. It hooks `ui.render` for `AgentRow` and sets the row's `body`.

## Install

Load it in every session, in `~/.claude/settings.json`. Separate several folders with `:`:

```json
"env": { "CLAUDE_CODE_PLUGIN_DIRS": "/path/to/claude-statusline/agent-rows" }
```

Or for one session: `claude --plugin-dir /path/to/claude-statusline/agent-rows`.

## Develop

`hooks/row.ts` builds a row from the agent's info; `hooks/register.ts` hooks it in. `claude plugin test agent-rows` runs `hooks/row.test.ts`; `claude plugin validate agent-rows` checks the module.
