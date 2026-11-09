# claude-statusline

Claude Code status lines — main session line + subagent rows — in one Rust binary. A render takes ~5ms and usually spawns no processes, so a 1s `refreshInterval` is cheap.

## Install

From a clone (needs a Rust toolchain):

```sh
git clone https://github.com/etrippler/claude-statusline && cd claude-statusline
cargo build --release && target/release/statusline install
```

Or a prebuilt binary, put at `~/.claude/statusline` (`$STATUSLINE_DIR` to relocate):

```sh
curl -fsSL https://raw.githubusercontent.com/etrippler/claude-statusline/main/statusline/install.sh | sh
```

- `install` points `statusLine` and `subagentStatusLine` in `~/.claude/settings.json` at the binary.
- `install PATH` targets another settings file instead — e.g. `project/.claude/settings.local.json`. Through the script: `sh -s -- PATH`.

## Configure

```sh
statusline toggles      # list segments and their state
statusline off cost     # hide a segment
statusline on cost
```

Toggles live in `~/.claude/statusline.json`; `$CLAUDE_STATUSLINE_CONFIG` relocates it.

## Update

Nothing updates on its own. `statusline update` does it when you ask: in a clone, `git pull --ff-only && cargo build --release`; otherwise it downloads the latest release over itself. Releases are built by CI from every push to `main` that touches the Rust code (`.github/workflows/release.yml`), as `statusline-{macos,linux}-{aarch64,x86_64}` on the rolling `latest` release.

## How it stays fast

- Repo, worktree, subdir and branch are read straight from `.git`. Upstream ahead/behind costs one `git for-each-ref`, cached until a ref file changes.
- The diff size and the PR's CI status refresh in detached workers. The diff refreshes at ~20× its own cost (1–60s), so it tracks edits within a second on a typical repo.
- On macOS, git calls skip Apple's `/usr/bin/git` xcrun shim (~10ms each).
- State lives in `/tmp/claude-statusline-rs-*`, written atomically.

The branch glyph is Powerline (U+E0A0); use a Powerline-capable font.

`archive/statusline.py` is the original Python version, kept for reference.
