import type { AgentInfo } from 'claude-code'
import { expect, test } from 'claude-code/testing'

import { effortShort, elapsed, row, same, short, tokens } from './row'

const plain = (s = '') => s.replace(/\x1b\[[\d;]*m/g, '')

const agent = (over: Partial<AgentInfo> = {}): AgentInfo => ({
  id: 'a1',
  description: 'Find the parser',
  type: 'Explore',
  status: 'running',
  kind: 'local_agent',
  taskStatus: 'running',
  label: 'Reading src/parse.rs',
  startTime: 0,
  cwd: '/repo',
  model: 'claude-haiku-5-5',
  tokenCount: 12_345,
  tokenSamples: [],
  hasRow: true,
  ...over,
})

const ctx = { sessionModel: 'claude-opus-5-5[1m]', cwd: '/repo', now: 192_000 }

test('formats', async () => {
  expect(short('claude-fable-5-1[1m]')).toBe('F5.1')
  expect(short('claude-opus-4-1-20250805')).toBe('O4.1')
  expect(short('claude-3-5-sonnet-20241022')).toBe('S3.5')
  expect(short('custom-thing')).toBe('Custom Thing')
  expect(same('claude-opus-5-5[1m]', 'claude-opus-5-5')).toBe(true)
  expect(same('', '')).toBe(false)
  expect(effortShort('xhigh')).toBe('xh')
  expect(effortShort(16_000)).toBe('16k')
  expect(tokens(950)).toBe('950')
  expect(tokens(12_345)).toBe('12.3k')
  expect(elapsed(125_300)).toBe('2m 5s')
  expect(elapsed(90_061_000)).toBe('1d 1h 1m')
})

test('a running agent on another model names its type, model and effort', async () => {
  const a = agent({ lastRequest: { at: 0, model: 'claude-haiku-5-5', effort: 'low' } })
  expect(plain(row(a, ctx))).toBe('Explore H5.5 lo · Reading src/parse.rs · 3m 12s · ↓ 12.3k tokens')
})

test('the session model is left out, the asked effort shows before the first request', async () => {
  const a = agent({ type: 'general-purpose', model: 'claude-opus-5-5', effort: 'high', tokenCount: 0 })
  expect(plain(row(a, ctx))).toBe('agent hi · Reading src/parse.rs · 3m 12s')
})

test('a request that sent no effort shows none', async () => {
  const a = agent({ effort: 'high', lastRequest: { at: 0, model: 'claude-haiku-5-5' } })
  expect(plain(row(a, ctx))).toBe('Explore H5.5 · Reading src/parse.rs · 3m 12s · ↓ 12.3k tokens')
})

test('a finished agent in a worktree: mark, no clock, right-justified', async () => {
  const a = agent({ name: 'fixer', type: 'claude', taskStatus: 'completed', cwd: '/repo/.claude/worktrees/wt1', tokenCount: 900 })
  const out = plain(row(a, { ...ctx, columns: 80 }))
  expect(out.startsWith('fixer (claude) ⎇ wt1 H5.5 · ✓ Reading src/parse.rs  ')).toBe(true)
  expect(out.endsWith('↓ 900 tokens')).toBe(true)
  expect([...out].length).toBe(80)
  expect(plain(row(a, { ...ctx, columns: 60 }))).toBe('fixer (claude) ⎇ wt1 H5.5 · ✓ Reading src/par…  ↓ 900 tokens')
})

test('no model yet keeps the engine row', async () => {
  expect(row(agent({ model: undefined }), ctx)).toBeUndefined()
})
