import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Area } from '../types'

// The vertical pane earlier versions opened; closed on load.
const OLD_PANE = 'areas'
const STALE_MS = 60 * 60 * 1000
// Change the tag when Area's fields change: it declines what an earlier version stored.
const areas = atom({ plugin: 'threads', key: 'areas' } as const, [], { shape: 'areas-3' })

const CLASSIFY = `Group the user's recent Claude Code sessions into their bodies of work: sustained efforts with a shared goal
that they come back to across sessions (a customer engagement, a product feature, a prototype, an internal tool, an
on-call rotation). Group by the effort, not the kind of activity: a Slack reply about an engagement belongs to that
engagement, and a Slack or Granola pass about a feature belongs to that feature. Use 4 to 8 bodies of work and put
sessions that fit none in "Misc". Misc is for small one-offs: a session with hours of active time is a sustained
effort even when it is the only one of its kind, so give it its own body of work. Where a previous name still fits,
keep it, so names stay stable between runs; drop a previous name that no longer fits. Name each the way the user
would say it, in a few words. "status" is one line: what the effort is and where it stands, from its latest sessions.

Each session line: id | last active | cwd | active hours | characters the user typed | title | the user's first messages.

Reply with JSON only: {"areas":[{"name":"...","status":"...","sessions":["<8-char id>", ...]}]}`

const STATE = `You write the note a Claude Code session reads after its context is compacted. The whole transcript stays on
disk with fixed line numbers. Beside your note, the session gets each of the user's asks by line with how the turn
ended, and the links, documents and files in play with their ids, so do not retell the history or list references.
Write what the next context needs to keep working, as short bullets under three headings:

Goal: what the user is trying to get done in this session, in one or two bullets.
Decided: decisions made and constraints the user set that later work must respect, each with the line it was settled
on (L123). Leave out what any project instructions would already say.
Now: what is done, what is in progress, the next step, and anything waiting on the user, with line numbers.

A long transcript reaches you with its middle left out. When your previous note is given, keep its Decided items that
still hold, with their lines, even when those lines fall in the part you cannot see; drop what later lines overturned.

At most 200 words. Plain text, no preamble.`
// Off: the engine compacts with its own summarizer and no state note is written; sessions find the text copy, refs and
// areas through the "Session history" note in ~/.claude/CLAUDE.md. On: compaction is built from the text copy (scaffold,
// refs in play, Sonnet's state note) and the engine's summarizer never runs.
const REPLACE_COMPACTION = false
// Write the state note in the background once the context is this large, and again each time the transcript grows by
// STATE_EVERY lines, so a compaction rarely waits on the model.
const STATE_FROM_TOKENS = 120_000
const STATE_EVERY = 250
// What the state model reads of a long transcript: its opening and as much of the end as fits.
const STATE_HEAD_CHARS = 30_000
const STATE_TAIL_CHARS = 170_000
// The desktop app and the VS Code extension drive the engine over the SDK, so it reports them as not interactive, but a
// person is typing in them. Only these entrypoints; -p and SDK scripts stay headless (cc-text's is_headless agrees).
const HUMAN_ENTRYPOINTS = new Set(['claude-desktop', 'claude-vscode'])
// Below this the text copy holds too little to point into (a fork starts its own), so the engine summarizes.
const MIN_LINES = 40

type Handoff = {
  lines: number
  // The last three asks; the scaffold has the rest.
  asks: { line: number; at: string; text: string }[]
  asksTotal: number
  scaffold: string
  refs: string
  refsTop: string
  tailStart: number
  tail: string
  area: string
  title: string
}
type State = { line: number; text: string; instructions?: string }

// Module state; a reload starts it over.
let sessionId = ''
let clearedFrom = ''
let previousId = ''
let isInteractive = false
let isClassifying = false
let pendingState: Promise<State | undefined> | undefined
const compacted = new Set<string>()

async function home($: EngineInterface) {
  return `${(await $.env.get('CLAUDE_CONFIG_DIR')) ?? `${await $.env.get('HOME')}/.claude`}/threads`
}

function ccText($: EngineInterface, args: string[], timeoutMs = 60_000) {
  return $.process.run(['python3', `${$.plugin.root}/bin/cc-text`, ...args], { timeoutMs })
}

