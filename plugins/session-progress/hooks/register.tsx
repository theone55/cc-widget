import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Task, View } from '../types'

const PLUGIN = 'session-progress'
const TOOL = `mcp__${PLUGIN}__progress`

const tasks = atom({ plugin: 'session-progress', key: 'tasks' } as const, [] as readonly Task[])
const view = atom({ plugin: 'session-progress', key: 'view' } as const, 'line' as View)
const PANE = 'session-progress'

// the labels; the task titles themselves are whatever language Claude writes them in
const WORDS = {
  en: {
    tasks: 'Tasks', empty: 'No tasks yet.', toSide: 'The task list is in a side pane now.',
    toLine: 'The task list is back above the prompt.', command: 'The session task list: in a side pane / back above the prompt',
  },
  ru: {
    tasks: 'Задачи', empty: 'Список задач пуст.', toSide: 'Список задач — сбоку.',
    toLine: 'Список задач — снова строкой над вводом.', command: 'Список задач сессии: сбоку / обратно строкой над вводом',
  },
}
let w = WORDS.en

const GUIDE = `# Session progress (${TOOL})
The user watches this session's task list live: a band above the prompt, the status line and the cc-widget desktop widget.
- Work of 2+ steps: call ${TOOL} with the whole list before starting (each task: short title in the user's language, percent 0-100, active=true on the one in progress).
- Call it again whenever a task finishes or moves noticeably (roughly every 25%), and when new tasks appear. Always send the full list; finished ones stay at 100.
- The percent is an honest rough estimate of how much of that task is done, not of time. A one-step answer needs no list.`

const clamp = (n: unknown) => Math.max(0, Math.min(100, Math.round(Number(n) || 0)))

const parse = (raw: unknown): Task[] =>
  (Array.isArray(raw) ? raw : []).slice(0, 30).map(t => {
    const o = (t ?? {}) as Record<string, unknown>
    return { title: String(o.title ?? '').slice(0, 120), percent: clamp(o.percent), active: o.active === true }
  }).filter(t => t.title)

const overall = (list: readonly Task[]) =>
  list.length ? Math.round(list.reduce((s, t) => s + t.percent, 0) / list.length) : 0

const bar = (percent: number, width: number) => {
  const full = Math.round((percent / 100) * width)
  return '█'.repeat(full) + '░'.repeat(width - full)
}

const statusText = (list: readonly Task[]) => {
  if (!list.length) return undefined
  const done = list.filter(t => t.percent >= 100).length
  const now = list.find(t => t.active && t.percent < 100) ?? list.find(t => t.percent < 100)
  return `☑ ${done}/${list.length} · ${overall(list)}%` + (now ? ` · ${now.title.slice(0, 40)} ${now.percent}%` : '')
}

// For cc-widget: <config dir>/session-progress/<session id>.json
const progressFile = async ($: EngineInterface) => {
  const dir = (await $.env.get('CLAUDE_CONFIG_DIR'))
    ?? `${(await $.env.get('USERPROFILE')) ?? (await $.env.get('HOME')) ?? '.'}/.claude`
  return `${dir.replace(/\\/g, '/')}/session-progress/${await $.session.id()}.json`
}

const publish = async ($: EngineInterface, list: readonly Task[]) => {
  $.ui.status(statusText(list))
  const doc = {
    sessionId: await $.session.id(),
    cwd: await $.session.cwd(),
    updatedAt: new Date(await $.clock.now()).toISOString(),
    overall: overall(list),
    tasks: list,
  }
  await $.fs.write(await progressFile($), JSON.stringify(doc, null, 2))
}

