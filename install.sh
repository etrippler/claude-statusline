#!/bin/sh
# Install the statusline binary and point Claude Code at it:
#   curl -fsSL https://raw.githubusercontent.com/etrippler/claude-statusline/main/install.sh | sh
# Arguments pass through to `statusline install` (e.g. a project's .claude/settings.json):
#   curl -fsSL …/install.sh | sh -s -- path/to/.claude/settings.json
set -eu

dir="${STATUSLINE_DIR:-$HOME/.claude}"
case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;; esac
case "$(uname -m)" in arm64 | aarch64) arch=aarch64 ;; x86_64 | amd64) arch=x86_64 ;; *) echo "unsupported CPU: $(uname -m)" >&2; exit 1 ;; esac

mkdir -p "$dir"
curl -fsSL "https://github.com/etrippler/claude-statusline/releases/download/latest/statusline-$os-$arch" -o "$dir/statusline.new"
chmod +x "$dir/statusline.new"
mv "$dir/statusline.new" "$dir/statusline"
"$dir/statusline" install "$@"
