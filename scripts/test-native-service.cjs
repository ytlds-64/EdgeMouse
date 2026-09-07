// Exercise the real native UI handlers with a minimal DOM and deferred IPC.
// No running app, user configuration, permissions, or network are changed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function element() {
  const classes = new Set();
  return {
    dataset: {}, disabled: false, textContent: '', handlers: {}, attributes: {},
    classList: {
      contains: (name) => classes.has(name),
      toggle(name, on) { if (on) classes.add(name); else classes.delete(name); },
      remove: (name) => classes.delete(name),
    },
    setAttribute(name, value) { this.attributes[name] = value; },
    removeAttribute(name) { delete this.attributes[name]; },
    addEventListener(name, callback) { this.handlers[name] = callback; },
    async click() {
      // Deliberately deliver even disabled clicks to test the handler's guard.
      await this.handlers.click?.({ currentTarget: this, stopImmediatePropagation() {} });
    },
  };
}

async function main() {
  const overview = element(), reconnect = element(), toggle = element(), label = element();
  const elements = new Map([
    ['.overview-connect-button', overview], ['.reconnect-button', reconnect],
    ['[data-service-toggle]', toggle], ['.service-state-label', label],
  ]);
  const snapshot = {
    desktopVersion: 'test', agent: { running: false },
    config: { valid: true, localName: 'Mac', peerScreenName: 'Windows', peerOn: 'left' },
    platform: { operatingSystem: 'macos', permissionGranted: false },
  };
  const calls = [], errors = [], intervals = [];
  let finishConnect;
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === 'get_app_snapshot') return snapshot;
    if (command === 'get_desktop_preferences') return {};
    if (command === 'repair_diagnostic') return { requiresUserAction: true, message: 'permission settings' };
    if (command === 'reconnect_agent') return new Promise((resolve) => { finishConnect = resolve; });
    return {};
  };
  const document = {
    documentElement: { dataset: {} }, body: element(),
    querySelector: (selector) => elements.get(selector) ?? null,
    querySelectorAll: (selector) => selector.includes('.overview-connect-button,')
      ? [overview, reconnect, toggle] : [],
    addEventListener() {},
  };
  const window = {
    __TAURI__: { core: { invoke } }, localStorage: { getItem() { return null; } },
    setInterval: (callback) => intervals.push(callback), setTimeout, clearTimeout,
    addEventListener() {},
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../ui-prototype/native.js'), 'utf8'), {
    window, document, navigator: { userAgent: 'Macintosh' },
    console: { error: (...args) => errors.push(args), log() {} }, setTimeout, clearTimeout,
  });
  await new Promise(setImmediate);
  assert.equal(overview.textContent, '开始连接');
  const first = overview.click();
  await reconnect.click();
  await overview.click();
  assert.equal(calls.filter((call) => call.command === 'reconnect_agent').length, 1);
  assert.ok(overview.disabled && reconnect.disabled && toggle.disabled);
  await intervals[0](); // A status refresh must not unlock pending actions.
  assert.ok(overview.disabled && reconnect.disabled && toggle.disabled);
  snapshot.agent = { running: true, connection: { state: 'waiting_permission' } };
  finishConnect({ running: true, message: 'started' });
  await first;
  assert.equal(overview.textContent, '打开权限设置');
  assert.equal(reconnect.textContent, '打开权限设置');
  assert.ok(!toggle.disabled && toggle.classList.contains('is-on'));
  assert.match(label.textContent, /等待授权/);
  await overview.click();
  assert.equal(calls.filter((call) => call.command === 'reconnect_agent').length, 1);
  assert.equal(calls.find((call) => call.command === 'repair_diagnostic').args.action, 'open_permissions');
  for (const state of ['connecting', 'reconnecting', 'input_unavailable']) {
    snapshot.agent.connection.state = state;
    await intervals[0]();
    assert.ok(overview.disabled && reconnect.disabled);
    assert.ok(!toggle.disabled); // Explicit stop remains available.
  }
  snapshot.agent.connection.state = 'connected';
  await intervals[0]();
  assert.equal(overview.textContent, '重新连接');
  assert.ok(!overview.disabled);
  assert.deepEqual(errors, []);
  console.log('Native service UI: duplicate clicks, polling, permission wait, and recovery states passed');
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
