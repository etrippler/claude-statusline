# claude-statusline

Tools for Claude Code, one folder each:

- [`statusline/`](statusline/): main session line and subagent rows in one Rust binary, ~5ms a render.
- [`session-helper/`](session-helper/): a mod that keeps plain-text copies of every session, ranks your bodies of work, and points a `/clear` at the session it ended.
