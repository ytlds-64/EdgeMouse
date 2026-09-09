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
  const overview = element(), stop = element(), reconnect = element(), toggle = element(), label = element();
  const elements = new Map([
    ['.overview-connect-button', overview], ['.reconnect-button', reconnect],
    ['.overview-stop-button', stop],
    ['[data-service-toggle]', toggle], ['.service-state-label', label],
  ]);
  const snapshot = {
    desktopVersion: 'test', agent: { running: false },
    config: { valid: true, localName: 'Mac', peerScreenName: 'Windows', peerOn: 'left' },
    platform: { operatingSystem: 'macos', permissionGranted: false },
  };
  const calls = [], errors = [], intervals = [];
  const profiles = { 'mac-to-windows': { speed: 100 }, 'windows-to-mac': { speed: 100 } };
  let inputDirty = [];
  let finishConnect, finishStop, failStop;
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === 'get_app_snapshot') return snapshot;
    if (command === 'get_desktop_preferences') return {};
    if (command === 'repair_diagnostic') return { requiresUserAction: true, message: 'permission settings' };
    if (command === 'reconnect_agent') return new Promise((resolve) => { finishConnect = resolve; });
    if (command === 'set_agent_running') return new Promise((resolve, reject) => { finishStop = resolve; failStop = reject; });
    return {};
  };
  const documentHandlers = {};
  const document = {
    documentElement: { dataset: {} }, body: element(),
    querySelector: (selector) => elements.get(selector) ?? null,
    querySelectorAll: (selector) => selector.includes('.overview-connect-button,')
      ? [overview, stop, reconnect, toggle] : [],
    addEventListener(name, callback) { documentHandlers[name] = callback; },
  };
  const window = {
    EdgeMouseInputSettings: {
      setOverviewProfile() {}, getActiveProfile: () => 'mac-to-windows',
      getProfile: (name) => profiles[name], getDirtyFields: () => inputDirty,
      isDirty: () => inputDirty.length > 0,
      applyLocalProfile(name, settings) {
        for (const [key, value] of Object.entries(settings)) if (value !== undefined && !inputDirty.includes(key)) profiles[name][key] = value;
      },
      markSaved() { inputDirty = []; },
    },
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
  assert.ok(stop.disabled);
  await stop.click();
  assert.equal(calls.filter((call) => call.command === 'set_agent_running').length, 0);
  const first = overview.click();
  await reconnect.click();
  await overview.click();
  assert.equal(calls.filter((call) => call.command === 'reconnect_agent').length, 1);
  assert.ok(overview.disabled && stop.disabled && reconnect.disabled && toggle.disabled);
  await intervals[0](); // A status refresh must not unlock pending actions.
  assert.ok(overview.disabled && reconnect.disabled && toggle.disabled);
  snapshot.agent = { running: true, connection: { state: 'waiting_permission' } };
  finishConnect({ running: true, message: 'started' });
  await first;
  assert.equal(overview.textContent, '打开权限设置');
  assert.equal(reconnect.textContent, '打开权限设置');
  assert.ok(!toggle.disabled && toggle.classList.contains('is-on'));
  assert.ok(!stop.disabled); // Permission waits must remain stoppable.
  assert.match(label.textContent, /等待授权/);
  await overview.click();
  assert.equal(calls.filter((call) => call.command === 'reconnect_agent').length, 1);
  assert.equal(calls.find((call) => call.command === 'repair_diagnostic').args.action, 'open_permissions');
  for (const state of ['connecting', 'reconnecting', 'input_unavailable']) {
    snapshot.agent.connection.state = state;
    await intervals[0]();
    assert.ok(overview.disabled && reconnect.disabled);
    assert.ok(!toggle.disabled); // Explicit stop remains available.
    assert.ok(!stop.disabled);
  }
  snapshot.agent.connection.state = 'connected';
  await intervals[0]();
  assert.equal(overview.textContent, '重新连接');
  assert.ok(!overview.disabled);
  const failedStop = stop.click();
  assert.equal(stop.textContent, '正在停止…');
  await stop.click();
  await overview.click();
  await intervals[0]();
  assert.ok(overview.disabled && stop.disabled && reconnect.disabled && toggle.disabled);
  assert.equal(calls.filter((call) => call.command === 'set_agent_running').length, 1);
  failStop('test stop failed');
  await failedStop;
  assert.ok(!stop.disabled && toggle.classList.contains('is-on'));
  const stopped = stop.click();
  assert.equal(calls.filter((call) => call.command === 'set_agent_running').at(-1).args.running, false);
  snapshot.agent = { running: false };
  finishStop({ running: false, message: 'stopped' });
  await stopped;
  assert.ok(stop.disabled && !toggle.classList.contains('is-on'));
  assert.equal(overview.textContent, '开始连接');
  assert.deepEqual(errors, []);
  snapshot.config.pointerSpeed = 150;
  snapshot.config.sharedSettings = { values: { 15: 150, 16: 80 }, pending: false };
  await intervals[0]();
  assert.equal(profiles['mac-to-windows'].speed, 150);
  assert.equal(profiles['windows-to-mac'].speed, 80);
  profiles['mac-to-windows'].speed = 175;
  inputDirty = ['speed'];
  await intervals[0](); // Polling must not overwrite an unsaved speed edit.
  assert.equal(profiles['mac-to-windows'].speed, 175);
  documentHandlers['edgemouse:settings-change']({ detail: { section: 'input', profile: 'mac-to-windows', commit: true } });
  await new Promise(setImmediate);
  const savedInput = calls.find((call) => call.command === 'save_input_settings');
  assert.equal(savedInput.args.pointerSpeed, 175);
  assert.equal(savedInput.args.profile, 'mac-to-windows');
  assert.deepEqual(savedInput.args.fields, ['speed']);
  assert.equal(profiles['windows-to-mac'].speed, 80);
  assert.deepEqual(errors, []);
  console.log('Native service UI: connect/stop, duplicate clicks, polling, failures, permission wait, and recovery states passed');
  console.log('Native input UI: independent speed profiles, shared snapshots, dirty edits, and automatic save IPC passed');
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