// The env vars follow the session id, which a /clear changes without a session.start.
async function ensureSession($: EngineInterface) {
  const id = await $.session.id()
  if (id === sessionId) return id
  sessionId = id
  const root = await home($)
  await $.env.set('CC_HOME', root)
  await $.env.set('CC_SESSIONS', `${root}/sessions`)
  await $.env.set('CC_INDEX', `${root}/index.tsv`)
  await $.env.set('CC_VIEWS', `${root}/views`)
  await $.env.set('CC_SESSION', `${root}/sessions/${id}.txt`)
  if (clearedFrom && clearedFrom !== id) {
    await $.env.set('CC_PREV_SESSION', `${root}/sessions/${clearedFrom}.txt`)
    await ccText($, ['link', id, clearedFrom])
    previousId = clearedFrom
    clearedFrom = ''
  }

  return id
}

// Ranks the classified areas by attention and shows them; cheap, no model call.
async function render($: EngineInterface) {
  const { exitCode, stdout } = await ccText($, ['areas'])
  if (exitCode !== 0) return
  const ranked = JSON.parse(stdout) as Area[]
  await update($, areas, () => ranked)
}

// One Sonnet call that regroups the last three weeks of sessions into bodies of work.
async function classify($: EngineInterface) {
  if (isClassifying) return
  isClassifying = true
  try {
    const views = `${await home($)}/views`
    const previous = await $.fs.read(`${views}/areas.json`).catch(() => '{"areas":[]}')
    const names = (JSON.parse(previous) as { areas: { name: string; status: string }[] }).areas
      .map(area => `- ${area.name}: ${area.status}`)
      .join('\n')
    const { exitCode, stdout } = await ccText($, ['areas-input'])
    if (exitCode !== 0 || !stdout.trim()) return
    const reply = await $.model.complete({
      model: 'sonnet',
      system: CLASSIFY,
      // The session lines read like a conversation, so they are fenced as data and the ask comes after them.
      prompt: `<previous_bodies_of_work>\n${names || '(none yet)'}\n</previous_bodies_of_work>\n\n<sessions>\n${stdout}</sessions>\n\nGroup the sessions above into bodies of work as instructed. Reply with the JSON object only.`,
      maxTokens: 4000,
      timeoutMs: 120_000,
    })
    if (!reply.isAnswered) return
    const json = reply.text.slice(reply.text.indexOf('{'), reply.text.lastIndexOf('}') + 1)
    const parsed = JSON.parse(json) as { areas?: unknown[] }
    if (!Array.isArray(parsed.areas) || parsed.areas.length === 0) return
    await $.fs.write(`${views}/areas.json`, JSON.stringify(parsed, null, 2))
  } catch (error) {
    $.ui.log(`areas classification failed: ${String(error)}`, { to: 'debug' })
  } finally {
    isClassifying = false
  }
}

async function handoff($: EngineInterface, id: string) {
  const { exitCode, stdout } = await ccText($, ['handoff', id])
  return exitCode === 0 ? (JSON.parse(stdout) as Handoff) : undefined
}

function asksList(asks: Handoff['asks']) {
  return asks.map(ask => `L${ask.line}`.padEnd(7) + ask.text).join('\n')
}

// One Sonnet call over this session's text copy: the goal, what was decided, where things stand. Kept beside the
// copy, so a compaction uses the latest one without waiting; a call already running is shared.
function writeState($: EngineInterface, id: string, instructions?: string) {
  pendingState ??= (async () => {
    try {
      const path = `${await home($)}/sessions/${id}.txt`
      const lines = (await $.fs.read(path)).split('\n')
      let numbered = lines.map((line, i) => `${i + 1}\t${line}`).join('\n')
      if (numbered.length > STATE_HEAD_CHARS + STATE_TAIL_CHARS) {
        numbered = `${numbered.slice(0, STATE_HEAD_CHARS)}\n[… middle left out …]\n${numbered.slice(-STATE_TAIL_CHARS)}`
      }
      const focus = instructions ? `\n\nThe user asked the note to keep or stress: ${instructions}` : ''
      // Carries decisions forward across compactions once the transcript is too long to send whole.
      const previous = await savedState($, id)
      const carried = previous ? `<previous_note as_of_line="${previous.line}">\n${previous.text}\n</previous_note>\n\n` : ''
      const reply = await $.model.complete({
        model: 'sonnet',
        system: STATE,
        // The transcript reads like a conversation, so it is fenced as data and the ask comes after it.
        prompt: `${carried}<transcript>\n${numbered}\n</transcript>\n\nWrite the note for the transcript above as instructed.${focus}`,
        maxTokens: 1500,
        timeoutMs: 90_000,
      })
      if (!reply.isAnswered || !reply.text.trim()) return undefined
      const state: State = { line: lines.length - 1, text: reply.text.trim(), instructions }
      await $.fs.write(`${await home($)}/sessions/${id}.state.json`, JSON.stringify(state))

      return state
    } catch (error) {
      $.ui.log(`state note failed: ${String(error)}`, { to: 'debug' })
      return undefined
    } finally {
      pendingState = undefined
    }
  })()

  return pendingState
}

