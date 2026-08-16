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

## Developing

Clone the repo and run `install` from the checkout — the working tree is then authoritative. Publish with `git push`.

The branch glyph is Powerline (U+E0A0); use a Powerline-capable font.
