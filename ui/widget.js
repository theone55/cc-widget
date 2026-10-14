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
let editing = false
let T = I18N.en

const esc = (t) => String(t ?? '').replace(/[&<>"]/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;'}[c]))
const folder = (p) => String(p || '').split(/[\\/]/).filter(Boolean).pop() || p

// A session "waits" when its turn is over (idle) and you haven't clicked it since: the click is the "seen".
const state = (s) => s.status === 'busy' ? 'busy'
  : s.status === 'idle' ? ((seen[s.sessionId] || 0) >= s.since ? 'idle' : 'wait')
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
  const busy = shown.filter(s => state(s) === 'busy').length
  const wait = shown.filter(s => state(s) === 'wait').length
  $('box').classList.toggle('waiting', wait > 0)
  document.body.classList.toggle('open', !!S.expanded)

  const values = Array.isArray(counter) ? counter.map(v => v ?? '–').join(' / ') : ''
  $('head').innerHTML =
    `<span class="dot ${wait ? 'wait' : busy ? 'busy' : 'idle'}" data-tauri-drag-region title="${T.sizeHint}"></span>` +
    `<span data-tauri-drag-region>${wait ? T.waiting(wait) : T.nobodyWaits}${busy ? ` <span class="muted">·</span> ${T.busy(busy)}` : ''}</span>` +
    `<span class="right">${values ? `<span class="counter-head" data-tauri-drag-region title="${esc(S.counter.label)}">${esc(S.counter.label ? S.counter.label + ' ' : '')}${esc(values)}</span>` : ''}` +
    `<button class="gear" id="gear" title="${T.settings}">⚙</button></span>`

  // limits: one block per account (a limit belongs to the account; every session of it spends the same one)
  const th = S.thresholds
  // a window Claude Code didn't send (nothing used in it yet) or whose reset time has passed = 0%, not a gap
  const fresh = (b) => b && b.used_percentage != null && !(b.resets_at && b.resets_at * 1000 < Date.now())
  const lim = (label, b) => {
    const p = fresh(b) ? Math.round(b.used_percentage) : 0
    return `<span class="lbl">${label}</span>${bar(p, th.limitWarn, th.limitFull)}<span class="pct">${p}%</span>`
  }
  $('limits').innerHTML = !S.showLimits ? '' : limits.length ? limits.map(l => {
    const resets = [fresh(l.five_hour) && l.five_hour.resets_at && `${T.h5} ${T.in} ${left(l.five_hour.resets_at)}`,
                    fresh(l.seven_day) && l.seven_day.resets_at && `${T.d7} ${T.in} ${left(l.seven_day.resets_at)}`].filter(Boolean).join(' · ')
    return `<div class="lim"><span class="tag" data-acc="${l.accountIndex}" title="${esc(l.account)}">${esc(l.account)}</span>
      ${lim(T.h5, l.five_hour)}${lim(T.d7, l.seven_day)}</div>
      ${resets ? `<div class="resets details">${T.resets}: ${resets}</div>` : ''}`
  }).join('') : `<div class="empty">${T.limitsLater}</div>`

  // sessions: one line each — status dot, name, account letter, context bar
  const many = S.accounts.length > 1
  $('list').innerHTML = shown.length ? shown.map(s => {
    const st = state(s)
    const ctx = s.context != null ? Math.round(s.context) : null
    const hint = ctx == null ? '' : ctx >= th.contextFull ? ' — ' + T.clear : ctx >= th.contextWarn ? ' — ' + T.compact : ''
    return `<div class="row ${st}${many ? ' acc' : ''}" data-id="${esc(s.sessionId)}"${many ? ` data-acc="${s.accountIndex}"` : ''} title="${esc(s.cwd)}">
      <span class="dot ${st}"></span>
      <span class="name">${esc(s.name || folder(s.cwd))}</span>
      ${many ? `<span class="letter" data-acc="${s.accountIndex}" title="${esc(s.account)}">${esc((s.account || '?').slice(0, 1).toUpperCase())}</span>` : ''}
      ${S.showContext ? `<span class="ctxbar" title="${T.context} ${ctx ?? '?'}%${hint}">${ctx != null
        ? bar(ctx, th.contextWarn, th.contextFull) + `<span class="pct">${ctx}%</span>` : '<span class="muted">—</span>'}</span>` : ''}
    </div>
    <div class="sub details">${esc(folder(s.cwd))}${s.kind === 'bg' ? ' · ' + T.bg : ''} · ${st === 'busy' ? T.working : st === 'wait' ? T.waitsForYou : T.free} ${ago(s.since)}${hint}</div>`
  }).join('') : `<div class="empty">${T.noSessions}</div>`

  $('counter').innerHTML = !S.counter.enabled ? '' : Array.isArray(counter)
    ? `<b>${esc(S.counter.label)}</b> ` + S.counter.fields.map((f, i) => `${esc(fieldName(f))} <b>${esc(counter[i] ?? '–')}</b>`).join(' · ')
    : `<b>${esc(S.counter.label)}</b> <span class="muted">${esc(counter === 'no-token' ? T.token + ': ' + T.tokenNone : counter || '…')}</span>`

  for (const i of document.querySelectorAll('.bar i')) i.style.width = i.dataset.p + '%'
  paintAccounts($('view'))
  fit()
}

// Size. Auto (the default): the window is as tall as what it shows, 300 px wide.
// Yours: once you drag an edge, the window keeps exactly that size and the widget never resizes it (so it doesn't
// fight the mouse); the layout adapts LIVE on every resize step, so what you see while dragging is what stays.
// Double-click the header to go back to auto.
// No scrolling: when everything doesn't fit, it gets denser step by step — 1 no details + lower rows,
// 2 sessions as chips, 3 only the status dots. Narrow widths drop the numbers (CSS).
let fixed = load('size', null)       // {w, h} in logical px, or null = auto
let ourResize = 0
const DENSE = ['dense1', 'dense2', 'dense3']
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
const natural = () => $('head').offsetHeight + contentOf(editing ? $('settings') : $('view')) + 16
// the least dense layout that fits the window's height
function densify() {
  document.body.classList.remove(...DENSE)
  if (!fixed || editing) return
  for (const d of DENSE) {
    if (natural() <= $('box').clientHeight + 2) break
    document.body.classList.add(d)
  }
}
async function fit() {
  densify()
  // the settings get room of their own; your size comes back when they close (closeSettings)
  if (fixed && !editing) return
  ourResize = Date.now()
  await win.setSize(new LogicalSize(editing ? 340 : 300, Math.max(40, Math.min(natural(), 640)))).catch(() => {})
}

// a click (not a drag) on the header toggles the details; ⚙ opens the settings; a click on a session = "seen"
let down = null
$('head').addEventListener('mousedown', e => { down = {x: e.screenX, y: e.screenY} })
function autoSize() {
  if (!fixed) return
  fixed = null
  save('size', null)
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
$('list').addEventListener('click', e => {
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
    <div class="hint">${T.token}: ${hasToken ? T.tokenSaved : T.tokenNone}</div><input class="wide" type="password" name="counterToken" autocomplete="off">
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
    },
  }
  await invoke('settings_set', {settings: next})
  // the token: typed = replace, a single space = forget, empty = keep
  const token = v('counterToken').value
  if (token) await invoke('counter_set_token', {token: token.trim()}).catch(() => {})
  await invoke(v('autostart').checked ? 'plugin:autostart|enable' : 'plugin:autostart|disable').catch(() => {})
  S = next
  closeSettings()
  pollCounter()
})

function closeSettings() {
  editing = false
  if (fixed) { ourResize = Date.now(); win.setSize(new LogicalSize(fixed.w, fixed.h)).catch(() => {}) }
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
  render()
}
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
  const pos = load('pos', null)
  if (pos) await win.setPosition(new PhysicalPosition(pos.x, pos.y)).catch(() => {})
  win.onMoved(({payload}) => save('pos', {x: payload.x, y: payload.y}))
  // a resize by you (not by fit()): from now on the size is yours; the layout follows every step
  win.onResized(async ({payload}) => {
    if (Date.now() - ourResize < 400) return
    const sf = await win.scaleFactor().catch(() => 1)
    fixed = {w: Math.round(payload.width / sf), h: Math.round(payload.height / sf)}
    save('size', fixed)
    requestAnimationFrame(densify)
  })
  if (fixed) await win.setSize(new LogicalSize(fixed.w, fixed.h)).catch(() => {})
  try { localStorage.removeItem('width'); localStorage.removeItem('maxH') } catch (e) {}
  await pollSessions()
  pollCounter()
  setInterval(pollSessions, 2000)
})()