export const register: Register = (on, options) => {
  w = options.language === 'ru' ? WORDS.ru : WORDS.en
  on('session.start', async ($, e, next) => {
    await $.tool.register({
      name: 'progress',
      description:
        'Shows the user this session\'s task list with a rough percent per task (band above the prompt, status line, desktop widget); the user watches it live. ' +
        'Use it for any work of 2+ steps: send the list before starting, then again whenever a task finishes or moves ~25%, or new tasks appear. ' +
        'Never call it as a step of its own: put it in the same message as your next real tool calls (each extra step re-reads the whole context). Write titles as plain text, not backslash-u escapes. ' +
        'Send the FULL list each time (it replaces the previous one; finished tasks stay at 100); active=true on the task in progress; titles short, in the user\'s language. ' +
        'The percent is an honest rough estimate of how much of the task is done. An empty list clears it.',
      inputSchema: {
        type: 'object',
        properties: {
          tasks: {
            type: 'array',
            items: {
              type: 'object',
              properties: {
                title: { type: 'string', description: 'Short task title in the user\'s language' },
                percent: { type: 'integer', minimum: 0, maximum: 100, description: 'Rough share of this task done' },
                active: { type: 'boolean', description: 'The task being worked on now' },
              },
              required: ['title', 'percent'],
            },
          },
        },
        required: ['tasks'],
      },
      isDeferred: false,
    })
    await $.command.register({ name: 'progress', description: w.command })
    const list = await read($, tasks)
    if (list.length) await publish($, list)
    return next(e)
  })

  on('prompt.compose', async ($, e, next) => {
    const composed = await next(e)
    if (!e.tools.includes(TOOL)) return composed
    return { sections: [...composed.sections, { id: `${PLUGIN}:guide`, text: GUIDE, scope: 'session' as const }] }
  })

  on('tool.call', { tool: TOOL }, async ($, e) => {
    const list = parse((e as unknown as Record<string, unknown>).tasks)
    await update($, tasks, () => list)
    await publish($, list)
    const done = list.filter(t => t.percent >= 100).length
    return { result: list.length ? `Shown: ${done}/${list.length} done, ${overall(list)}% overall.` : 'Cleared.' }
  })


  on('command.run', { command: 'progress' }, async $ => {
    const isUp = (await $.ui.panes()).some(p => p.id === PANE)
    if (isUp) {
      await $.ui.close({ id: PANE })
      return { text: w.toLine }
    }
    await openSide($)
    return { text: w.toSide }
  })

  on('ui.close', async ($, e, next) => {
    if (e.id === PANE) await update($, view, () => 'line' as const)
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const list = await read($, tasks)
    if (!list.length) return <Text dimColor>{w.empty}</Text>
    return <Box flexDirection="column">{rows($, e, list, e.props.bodyColumns ?? 40, true)}</Box>
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const list = await read($, tasks)
    const mode = await read($, view)
    if (e.props.hasSurvey || !list.length || mode === 'side') return next(e)

    const { Box, Button, Text } = $.ui.resolve(e)
    const columns = e.props.bodyColumns ?? 80
    const done = list.filter(t => t.percent >= 100).length
    const now = current(list)
    const nowText = now && mode === 'line' ? ` · ▶ ${now.title} ${now.percent}%` : ''
    const room = Math.max(0, columns - 40)
    const header = (
      <Box key="head">
        <Text bold>{w.tasks} {done}/{list.length} · {overall(list)}% </Text>
        <Text dimColor>{bar(overall(list), 8)}</Text>
        <Text color="cyan">{nowText.length > room ? nowText.slice(0, Math.max(0, room - 1)) + '…' : nowText} </Text>
        <Button key="fold" label={mode === 'list' ? '▴' : '▾'} onPress={() => update($, view, v => (v === 'list' ? 'line' : 'list'))} />
        <Text> </Text>
        <Button key="side" label="⇥" onPress={() => openSide($)} />
      </Box>
    )
    if (mode === 'line') return header
    return (
      <Box flexDirection="column">
        {header}
        {rows($, e, list, columns, false)}
      </Box>
    )
  })
}

// the side pane: opened from the band's button or /progress; closing it brings the one-line band back
const openSide = async ($: EngineInterface) => {
  await update($, view, () => 'side' as const)
  await $.ui.open({ id: PANE, title: w.tasks })
}

const current = (list: readonly Task[]) => list.find(t => t.active && t.percent < 100) ?? list.find(t => t.percent < 100)

// one row per task: mark, bar, percent, title cut to the width (in the pane the bar is shorter)
const rows = ($: EngineInterface, e: Parameters<EngineInterface['ui']['resolve']>[0], list: readonly Task[], columns: number, narrow: boolean) => {
  const { Box, Text } = $.ui.resolve(e)
  const barWidth = narrow || columns < 70 ? 6 : 10
  const titleWidth = Math.max(8, columns - barWidth - 9)
  return list.map((t, i) => {
    const isDone = t.percent >= 100
    const mark = isDone ? '✓' : t.active ? '▶' : '·'
    const title = t.title.length > titleWidth ? t.title.slice(0, titleWidth - 1) + '…' : t.title
    return (
      <Box key={`t${i}`}>
        <Text color={isDone ? 'green' : t.active ? 'cyan' : undefined} dimColor={isDone}>
          {mark} {bar(t.percent, barWidth)} {String(t.percent).padStart(3)}% {title}
        </Text>
      </Box>
    )
  })
}
