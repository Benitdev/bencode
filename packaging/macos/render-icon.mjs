// Renders icon.svg to icon-1024.png with Chromium (Playwright), which draws
// the SVG's blur and shadow filters exactly as a browser does.
//
//   NODE_PATH="$(npm root -g)" node packaging/macos/render-icon.mjs
//
// `bundle.sh` builds AppIcon.icns from the PNG; re-render after editing the SVG.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = createRequire(import.meta.url)('playwright');
const here = dirname(fileURLToPath(import.meta.url));
const svg = readFileSync(join(here, 'icon.svg'), 'utf8');

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1024, height: 1024 } });
await page.setContent(`<html><body style="margin:0;background:transparent">${svg}</body></html>`);
await page.locator('svg').screenshot({ path: join(here, 'icon-1024.png'), omitBackground: true });
await browser.close();
