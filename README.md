# claude-statusline

Claude Code status lines — main session line + subagent rows.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/etrippler/claude-statusline/main/statusline.py -o ~/.claude/statusline.py
python3 ~/.claude/statusline.py install
```

- `install` points `statusLine` and `subagentStatusLine` in `~/.claude/settings.json` at the file. 
- `install PATH` targets another settings file instead - e.g. `project/.claude/settings.local.json`.
- `install --no-update` disables background updates.

## Configure

```sh
python3 ~/.claude/statusline.py toggles      # list toggles and their state
python3 ~/.claude/statusline.py off cost     # hide a segment
python3 ~/.claude/statusline.py on cost
```

- See `~/.claude/statusline.json`, configure with `$CLAUDE_STATUSLINE_CONFIG` 

## Updating

- Renders spawn a background version check at most hourly; replaces itself atomically. 
- `update` fetches immediately. `off update` disables the background check; manual `update` still works.
- A copy inside a git checkout never self-updates.

## Rust build

`src/` is a Rust port with the same segments, commands and config. A render takes ~5ms vs ~70ms for the Python version (plus your interpreter's startup), which makes a 1s `refreshInterval` cheap:

- Repo, worktree, subdir and branch come from reading `.git` directly; upstream ahead/behind costs one `git for-each-ref`, cached until a ref file changes. A steady-state render spawns no processes.
- The diff size and PR CI status refresh in detached workers, as before. Its cache and session files are separate from the Python version's (`/tmp/claude-statusline-rs-*`).

```sh
curl -fsSL https://github.com/etrippler/claude-statusline/releases/download/latest/statusline-macos-aarch64 -o ~/.claude/statusline
chmod +x ~/.claude/statusline && ~/.claude/statusline install
```

Assets: `statusline-{macos,linux}-{aarch64,x86_64}`. A push to `main` that touches the Rust build re-publishes them on the rolling `latest` release (`.github/workflows/release.yml`), and installed binaries pick that up through `update`. From a checkout: `cargo build --release && target/release/statusline install` (a binary inside the checkout never self-updates).

## Developing

Clone the repo and run `install` from the checkout — the working tree is then authoritative. Publish with `git push`.

The branch glyph is Powerline (U+E0A0); use a Powerline-capable font.
