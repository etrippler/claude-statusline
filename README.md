# claude-statusline

Tools for Claude Code, one folder each:

- [`statusline/`](statusline/): the main status line in one Rust binary, ~5ms a render.
- [`agent-rows/`](agent-rows/): a mod that draws the subagent rows under the prompt with each agent's type, model and effort.
- [`session-helper/`](session-helper/): a mod that keeps plain-text copies of every session, ranks your bodies of work, and points a `/clear` at the session it ended.