async function savedState($: EngineInterface, id: string) {
  const text = await $.fs.read(`${await home($)}/sessions/${id}.state.json`).catch(() => '')
  return text ? (JSON.parse(text) as State) : undefined
}

// The conversation a compaction leaves: one message pointing into the text copy, which keeps every line, with the
// scaffold (asks and how each turn ended), the references in play, the state note, and for a compaction mid-task the
// last exchange.
async function compactMessage($: EngineInterface, id: string, trigger: string, instructions?: string) {
  await ccText($, ['sync', id])
  const facts = await handoff($, id)
  if (!facts || facts.lines < MIN_LINES) return undefined
  let state = await savedState($, id)
  const isStale = !state || state.instructions !== instructions || facts.lines - state.line > STATE_EVERY
  if (isStale) {
    // A manual compaction waits longer than one the engine needs to finish a turn.
    state = (await Promise.race([writeState($, id, instructions), $.clock.sleep(trigger === 'auto' ? 40_000 : 75_000).then(() => undefined)])) ?? state
  }
  const path = `${await home($)}/sessions/${id}.txt`
  const parts = [
    `This session is being continued from a previous conversation that was compacted. Nothing was lost: the whole conversation is in $CC_SESSION (${path}), lines 1–${facts.lines}, and its line numbers never change. Read the lines you need there (sed -n '120,180p', rg -n) rather than guessing from this note.`,
  ]
  if (facts.scaffold) {
    parts.push(`The conversation by line: each of the user's asks, then how the turn ended (${facts.asksTotal} asks in all: rg -n '^you: \\[' "$CC_SESSION")\n${facts.scaffold}`)
  }
  if (facts.refs) {
    parts.push(`In play: what this session has used, weighted toward the latest work. Every one, with lines and titles, is in $CC_SESSIONS/${id}.refs.tsv; rg a key across $CC_SESSIONS/*.refs.tsv to find other sessions that used it.\n${facts.refs}`)
  }
  if (state) parts.push(`Where things stand, as of line ${state.line}\n${state.text}`)
  if (instructions) parts.push(`The user asked this compaction to keep: ${instructions}`)
  if (trigger === 'auto') {
    parts.push(`The last exchange, lines ${facts.tailStart}–${facts.lines}\n${facts.tail}`)
    parts.push('Continue the task from where it left off. Read the transcript for anything you need rather than asking the user.')
  }

  return parts.join('\n\n')
}

async function areasBlock($: EngineInterface) {
  let list = await read($, areas)
  if (list.length === 0) {
    await render($)
    list = await read($, areas)
  }
  if (list.length === 0) return undefined
  const classified = await $.fs.read(`${await home($)}/views/areas.json`).catch(() => '{"areas":[]}')
  const status = new Map(
    (JSON.parse(classified) as { areas: { name: string; status?: string }[] }).areas.map(area => [area.name, area.status ?? '']),
  )
  const rows = list
    .filter(area => area.name !== 'Misc')
    .map(area => `- ${area.name} ${Math.round(area.share * 100)}%: ${status.get(area.name) ?? ''}`.trimEnd())

  return `The user's current bodies of work, ranked by attention over the last 21 days. Each one's sessions, newest first, are in $CC_VIEWS/areas.md.\n${rows.join('\n')}`
}

