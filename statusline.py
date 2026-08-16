#!/usr/bin/env python3
"""Claude Code status lines — main session line + subagent rows — in one
self-updating, stdlib-only file. `statusline.py --help` for commands.

The branch glyph is Powerline (U+E0A0): use a Powerline-capable font.
"""

import json, os, re, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

RAW = "https://raw.githubusercontent.com/etrippler/claude-statusline/main/statusline.py"
SELF = Path(__file__).resolve()
CLAUDE = Path.home() / ".claude"
CONF = Path(os.environ.get("CLAUDE_STATUSLINE_CONFIG") or CLAUDE / "statusline.json").expanduser()
CACHE = Path("/tmp/claude-statusline-cache.json")
STAMP = Path("/tmp/claude-statusline-update-stamp")

SEGMENTS = "repo branch diff ci model context percent cost limits version".split()
DEFAULTS = dict.fromkeys(SEGMENTS + ["update"], True)  # "update" gates self-update, not a segment

# A statusline must never take the index lock: `git diff` opportunistically
# rewrites the index, and sessions contending on that lock turn a slow render
# into a stuck one.
GIT_ENV = {"GIT_OPTIONAL_LOCKS": "0"}

def C(c): return lambda s: f"\x1b[{c}m{s}\x1b[0m"
dim, red, green, yellow, blue, magenta, cyan, white, bcyan = map(C, "2 31 32 33 34 35 36 37 1;36".split())
BOLD = "\x1b[1m"

def link(url, s): return f"\x1b]8;;{url}\a{s}\x1b]8;;\a"

def vw(s):  # width as the terminal renders it: SGR and OSC 8 are zero-width
    return len(re.sub(r"\x1b\[[0-9;]*m|\x1b]8;;[^\a]*\a", "", s))

def jread(p):  # config/cache files legitimately may not exist yet
    try:
        return json.loads(p.read_text())
    except Exception:
        return {}

def run(cmd, timeout, env=None):  # stdout, or None on any failure — git/gh may
    try:                          # be absent, unauthed, or hung (timeout kills)
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           stdin=subprocess.DEVNULL, env={**os.environ, **(env or {})})
        return p.stdout.strip() if p.returncode == 0 else None
    except Exception:
        return None

def git(cwd, *a): return run(["git", "-C", cwd, *a], 2, GIT_ENV)

