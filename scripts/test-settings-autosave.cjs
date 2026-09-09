// Browser regression tests against the actual UI with deferred, simulated IPC.
// Requires Playwright and a browser; never touches user settings or input services.
// Optional: EDGEMOUSE_TEST_BROWSER=/path/to/browser, EDGEMOUSE_TEST_OUTPUT=/path.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('playwright');
const root = path.resolve(__dirname, '../ui-prototype');
const outputs = process.env.EDGEMOUSE_TEST_OUTPUT || path.resolve(__dirname, '../work/autosave-test');
fs.mkdirSync(outputs, { recursive: true });
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function installBackend(platform) {
  const initial = {
    preferences: { autostart: false, background: true, notifications: false, theme: 'system', language: 'zh-CN', updateChannel: 'stable' },
    snapshot: {
      desktopVersion: '0.6.12', agent: { running: false },
      config: { valid: true, localName: platform === 'macos' ? 'Mac' : 'Windows', peerScreenName: 'Other computer', peerOn: platform === 'macos' ? 'left' : 'right',
        autoReconnect: true, entryHysteresis: 8, layoutSyncPending: false,
        sharedSettings: { pending: false, values: [1, 8, 1, 0, 0, 52, 1, 1, 1, 0, 0, 52, 1, 1, 1, 100, 100] } },
      platform: { operatingSystem: platform, permissionGranted: true, desktopWidth: 1920, desktopHeight: 1080, displayCount: 1 },
    },
  };
  const state = JSON.parse(localStorage.getItem('autosave-test-backend') || 'null') || JSON.parse(JSON.stringify(initial));
  const calls = [], holds = {}, waiters = {}, fail = {};
  if (!localStorage.getItem('autosave-test-backend')) { holds.get_app_snapshot = true; holds.get_desktop_preferences = true; }
  let concurrent = 0, maxConcurrent = 0;
  const clone = (value) => JSON.parse(JSON.stringify(value));
  window.testBackend = {
    state, calls, holds, waiters, fail,
    maxConcurrent: () => maxConcurrent,
    release: (command) => { const done = waiters[command]; delete waiters[command]; done(); },
  };
  window.__TAURI__ = { core: { invoke: async (command, args) => {
    calls.push({ command, args: args && clone(args) });
    const writing = command.startsWith('save_') || command === 'reset_preferences';
    if (writing) { concurrent++; maxConcurrent = Math.max(maxConcurrent, concurrent); }
    const capturedSnapshot = command === 'get_app_snapshot' ? clone(state.snapshot) : null;
    try {
      if (holds[command]) {
        holds[command] = false;
        await new Promise((resolve) => { waiters[command] = resolve; });
      }
      if (fail[command]) { fail[command] = false; throw new Error('simulated disk write failure'); }
      if (command === 'get_app_snapshot') return capturedSnapshot;
      if (command === 'get_desktop_preferences') return clone(state.preferences);
      if (command === 'save_desktop_preferences') {
        state.preferences = { ...args, notifications: false }; // Model a declined notification permission.
        return { preferences: clone(state.preferences), message: '桌面设置已保存', notificationReady: false };
      }
      if (command === 'save_input_settings') {
        const base = args.profile === 'mac-to-windows' ? 3 : 9;
        const mappings = { horizontal: ['reverseHorizontal', base], vertical: ['reverseVertical', base + 1], smoothing: ['pointerSmoothing', base + 2],
          keyboard: ['keyboardEnabled', base + 3], reclaim: ['reclaimEnabled', base + 4], dragLock: ['dragLock', base + 5], speed: ['pointerSpeed', base === 3 ? 15 : 16] };
        for (const field of args.fields) {
          const [name, index] = mappings[field];
          state.snapshot.config.sharedSettings.values[index] = Number(args[name]);
        }
        return { restarted: false, warning: null };
      }
      if (command === 'save_layout') {
        state.snapshot.config.peerOn = platform === 'macos' ? ({ left: 'right', right: 'left', top: 'bottom', bottom: 'top' })[args.peerOn] : args.peerOn;
        state.snapshot.config.entryHysteresis = args.edgeProtection ? 8 : 0;
        return { appliedLive: false, warning: '布局已保存；下次连接后将自动同步到另一台电脑，无需再次保存' };
      }
      if (command === 'save_connection_settings') { state.snapshot.config.autoReconnect = args.autoReconnect; return {}; }
      if (command === 'reset_preferences') {
        state.preferences = clone(initial.preferences);
        state.snapshot.config.sharedSettings.values = clone(initial.snapshot.config.sharedSettings.values);
        return { preferences: clone(state.preferences), message: '已恢复推荐设置' };
      }
      return {};
    } finally {
      if (writing) { concurrent--; localStorage.setItem('autosave-test-backend', JSON.stringify(state)); }
    }
  } } };
}