async function previousBlock($: EngineInterface, id: string) {
  const facts = await handoff($, id)
  if (!facts) return undefined
  const what = [facts.area, `${facts.lines} lines`].filter(Boolean).join(', ')
  const parts = [
    `This conversation started with /clear, ending session ${id.slice(0, 8)} (${what}). None of it is loaded. If the user's ask continues it, read $CC_PREV_SESSION first, starting with tail -n 200.`,
  ]
  if (facts.asks.length > 0) parts.push(`Its last asks:\n${asksList(facts.asks)}`)
  if (facts.refsTop) parts.push(`What it was using:\n${facts.refsTop}`)

  return parts.join('\n')
}

async function refresh($: EngineInterface, isForced: boolean) {
  await ccText($, ['sync'], 600_000)
  const stat = await $.fs.stat(`${await home($)}/views/areas.json`).catch(() => undefined)
  if (isForced || !stat || (await $.clock.now()) - stat.mtimeMs > STALE_MS) await classify($)
  await render($)
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    isInteractive = e.isInteractive || HUMAN_ENTRYPOINTS.has((await $.env.get('CLAUDE_CODE_ENTRYPOINT')) ?? '')
    // Headless runs (-p, SDK scripts) keep their transcript current but never call a model.
    if (!isInteractive) {
      await ensureSession($)
      return started
    }
    await $.command.register({ name: 'areas', description: 'Regroup recent sessions into bodies of work, ranked by attention' })
    await ensureSession($)
    void $.ui.close({ id: OLD_PANE })
    void refresh($, false)

    return started
  })

  on('session.end', ($, e, next) => {
    if (e.reason === 'clear') clearedFrom = e.sessionId

    return next(e)
  })

  // A failure here must never hold up the prompt: fall through to the engine.
  on('prompt.submit', async ($, e, next) => {
    await ensureSession($)

    return next(e)
  }).catch(($, e, next) => next(e))

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    if (e.agentId === undefined) void ccText($, ['sync', await ensureSession($)])

    return done
  })

  // Keeps the state note current in long sessions, so compaction finds one ready.
  on('session.measure', async ($, e, next) => {
    const measured = await next(e)
    if (!REPLACE_COMPACTION || !isInteractive || pendingState || (e.context.tokens ?? 0) < STATE_FROM_TOKENS) return measured
    const id = await ensureSession($)
    const state = await savedState($, id)
    const meta = await $.fs.read(`${await home($)}/sessions/${id}.meta`).catch(() => '{}')
    const lines = (JSON.parse(meta) as { lines?: number }).lines ?? 0
    if (!state || lines - state.line >= STATE_EVERY) void writeState($, id)

    return measured
  }).catch(($, e, next) => next(e))

  // Compaction from the text copy: no summarizer over the whole context. Subagents' compactions, headless runs and any
  // failure here go to the engine's own. Ahead-of-time runs only refresh the state note.
  on('session.compact', async ($, e, next) => {
    if (e.agentId !== undefined || !isInteractive) return next(e)
    const id = await ensureSession($)
    if (!REPLACE_COMPACTION) {
      // Still noted, so the /clear pointer isn't sent again after the engine's compaction.
      if (e.trigger !== 'precompute') compacted.add(id)
      return next(e)
    }
    if (e.trigger === 'precompute') {
      void writeState($, id)
      return { skip: 'threads compacts from the text copy' }
    }
    const text = await compactMessage($, id, e.trigger, e.instructions)
    if (!text) return next(e)
    compacted.add(id)

    return { messages: [{ role: 'user', text, toolUses: [] }] }
  }).catch(($, e, next) => next(e))

  // Context blocks for the first message of each conversation, sent again after a compaction: the bodies of work, and
  // after a /clear, a pointer to the session it cleared.
  on('prompt.context', async ($, e, next) => {
    const context = await next(e)
    if (!isInteractive) return context
    const id = await ensureSession($)
    const blocks = [...context.blocks]
    if (previousId && !compacted.has(id)) {
      const text = await previousBlock($, previousId)
      if (text) blocks.push({ name: 'threadsPrevious', text })
    }
    const text = await areasBlock($)
    if (text) blocks.push({ name: 'threadsAreas', text })

    return { ...context, blocks }
  }).catch(($, e, next) => next(e))

  on('command.run', { command: 'areas' }, async $ => {
    await refresh($, true)
    const ranked = await read($, areas)

    return { text: `Regrouped into ${ranked.length} bodies of work; details in $CC_VIEWS/areas.md.` }
  })
}
