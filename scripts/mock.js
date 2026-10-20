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
  const sec = Math.floor(now / 1000)
  const limits = [
    {account: 'work', accountIndex: 0, five_hour: {used_percentage: 35, resets_at: sec + 117 * 60}, seven_day: {used_percentage: 86, resets_at: sec + 25 * 3600}},
    {account: 'personal', accountIndex: 1, five_hour: {used_percentage: 5, resets_at: sec + 97 * 60}, seven_day: {used_percentage: 20, resets_at: sec + 106 * 3600}},
  ]
  const answers = {
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
