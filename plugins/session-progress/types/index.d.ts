export type Task = { title: string; percent: number; active: boolean }
/** how the list is shown: one line above the prompt, the list unfolded under it, or a side pane */
export type View = 'line' | 'list' | 'side'

declare module 'claude-code' {
  interface PluginState {
    'session-progress': { tasks: readonly Task[]; view: View }
  }
}
