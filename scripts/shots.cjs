// README screenshots: the real ui/ with scripts/mock.js instead of Tauri, rendered by headless Chrome into docs/.
// node scripts/shots.cjs   (CHROME=<path> if Chrome isn't in the usual place)
const fs = require('fs'), path = require('path'), {execFileSync} = require('child_process')
const root = path.join(__dirname, '..')
const chrome = process.env.CHROME || [
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/usr/bin/google-chrome', '/usr/bin/chromium',
].find(p => fs.existsSync(p))
const page = fs.readFileSync(path.join(root, 'ui/index.html'), 'utf8')
  .replace('<link rel="stylesheet" href="widget.css">', '<link rel="stylesheet" href="../ui/widget.css"><style>html,body{min-height:100%}body{margin:0;padding:24px;box-sizing:border-box;min-height:100vh;background:linear-gradient(135deg,#2b3a55,#5b3f6b)}body.lightbg{background:linear-gradient(135deg,#dfe7f3,#efe2ef)}#box{width:300px;height:auto}</style>')
  .replace('<script src="i18n.js"></script>', '<script src="../scripts/mock.js"></script><script src="../ui/i18n.js"></script><script>if(location.search.includes("theme=light"))document.body.classList.add("lightbg");</script>')
  .replace('<script src="widget.js"></script>', '<script src="../ui/widget.js"></script>')
const tmp = path.join(root, 'docs', '_shot.html')
fs.writeFileSync(tmp, page)
const shots = [
  ['widget.png', '', 348, 330],
  ['widget-details.png', 'open=1', 348, 440],
  ['widget-light.png', 'theme=light', 348, 330],
  ['widget-settings.png', 'settings=1', 388, 760],
  ['widget-gpt.png', 'gpt=1&open=1', 348, 520],
  ['widget-gpt-ru-settings.png', 'gpt=1&settings=1&lang=ru', 388, 760],
  ['widget-ru.png', 'lang=ru&open=1', 348, 440],
  ['widget-remote.png', 'remote=1&open=1', 348, 760],
  ['widget-dense1.png', 'level=1', 348, 330],
  ['widget-dense2.png', 'level=2', 348, 300],
  ['widget-dense3.png', 'level=3', 348, 210],
]
for (const [name, query, w, h] of shots) {
  execFileSync(chrome, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=2',
    `--window-size=${w},${h}`, '--virtual-time-budget=1500', '--allow-file-access-from-files',
    `--screenshot=${path.join(root, 'docs', name)}`, 'file:///' + tmp.split(path.sep).join('/') + '?' + query], {stdio: 'ignore'})
  console.log('docs/' + name)
}
// docs/redesign/compare.png: the chosen mockup (left) and the real widget (right), same scale, same backdrop
const url = (p) => 'file:///' + p.split(path.sep).join('/')
const cmp = path.join(root, 'docs', '_compare.html')
fs.writeFileSync(cmp, `<!doctype html><meta charset="utf-8"><style>html{color-scheme:dark}body{margin:0;display:flex;height:100vh;
background:linear-gradient(135deg,#2b3a55,#5b3f6b)}iframe{border:0;background:transparent;height:100vh}
.m{padding:24px 24px 0;box-sizing:border-box;width:348px}.m iframe{width:300px}.w iframe{width:348px}</style>
<div class="m"><iframe src="${url(path.join(root, 'docs/redesign/windows11.html'))}"></iframe></div>
<div class="w"><iframe src="${url(tmp)}?sys=1&gpt=1&gemini=1&remote=1&bg=1&open=1"></iframe></div>`)
execFileSync(chrome, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=2',
  '--window-size=696,1000', '--virtual-time-budget=2500', '--allow-file-access-from-files',
  `--screenshot=${path.join(root, 'docs/redesign/compare.png')}`, url(cmp)], {stdio: 'ignore'})
console.log('docs/redesign/compare.png')
fs.unlinkSync(cmp)
fs.unlinkSync(tmp)
