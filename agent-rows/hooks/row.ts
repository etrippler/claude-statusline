// One agent's row: "agent (type) ⎇ wt F5.1 hi · ✓ label        3m 12s · ↓ 12.3k tokens"
import type { AgentInfo, ModelEffort } from 'claude-code'

const SEP = ' · '
const LEVELS = ['low', 'medium', 'high', 'xhigh', 'max']
const SHORT = ['lo', 'med', 'hi', 'xh', 'max']

const sgr = (code: string) => (s: string) => `\x1b[${code}m${s}\x1b[0m`
const dim = sgr('2')
const red = sgr('31')
const green = sgr('32')
const yellow = sgr('33')

// Visible width: SGR sequences take no cells.
const width = (s: string) => [...s.replace(/\x1b\[[\d;]*m/g, '')].length

// "claude-opus-5-5[1m]" → "claude-opus-5-5"
export const base = (id: string) => id.replace(/\[\d+m\]$/, '')

export const same = (a: string, b: string) => a !== '' && base(a) === base(b)

const capitalize = (w: string) => w.charAt(0).toUpperCase() + w.slice(1).toLowerCase()

// "claude-fable-5-1[1m]" → "F5.1"; "claude-custom-thing" → "Custom Thing"
export function short(id: string) {
  const b = base(id)
  const toks = (b.startsWith('claude-') ? b.slice(7) : b).split('-').filter(Boolean)
  const ver = toks.filter(t => /^\d+$/.test(t) && t.length !== 8) // 8 digits: a date stamp
  const words = toks.filter(t => /[a-z]/i.test(t))
  if (words[0] && ver.length) return capitalize(words[0]).charAt(0) + ver.join('.')

  return words.map(capitalize).join(' ')
}

export const effortShort = (e: ModelEffort | number) =>
  typeof e === 'number' ? `${Math.round(e / 1000)}k` : (SHORT[LEVELS.indexOf(e)] ?? e)

// Compact en-US counts, as Claude Code prints them: 950, 12.3k, 2.5m.
export function tokens(n: number) {
  for (const [div, suf] of [[1e9, 'b'], [1e6, 'm'], [1e3, 'k']] as const) {
    if (n >= div) return `${(n / div).toFixed(1)}${suf}`
  }

  return String(Math.round(n))
}

// Matches the built-in: 42s, 3m 12s, 1h 2m 3s, 1d 2h 3m.
export function elapsed(ms: number) {
  const secs = Math.round(Math.max(ms, 0) / 1000)
  const [d, h, m, s] = [Math.floor(secs / 86_400), Math.floor(secs / 3_600) % 24, Math.floor(secs / 60) % 60, secs % 60]
  if (d) return `${d}d ${h}h ${m}m`
  if (h) return `${h}h ${m}m ${s}s`
  if (m) return `${m}m ${s}s`

  return `${s}s`
}

export type Context = { sessionModel: string; cwd: string; now: number; columns?: number }

// The row's body, or undefined to keep the engine's row (no model known yet).
export function row(a: AgentInfo, ctx: Context): string | undefined {
  // What the last request carried beats what was asked for: a model can cap
  // the effort, or take none. Before the first request, what was asked for.
  const model = a.lastRequest?.model ?? a.model
  if (!model) return undefined
  const effort = a.lastRequest ? a.lastRequest.effort : a.effort

  const name = a.name ?? ''
  const type = a.type === 'general-purpose' ? '' : a.type
  let head =
    !name && !type ? 'agent'
    : !type ? name
    : !name || name.toLowerCase() === type.toLowerCase() ? type
    : `${name} (${type})`
  if (ctx.cwd && a.cwd && a.cwd !== ctx.cwd) head += ` ⎇ ${a.cwd.split('/').filter(Boolean).at(-1)}` // own worktree
  if (!same(model, ctx.sessionModel)) head += ` ${short(model)}`
  if (effort !== undefined) head += ` ${effortShort(effort)}`

  const mark = { completed: green('✓'), failed: red('✗'), killed: yellow('✗') }[a.taskStatus as string]
  const right = [
    a.taskStatus === 'running' ? elapsed(ctx.now - a.startTime) : '', // no end time: a finished row's would keep counting
    a.tokenCount > 0 ? `↓ ${tokens(a.tokenCount)} tokens` : '',
  ]
    .filter(Boolean)
    .join(SEP)

  const label = (a.label.split('\n')[0] ?? '').trim()
  const cols = ctx.columns || undefined
  const room = cols === undefined ? Infinity : cols - width(head) - SEP.length - (mark ? 2 : 0) - width(right) - 2
  let out = dim(head)
  if (label && room > 8) {
    out += dim(SEP) + (mark ? `${mark} ` : '') + dim(width(label) > room ? [...label].slice(0, room - 1).join('') + '…' : label)
  } else if (mark) {
    out += ` ${mark}`
  }
  if (!right) return out

  // right flush against the edge; just "left · right" when the width is unknown or there's no room
  const pad = cols === undefined ? 0 : cols - width(out) - width(right)

  return pad > 0 ? out + ' '.repeat(pad) + dim(right) : out + dim(SEP) + dim(right)
}
