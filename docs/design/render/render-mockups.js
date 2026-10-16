// Renders every mockup in docs/design/ui-handoff/mockups to a PNG in ../screens, so agents
// and reviewers can look at the designs as images.
//
//   node docs/design/render/render-mockups.js [mockups-dir] [out-dir]
//
// Needs Playwright with a Chromium (CHROMIUM_PATH picks a specific binary). The mockups load
// the design tool's support.js, which isn't in the handoff, so it is served from shim.js.
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const handoff = path.join(__dirname, '..', 'ui-handoff');
const [dir = path.join(handoff, 'mockups'), out = path.join(handoff, 'screens')] =
  process.argv.slice(2);
const shim = fs.readFileSync(path.join(__dirname, 'shim.js'), 'utf8');

(async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH });
  fs.mkdirSync(out, { recursive: true });
  for (const file of fs.readdirSync(dir).filter((f) => f.endsWith('.html'))) {
    const html = fs.readFileSync(path.join(dir, file), 'utf8');
    const size = html.match(/"\$preview":\{"width":(\d+),"height":(\d+)\}/);
    const [width, height] = size ? [+size[1], +size[2]] : [1440, 900];
    const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
    await page.route('**/support.js', (route) =>
      route.fulfill({ contentType: 'text/javascript', body: shim }),
    );
    page.on('pageerror', (err) => console.log(file, 'error:', err.message));
    await page.goto('file://' + path.resolve(dir, file), { waitUntil: 'load' });
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(out, file.replace(/\.html$/, '.png')), fullPage: true });
    console.log('rendered', file, `${width}×${height}`);
  }
  await browser.close();
})();
