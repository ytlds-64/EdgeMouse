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
  const macDesktop = { originX: 0, originY: -1080, width: 1920, height: 2036, displays: [
    { originX: 0, originY: -1080, width: 1920, height: 1080, pixelWidth: 1920, pixelHeight: 1080, primary: false },
    { originX: 240, originY: 0, width: 1470, height: 956, pixelWidth: 2940, pixelHeight: 1912, primary: true },
  ] };
  const winDesktop = { originX: 0, originY: 0, width: 2160, height: 3840, displays: [
    { originX: 0, originY: 0, width: 2160, height: 3840, pixelWidth: 2160, pixelHeight: 3840, primary: true },
  ] };
  const initial = {
    preferences: { autostart: false, background: true, clipboardSync: true, notifications: false, theme: 'system', language: 'zh-CN', updateChannel: 'stable' },
    snapshot: {
      desktopVersion: '0.6.18', agent: { running: true, connection: { state: 'connected', peerDesktop: platform === 'macos' ? winDesktop : macDesktop } },
      config: { valid: true, localName: platform === 'macos' ? 'Mac' : 'Windows', peerScreenName: 'Other computer', peerOn: platform === 'macos' ? 'left' : 'right',
        autoReconnect: true, entryHysteresis: 8, layoutSyncPending: false, displayLayout: { layout: null, pending: false },
        sharedSettings: { pending: false, values: [1, 8, 1, 0, 0, 52, 1, 1, 1, 0, 0, 52, 1, 1, 1, 100, 100] } },
      platform: { operatingSystem: platform, permissionGranted: true, desktop: platform === 'macos' ? macDesktop : winDesktop, desktopWidth: 1920, desktopHeight: 1080, displayCount: 1 },
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
        state.snapshot.config.displayLayout = { layout: clone(args.displayLayout), pending: false };
        return { appliedLive: false, warning: '布局已保存；下次连接后将自动同步到另一台电脑，无需再次保存' };
      }
      if (command === 'save_connection_settings') { state.snapshot.config.autoReconnect = args.autoReconnect; return {}; }
      if (command === 'reset_preferences') {
        state.preferences = clone(initial.preferences);
        state.snapshot.config.displayLayout = { layout: null, pending: false };
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
      await page.locator('[data-general-setting="clipboardSync"]').click();
      await page.locator('#language').selectOption('en');
      await page.evaluate(() => testBackend.release('save_desktop_preferences'));
      await page.waitForFunction(() => EdgeMouseDesktopSettings.getDirtyFields().length === 0);
      assert.equal(await page.evaluate(() => testBackend.state.preferences.theme), 'light');
      assert.equal(await page.evaluate(() => testBackend.state.preferences.autostart), true);
      assert.equal(await page.evaluate(() => testBackend.state.preferences.clipboardSync), false);
      assert.equal(await page.locator('#language').inputValue(), 'en');
      assert.equal(await page.locator('[data-general-setting="notifications"]').getAttribute('aria-checked'), 'false');

      // System-style monitor dragging: exact overlap determines crossing regions.
      await navigate('layout');
      let layouts = await savedCount('save_layout');
      await page.locator('.layout-direction [data-edge="left"]').click();
      await waitWrite('save_layout', ++layouts);
      await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      const winTile = page.locator('.monitor-space [data-display-side="windows"]');
      const moveDisplay = async (side, index, x, y, cancel = false) => {
        const frame = await page.evaluate(([side, index]) => EdgeMouseLayout.get().displayLayout.positions[side][index], [side, index]);
        const tile = page.locator(`.monitor-space [data-display-side="${side}"]`).nth(index);
        await tile.hover(); // Wait for navigation animation and the next painted layout.
        const box = await tile.boundingBox(), scale = box.height / frame[3];
        const cx = box.x + box.width / 2, cy = box.y + box.height / 2;
        const count = await savedCount('save_layout');
        await page.mouse.move(cx, cy); await page.mouse.down();
        await page.mouse.move(cx + (x - frame[0]) * scale, cy + (y - frame[1]) * scale, { steps: 8 });
        assert.equal(await savedCount('save_layout'), count, 'drag preview never saves');
        if (cancel) await page.keyboard.press('Escape');
        await page.mouse.up();
      };
      const moveWindows = (x, y, cancel = false) => moveDisplay('windows', 0, x, y, cancel);
      const assertMacCentered = async () => {
        const boxes = await page.locator('.monitor-space [data-display-side="mac"]').evaluateAll((tiles) => tiles.map((tile) => { const r = tile.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y, bottom: r.bottom }; }));
        assert.ok(Math.abs(boxes[0].x - boxes[1].x) < 1, 'upper and lower Mac displays must share a centerline');
        assert.ok(Math.abs(boxes[0].bottom - boxes[1].y) < 1, 'centering must retain vertical edge contact');
      };
      await assertMacCentered();
      assert.equal((await page.evaluate(() => EdgeMouseDisplays.routes())).length, 1, 'the inset lower edge must not show a crossing across a gap');
      const centered = await page.evaluate(() => EdgeMouseLayout.get().displayLayout.positions.mac);
      const lower = centered[1], alignedX = -lower[2];
      await moveDisplay('mac', 1, alignedX, lower[1]);
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal((await page.evaluate(() => EdgeMouseDisplays.routes())).length, 2, 'manual edge alignment still connects both Mac displays');
      await moveDisplay('mac', 1, lower[0] + 8, lower[1]);
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      await assertMacCentered();
      assert.deepEqual(await page.evaluate(() => testBackend.state.snapshot.config.displayLayout.layout.positions.mac), centered, 'center snapping must save the actual centered positions');
      await moveDisplay('mac', 1, alignedX, lower[1]);
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal((await page.evaluate(() => EdgeMouseDisplays.routes())).length, 2);
      await moveWindows(0, -600);
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      let routes = await page.evaluate(() => EdgeMouseDisplays.routes());
      assert.equal(routes.length, 1); assert.equal(routes[0].b.actual[1], -1080);
      assert.ok(Math.abs(routes[0].from) < 0.001 && Math.abs(routes[0].to - 400) < 0.1, 'only touching part of the edge participates');
      await page.screenshot({ path: path.join(outputs, `EdgeMouse-${platform}-upper-only.png`) });
      await page.evaluate(() => { testBackend.holds.save_layout = true; });
      await moveWindows(0, 600);
      await page.waitForFunction(() => testBackend.waiters.save_layout);
      routes = await page.evaluate(() => EdgeMouseDisplays.routes());
      assert.equal(routes.length, 1); assert.equal(routes[0].b.actual[1], 0);
      await page.screenshot({ path: path.join(outputs, `EdgeMouse-${platform}-lower-only.png`) });
      await moveWindows(0, 0);
      await page.evaluate(() => testBackend.release('save_layout'));
      await waitWrite('save_layout', layouts += 2); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal((await page.evaluate(() => EdgeMouseDisplays.routes())).length, 2);
      await page.screenshot({ path: path.join(outputs, `EdgeMouse-${platform}-both-screens.png`) });
      await moveWindows(200, 0);
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal((await page.evaluate(() => EdgeMouseDisplays.routes())).length, 0, 'a gap disables crossing');
      const beforeCancel = await page.evaluate(() => EdgeMouseLayout.get());
      await moveWindows(200, 300, true);
      assert.deepEqual(await page.evaluate(() => EdgeMouseLayout.get()), beforeCancel);
      await moveWindows(-100, 0);
      assert.deepEqual(await page.evaluate(() => EdgeMouseLayout.get()), beforeCancel, 'overlapping drop reverts');
      assert.equal(await savedCount('save_layout'), layouts);
      await page.locator('[data-display-all]').click();
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      await winTile.focus(); await page.keyboard.press('Shift+ArrowUp');
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      assert.equal((await page.evaluate(() => EdgeMouseLayout.get())).displayLayout.positions.windows[0][1], -100);
      await page.locator('[data-display-all]').click();
      await waitWrite('save_layout', ++layouts); await page.waitForFunction(() => !EdgeMouseLayout.isDirty());
      await assertMacCentered();
      const savedDisplayLayout = await page.evaluate(() => EdgeMouseLayout.get().displayLayout);
      await page.locator('[data-layout-setting="edgeProtection"]').click();
      await waitWrite('save_layout', ++layouts);

      await navigate('connection');
      await page.locator('.auto-reconnect-toggle').click();
      await waitWrite('save_connection_settings', 1);
      assert.equal(await page.evaluate(() => testBackend.maxConcurrent()), 1);

      // Reopening hydrates saved values with zero new writes.
      await page.reload();
      await page.waitForFunction(() => !document.querySelector('[data-input-setting]').disabled);
      await navigate('settings');
      assert.equal(await page.locator('[data-general-setting="autostart"]').getAttribute('aria-checked'), 'true');
      assert.equal(await page.locator('[data-general-setting="clipboardSync"]').getAttribute('aria-checked'), 'false');
      assert.equal(await page.locator('#language').inputValue(), 'en');
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.getProfile('windows-to-mac').speed), 90);
      assert.equal(await page.evaluate(() => EdgeMouseInputSettings.getProfile('mac-to-windows').speed), 175);
      assert.equal(await savedCount('save_desktop_preferences'), 0);
      assert.equal(await savedCount('save_layout'), 0);
      assert.deepEqual((await page.evaluate(() => EdgeMouseLayout.get())).displayLayout, savedDisplayLayout);
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
      console.log(`${platform}: autosave, rapid edits, profiles, slider commit, retry, stale snapshots, monitor drag, centered arrangement and snapping, edge overlap, gap, cancel, collision, keyboard movement, restart hydration passed`);
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
    // With no peer geometry, the local Mac preview should still be symmetric,
    // without writing a layout or enabling a disconnected drag operation.
    const offline = await browser.newContext({ viewport: { width: 1470, height: 980 } });
    await offline.addInitScript(installBackend, 'macos');
    const offlinePage = await offline.newPage();
    offlinePage.on('pageerror', (error) => errors.push(String(error)));
    await offlinePage.goto(`http://127.0.0.1:${server.address().port}`);
    await offlinePage.waitForFunction(() => testBackend.waiters.get_app_snapshot && testBackend.waiters.get_desktop_preferences);
    await offlinePage.evaluate(() => {
      testBackend.state.snapshot.agent = { running: false };
      testBackend.release('get_app_snapshot'); testBackend.release('get_desktop_preferences');
    });
    await offlinePage.locator('[data-page="layout"]').click();
    await offlinePage.waitForFunction(() => document.querySelectorAll('.monitor-space [data-display-side="windows"]').length === 0
      && document.querySelectorAll('.monitor-space [data-display-side="mac"]').length === 2);
    await offlinePage.waitForFunction(() => Number(getComputedStyle(document.querySelector('#page-layout')).opacity) > 0.99);
    const offlineBoxes = await offlinePage.locator('.monitor-space [data-display-side="mac"]').evaluateAll((tiles) => tiles.map((tile) => { const r = tile.getBoundingClientRect(); return { center: r.x + r.width / 2, disabled: tile.disabled }; }));
    assert.ok(Math.abs(offlineBoxes[0].center - offlineBoxes[1].center) < 1, 'offline Mac lower display must be centered');
    assert.ok(offlineBoxes.every((box) => box.disabled));
    assert.equal(await offlinePage.evaluate(() => testBackend.calls.filter((call) => call.command === 'save_layout').length), 0, 'centering a preview must not change saved settings');
    await offlinePage.locator('.layout-canvas').screenshot({ animations: 'disabled', path: path.join(outputs, 'EdgeMouse-macos-centered-offline.png') });
    await offline.close();
    console.log('Offline Mac preview: centered lower screen, disabled drag and zero settings writes passed');
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
    await previewPage.locator('[data-page="layout"]').click();
    await previewPage.waitForFunction(() => document.querySelectorAll('.monitor-space .display-tile').length === 3);
    assert.equal(await previewPage.locator('.mac-map').evaluate((el) => getComputedStyle(el).backgroundColor), 'rgba(0, 0, 0, 0)', 'dark mode keeps physical monitors separate');
    await previewPage.waitForFunction(() => Number(getComputedStyle(document.querySelector('#page-layout')).opacity) > 0.99);
    await previewPage.screenshot({ animations: 'disabled', path: path.join(outputs, 'EdgeMouse-preview-dark-layout.png') });
    await previewPage.locator('.layout-direction [data-edge="left"]').click();
    await previewPage.locator('.monitor-space [data-display-side="windows"]').focus();
    await previewPage.keyboard.press('Shift+ArrowUp');
    const previewLayout = await previewPage.evaluate(() => EdgeMouseLayout.get().displayLayout);
    await previewPage.reload();
    assert.deepEqual((await previewPage.evaluate(() => EdgeMouseLayout.get())).displayLayout, previewLayout, 'standalone preview restores screen positions');
    console.log('Standalone preview: automatic local persistence and reopening passed');
    await preview.close();
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
    server.close();
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
