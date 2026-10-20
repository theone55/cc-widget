// cc-widget's page: draws what the Rust side reads (sessions, limits, the optional counter) and edits the settings.
const {invoke} = window.__TAURI__.core
const win = window.__TAURI__.window.getCurrentWindow()
const {LogicalSize, PhysicalPosition} = window.__TAURI__.dpi
const $ = (id) => document.getElementById(id)

// per-window conveniences (where the window is, which sessions you've seen) — localStorage; the rest is settings
const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d } catch (e) { return d } }
const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)) } catch (e) {} }

let S = null              // the settings (Rust's settings.rs)
let seen = load('seen', {})
let sessions = []
let limits = []
let counter = null        // [values] | 'no-token' | 'error…' | null
let sys = null            // the System section's last numbers (system.rs Stats) or null
const cpuHist = [], memHist = []   // the last 2 minutes, one sample per 2 s
let editing = false
let T = I18N.en

const esc = (t) => String(t ?? '').replace(/[&<>"]/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;'}[c]))
const folder = (p) => String(p || '').split(/[\\/]/).filter(Boolean).pop() || p

// A session "waits" when its turn is over (idle) and you haven't clicked it since: the click is the "seen".
// "shell": the turn is over but a background command still runs (a dev server, a watcher) — the turn is yours like idle;
// counting it as working left finished sessions "working" for hours (owner 2026-10-20)
const state = (s) => s.status === 'busy' ? 'busy'
  : s.status === 'idle' || s.status === 'shell' ? ((seen[s.sessionId] || 0) >= s.since ? 'idle' : 'wait')
  : 'wait'                // anything else (a permission prompt…) needs you
const ago = (ms) => {
  const m = Math.max(0, Math.round((Date.now() - ms) / 60000))
  return m < 1 ? T.now : m < 60 ? `${m} ${T.min}` : `${Math.floor(m / 60)} ${T.hr} ${m % 60} ${T.min}`
}
const left = (sec) => {
  const m = Math.max(0, Math.round((sec * 1000 - Date.now()) / 60000))
  return m >= 1440 ? `${Math.floor(m / 1440)}${T.day} ${Math.floor(m % 1440 / 60)}${T.hr}`
    : m >= 60 ? `${Math.floor(m / 60)}${T.hr} ${m % 60}${T.min}` : `${m}${T.min}`
}
const level = (p, warn, full) => p >= full ? 'full' : p >= warn ? 'hot' : ''
// a bar's fill: the width goes in from JS after render (data-p), the class picks the color
const bar = (p, warn, full) => `<span class="bar"><i class="${level(p, warn, full)}" data-p="${Math.min(100, Math.max(0, p))}"></i></span>`
// each account's color: its own from the settings, or the palette's by position
// a counter field is "path" or "name=path" (visitors.today, today=visitors.today)
const fieldName = (f) => f.includes('=') ? f.split('=')[0].trim() : f.split('.').pop()
const PALETTE = ['#60a5fa', '#a78bfa', '#f472b6', '#34d399', '#fbbf24', '#22d3ee']
const accColor = (i) => S.accounts[i]?.color || PALETTE[i % PALETTE.length]
// colors go in from JS (the CSP drops style=""): [data-acc] gets its account's color
function paintAccounts(root) {
  for (const el of root.querySelectorAll('[data-acc]')) {
    const c = accColor(Number(el.dataset.acc))
    // a session row gets a soft background in its account's color (CSS mixes it from --acc), a tag / letter the color itself
    if (el.classList.contains('row')) el.style.setProperty('--acc', c)
    else { el.style.color = c; el.style.borderColor = c }
  }
}

// ---- the look: theme, colors, opacity, language ----
const dark = window.matchMedia('(prefers-color-scheme: dark)')
// look = the saved settings, or (while the settings are open) a preview of the form; the language only changes on save
function applyLook(look = S) {
  const root = document.documentElement
  const theme = look.theme === 'auto' ? (dark.matches ? 'dark' : 'light') : look.theme
  root.classList.toggle('light', theme === 'light')
  for (const k of ['ok', 'warn', 'full', 'accent']) root.style.setProperty('--' + k, look.colors[k])
  root.style.setProperty('--alpha', String(Math.min(100, Math.max(30, look.opacity)) / 100))
  if (look !== S) return
  const lang = S.language === 'auto' ? (navigator.language || 'en').slice(0, 2) : S.language
  T = I18N[lang] || I18N.en
  root.lang = I18N[lang] ? lang : 'en'
}
dark.addEventListener('change', () => { if (S) { applyLook(); render() } })

// ---- the widget ----
function render() {
  if (!S || editing) return
  const shown = sessions.filter(s => S.showBackground || s.kind !== 'bg')
  const busy = shown.filter(s => state(s) === 'busy').length + (S.remote.enabled ? remoteBusy() : 0)
  const wait = shown.filter(s => state(s) === 'wait').length
  $('box').classList.toggle('waiting', wait > 0)
  document.body.classList.toggle('open', !!S.expanded)

  const values = Array.isArray(counter) ? counter.map(v => v ?? '–').join(' / ') : ''
  $('head').innerHTML =
    `<span class="dot ${wait ? 'wait' : busy ? 'busy' : 'idle'}" data-tauri-drag-region title="${T.sizeHint}"></span>` +
    `<span data-tauri-drag-region>${wait ? T.waiting(wait) : T.nobodyWaits}${busy ? ` <span class="muted">·</span> ${T.busy(busy)}` : ''}</span>` +
    `<span class="right">${values ? `<span class="counter-head" data-tauri-drag-region title="${esc(counterInfo())}">${esc(S.counter.label ? S.counter.label + ' ' : '')}${esc(values)}</span>` : ''}` +
    `<button class="gear" id="gear" title="${T.settings}">⚙</button></span>`

  // limits: one block per account (a limit belongs to the account; every session of it spends the same one)
  const th = S.thresholds
  // a window Claude Code didn't send (nothing used in it yet) or whose reset time has passed = 0%, not a gap
  const fresh = (b) => b && b.used_percentage != null && !(b.resets_at && b.resets_at * 1000 < Date.now())
  const lim = (label, b) => {
    const p = fresh(b) ? Math.round(b.used_percentage) : 0
    return `<span class="lbl">${label}</span>${bar(p, th.limitWarn, th.limitFull)}<span class="pct">${p}%</span>`
  }
  // the whole line's tooltip says everything (owner 2026-10-17: a dense layout hides the words, a hover brings them)
  const limInfo = (label, b) => {
    const p = fresh(b) ? Math.round(b.used_percentage) : 0
    return `${label}: ${p}%` + (fresh(b) && b.resets_at ? ` · ${T.resets} ${T.in} ${left(b.resets_at)}` : '')
  }
  $('limits').innerHTML = !S.showLimits ? '' : limits.length ? limits.map(l => {
    const info = [l.account, limInfo(T.h5, l.five_hour), limInfo(T.d7, l.seven_day)].join('\n')
    const resets = [fresh(l.five_hour) && l.five_hour.resets_at && `${T.h5} ${T.in} ${left(l.five_hour.resets_at)}`,
                    fresh(l.seven_day) && l.seven_day.resets_at && `${T.d7} ${T.in} ${left(l.seven_day.resets_at)}`].filter(Boolean).join(' · ')
    return `<div class="lim" title="${esc(info)}"><span class="tag" data-acc="${l.accountIndex}">${esc(l.account)}</span>
      ${lim(T.h5, l.five_hour)}${lim(T.d7, l.seven_day)}</div>
      ${resets ? `<div class="resets details">${T.resets}: ${resets}</div>` : ''}`
  }).join('') : `<div class="empty">${T.limitsLater}</div>`

  // sessions: one line each — status dot, name, account letter, context bar
  const many = S.accounts.length > 1
  $('list').innerHTML = shown.length ? shown.map(s => {
    const st = state(s)
    const ctx = s.context != null ? Math.round(s.context) : null
    const hint = ctx == null ? '' : ctx >= th.contextFull ? ' — ' + T.clear : ctx >= th.contextWarn ? ' — ' + T.compact : ''
    const doing = `${st === 'busy' ? T.working : st === 'wait' ? T.waitsForYou : T.free}${s.status === 'shell' ? ' · ' + T.background : ''} ${ago(s.since)}`
    // the row's tooltip: what the details line and the hidden parts would say (chips and dots show no words)
    const info = [s.name || folder(s.cwd), s.cwd, doing + (s.kind === 'bg' ? ' · ' + T.bg : ''),
      ctx != null ? `${T.context} ${ctx}%${hint}` : '', many ? s.account : ''].filter(Boolean).join('\n')
    return `<div class="row ${st}${many ? ' acc' : ''}" data-id="${esc(s.sessionId)}"${many ? ` data-acc="${s.accountIndex}"` : ''} title="${esc(info)}">
      <span class="dot ${st}"></span>
      <span class="name">${esc(s.name || folder(s.cwd))}</span>
      ${many ? `<span class="letter" data-acc="${s.accountIndex}">${esc((s.account || '?').slice(0, 1).toUpperCase())}</span>` : ''}
      ${S.showContext ? `<span class="ctxbar">${ctx != null
        ? bar(ctx, th.contextWarn, th.contextFull) + `<span class="pct">${ctx}%</span>` : '<span class="muted">—</span>'}</span>` : ''}
    </div>
    <div class="sub details">${esc(folder(s.cwd))}${s.kind === 'bg' ? ' · ' + T.bg : ''} · ${doing}${hint}</div>`
  }).join('') : `<div class="empty">${T.noSessions}</div>`

  $('counter').innerHTML = !S.counter.enabled ? '' : Array.isArray(counter)
    ? `<b>${esc(S.counter.label)}</b> ` + S.counter.fields.map((f, i) => `${esc(fieldName(f))} <b>${esc(counter[i] ?? '–')}</b>`).join(' · ')
    : `<b>${esc(S.counter.label)}</b> <span class="muted">${esc(counter === 'no-token' ? T.token + ': ' + T.tokenNone : counter || '…')}</span>`

  drawSystem()
  for (const i of document.querySelectorAll('.bar i')) i.style.width = i.dataset.p + '%'
  paintAccounts($('view'))
  fit()
}

// the counter's tooltip: every field with its name
const counterInfo = () => [S.counter.label, ...S.counter.fields.map((f, i) => `${fieldName(f)}: ${Array.isArray(counter) ? counter[i] ?? '–' : '–'}`)].filter(Boolean).join('\n')

// ---- the System section ----
// Three cards — CPU, RAM, GPU: each fills from the bottom with the load now, a line over it draws the last 2 minutes,
// the number on top. Under them a column per physical core, split into its threads (two thin bars side by side),
// each with a glowing cap; their color follows the load.
const tempLevel = (c) => c >= 80 ? 'full' : c >= 62 ? 'hot' : ''
const temp = (c) => c == null ? '' : `<span class="temp ${tempLevel(c)}">${Math.round(c)}°</span>`
const gb = (b) => (b / 1073741824).toFixed(b >= 10 * 1073741824 ? 0 : 1)
const SYS_COLORS = {cpu: 'var(--accent)', ram: '#fb923c', gpu: 'var(--ok)'}
// the last 2 minutes as a line in a 100×100 box; `top` = the value at the card's top (always 100%: the fill and the
// line must match the number — a scale that zoomed in on a quiet machine made 30% look nearly full)
// newest on the right edge, like Task Manager: a short history starts part-way in
const histLine = (h, top) => h.map((v, i) => `${((i + 60 - h.length) * 100 / 59).toFixed(1)},${(100 - Math.min(v, top) / top * 90).toFixed(1)}`).join(' ')
function sysCard(key, label, right, value, sub, now, hist, top, title = '') {
  return `<div class="scard" data-c="${key}" title="${esc(title)}">
    <div class="sfill" data-h="${Math.max(3, Math.min(now, top) / top * 90)}"></div>
    ${hist.length > 1 ? `<svg viewBox="0 0 100 100" preserveAspectRatio="none"><polyline points="${histLine(hist, top)}" vector-effect="non-scaling-stroke"/></svg>` : ''}
    <div class="shd"><span>${label}</span>${right}</div><div class="sval">${value}</div><div class="ssub">${sub}</div></div>`
}
function drawSystem() {
  drawSys($('system'), S.system.enabled ? sys : null, {cpu: cpuHist, mem: memHist, gpu: gpuHist}, S.system.threads)
}
// the cards + cores + one-line form of `sys` (this machine's or the remote one's) into `el`; hist = their 2 minutes
function drawSys(el, sys, hist, threadsOn) {
  if (!sys) { if (el.innerHTML) el.innerHTML = ''; return }
  const before = el.offsetHeight
  const cpu = Math.round(sys.cpu)
  const memP = sys.memTotal ? sys.memUsed / sys.memTotal * 100 : 0
  const g = sys.gpu
  const n = sys.cores.length
  // `.opt`: a card's side note that a narrow window drops (a temperature stays)
  // tooltips: the full numbers, also where a dense layout shows only a bar
  const deg = (c) => c == null ? '' : ` · ${Math.round(c)}°C`
  const tip = {
    cpu: `CPU ${cpu}% · ${T.cores(n)}${deg(sys.cpuTemp)}`,
    ram: `RAM ${gb(sys.memUsed)} / ${gb(sys.memTotal)} ${T.gb} (${Math.round(memP)}%)`,
    gpu: g ? `${g.name} ${Math.round(g.load)}%${deg(g.temp)}\nVRAM ${gb(g.memUsed)} / ${gb(g.memTotal)} ${T.gb}` : '',
  }
  const cards = [
    sysCard('cpu', 'CPU', temp(sys.cpuTemp) || `<span class="opt">${T.cores(n)}</span>`, `${cpu}%`, T.lastMin, cpu, hist.cpu, 100, tip.cpu),
    sysCard('ram', 'RAM', `<span class="opt">${gb(sys.memTotal)} ${T.gb}</span>`, `${gb(sys.memUsed)}<small> ${T.gb}</small>`, `${Math.round(memP)}%`, memP, hist.mem, 100, tip.ram),
    g ? sysCard('gpu', 'GPU', temp(g.temp), `${Math.round(g.load)}%`, esc(g.name.replace(/^NVIDIA (GeForce )?/i, '')), g.load, hist.gpu, 100, tip.gpu) : '',
  ].join('')
  // a core = its threads side by side; the tooltip has the numbers
  const cores = threadsOn && n ? `<div class="cores" data-n="${n}">${sys.cores.map((c, i) => {
    const th = sys.threads.slice(i * sys.perCore, (i + 1) * sys.perCore)
    return `<span class="core" title="${T.core} ${i + 1}: ${Math.round(c)}%${th.length > 1 ? ` (${T.threads}: ${th.map(t => Math.round(t) + '%').join(' · ')})` : ''}">` +
      th.map(t => `<i class="${level(t, 70, 90)}" data-h="${Math.max(5, t)}"></i>`).join('') + '</span>'
  }).join('')}</div>` : ''
  // the same numbers as one line, for the denser layouts (CSS shows it instead of the cards)
  const item = (label, p, text) => `<span class="si" title="${esc(tip[label.toLowerCase()])}"><b>${label}</b>${bar(p, 70, 90)}<span class="pct">${text}</span></span>`
  const line = `<div class="sysline">${item('CPU', cpu, cpu + '%')}${item('RAM', memP, Math.round(memP) + '%')}${g ? item('GPU', g.load, Math.round(g.load) + '%') : ''}</div>`
  el.innerHTML = `<div class="scards${g ? '' : ' two'}">${cards}</div>${cores}${line}`
  for (const i of el.querySelectorAll('.bar i')) i.style.width = i.dataset.p + '%'
  for (const c of el.querySelectorAll('.scard')) c.style.setProperty('--c', SYS_COLORS[c.dataset.c])
  for (const f of el.querySelectorAll('.sfill, .core i')) f.style.height = f.dataset.h + '%'
  const grid = el.querySelector('.cores')
  if (grid) grid.style.gridTemplateColumns = `repeat(${grid.dataset.n}, 1fr)`
  if (el.offsetHeight !== before) fit()
}
let sysTimer = null
const gpuHist = []
async function pollSystem() {
  clearTimeout(sysTimer)
  if (!S.system.enabled) { sys = null; cpuHist.length = memHist.length = gpuHist.length = 0; drawSystem(); return }
  const s = await invoke('system_stats').catch(() => null)
  if (s) {
    sys = s
    cpuHist.push(s.cpu)
    memHist.push(s.memTotal ? s.memUsed / s.memTotal * 100 : 0)
    if (s.gpu) gpuHist.push(s.gpu.load)
    for (const h of [cpuHist, memHist, gpuHist]) while (h.length > 60) h.shift()
    if (!editing) drawSystem()
  }
  sysTimer = setTimeout(pollSystem, 2000)
}

// ---- the other machine (settings.remote): its Claude runs and its load, a group of its own ----
let remoteSys = null, remoteRuns = null, remoteErr = ''
const rHist = {cpu: [], mem: [], gpu: []}
let remoteTimer = null
async function pollRemote() {
  clearTimeout(remoteTimer)
  const r = S.remote
  if (!r.enabled) { remoteSys = remoteRuns = null; remoteErr = ''; rHist.cpu.length = rHist.mem.length = rHist.gpu.length = 0; drawRemote(); return }
  const [h, runs] = await Promise.all([
    r.hostUrl ? invoke('remote_host').then(v => [v]).catch(e => [null, e]) : [null],
    r.runsUrl ? invoke('remote_runs').then(v => [v]).catch(e => [null, e]) : [null],
  ])
  remoteSys = h[0]
  remoteRuns = runs[0]
  remoteErr = [h[1], runs[1]].filter(Boolean).map(String).join(' · ')
  if (remoteSys) {
    rHist.cpu.push(remoteSys.cpu)
    rHist.mem.push(remoteSys.memTotal ? remoteSys.memUsed / remoteSys.memTotal * 100 : 0)
    if (remoteSys.gpu) rHist.gpu.push(remoteSys.gpu.load)
    for (const x of Object.values(rHist)) while (x.length > 30) x.shift()   // 2 minutes at 4 s
  }
  if (!editing) { drawRemote(); render() }
  remoteTimer = setTimeout(pollRemote, 4000)
}
const remoteBusy = () => (remoteRuns || []).filter(r => r.status === 'busy').length
function drawRemote() {
  const el = $('remote')
  const r = S.remote
  if (!r.enabled) { if (el.innerHTML) el.innerHTML = ''; return }
  const before = el.offsetHeight
  const since = (run) => Date.parse(run.status === 'busy' ? run.startedAt : run.finishedAt || run.startedAt) || Date.now()
  const label = r.label || T.remote
  const rows = (remoteRuns || []).map(run => {
    const st = run.status === 'busy' ? 'busy' : run.status === 'error' ? 'wait' : 'idle'
    const doing = `${run.status === 'busy' ? T.working : run.status === 'error' ? T.failed : T.finished} ${ago(since(run))}`
    return `<div class="row ${st}" title="${esc([run.name, doing, run.origin].filter(Boolean).join('\n'))}">
      <span class="dot ${st}"></span><span class="name">${esc(run.name || '#' + run.id)}</span><span class="muted rago">${ago(since(run))}</span></div>`
  }).join('')
  el.innerHTML = `<div class="rhead"><span class="tag" id="rtag">${esc(label)}</span>` +
    `<span class="muted">${remoteRuns ? (remoteBusy() ? T.busy(remoteBusy()) : T.nothingRuns) : ''}</span>` +
    `${remoteErr ? `<span class="rerr" title="${esc(remoteErr)}">!</span>` : ''}</div>${rows}<div class="rsys"></div>`
  const c = r.color || PALETTE[S.accounts.length % PALETTE.length]
  const tag = $('rtag'); tag.style.color = c; tag.style.borderColor = c
  drawSys(el.querySelector('.rsys'), remoteSys, rHist, true)
  if (el.offsetHeight !== before) fit()
}

// Size. Automatic (nothing dragged yet, or a double-click on the header): 300 px wide, the full layout, the height
// follows what the widget shows. Yours (an edge dragged): the width AND the height stay exactly where you let go — the
// layout is the least dense one that fits that height, picked live while you drag, so what you see while dragging is
// what stays (owner 2026-10-17). Room to spare: the System cards grow into it, or it stays empty. The window can't be
// dragged smaller than the densest layout (its minimum follows the content); content that outgrows even that (a new
// session) makes the window grow. Narrow widths drop side notes and numbers (CSS). No scrolling.
// The layouts, each a step denser (CSS body.denseN, cumulative, gentle first): 1 shorter cards → 2 no details →
// 3 tighter rows → 4 System as one line → 5 sessions as chips → 6 limits as text → 7 no System, dots → 8 no limits.
let fixed = null                     // {w, h} — your size, or null = automatic (settings.size)
let ourResize = 0
let dragging = null                  // the timer that ends a drag
const LEVELS = 8
// the content's real height: the lowest visible child's bottom (scrollHeight is never below the box's own height)
const contentOf = (el) => {
  const top = el.getBoundingClientRect().top - el.scrollTop
  let bottom = 0
  for (const c of el.children) {
    const r = c.getBoundingClientRect()
    if (r.height) bottom = Math.max(bottom, r.bottom - top + parseFloat(getComputedStyle(c).marginBottom || 0))
  }
  return bottom
}
// what the content needs, measured without the stretch into spare room
const natural = () => {
  document.body.classList.add('measuring')
  const h = $('head').offsetHeight + contentOf(editing ? $('settings') : $('view')) + 16
  document.body.classList.remove('measuring')
  return h
}
const setLevel = (level) => { for (let i = 1; i <= LEVELS; i++) document.body.classList.toggle('dense' + i, !editing && i <= level) }
// the least dense layout that fits the height `h` (the densest if none does); it stays set
function levelFor(h) {
  let level = 0
  setLevel(0)
  while (level < LEVELS && natural() > h + 1) setLevel(++level)
  return level
}
// the densest layout's height: the window's minimum
const smallest = () => { setLevel(LEVELS); return Math.ceil(natural()) }
async function fit() {
  if (dragging) return                // the mouse owns the size until it lets go
  ourResize = Date.now()
  if (editing) {
    setLevel(0)
    return win.setSize(new LogicalSize(Math.max(fixed ? fixed.w : 300, 340), Math.max(40, Math.min(natural(), 640)))).catch(() => {})
  }
  const min = Math.max(40, smallest())
  await win.setMinSize(new LogicalSize(220, min)).catch(() => {})
  let w = 300, h
  if (fixed) {
    // a size saved before 0.1.6 has no height: the full layout's (its old dense level opened cramped — owner
    // 2026-10-17), saved so it stays
    if (fixed.h == null) {
      setLevel(0)
      fixed.h = Math.round(natural())
      invoke('size_set', {size: {w: fixed.w, h: fixed.h, level: 0}}).catch(() => {})
    }
    levelFor(fixed.h)
    w = fixed.w
    h = Math.max(fixed.h, min, natural())
  } else {
    setLevel(0)
    h = Math.min(natural(), 900)
  }
  await win.setSize(new LogicalSize(w, Math.max(40, h))).catch(() => {})
}

// a click (not a drag) on the header toggles the details; ⚙ opens the settings; a click on a session = "seen"
let down = null
$('head').addEventListener('mousedown', e => { down = {x: e.screenX, y: e.screenY} })
function autoSize() {
  if (!fixed) return
  fixed = null
  clearTimeout(dragging)
  dragging = null
  invoke('size_set', {size: null}).catch(() => {})
  fit()
}
$('head').addEventListener('dblclick', e => { if (!e.target.closest('#gear')) autoSize() })
// the tray menu (tray.rs)
window.__TAURI__.event.listen('tray:settings', () => { if (!editing) openSettings() })
window.__TAURI__.event.listen('tray:autosize', autoSize)
$('head').addEventListener('mouseup', async e => {
  const click = down && Math.abs(e.screenX - down.x) < 4 && Math.abs(e.screenY - down.y) < 4
  down = null
  if (!click) return
  if (e.target.closest('#gear')) return openSettings()
  S.expanded = !S.expanded
  await invoke('settings_set', {settings: S}).catch(() => {})
  render()
})
// mousedown, not click: the list is redrawn while you hold the button, and a click whose row was replaced never fires
$('list').addEventListener('mousedown', e => {
  if (e.button !== 0) return
  const row = e.target.closest('.row')
  const s = row && sessions.find(x => x.sessionId === row.dataset.id)
  if (!s) return
  seen[s.sessionId] = s.since
  save('seen', seen)
  render()
  // bring up the terminal / IDE window the session runs in
  invoke('focus_session', {pid: s.pid, cwd: s.cwd}).catch(() => {})
})

// ---- settings ----
async function openSettings() {
  editing = true
  $('view').hidden = true
  const f = $('settings')
  f.hidden = false
  const auto = await invoke('plugin:autostart|is_enabled').catch(() => false)
  const hasToken = await invoke('counter_has_token').catch(() => false)
  const version = await window.__TAURI__.app?.getVersion().catch(() => '') ?? ''
  const opt = (v, cur, label) => `<option value="${v}"${v === cur ? ' selected' : ''}>${label}</option>`
  const num = (name, v) => `<input type="number" min="1" max="100" name="${name}" value="${v}">`
  const check = (name, on, label) => `<label><span>${label}</span><input type="checkbox" name="${name}"${on ? ' checked' : ''}></label>`
  f.innerHTML = `
    <h4>${T.look}</h4>
    <label><span>${T.language}</span><select name="language">${opt('auto', S.language, T.auto)}${opt('en', S.language, 'English')}${opt('ru', S.language, 'Русский')}</select></label>
    <label><span>${T.theme}</span><select name="theme">${opt('auto', S.theme, T.auto)}${opt('dark', S.theme, T.dark)}${opt('light', S.theme, T.light)}</select></label>
    <label><span>${T.opacity}</span><input type="range" min="30" max="100" name="opacity" value="${S.opacity}"></label>
    <label><span>${T.colors}</span>
      <input type="color" name="ok" value="${S.colors.ok}" title="${T.ok}"><input type="color" name="warn" value="${S.colors.warn}" title="${T.warn}">
      <input type="color" name="full" value="${S.colors.full}" title="${T.full}"><input type="color" name="accent" value="${S.colors.accent}" title="${T.accent}"></label>
    <h4>${T.thresholds}</h4>
    <label><span>${T.ctxWarn}</span>${num('contextWarn', S.thresholds.contextWarn)}</label>
    <label><span>${T.ctxFull}</span>${num('contextFull', S.thresholds.contextFull)}</label>
    <label><span>${T.limWarn}</span>${num('limitWarn', S.thresholds.limitWarn)}</label>
    <label><span>${T.limFull}</span>${num('limitFull', S.thresholds.limitFull)}</label>
    <h4>${T.show}</h4>
    ${check('showLimits', S.showLimits, T.showLimits)}${check('showContext', S.showContext, T.showContext)}${check('showBackground', S.showBackground, T.showBg)}
    ${check('autostart', auto, T.autostart)}
    <h4>${T.accounts}</h4>
    <div class="hint">${T.accountsHint}</div>
    <div id="accs"></div>
    <div style="display:flex;gap:6px"><button type="button" id="acc-add">${T.add}</button><button type="button" id="acc-find">${T.find}</button></div>
    <h4>${T.counter}</h4>
    <div class="hint">${T.counterHint}</div>
    ${check('counterEnabled', S.counter.enabled, T.enabled)}
    <label><span>${T.label}</span><input type="text" name="counterLabel" value="${esc(S.counter.label)}"></label>
    <div class="hint">${T.url}</div><input class="wide" type="text" name="counterUrl" value="${esc(S.counter.url)}" placeholder="https://…">
    <div class="hint">${T.passUrl}</div><input class="wide" type="text" name="counterPassUrl" value="${esc(S.counter.passUrl)}">
    <div class="hint">${T.fields}</div><input class="wide" type="text" name="counterFields" value="${esc(S.counter.fields.join(', '))}" placeholder="visitors.today, visitors.days7">
    <label><span>${T.every}</span><input type="number" min="1" max="1440" name="counterEvery" value="${S.counter.every || 5}"></label>
    <div class="hint">${T.token}: ${hasToken ? T.tokenSaved : T.tokenNone}</div><input class="wide" type="password" name="counterToken" autocomplete="off">
    <h4>${T.remote}</h4>
    <div class="hint">${T.remoteHint}</div>
    ${check('remoteEnabled', S.remote.enabled, T.enabled)}
    <label><span>${T.label}</span><input type="text" name="remoteLabel" value="${esc(S.remote.label)}" placeholder="server"><input type="color" name="remoteColor" value="${S.remote.color || PALETTE[S.accounts.length % PALETTE.length]}"></label>
    <div class="hint">${T.remoteHost}</div><input class="wide" type="text" name="remoteHost" value="${esc(S.remote.hostUrl)}" placeholder="http://…/api/v1/monitor/host">
    <div class="hint">${T.remoteRuns}</div><input class="wide" type="text" name="remoteRuns" value="${esc(S.remote.runsUrl)}" placeholder="http://…/api/admin/usage">
    <div class="hint">${T.passUrl}</div><input class="wide" type="text" name="remotePass" value="${esc(S.remote.passUrl)}">
    <h4>${T.system}</h4>
    ${check('sysEnabled', S.system.enabled, T.sysOn)}${check('sysThreads', S.system.threads, T.sysThreads)}${check('sysGpu', S.system.gpu, T.sysGpu)}
    <div class="hint">${T.sysHint}</div>
    <div class="hint version">cc-widget ${esc(version)}</div>
    <div class="actions"><button type="button" id="cancel">${T.cancel}</button><button type="submit" class="primary">${T.save}</button></div>`
  drawAccounts(S.accounts.map(a => ({...a})))
  for (const i of f.querySelectorAll('.bar i')) i.style.width = i.dataset.p + '%'
  await fit()
}

async function drawAccounts(list) {
  const box = $('accs')
  box.innerHTML = list.map((a, i) => `<div class="acc" data-i="${i}">
      <input type="color" name="color" value="${a.color || PALETTE[i % PALETTE.length]}" title="color">
      <input type="text" name="label" value="${esc(a.label)}"><input type="text" name="dir" value="${esc(a.dir)}">
      <span class="st" data-dir="${esc(a.dir)}"></span><button type="button" class="acc-del" title="×">×</button></div>`).join('')
  for (const span of box.querySelectorAll('.st')) {
    const st = await invoke('statusline_status', {dir: span.dataset.dir}).catch(() => 'off')
    span.innerHTML = st === 'connected'
      ? `<button type="button" class="acc-off">${T.disconnect}</button>`
      : st === 'other' ? `<span class="st-other" title="${T.other}">✓</span>`
      : `<button type="button" class="acc-on">${T.connect}</button>`
  }
  fit()
}
const readAccounts = () => [...document.querySelectorAll('#accs .acc')].map(r => ({
  label: r.querySelector('[name=label]').value.trim(), dir: r.querySelector('[name=dir]').value.trim(),
  color: r.querySelector('[name=color]').value,
})).filter(a => a.dir)

// the look changes live while you move the slider or pick a color; Cancel goes back to the saved one
$('settings').addEventListener('input', e => {
  const f = $('settings')
  if (!['theme', 'opacity', 'ok', 'warn', 'full', 'accent'].includes(e.target.name)) return
  applyLook({
    theme: f.elements.theme.value,
    opacity: Number(f.elements.opacity.value),
    colors: {ok: f.elements.ok.value, warn: f.elements.warn.value, full: f.elements.full.value, accent: f.elements.accent.value},
  })
})

$('settings').addEventListener('click', async e => {
  const t = e.target
  if (t.id === 'cancel') return closeSettings()
  if (t.id === 'acc-add') return drawAccounts([...readAccounts(), {label: 'claude', dir: '.claude'}])
  if (t.id === 'acc-find') {
    const found = await invoke('detect_accounts')
    const have = readAccounts()
    for (const dir of found) if (!have.some(a => a.dir === dir)) have.push({label: dir.replace(/^\.claude-?/, '') || 'claude', dir})
    return drawAccounts(have)
  }
  if (t.classList.contains('acc-del')) {
    const list = readAccounts()
    list.splice(Number(t.closest('.acc').dataset.i), 1)
    return drawAccounts(list)
  }
  if (t.classList.contains('acc-on') || t.classList.contains('acc-off')) {
    const dir = t.closest('.acc').querySelector('[name=dir]').value.trim()
    await invoke(t.classList.contains('acc-on') ? 'statusline_connect' : 'statusline_disconnect', {dir}).catch(err => alertIn(t, err))
    return drawAccounts(readAccounts())
  }
})
const alertIn = (el, err) => { el.title = String(err); el.textContent = '!' }

$('settings').addEventListener('submit', async e => {
  e.preventDefault()
  const f = e.target
  const v = (n) => f.elements[n]
  const pct = (n, d) => Math.min(100, Math.max(1, Number(v(n).value) || d))
  const next = {
    ...S,
    language: v('language').value,
    theme: v('theme').value,
    opacity: Number(v('opacity').value),
    colors: {ok: v('ok').value, warn: v('warn').value, full: v('full').value, accent: v('accent').value},
    thresholds: {contextWarn: pct('contextWarn', 70), contextFull: pct('contextFull', 85), limitWarn: pct('limitWarn', 70), limitFull: pct('limitFull', 90)},
    showLimits: v('showLimits').checked,
    showContext: v('showContext').checked,
    showBackground: v('showBackground').checked,
    accounts: readAccounts().map(a => ({label: a.label || a.dir, dir: a.dir, color: a.color})),
    counter: {
      ...S.counter,
      enabled: v('counterEnabled').checked,
      label: v('counterLabel').value.trim(),
      url: v('counterUrl').value.trim(),
      passUrl: v('counterPassUrl').value.trim(),
      fields: v('counterFields').value.split(',').map(s => s.trim()).filter(Boolean),
      every: Math.min(1440, Math.max(1, Number(v('counterEvery').value) || 5)),
    },
    remote: {enabled: v('remoteEnabled').checked, label: v('remoteLabel').value.trim(), color: v('remoteColor').value,
      hostUrl: v('remoteHost').value.trim(), runsUrl: v('remoteRuns').value.trim(), passUrl: v('remotePass').value.trim()},
    system: {enabled: v('sysEnabled').checked, threads: v('sysThreads').checked, gpu: v('sysGpu').checked},
  }
  await invoke('settings_set', {settings: next})
  // the token: typed = replace, a single space = forget, empty = keep
  const token = v('counterToken').value
  if (token) await invoke('counter_set_token', {token: token.trim()}).catch(() => {})
  await invoke(v('autostart').checked ? 'plugin:autostart|enable' : 'plugin:autostart|disable').catch(() => {})
  S = next
  closeSettings()
  pollCounter()
  pollSystem()
  pollRemote()
})

function closeSettings() {
  editing = false
  $('settings').hidden = true
  $('settings').innerHTML = ''
  $('view').hidden = false
  applyLook()
  render()
}

// ---- polling ----
async function pollSessions() {
  sessions = await invoke('sessions').catch(() => [])
  limits = await invoke('limits').catch(() => [])
  // forget "seen" marks of sessions that are gone
  const ids = new Set(sessions.map(s => s.sessionId))
  for (const k of Object.keys(seen)) if (!ids.has(k)) delete seen[k]
  save('seen', seen)
  // redraw only when something changed (the ages in the details tick by the minute)
  const key = JSON.stringify([sessions, limits, Math.floor(Date.now() / 60000)])
  if (key === lastPoll) return
  lastPoll = key
  render()
}
let lastPoll = ''
let counterTimer = null
async function pollCounter() {
  clearTimeout(counterTimer)
  if (!S.counter.enabled) { counter = null; render(); return }
  counter = await invoke('counter_fetch').catch(e => e === 'no-token' ? 'no-token' : T.noConnection + ': ' + e)
  render()
  counterTimer = setTimeout(pollCounter, Math.max(1, S.counter.every || 5) * 60 * 1000)
}

;(async () => {
  S = await invoke('settings_get')
  applyLook()
  // the window remembers where you put it
  // the window's place and size live in the settings file (saved at once); a value left in localStorage by an older
  // version is taken over once
  const oldPos = load('pos', null), oldSize = load('size', null)
  if (!S.pos && oldPos) { S.pos = oldPos; invoke('pos_set', oldPos).catch(() => {}) }
  if (!S.size && oldSize) { S.size = oldSize; invoke('size_set', {size: oldSize}).catch(() => {}) }
  try { localStorage.removeItem('pos'); localStorage.removeItem('size') } catch (e) {}
  fixed = S.size || null
  const pos = S.pos
  if (pos) await win.setPosition(new PhysicalPosition(pos.x, pos.y)).catch(() => {})
  // a move fires many events: save the last one a moment after it stops
  let moveTimer = null
  win.onMoved(({payload}) => {
    clearTimeout(moveTimer)
    moveTimer = setTimeout(() => invoke('pos_set', {x: payload.x, y: payload.y}).catch(() => {}), 300)
  })
  // a resize by you (not by fit()): from now on the size is yours; the layout follows every step, live
  win.onResized(async ({payload}) => {
    if (Date.now() - ourResize < 400 || editing) return
    const sf = await win.scaleFactor().catch(() => 1)
    const size = {w: Math.round(payload.width / sf), h: Math.round(payload.height / sf)}
    fixed = size
    clearTimeout(dragging)
    // let go → nothing moves (the layout is already the one for this height); the size is saved once, not every step
    dragging = setTimeout(() => {
      dragging = null
      invoke('size_set', {size: {...size, level: levelFor(size.h)}}).catch(() => {})
      fit()
    }, 450)
    requestAnimationFrame(() => levelFor(size.h))
  })
  try { localStorage.removeItem('width'); localStorage.removeItem('maxH') } catch (e) {}
  await pollSessions()
  pollCounter()
  pollSystem()
  pollRemote()
  setInterval(pollSessions, 2000)
})()
