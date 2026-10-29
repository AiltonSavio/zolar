/** Layout regression against the running shop. No wallet connection, order or payment is created.
 * BASE=http://localhost:5177 PW=/path/to/playwright-core node scripts/check-payment-dialog.mjs
 */
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PW ?? 'playwright-core');
const root = fileURLToPath(new URL('..', import.meta.url));
const fixture = await mkdtemp(join(root, '.modal-layout-'));
const out = process.env.OUT ?? join(tmpdir(), 'zolar-payment-dialog');
await mkdir(out, { recursive: true });
await writeFile(join(fixture, 'main.tsx'), `
import React from 'react';
import { createRoot } from 'react-dom/client';
import { PaymentDialog } from '../src/components/PaymentDialog';
import '../src/styles.css';
const state = new URLSearchParams(location.search).get('state');
createRoot(document.getElementById('root')!).render(<PaymentDialog open onClose={() => {}} order={null}
  item="General admission" price={100000} payout="888Hn7eBJFUcAvLPYQhgva9jSk1eG21QSsTRiCN5EkwE" network="testnet"
  approvalUrl={state === 'preparing' ? null : 'http://127.0.0.1:7877/approve/layout-only'}
  onOpenWallet={() => {}} opened={state === 'opened'} blocked={state === 'blocked'} />);
`);
const browser = await chromium.launch({ ...(process.env.CHROME ? { executablePath: process.env.CHROME } : {}) });
const results = [];
try {
  for (const [width, height] of [[1512, 714], [1440, 900], [1366, 650], [390, 844], [390, 664], [320, 568]]) {
    const page = await browser.newPage({ viewport: { width, height } });
    await page.route('**/src/main.tsx*', async route => {
      const response = await route.fetch({ url: `${process.env.BASE ?? 'http://localhost:5177'}/${fixture.split('/').at(-1)}/main.tsx` });
      await route.fulfill({ response });
    });
    for (const state of ['ready', 'blocked', 'opened', 'preparing']) {
      await page.goto(`${process.env.BASE ?? 'http://localhost:5177'}/?state=${state}`);
      await page.getByRole('dialog', { name: 'Pay with Zcash' }).waitFor();
      await page.evaluate(() => document.fonts.ready);
      await page.locator('dialog').evaluate(async dialog => { await Promise.all(dialog.getAnimations().map(animation => animation.finished.catch(() => {}))); });
      const bounds = await page.locator('dialog').evaluate(dialog => {
        const box = dialog.getBoundingClientRect();
        const button = dialog.querySelector('.dialog-primary').getBoundingClientRect();
        const footnote = dialog.querySelector('.modal-footnote').getBoundingClientRect();
        return { height: box.height, clientHeight: dialog.clientHeight, scrollHeight: dialog.scrollHeight,
          clientWidth: dialog.clientWidth, scrollWidth: dialog.scrollWidth, top: box.top, bottom: box.bottom,
          buttonHeight: button.height, buttonBottom: button.bottom, footnoteBottom: footnote.bottom };
      });
      results.push({ viewportWidth: width, viewportHeight: height, state, ...bounds });
      await page.screenshot({ path: join(out, `${width}x${height}-${state}.png`) });
      await writeFile(join(out, 'measurements.json'), JSON.stringify(results, null, 2));
      assert.ok(bounds.scrollHeight <= bounds.clientHeight + 1, `${width}x${height} ${state}: modal scrolls vertically`);
      assert.ok(bounds.scrollWidth <= bounds.clientWidth + 1, `${width}x${height} ${state}: modal scrolls horizontally`);
      assert.ok(bounds.top >= 0 && bounds.bottom <= height, 'modal fits viewport');
      assert.ok(bounds.buttonHeight >= 44 && bounds.buttonBottom < bounds.bottom, 'approval action fits and remains touch sized');
      assert.ok(bounds.footnoteBottom < bounds.bottom, 'privacy note is visible without scrolling');
    }
    await page.close();
  }
  console.log(`PASS: ${results.length} payment modal viewport/state checks. Screenshots: ${out}`);
} finally { await browser.close(); await rm(fixture, { recursive: true, force: true }); }
