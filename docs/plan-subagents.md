# Subagents under their session (design, 2026-11-06)

Goal: the widget lists a session's subagents (Agent tool, incl. background ones) under it, visibly different from
delegates (bg sessions: `↳` + «фон»), each with a progress bar by the session-row rule ("2/5 · 47%").

## What a widget can read per subagent (no stream-json)

| Data | Source | Notes |
|---|---|---|
| exists / which session | `<config dir>/projects/<key>/<sessionId>/subagents/agent-<agentId>.jsonl` | `<key>` = cwd with non-alnum → `-`, but a worktree session's cwd changes, so the widget scans `projects/*/<sid>/subagents` (a few stats per poll) |
| type, description, running/done, background | `agent-<id>.meta.json`: `agentType`, `description`, `toolUseId`, `requestShape` (`"background"`), `model`, `finished` (true when over) | checked on real files (2.1.319); `finished` is written when the agent ends |
| start time | creation time of the `.jsonl` (fallback: mtime) | |
| liveness | `.jsonl` mtime (grows with every tool call) | an unfinished agent silent for > 10 min is treated as dead and hidden |
| its own task list | `<config dir>/session-progress/<sid>/agent-<agentId>.json` | needs the mod change below |

Not used: the transcript body (hundreds of KB per agent; parsing it every 2 s is wasteful) — tokens / last tool are left out.

## The mod: subagent calls of `progress` overwrite the session's list (bug)

A subagent's `mcp__session-progress__progress` runs inside the SAME session id. The mod's `tool.call` hook for the
tool wrote `session-progress/<sid>.json` and the `tasks` atom unconditionally, so a subagent's list replaced the main
session's (the nudge hook already ignores `e.agentId` calls; the progress hook did not). Fix: when `e.agentId` is set, write
`session-progress/<sid>/agent-<agentId>.json` and leave the atoms, band, status line and main file alone.
No subagent in the local history has called the tool yet (the guide section tells only the main loop to), so the
agent file stays optional: a subagent without it shows no bar, just a pulse while running.
Assumption to confirm on a live subagent: the hook's `e.agentId` equals the transcript's `agentId`.

## Design

- `sessions.rs`: every live session gets `subagents: Vec<Subagent>` (running + finished in the last 3 min, newest first,
  at most 8): `{id, agentType, description, running, background, since, progress}`. Not separate sessions, so `under_parents`
  and the busy/wait counters are untouched.
- UI: right under the parent row (and its delegate children) — an indented row: violet `◆` mark + violet «суб» chip
  (delegates: `↳` + grey «фон»), the description as name, `2/5 · 47%` + a thin violet bar from its list. Finished ones are dimmed
  with ✓. Tooltip: type, description, background?, started ago, its tasks.
- Violet `#9b6bff` as in claude-chat. Always shown (a `showSubagents: false` in settings.json would hide them; no ⚙ switch yet).
  The dense layouts (levels 3+) hide the subagent rows.
- Mock: `docs/mock-subagents.html` (defaults `?sa=1&bg=1&open=1&lang=ru`; `scripts/mock.js` is the same stand-in as `?bg=1`).

## The model in every hover (owner's extra ask, same day)

Where a model can be read: the session file `sessions/<pid>.json` has none; the statusline input has `model.display_name`
but only the statusline process sees it (the widget's `--statusline` hook doesn't store it); `jobs/<id>/state.json`
only holds `respawnFlags` (`--model haiku`, bg jobs only). The reliable source for ALL of them is the **transcript**: the
last assistant message's `"model":"claude-opus-5-5"` — the session's `projects/<key>/<sid>.jsonl` (main, bg and delegates alike)
and a subagent's own `agent-<id>.jsonl` (fallback: the alias in its `meta.json`, e.g. `sonnet`). Only the last 64 KB is read,
cached by (path, mtime+size), so a 2 s poll costs nothing while nothing changes. `claude-opus-5-5` → `opus 5.5`,
`claude-haiku-4-5-20251001` → `haiku 4.5` (date dropped), an alias stays. Shown as a «модель: opus 5.5» line in the tooltip of
every session row (incl. bg) and every subagent row. A model switched mid-session shows after its next reply.
