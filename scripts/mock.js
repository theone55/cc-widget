// A stand-in for Tauri in a plain browser: demo data for the README screenshots (scripts/shots.cjs).
;(() => {
  const q = new URLSearchParams(location.search)
  // a height cap for the dense layouts' screenshots
  try { if (q.get('level')) localStorage.setItem('size', JSON.stringify({w: 300, level: Number(q.get('level'))})); else localStorage.removeItem('size') } catch (e) {}
  const now = Date.now()
  const settings = {
    theme: q.get('theme') || 'dark', language: q.get('lang') || 'en', opacity: 96,
    colors: {ok: '#4ade80', warn: '#fb923c', full: '#f87171', accent: '#a78bfa'},
    thresholds: {contextWarn: 70, contextFull: 85, limitWarn: 70, limitFull: 90},
    accounts: [{label: 'work', dir: '.claude'}, {label: 'personal', dir: '.claude-personal'}],
    showLimits: true, showContext: true, showBackground: true, expanded: q.get('open') === '1',
    counter: {enabled: false, label: '', url: '', passUrl: '', fields: [], every: 5},
    system: {enabled: false, threads: true, gpu: true},
    remote: {enabled: q.get('remote') === '1', label: 'server', color: '', hostUrl: 'http://server/host', runsUrl: 'http://server/runs', hubUrl: 'http://server/hub', passUrl: ''},
  }
  const s = (id, account, accountIndex, name, cwd, status, minAgo, context) =>
    ({account, accountIndex, pid: 1, sessionId: id, name, cwd, kind: 'interactive', status, since: now - minAgo * 60000, context})
  const sessions = [
    s('a', 'work', 0, 'api-gateway', '/home/me/work/api-gateway', 'idle', 0, 43),
    s('b', 'personal', 1, 'side-project', '/home/me/side-project', 'idle', 2, 88),
    s('c', 'work', 0, 'billing-service', '/home/me/work/billing', 'busy', 1, 72),
    s('d', 'work', 0, 'docs', '/home/me/work/docs', 'busy', 0, 12),
  ]
  // background sessions (claude --bg): one working, one blocked by a permission prompt
  if (q.get('bg') === '1') sessions.push({...s('e', 'work', 0, 'nightly-refactor', '/home/me/work/api-gateway', 'busy', 3, 30), kind: 'bg', attachId: 'e1', configDir: ''},
    {...s('f', 'personal', 1, 'migrate-db', '/home/me/side-project', 'busy', 9, 55), kind: 'bg', attachId: 'f1', configDir: '', blocked: true},
    {...s('g', 'work', 0, 'schema-review', '/home/me/work/billing', 'idle', 4, 20), kind: 'bg', attachId: 'g1', configDir: '', blocked: true, needs: 'Which database should the migration target: staging or prod?'})
  // the session-progress mod's task list on the busy billing session
  sessions[2].progress = {overall: 55, tasks: [
    {title: 'Read the invoice flow', percent: 100, active: false},
    {title: 'Retry failed webhooks', percent: 60, active: true},
    {title: 'Tests for the retry', percent: 5, active: false},
  ]}
  // the model shown in every row's hover ("model: opus 5.5")
  ;['opus 5.5', 'opus 5.5', 'sonnet 5.5', 'haiku 5.5', 'sonnet 5.5', 'opus 5.5', 'opus 5.5'].forEach((m, i) => { if (sessions[i]) sessions[i].model = m })
  // subagents (?sa=1): under the busy billing session — one with its own task list, a background one without, a finished one;
  // and one under the bg session when ?bg=1 too
  if (q.get('sa') === '1') {
    sessions[2].subagents = [
      {id: 'a1', agentType: 'worker', description: 'Port the retry tests', running: true, background: false, model: 'sonnet 5.5', since: now - 90000,
        progress: {overall: 47, tasks: [
          {title: 'Read existing tests', percent: 100, active: false}, {title: 'Write 3 retry tests', percent: 40, active: true},
          {title: 'Run the suite', percent: 0, active: false}, {title: 'Fix flakes', percent: 0, active: false}, {title: 'Report', percent: 0, active: false}]}},
      {id: 'a2', agentType: 'researcher', description: 'Audit webhook signature checks', running: true, background: true, model: 'opus 5.5', since: now - 5 * 60000, progress: null},
      {id: 'a3', agentType: 'scout', description: 'Find callers of InvoiceService', running: false, background: false, model: 'haiku 5.5', since: now - 2 * 60000, progress: null},
    ]
    sessions[0].subagents = [{id: 'a4', agentType: 'general-purpose', description: 'Review the gateway diff', running: true, background: true, model: 'sonnet 5.5', since: now - 40000,
      progress: {overall: 80, tasks: [{title: 'Read diff', percent: 100, active: false}, {title: 'Check auth paths', percent: 60, active: true}]}}]
  }
  const sec = Math.floor(now / 1000)
  const limits = [
    {account: 'work', accountIndex: 0, five_hour: {used_percentage: 35, resets_at: sec + 117 * 60}, seven_day: {used_percentage: 86, resets_at: sec + 25 * 3600}},
    {account: 'personal', accountIndex: 1, five_hour: {used_percentage: 5, resets_at: sec + 97 * 60}, seven_day: {used_percentage: 20, resets_at: sec + 106 * 3600}},
  ]
  // GPT (Codex) accounts (?gpt=1): a paid one with 5 h + 7 d windows, a free one with a single 30-day window
  const gpt = []
  if (q.get('gpt') === '1') {
    settings.accounts.push({label: 'gpt', dir: '.codex', color: '', engine: 'codex'}, {label: 'gpt-2', dir: '.codex-2', color: '', engine: 'codex'})
    gpt.push({account: 'gpt', accountIndex: 2, email: 'me@example.com', plan: 'plus', reached: false, error: null, updatedMs: now - 3 * 60000, callsDay: 42,
        windows: [{mins: 300, used: 31, resetsAt: sec + 140 * 60}, {mins: 10080, used: 64, resetsAt: sec + 71 * 3600}]},
      {account: 'gpt-2', accountIndex: 3, email: 'free@example.com', plan: 'free', reached: false, error: null, updatedMs: now - 4 * 60000, callsDay: null,
        windows: [{mins: 43200, used: 12, resetsAt: sec + 19 * 86400}]})
  }
  const answers = {
    gpt_limits: () => gpt, detect_codex: () => ['.codex', '.codex-2'],
    settings_get: () => settings, settings_set: () => null, sessions: () => sessions, limits: () => limits,
    statusline_status: ({dir}) => dir === '.claude' ? 'connected' : 'off', detect_accounts: () => ['.claude', '.claude-personal'],
    counter_has_token: () => false, 'plugin:autostart|is_enabled': () => true,
    // the other machine (?remote=1)
    remote_host: () => ({cpu: 23, threads: [30, 12, 41, 8, 25, 19, 60, 5], cores: [21, 25, 22, 33], perCore: 2,
      memUsed: 21.4 * 2 ** 30, memTotal: 32 * 2 ** 30, cpuTemp: null, gpu: {name: 'NVIDIA GeForce RTX 3060', load: 4, memUsed: 1.1 * 2 ** 30, memTotal: 12 * 2 ** 30, temp: 41}}),
    remote_waiting: () => [{sessionId: 'c', reason: 'Claude needs your permission to use Bash'}],
    remote_runs: () => [
      {id: 161, name: 'Morning digest', status: 'busy', startedAt: new Date(now - 2 * 60000).toISOString(), origin: 'SCHEDULE'},
      {id: 160, name: '[order #92] widget: remote sessions', status: 'done', startedAt: new Date(now - 40 * 60000).toISOString(), finishedAt: new Date(now - 12 * 60000).toISOString(), origin: 'ORDER'},
    ],
  }
  window.__TAURI__ = {
    core: {invoke: async (cmd, args) => { if (!(cmd in answers)) throw 'no ' + cmd; return answers[cmd](args || {}) }},
    window: {getCurrentWindow: () => ({setSize: async () => {}, setMinSize: async () => {}, setPosition: async () => {}, onMoved: () => {}, onResized: () => {}, scaleFactor: async () => 1})},
    app: {getVersion: async () => '0.1.8'},
    dpi: {LogicalSize: function () {}, PhysicalPosition: function () {}},
    event: {listen: async () => () => {}},
  }
  if (q.get('settings') === '1') setTimeout(() => { const g = document.getElementById('gear'); for (const t of ['mousedown', 'mouseup']) g?.dispatchEvent(new MouseEvent(t, {bubbles: true})) }, 300)
})()