def detach(*a):
    subprocess.Popen([sys.executable, str(SELF), *a], start_new_session=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

# ---------------------------------------------------------------- self-update

def in_checkout(): return (SELF.parent / ".git").exists()

def maybe_update():  # rides along on every render
    if in_checkout(): return  # dev copy: git is authoritative
    if STAMP.exists() and time.time() - STAMP.stat().st_mtime < 3600: return
    if not conf()["update"]: return  # `off update`: manual `update` only
    STAMP.touch()  # before spawning, so concurrent sessions stay single-flight
    detach("update")

def cmd_update():
    if in_checkout(): sys.exit("this copy lives in a git checkout — use git pull")
    from urllib.request import urlopen
    new = urlopen(RAW, timeout=10).read()
    STAMP.touch()
    if new == SELF.read_bytes(): return print("already up to date")
    compile(new, str(SELF), "exec")  # a file that can't parse never gets installed
    tmp = SELF.with_suffix(".new")
    tmp.write_bytes(new)
    tmp.replace(SELF)  # atomic: in-flight renders keep reading the old inode
    print(f"updated {SELF}")

# --------------------------------------------------------------------- config

def conf():  # config overrides > defaults
    return {**DEFAULTS, **jread(CONF)}

def cmd_toggle(seg, val):
    if seg not in DEFAULTS: sys.exit(f"unknown toggle {seg!r} — see: statusline.py toggles")
    o = jread(CONF)
    o.pop(seg, None)
    if DEFAULTS[seg] != val: o[seg] = val  # store only deviations: new defaults still propagate
    CONF.write_text(json.dumps(o) + "\n") if o else CONF.unlink(missing_ok=True)
    print(f"{seg}: {'on' if val else 'off'}")

def cmd_toggles():
    on, o = conf(), jread(CONF)
    for s in DEFAULTS:
        print(f"  {green('on ') if on[s] else red('off')} {s}" + (dim("  (override)") if s in o else ""))

# ----------------------------------------------------------------- formatters

def fmt_cost(c): return f"${c:.2f}" if c >= 0.01 else f"${c:.4f}" if c > 0 else "$0.00"

def fmt_effort(e, fallback=""):  # low|medium|high|xhigh|max, or a token budget
    e = e or fallback
    if not e: return ""
    if isinstance(e, (int, float)): return f"{round(e / 1000)}k"
    return "XHigh" if e == "xhigh" else e[0].upper() + e[1:]

def fmt_reset(epoch):
    return datetime.fromtimestamp(epoch).strftime("%-I:%M%p").replace(":00", "")  # 7:00PM → 7PM

def fmt_limits(rl):
    fh = (rl or {}).get("five_hour") or {}
    pct = fh.get("used_percentage") or 0
    if pct <= 0: return ""
    c = red if pct >= 80 else yellow if pct >= 50 else green
    reset = fh.get("resets_at")
    return c(f"{round(pct)}%") + (" " + dim(f"@ {fmt_reset(reset)}") if reset else "")

# ------------------------------------------------------------------------ git

def rcache():
    c = jread(CACHE)
    return {"pr": c.get("pr", {}), "diff": c.get("diff", {})}

def wcache(c): CACHE.write_text(json.dumps(c))

def git_info(cwd):
    cmds = [["rev-parse", "--path-format=absolute", "--git-common-dir"],
            ["rev-parse", "--show-toplevel"],
            ["branch", "--show-current"],
            ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            ["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            ["rev-parse", "--abbrev-ref", "origin/HEAD"]]
    with ThreadPoolExecutor(len(cmds)) as ex:
        common, top, branch, upstream, rev, ohead = ex.map(lambda a: git(cwd, *a), cmds)

    main = wt = sub = None
    if common and top:
        root = os.path.dirname(common)
        main, wt = os.path.basename(root), None if root == top else os.path.basename(top)
        rel = os.path.relpath(cwd, top)
        sub = None if rel == "." or rel.startswith("..") else rel
    ahead, behind = map(int, rev.split()) if rev else (0, 0)

    # The full-tree shortstat is the one expensive call on a big repo — never in
    # the render path. Serve the cached mass; stamp t before spawning so the
    # detached refresh stays single-flight across concurrent sessions.
    mass = None
    if top and branch:
        c = rcache()
        k = f"{top}:{branch}"
        e = c["diff"].get(k)
        if e: mass = (e["a"], e["r"])
        if not e or time.time() - e["t"] > 15:
            c["diff"][k] = {"a": e["a"] if e else 0, "r": e["r"] if e else 0, "t": time.time()}
            wcache(c)
            detach("--refresh-diff", cwd)

    return dict(main=main, wt=wt, sub=sub, branch=branch, mass=mass,
                default=ohead.removeprefix("origin/") if ohead else None,
                upstream=bool(upstream), ahead=ahead, behind=behind)

# ------------------------------------------------------------------------- pr

def fetch_pr(branch, slug):
    out = run(["gh", "pr", "view", branch, "--repo", slug, "--json", "url,statusCheckRollup"], 5)
    if not out: return None
    d = json.loads(out)
    # statusCheckRollup mixes CheckRun {status, conclusion} and StatusContext
    # {state} nodes; read both or commit-status failures pass silently.
    checks, bad = d.get("statusCheckRollup") or [], {"FAILURE", "ERROR"}
    ci = ("NONE" if not checks else
          "FAILURE" if any(c.get("conclusion") in bad or c.get("state") in bad for c in checks) else
          "PENDING" if any(c.get("status") in ("IN_PROGRESS", "PENDING", "QUEUED")
                           or c.get("state") in ("PENDING", "EXPECTED") for c in checks) else
          "SUCCESS")
    return {"ci": ci, "url": d.get("url", ""), "t": time.time()}

def ci_from_cc(url):
    # Claude Code persists its own GraphQL PR rollup keyed by URL — share it.
    ch = jread(CLAUDE / "gh-pr-status-cache.json").get(url, {}).get("checks")
    if not ch: return None
    return ("FAILURE" if ch.get("failed") else "PENDING" if ch.get("pending")
            else "SUCCESS" if ch.get("passed") else "NONE")

def fmt_ci(pr):
    if not pr or pr["ci"] == "NONE": return ""
    glyph, c = {"FAILURE": ("✗", red), "PENDING": ("●", yellow), "SUCCESS": ("✓", green)}[pr["ci"]]
    t = c(f"CI {glyph}")
    return link(pr["url"], t) if pr.get("url") else t

def ci_segment(inp, g, repo):
    if not (repo.get("owner") and g["branch"] and g["branch"] != g["default"] and g["upstream"]):
        return ""
    pru = (inp.get("pr") or {}).get("url")
    shared = ci_from_cc(pru) if pru else None
    if shared is not None:
        return fmt_ci({"ci": shared, "url": pru})
    # Claude Code reports no PR on non-token hosts and unseen branches — own the fetch.
    c = rcache()
    k = f"{repo['owner']}_{repo['name']}:{g['branch']}"
    pr = c["pr"].get(k)
    if not pr:  # first sight of this branch: fetch synchronously for instant feedback
        pr = c["pr"][k] = fetch_pr(g["branch"], f"{repo['owner']}/{repo['name']}") or {"ci": "NONE", "url": "", "t": time.time()}
        wcache(c)
    elif time.time() - pr["t"] > 30:  # stamp before spawning → single-flight
        c["pr"][k] = {**pr, "t": time.time()}
        wcache(c)
        detach("--refresh-pr", g["branch"], repo["owner"], repo["name"])
    return fmt_ci(pr)

# ------------------------------------------------------------------ main line

def render():
    on = conf()
    inp = json.load(sys.stdin)
    ws = inp.get("workspace") or {}
    cwd = ws.get("current_dir") or os.getcwd()
    repo = ws.get("repo") or {}
    g = git_info(cwd)
    url = (f"https://{repo['host']}/{repo['owner']}/{repo['name']}/tree/{g['branch']}"
           if repo.get("host") and g["branch"] else None)

    # When the branch just restates the worktree name, the worktree chip absorbs
    # it: no "on ⎇ branch", hyperlink moves to the chip.
    norm = lambda s: re.sub(r"^(worktree|wt)[-/]", "", s)
    redundant = g["branch"] and g["wt"] and norm(g["branch"]) == norm(g["wt"])
    parts = []
    if on["repo"] and g["main"]:
        s = bcyan(g["main"])
        if g["wt"]:
            w = cyan(f"\u2387 {g['wt']}")
            s += " " + (link(url, w) if redundant and url else w)
        if g["sub"]: s += cyan("/" + g["sub"])
        parts.append(s)
    if on["branch"] and g["branch"]:
        s = ""
        if not redundant:
            b = magenta(f"\ue0a0 {g['branch']}")
            s = (dim("on") + " " if parts else "") + (link(url, b) if url else b)
        marks = []  # quiet working-state markers; only behind/no-upstream go yellow
        if g["upstream"]:
            if g["ahead"]: marks.append(dim(f"↑{g['ahead']}"))
            if g["behind"]: marks.append(yellow(f"↓{g['behind']}"))
        else:
            marks.append(yellow("(local)"))
        if on["diff"] and g["mass"] and any(g["mass"]):
            marks.append(dim(f"Δ+{g['mass'][0]}/-{g['mass'][1]}"))
        parts.append(" ".join(x for x in [s, *marks] if x))
    left = " ".join(parts) or bcyan(os.path.basename(cwd) or "?")

    cw = inp.get("context_window") or {}
    u = cw.get("current_usage") or {}
    size = cw.get("context_window_size") or 200_000
    tok = sum(u.get(k) or 0 for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))

    right = []
    if on["ci"]:
        ci = ci_segment(inp, g, repo)
        if ci: right.append(ci)
    if on["model"]:
        m = (inp.get("model") or {}).get("display_name") or "Claude"
        m = re.sub("^Claude ", "", m).replace(" context)", ")")
        eff = fmt_effort((inp.get("effort") or {}).get("level"))
        s = blue(f"{m} {BOLD}{eff}" if eff else m)
        style = (inp.get("output_style") or {}).get("name") or ""
        if style and style.lower() != "default": s += " " + dim(f"~ {style}")
        right.append(s)
    if on["percent"]: right.append(yellow(f"{tok * 100 / size:.1f}%"))
    if on["context"] and tok: right.append(white(f"{tok / 1000:.1f}/{round(size / 1000)}"))
    if on["cost"]: right.append(green(fmt_cost((inp.get("cost") or {}).get("total_cost_usd") or 0)))

    sep = f" {dim('|')} "
    line = sep.join([left] + right)
    # Account-level meta right-justifies. Claude Code's statusline area is a few
    # cells narrower than COLUMNS; overshooting gets the tail truncated to "…".
    meta = sep.join(x for x in (fmt_limits(inp.get("rate_limits")) if on["limits"] else "",
                                dim("v" + inp["version"]) if on["version"] and inp.get("version") else "") if x)
    if meta:
        cols = int(c) if (c := os.environ.get("COLUMNS", "")).isdigit() else 0
        pad = cols - 4 - vw(line) - vw(meta) if cols else 0
        line += " " * pad + meta if pad > 0 else sep + meta
    print(line)

# -------------------------------------------------------------- subagent rows
# `statusline.py subagent` gets every visible subagent row as one JSON object on
# stdin and emits one {"id", "content"} line per row it overrides; skipped rows
# keep Claude Code's default rendering.

def fmt_model_id(mid):  # "claude-opus-4-8[1m]" → "Opus 4.8[1m]"
    ctx = (re.search(r"\[\d+m\]$", mid) or [""])[0]
    toks = [t for t in mid.removesuffix(ctx).removeprefix("claude-").split("-")
            if not re.fullmatch(r"\d{8}", t)]
    name = " ".join(t.capitalize() for t in toks if re.search("[a-z]", t, re.I))
    ver = ".".join(t for t in toks if t.isdigit())
    return (f"{name} {ver}" if ver else name) + ctx

def fmt_elapsed(ms):  # matches the built-in: 42s, 3m 12s, 1h 2m 3s, 1d 2h 3m
    ms = max(0, ms)
    if ms < 60_000: return f"{int(ms // 1000)}s"
    d, h, m = int(ms // 86_400_000), int(ms % 86_400_000 // 3_600_000), int(ms % 3_600_000 // 60_000)
    s = round(ms % 60_000 / 1000) % 60
    return f"{d}d {h}h {m}m" if d else f"{h}h {m}m {s}s" if h else f"{m}m {s}s"

def fmt_tokens(n):  # approximates the built-in compact en-US formatter
    for div, suf in ((1e9, "b"), (1e6, "m"), (1e3, "k")):
        if n >= div: return f"{n / div:.1f}{suf}"
    return str(n)

def build_row(t, fallback, cols, now):
    sep = " · "
    eff = fmt_effort(t.get("effort"), fallback)
    model = fmt_model_id(t.get("model") or "")
    left = sep.join(x for x in (t.get("name"), f"{model} {eff}" if eff else model) if x)
    right = sep.join(x for x in (fmt_elapsed(now - t["startTime"]) if t.get("startTime") else "",
                                 f"↓ {fmt_tokens(t['tokenCount'])} tokens" if t.get("tokenCount") else "") if x)
    desc = t.get("description")
    room = cols - len(left) - len(right) - 5 if cols else 9e9
    if desc and room > 8:
        left += sep + (desc[:int(room) - 1] + "…" if len(desc) > room else desc)
    pad = cols - len(left) - len(right) if cols else 0
    return dim(left + " " * pad + right if right and pad > 0 else sep.join(x for x in (left, right) if x))

def subagent():
    inp = json.load(sys.stdin)
    # The payload omits effort when a task inherits the session level, so fall
    # back to the persisted effortLevel (misses mid-session /effort changes).
    fb = jread(CLAUDE / "settings.json").get("effortLevel") or ""
    now = time.time() * 1000
    for t in inp.get("tasks") or []:
        if t.get("model"):  # model unresolved → keep the default row
            print(json.dumps({"id": t["id"], "content": build_row(t, fb, inp.get("columns"), now)}))

# --------------------------------------------------------- background workers

def refresh_pr(branch, owner, name):
    c = rcache()
    c["pr"][f"{owner}_{name}:{branch}"] = fetch_pr(branch, f"{owner}/{name}") or {"ci": "NONE", "url": "", "t": time.time()}
    wcache(c)

def refresh_diff(cwd):
    top, branch, base = (git(cwd, *a) for a in
                         (["rev-parse", "--show-toplevel"], ["branch", "--show-current"],
                          ["merge-base", "origin/HEAD", "HEAD"]))
    if not (top and branch): return
    # Working tree + branch commits vs the default-branch merge-base: the size
    # of the eventual PR, independent of session boundaries.
    st = (run(["git", "-C", cwd, "diff", "--shortstat", base], 30, GIT_ENV) or "") if base else ""
    n = lambda w: int((re.search(rf"(\d+) {w}", st) or [0, 0])[1])
    c = rcache()
    c["diff"][f"{top}:{branch}"] = {"a": n("insertion"), "r": n("deletion"), "t": time.time()}
    wcache(c)

# ----------------------------------------------------------------------- main

USAGE = f"""claude-statusline — Claude Code status lines in one self-updating file

usage: statusline.py [command]     (no command: render the main line; JSON on stdin)

  subagent      render subagent task rows (JSON on stdin)
  toggles       list toggles: the segments, plus 'update' (the hourly background check)
  on|off KEY    flip a toggle persistently
  update        fetch the latest version now (works even with 'update' toggled off)
  install       point ~/.claude/settings.json at this file (--no-update: also 'off update')

segments: {' '.join(SEGMENTS)}
config:   {CONF} — set $CLAUDE_STATUSLINE_CONFIG to relocate
"""

def install(no_update=False):
    settings = CLAUDE / "settings.json"
    s = jread(settings)
    cmd = "python3 " + str(SELF).replace(str(Path.home()), "~", 1)
    s["statusLine"] = {"type": "command", "command": cmd, "padding": 0, "refreshInterval": 10}
    s["subagentStatusLine"] = {"type": "command", "command": cmd + " subagent"}
    settings.write_text(json.dumps(s, indent=2) + "\n")
    print(f"settings.json statusLine + subagentStatusLine → {cmd}"
          + ("\n(running from a git checkout: auto-update stays off; publish with git)" if in_checkout() else ""))
    if no_update: cmd_toggle("update", False)

def main():
    a = sys.argv[1:]
    cmd = a[0] if a else ""
    if cmd in ("-h", "--help", "help"): return print(USAGE, end="")
    if cmd == "update": return cmd_update()
    if cmd == "install": return install("--no-update" in a)
    if cmd == "toggles": return cmd_toggles()
    if cmd in ("on", "off"):
        if len(a) < 2: sys.exit("usage: statusline.py on|off SEG")
        return cmd_toggle(a[1], cmd == "on")
    if cmd == "--refresh-pr": return refresh_pr(*a[1:4])
    if cmd == "--refresh-diff": return refresh_diff(a[1])
    maybe_update()
    return subagent() if cmd == "subagent" else render()

if __name__ == "__main__":
    main()
