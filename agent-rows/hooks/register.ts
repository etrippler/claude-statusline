import type { EngineInterface, Register, Timer } from 'claude-code'

import { row } from './row'

// Rows redraw on their own every five seconds; a running row's elapsed time
// wants every second. The tick stops once no running agent has a row.
let tick: Timer | undefined

function startTick($: EngineInterface) {
  tick ??= $.clock.every(1000, async () => {
    const agents = await $.agent.list()
    if (agents.some(a => a.hasRow && a.taskStatus === 'running')) return $.ui.invalidate('ui.render')
    tick?.cancel()
    tick = undefined
  })
}

export const register: Register = on => {
  on('ui.render', { component: 'AgentRow' }, async ($, e, next) => {
    const [sessionModel, cwd, now] = await Promise.all([$.session.model(), $.session.cwd(), $.clock.now()])
    const body = row(e.props, { sessionModel, cwd, now, columns: e.props.columns })
    if (e.props.taskStatus === 'running') startTick($)

    return next(body === undefined ? e : { ...e, props: { ...e.props, body } })
  })
}
