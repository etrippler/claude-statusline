import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Area } from '../types'

const STALE_MS = 60 * 60 * 1000
// Change the tag when Area's fields change: it declines what an earlier version stored.
const areas = atom({ plugin: 'session-helper', key: 'areas' } as const, [], { shape: 'areas-3' })

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

// The desktop app and the VS Code extension drive the engine over the SDK, so it reports them as not interactive, but a
// person is typing in them. Only these entrypoints; -p and SDK scripts stay headless (cc-text's is_headless agrees).
const HUMAN_ENTRYPOINTS = new Set(['claude-desktop', 'claude-vscode'])

// What the /clear pointer shows of the session it cleared.
type Previous = {
  lines: number
  // The last three asks.
  asks: { line: number; at: string; text: string }[]
  refsTop: string
  area: string
}

// Module state; a reload starts it over.
let sessionId = ''
let clearedFrom = ''
let previousId = ''
let isInteractive = false
let isClassifying = false
// Sessions that have compacted; the /clear pointer isn't sent again after one.
const compacted = new Set<string>()

async function home($: EngineInterface) {
  return `${(await $.env.get('CLAUDE_CONFIG_DIR')) ?? `${await $.env.get('HOME')}/.claude`}/session-helper`
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
    const prior = await $.fs.read(`${views}/areas.json`).catch(() => '{"areas":[]}')
    const names = (JSON.parse(prior) as { areas: { name: string; status: string }[] }).areas
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

async function previous($: EngineInterface, id: string) {
  const { exitCode, stdout } = await ccText($, ['previous', id])
  return exitCode === 0 ? (JSON.parse(stdout) as Previous) : undefined
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
  const facts = await previous($, id)
  if (!facts) return undefined
  const what = [facts.area, `${facts.lines} lines`].filter(Boolean).join(', ')
  const parts = [
    `This conversation started with /clear, ending session ${id.slice(0, 8)} (${what}). None of it is loaded. If the user's ask continues it, read $CC_PREV_SESSION first, starting with tail -n 200.`,
  ]
  if (facts.asks.length > 0) parts.push(`Its last asks:\n${facts.asks.map(ask => `L${ask.line}`.padEnd(7) + ask.text).join('\n')}`)
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

  // Compaction stays with the engine; this only notes it, so the /clear pointer isn't sent again afterwards.
  on('session.compact', async ($, e, next) => {
    if (e.agentId === undefined && e.trigger !== 'precompute') compacted.add(await ensureSession($))

    return next(e)
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
      if (text) blocks.push({ name: 'previousSession', text })
    }
    const text = await areasBlock($)
    if (text) blocks.push({ name: 'bodiesOfWork', text })

    return { ...context, blocks }
  }).catch(($, e, next) => next(e))

  on('command.run', { command: 'areas' }, async $ => {
    await refresh($, true)
    const ranked = await read($, areas)

    return { text: `Regrouped into ${ranked.length} bodies of work; details in $CC_VIEWS/areas.md.` }
  })
}
