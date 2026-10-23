// The tray icons, each drawn by headless Chrome straight at the size it is shown at (a scaled one was blurry):
// assets/tray-color.svg at every Windows small-icon size (16 px at 100 % … 48 px at 300 %; Linux takes 24),
// assets/tray-template.svg 36 px high = the macOS menu bar's 18 pt on a Retina screen. tray.rs picks the one that fits.
// node scripts/tray-icons.cjs   (CHROME=<path> if Chrome isn't in the usual place)
const fs = require('fs'), path = require('path'), {execFileSync} = require('child_process')
const root = path.join(__dirname, '..'), out = path.join(root, 'src-tauri', 'icons', 'tray')
const chrome = process.env.CHROME || ['C:/Program Files/Google/Chrome/Application/chrome.exe',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/usr/bin/google-chrome'].find(p => fs.existsSync(p))
const url = p => 'file:///' + p.split(path.sep).join('/')
fs.mkdirSync(out, {recursive: true})
// Chrome draws an <img> of the SVG at the exact size (an SVG opened directly can come out cropped at small sizes)
const draw = (svgName, w, h, png) => {
  const tmp = path.join(out, '_tmp.html')
  fs.writeFileSync(tmp, `<body style="margin:0"><img src="${url(path.join(root, 'assets', svgName))}" style="display:block;width:${w}px;height:${h}px">`)
  execFileSync(chrome, ['--headless=new', '--disable-gpu', '--hide-scrollbars', '--default-background-color=00000000',
    `--window-size=${w},${h}`, `--screenshot=${path.join(out, png)}`, url(tmp)], {stdio: 'ignore'})
  fs.unlinkSync(tmp)
  console.log('src-tauri/icons/tray/' + png)
}
for (const size of [16, 20, 24, 32, 40, 48]) draw('tray-color.svg', size, size, `color-${size}.png`)
draw('tray-template.svg', 42, 36, 'template.png') // its viewBox is 88 × 76
