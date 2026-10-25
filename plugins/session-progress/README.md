# session-progress — Claude's task list, live

A Claude Code plugin that lets you watch what Claude is doing in a session: its task list with a rough percent for each task, updated as it works.

- **Above the prompt:** one line, `Tasks 2/5 · 47% ████░░░░ · ▶ current task 60%  [▾] [⇥]`. `▾` unfolds the whole list under it, `⇥` moves it to a side pane (closing the pane brings the line back). `/progress` toggles the pane too.
- **In the status line:** `☑ 2/5 · 47% · current task 60%`.
- **In [cc-widget](../../README.md):** "2/5 · 47%" in the session's row, the task in progress under it, every task in the tooltip. The plugin writes `<config dir>/session-progress/<session id>.json`; the widget reads it (0.1.11 and later).

Claude gets a tool, `progress`, whose description tells it when to use it: before work of two or more steps, then whenever a task finishes or moves about a quarter. It sends the task list along with its other tool calls, so it adds no extra model steps: a few hundred tokens per update.

## Install

At the prompt of a Claude Code terminal session:

```
/plugin install session-progress --marketplace theone55/cc-widget
```

Answer `y` to add the marketplace, then pick a scope (user = every session).

From a shell:

```
claude plugin install session-progress --marketplace theone55/cc-widget
```

## Options

- `language`: `en` (default) or `ru` — the labels of the band and the pane. The task titles are in whatever language Claude writes them.

## License

MIT, like the rest of this repository.
