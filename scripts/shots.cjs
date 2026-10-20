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
  ['widget.png', '', 348, 230],
  ['widget-details.png', 'open=1', 348, 330],
  ['widget-light.png', 'theme=light', 348, 230],
  ['widget-settings.png', 'settings=1', 388, 700],
  ['widget-ru.png', 'lang=ru&open=1', 348, 330],
  ['widget-remote.png', 'remote=1&open=1', 348, 470],
  ['widget-dense1.png', 'level=1', 348, 260],
  ['widget-dense2.png', 'level=2', 348, 200],
  ['widget-dense3.png', 'level=3', 348, 140],
]
for (const [name, query, w, h] of shots) {
  execFileSync(chrome, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=2',
    `--window-size=${w},${h}`, '--virtual-time-budget=1500', '--allow-file-access-from-files',
    `--screenshot=${path.join(root, 'docs', name)}`, 'file:///' + tmp.split(path.sep).join('/') + '?' + query], {stdio: 'ignore'})
  console.log('docs/' + name)
}
fs.unlinkSync(tmp)