async function main() {
  const server = http.createServer((req, res) => {
    const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
    const file = path.resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
    if (!file.startsWith(root + path.sep)) { res.writeHead(403).end(); return; }
    try {
      const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
      res.setHeader('Content-Type', types[path.extname(file)] || 'application/octet-stream');
      res.end(fs.readFileSync(file));
    } catch { res.writeHead(404).end(); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: true, executablePath: process.env.EDGEMOUSE_TEST_BROWSER || undefined }).catch((error) => { server.close(); throw error; });
  const errors = [];
  try {
    for (const platform of ['macos', 'windows']) {
      const context = await browser.newContext({ viewport: { width: 1470, height: 980 }, userAgent: platform === 'windows' ? 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36' : 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36' });
      await context.addInitScript(installBackend, platform);
      const page = await context.newPage();
      page.on('pageerror', (error) => errors.push(String(error)));
      const navigate = (name) => page.locator(`[data-page="${name}"]`).click();
      const savedCount = (command) => page.evaluate((cmd) => testBackend.calls.filter((c) => c.command === cmd).length, command);
      const waitWrite = (command, count) => page.waitForFunction(([cmd, n]) => testBackend.calls.filter((c) => c.command === cmd).length >= n, [command, count]);
      await page.goto(`http://127.0.0.1:${server.address().port}`);
      await page.waitForFunction(() => testBackend.waiters.get_app_snapshot && testBackend.waiters.get_desktop_preferences);
      await navigate('input');
      assert.equal(await page.locator('[data-input-setting="keyboard"]').isDisabled(), true);
      await page.locator('[data-profile="windows-to-mac"]').click();
      assert.equal(await page.locator('[data-input-setting="keyboard"]').isDisabled(), true, 'profile selection cannot bypass hydration');
      await page.evaluate(() => { testBackend.release('get_app_snapshot'); testBackend.release('get_desktop_preferences'); });
      await page.waitForFunction(() => !document.querySelector('[data-input-setting]').disabled);
      assert.equal(await page.locator('.save-button').count(), 0);
      assert.equal(await page.evaluate(() => document.documentElement.dataset.desktopPlatform), platform);
      assert.equal(await savedCount('save_input_settings'), 0, 'hydration must not write defaults');

      // Overview edits apply immediately to the local direction only.
      await navigate('overview');
      await page.locator('[data-overview-setting="horizontal"]').click();
      await waitWrite('save_input_settings', 1);
      assert.equal(await page.evaluate(() => testBackend.calls.find((c) => c.command === 'save_input_settings').args.profile), platform === 'macos' ? 'mac-to-windows' : 'windows-to-mac');
      await navigate('input');
      await page.locator('[data-profile="mac-to-windows"]').click();
      await page.locator('[data-input-choice="trigger"] [data-value="push"]').click();
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.isDirty()), false, 'fixed trigger must not create an unsavable edit');
      let count = await savedCount('save_input_settings');
      await page.locator('#pointer-speed').evaluate((el) => { el.value = '175'; el.dispatchEvent(new Event('input', { bubbles: true })); });
      await delay(1150);
      assert.equal(await savedCount('save_input_settings'), count, 'slider preview must not save');
      await page.locator('#pointer-speed').dispatchEvent('change');
      await waitWrite('save_input_settings', count + 1);

      // A same-field edit during a held save must survive its old acknowledgement.
      await page.evaluate(() => { testBackend.holds.save_input_settings = true; });
      await page.locator('[data-input-setting="keyboard"]').click();
      await page.waitForFunction(() => !!testBackend.waiters.save_input_settings);
      await page.locator('[data-input-setting="keyboard"]').click();
      await page.locator('[data-profile="windows-to-mac"]').click();
      await page.locator('#pointer-speed').evaluate((el) => { el.value = '85'; el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); });
      await page.evaluate(() => testBackend.release('save_input_settings'));
      await page.waitForFunction(() => !EdgeMouseInputSettings.isDirty());
      assert.equal(await page.evaluate(() => testBackend.state.snapshot.config.sharedSettings.values[6]), 1);
      assert.equal(await page.evaluate(() => testBackend.state.snapshot.config.sharedSettings.values[15]), 175);
      assert.equal(await page.evaluate(() => testBackend.state.snapshot.config.sharedSettings.values[16]), 85);

      // Failed writes stay visibly dirty through polling and can be retried.
      await page.evaluate(() => { testBackend.fail.save_input_settings = true; });
      await page.locator('[data-input-setting="vertical"]').click();
      const retry = page.locator('#page-input [data-retry-settings="input"]');
      await retry.waitFor({ state: 'visible' });
      await delay(1150);
      assert.equal(await retry.isVisible(), true);
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.isDirty()), true);
      await retry.click();
      await page.waitForFunction(() => !EdgeMouseInputSettings.isDirty());
      assert.equal(await retry.isVisible(), false);

      // A snapshot captured before saving must not undo the new value when it arrives late.
      await page.evaluate(() => { testBackend.holds.get_app_snapshot = true; });
      await page.waitForFunction(() => !!testBackend.waiters.get_app_snapshot);
      await page.locator('#pointer-speed').evaluate((el) => { el.value = '90'; el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); });
      await page.waitForFunction(() => !EdgeMouseInputSettings.isDirty());
      await page.evaluate(() => testBackend.release('get_app_snapshot'));
      await delay(60);
      assert.equal(await page.locator('#pointer-speed').inputValue(), '90');

      // Desktop changes during a held save must remain selected and persist in order.
      await navigate('settings');
      await page.evaluate(() => { testBackend.holds.save_desktop_preferences = true; });
      await page.locator('[data-theme="dark"]').click();
      await page.waitForFunction(() => !!testBackend.waiters.save_desktop_preferences);
      await page.locator('[data-theme="light"]').click();
      await page.locator('[data-general-setting="autostart"]').click();
      await page.locator('[data-general-setting="notifications"]').click();
      await page.locator('#language').selectOption('en');
      await page.evaluate(() => testBackend.release('save_desktop_preferences'));
      await page.waitForFunction(() => EdgeMouseDesktopSettings.getDirtyFields().length === 0);
      assert.equal(await page.evaluate(() => testBackend.state.preferences.theme), 'light');
      assert.equal(await page.evaluate(() => testBackend.state.preferences.autostart), true);
      assert.equal(await page.locator('#language').inputValue(), 'en');
      assert.equal(await page.locator('[data-general-setting="notifications"]').getAttribute('aria-checked'), 'false');

      // Layout clicks save once; pointer motion is only a preview, cancel is not a commit.
      await navigate('layout');
      let layouts = await savedCount('save_layout');
      await page.locator('.layout-direction [data-edge="right"]').click();
      await delay(50);
      assert.equal(await savedCount('save_layout'), layouts, 'unchanged direction must not save');
      await page.locator('.layout-direction [data-edge="top"]').click();
      await waitWrite('save_layout', layouts + 1);
      await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      const mac = await page.locator('.layout-canvas .screen-mac').boundingBox();
      const win = await page.locator('.layout-canvas .screen-win').boundingBox();
      await page.mouse.move(mac.x + mac.width / 2, mac.y + mac.height / 2);
      await page.mouse.down();
      await page.mouse.move(win.x + win.width + 140, win.y + win.height / 2, { steps: 12 });
      await delay(1150);
      assert.equal(await savedCount('save_layout'), layouts + 1, 'dragging must not save or reconnect');
      await page.mouse.up();
      await waitWrite('save_layout', layouts + 2);
      await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal(await page.evaluate(() => EdgeMouseLayout.getEdge()), 'right');
      const screen = page.locator('.layout-canvas .screen-mac');
      await screen.dispatchEvent('pointerdown', { button: 0, pointerId: 77, clientX: 100, clientY: 100 });
      await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointermove', { pointerId: 77, clientX: 200, clientY: 400 })));
      await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel', { pointerId: 77 })));
      assert.equal(await savedCount('save_layout'), layouts + 2, 'cancelled drag must not commit');
      await page.locator('[data-layout-setting="edgeProtection"]').click();
      await waitWrite('save_layout', layouts + 3);

      await navigate('connection');
      await page.locator('.auto-reconnect-toggle').click();
      await waitWrite('save_connection_settings', 1);
      assert.equal(await page.evaluate(() => testBackend.maxConcurrent()), 1);

      // Reopening hydrates saved values with zero new writes.
      await page.reload();
      await page.waitForFunction(() => !document.querySelector('[data-input-setting]').disabled);
      await navigate('settings');
      assert.equal(await page.locator('[data-general-setting="autostart"]').getAttribute('aria-checked'), 'true');
      assert.equal(await page.locator('#language').inputValue(), 'en');
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.getProfile('windows-to-mac').speed), 90);
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.getProfile('mac-to-windows').speed), 175);
      assert.equal(await savedCount('save_desktop_preferences'), 0);
      assert.equal(await savedCount('save_layout'), 0);
      await page.locator('#language').selectOption('zh-CN');
      await page.waitForFunction(() => EdgeMouseDesktopSettings.getDirtyFields().length === 0);
      for (const name of ['settings', 'input', 'layout']) {
        await navigate(name);
        await page.waitForFunction((pageName) => Number(getComputedStyle(document.querySelector(`#page-${pageName}`)).opacity) > 0.99, name);
        await page.screenshot({ animations: 'disabled', path: path.join(outputs, `EdgeMouse-${platform}-${name}-autosave.png`) });
        await page.setViewportSize({ width: 1040, height: 900 });
        // Windows deliberately paints one pixel past the transparent window edge.
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1
          && document.querySelector('.content').scrollWidth <= document.querySelector('.content').clientWidth), true, 'minimum window width must not clip page content');
        await page.screenshot({ animations: 'disabled', path: path.join(outputs, `EdgeMouse-${platform}-${name}-compact.png`) });
        await page.setViewportSize({ width: 1470, height: 980 });
      }
      console.log(`${platform}: autosave, rapid edits, profiles, slider commit, retry, stale snapshots, layout drag/cancel, restart hydration passed`);
      // Reset waits for an in-flight save, and locks edits until it completes.
      await navigate('settings');
      await page.evaluate(() => { testBackend.holds.save_desktop_preferences = true; });
      await page.locator('[data-theme="dark"]').click();
      await page.waitForFunction(() => !!testBackend.waiters.save_desktop_preferences);
      await page.locator('.reset-settings-button').click();
      await page.locator('.confirm-reset-button').click();
      assert.equal(await savedCount('reset_preferences'), 0);
      assert.equal(await page.locator('[data-theme="light"]').isDisabled(), true);
      await page.evaluate(() => testBackend.release('save_desktop_preferences'));
      await waitWrite('reset_preferences', 1);
      await page.waitForFunction(() => !document.querySelector('[data-theme="light"]').disabled);
      assert.equal(await page.evaluate(() => testBackend.state.preferences.theme), 'system');
      assert.equal(await page.evaluate(() => EdgeMouseDesktopSettings.getDirtyFields().length), 0);
      assert.equal(await page.evaluate(() => testBackend.maxConcurrent()), 1);
      console.log(`${platform}: guarded hydration and reset ordering passed`);
      await context.close();
    }
    const preview = await browser.newContext({ viewport: { width: 1470, height: 980 } });
    const previewPage = await preview.newPage();
    previewPage.on('pageerror', (error) => errors.push(String(error)));
    await previewPage.goto(`http://127.0.0.1:${server.address().port}`);
    await previewPage.locator('[data-page="settings"]').click();
    await previewPage.locator('[data-theme="dark"]').click();
    await previewPage.locator('[data-page="layout"]').click();
    await previewPage.locator('.layout-direction [data-edge="bottom"]').click();
    await previewPage.reload();
    assert.equal(await previewPage.evaluate(() => EdgeMouseDesktopSettings.get().theme), 'dark');
    assert.equal(await previewPage.evaluate(() => EdgeMouseLayout.getEdge()), 'bottom');
    console.log('Standalone preview: automatic local persistence and reopening passed');
    await preview.close();
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
    server.close();
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
