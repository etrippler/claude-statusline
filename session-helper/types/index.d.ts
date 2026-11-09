export type Area = {
  name: string
  share: number
  last: string
  sessions: string[]
}

declare module 'claude-code' {
  interface PluginState {
    'session-helper': { areas: Shaped<Area[]> }
  }
}
