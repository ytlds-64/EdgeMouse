const navItems = [...document.querySelectorAll(".nav-item")];
const pages = [...document.querySelectorAll(".page")];
const toast = document.querySelector(".toast");
let appVersion = document.querySelector('meta[name="edgemouse-version"]')?.content ?? "0.6.18";
let toastTimer;

const sidebarToggle = document.querySelector(".sidebar-toggle");
function setSidebarCollapsed(collapsed) {
  document.querySelector(".app-window").classList.toggle("sidebar-collapsed", collapsed);
  sidebarToggle.setAttribute("aria-expanded", String(!collapsed));
  const sourceLabel = collapsed ? "展开侧栏" : "收起侧栏";
  const label = window.EdgeMouseI18n?.translate(sourceLabel) ?? sourceLabel;
  sidebarToggle.setAttribute("aria-label", label);
  sidebarToggle.title = label;
  sidebarToggle.querySelector("span").textContent = label;
  navItems.forEach((item) => { item.title = item.querySelector("span").textContent; });
  try { localStorage.setItem("edgemouse-sidebar-collapsed", String(collapsed)); } catch { /* Optional local UI preference. */ }
}
try { setSidebarCollapsed(localStorage.getItem("edgemouse-sidebar-collapsed") === "true"); } catch { setSidebarCollapsed(false); }
sidebarToggle.addEventListener("click", () => setSidebarCollapsed(sidebarToggle.getAttribute("aria-expanded") === "true"));

document.querySelectorAll("[data-app-version]").forEach((element) => {
  element.textContent = appVersion;
});

window.setEdgeMouseAppVersion = (version) => {
  if (!version) return;
  appVersion = version;
  document.querySelector('meta[name="edgemouse-version"]')?.setAttribute("content", version);
  document.querySelectorAll("[data-app-version]").forEach((element) => {
    element.textContent = version;
  });
};

function showToast(message, kind = "auto") {
  const isError = kind === "error" || (kind === "auto" && /^(无法|失败|错误|配对失败|设备操作失败|诊断失败)/.test(message));
  toast.classList.toggle("is-error", isError);
  toast.querySelector("span").textContent = isError ? "!" : "✓";
  toast.querySelector("b").textContent = message;
  toast.classList.add("is-visible");
  window.clearTimeout(toastTimer);
  const duration = Math.min(8000, Math.max(1800, String(message).length * 75));
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), duration);
}

window.showEdgeMouseToast = showToast;

function showPage(name) {
  navItems.forEach((item) => item.classList.toggle("is-active", item.dataset.page === name));
  pages.forEach((page) => page.classList.toggle("is-active", page.id === `page-${name}`));
  const current = pages.find((page) => page.id === `page-${name}`);
  document.title = `EdgeMouse · ${current?.dataset.title ?? "桌面应用"}`;
  document.querySelector(".content").scrollTo({ top: 0, behavior: "smooth" });
}

navItems.forEach((item) => item.addEventListener("click", () => showPage(item.dataset.page)));
document.querySelectorAll("[data-go]").forEach((button) => button.addEventListener("click", () => showPage(button.dataset.go)));

document.querySelectorAll(".toggle").forEach((toggle) => {
  toggle.addEventListener("click", () => {
    const enabled = toggle.classList.toggle("is-on");
    toggle.setAttribute("aria-checked", String(enabled));
  });
});

document.querySelectorAll(".segmented").forEach((group) => {
  group.querySelectorAll("button").forEach((button) => {
    button.addEventListener("click", () => {
      group.querySelectorAll("button").forEach((item) => item.classList.remove("is-selected"));
      button.classList.add("is-selected");
    });
  });
});

const inputProfiles = {
  "mac-to-windows": {
    horizontal: true,
    vertical: false,
    smoothing: 64,
    speed: 100,
    keyboard: true,
    reclaim: true,
    dragLock: true,
    trigger: "push",
    primary: "Windows Ctrl",
    secondary: "Windows Alt",
    language: "Windows Shift + Space",
  },
  "windows-to-mac": {
    horizontal: false,
    vertical: false,
    smoothing: 52,
    speed: 100,
    keyboard: true,
    reclaim: true,
    dragLock: true,
    trigger: "push",
    primary: "Mac Command",
    secondary: "Mac Option",
    language: "Mac 中 / 英",
  },
};

const inputProfileMeta = {
  "mac-to-windows": {
    horizontalDescription: "修正 macOS 触控板控制 Windows 时的方向",
    sources: { primary: "Mac Command", secondary: "Mac Option", language: "Mac 中 / 英" },
    choices: {
      primary: ["Windows Ctrl", "Windows 键", "保持原键"],
      secondary: ["Windows Alt", "Windows Ctrl", "保持原键"],
      language: ["Windows Shift + Space", "Windows 键 + Space", "不映射"],
    },
  },
  "windows-to-mac": {
    horizontalDescription: "单独调整 Windows 鼠标在 macOS 中的滚动方向",
    sources: { primary: "Windows Ctrl", secondary: "Windows Alt", language: "Windows Shift + Space" },
    choices: {
      primary: ["Mac Command", "Mac Control", "保持原键"],
      secondary: ["Mac Option", "Mac Command", "保持原键"],
      language: ["Mac 中 / 英", "Mac Control + Space", "不映射"],
    },
  },
};

let activeInputProfile = "mac-to-windows";
let overviewInputProfile = "mac-to-windows";
let inputSettingsDirty = false;
const inputDirtyFields = { "mac-to-windows": new Set(), "windows-to-mac": new Set() };
const inputEditingFields = { "mac-to-windows": new Set(), "windows-to-mac": new Set() };
const smoothingRange = document.querySelector("#pointer-smoothing");
const smoothingOutput = document.querySelector('output[for="pointer-smoothing"]');
const speedRange = document.querySelector("#pointer-speed");
const speedOutput = document.querySelector('output[for="pointer-speed"]');
const inputSaveStatus = document.querySelector(".input-save-status");

function settingsChanged(section, detail = {}) {
  document.dispatchEvent(new CustomEvent("edgemouse:settings-change", {
    detail: { section, commit: true, ...detail },
  }));
}

function smoothingLabel(value) {
  if (value < 34) return "跟手";
  if (value < 68) return "均衡";
  return "更平滑";
}

function setToggleState(toggle, enabled) {
  toggle.classList.toggle("is-on", enabled);
  toggle.setAttribute("aria-checked", String(enabled));
}

function markInputSettingsDirty(field, profile = activeInputProfile, commit = true) {
  // Fixed mappings and the already-selected trigger have no editable field.
  if (!field) return;
  if (field) inputDirtyFields[profile].add(field);
  if (field) {
    if (commit) inputEditingFields[profile].delete(field);
    else inputEditingFields[profile].add(field);
  }
  inputSettingsDirty = true;
  inputSaveStatus.textContent = commit ? "正在自动保存…" : "调整完成后自动应用";
  inputSaveStatus.classList.add("is-dirty");
  settingsChanged("input", { profile, commit });
}

function syncOverviewInputSettings() {
  const profile = inputProfiles[overviewInputProfile];
  document.querySelectorAll("[data-overview-setting]").forEach((toggle) => {
    setToggleState(toggle, profile[toggle.dataset.overviewSetting]);
  });
}

function renderInputProfile() {
  const profile = inputProfiles[activeInputProfile];
  const meta = inputProfileMeta[activeInputProfile];
  document.querySelectorAll(".input-profile-button").forEach((button) => {
    const selected = button.dataset.profile === activeInputProfile;
    button.classList.toggle("is-selected", selected);
    button.setAttribute("aria-selected", String(selected));
  });
  document.querySelectorAll("[data-input-setting]").forEach((toggle) => {
    const key = toggle.dataset.inputSetting;
    setToggleState(toggle, profile[key]);
    toggle.setAttribute("aria-label", toggle.closest(".setting-row").querySelector("strong").textContent);
    toggle.title = "修改后自动保存并同步到另一台电脑";
  });
  document.querySelector('[data-input-description="horizontal"]').textContent = meta.horizontalDescription;
  smoothingRange.value = String(profile.smoothing);
  smoothingRange.title = "调整完成后自动保存并同步到另一台电脑";
  smoothingOutput.textContent = smoothingLabel(profile.smoothing);
  speedRange.value = String(profile.speed);
  speedOutput.textContent = `${profile.speed}%`;
  speedRange.setAttribute("aria-valuetext", `${profile.speed}%`);
  document.querySelectorAll("[data-map-source]").forEach((source) => {
    source.textContent = meta.sources[source.dataset.mapSource];
  });
  document.querySelectorAll("[data-input-map]").forEach((select) => {
    const key = select.dataset.inputMap;
    select.replaceChildren(...meta.choices[key].map((choice) => new Option(choice, choice)));
    select.value = profile[key];
    select.disabled = true;
    select.title = "当前版本会自动采用经过验证的跨平台映射";
  });
  document.querySelectorAll('[data-input-choice="trigger"] button').forEach((button) => {
    button.classList.toggle("is-selected", button.dataset.value === profile.trigger);
  });
  if (!inputSettingsDirty) {
    inputSaveStatus.textContent = "修改后自动保存并同步；两个控制方向独立设置";
    inputSaveStatus.classList.remove("is-dirty");
  }
}

document.querySelectorAll(".input-profile-button").forEach((button) => {
  button.addEventListener("click", () => {
    activeInputProfile = button.dataset.profile;
    renderInputProfile();
  });
});

document.querySelectorAll("[data-input-setting]").forEach((toggle) => {
  toggle.addEventListener("click", () => {
    inputProfiles[activeInputProfile][toggle.dataset.inputSetting] = toggle.classList.contains("is-on");
    markInputSettingsDirty(toggle.dataset.inputSetting);
    syncOverviewInputSettings();
    if (toggle.dataset.inputSetting === "keyboard") renderInputProfile();
  });
});

document.querySelectorAll("[data-overview-setting]").forEach((toggle) => {
  toggle.addEventListener("click", () => {
    inputProfiles[overviewInputProfile][toggle.dataset.overviewSetting] = toggle.classList.contains("is-on");
    markInputSettingsDirty(toggle.dataset.overviewSetting, overviewInputProfile);
    document.querySelector(".overview-save-status").textContent = "正在自动保存…";
    if (activeInputProfile === overviewInputProfile) renderInputProfile();
  });
});

smoothingRange.addEventListener("input", () => {
  inputProfiles[activeInputProfile].smoothing = Number(smoothingRange.value);
  smoothingOutput.textContent = smoothingLabel(Number(smoothingRange.value));
  markInputSettingsDirty("smoothing", activeInputProfile, false);
});

speedRange.addEventListener("input", () => {
  const speed = Number(speedRange.value);
  inputProfiles[activeInputProfile].speed = speed;
  speedOutput.textContent = `${speed}%`;
  speedRange.setAttribute("aria-valuetext", `${speed}%`);
  markInputSettingsDirty("speed", activeInputProfile, false);
});

smoothingRange.addEventListener("change", () => markInputSettingsDirty("smoothing"));
speedRange.addEventListener("change", () => markInputSettingsDirty("speed"));

document.querySelectorAll("[data-input-map]").forEach((select) => {
  select.addEventListener("change", () => {
    inputProfiles[activeInputProfile][select.dataset.inputMap] = select.value;
    markInputSettingsDirty();
  });
});

document.querySelectorAll('[data-input-choice="trigger"] button').forEach((button) => {
  button.addEventListener("click", () => {
    inputProfiles[activeInputProfile].trigger = button.dataset.value;
    markInputSettingsDirty();
  });
});

window.EdgeMouseInputSettings = {
  getActiveProfile() {
    return activeInputProfile;
  },
  getProfile(name) {
    const profile = inputProfiles[name];
    return profile ? { ...profile } : undefined;
  },
  setOverviewProfile(name) {
    if (!inputProfiles[name]) return;
    overviewInputProfile = name;
    const label = document.querySelector("[data-overview-profile-label]");
    if (label) label.textContent = name === "windows-to-mac" ? "Windows → Mac" : "Mac → Windows";
    syncOverviewInputSettings();
  },
  getOverviewProfile() {
    return overviewInputProfile;
  },
  getDirtyFields(name) { return [...inputDirtyFields[name]].filter((field) => !inputEditingFields[name].has(field)); },
  isDirty() { return inputSettingsDirty; },
  applyLocalProfile(name, settings) {
    const profile = inputProfiles[name];
    if (!profile) return;
    for (const key of ["horizontal", "vertical", "smoothing", "speed", "keyboard", "reclaim", "dragLock"]) {
      if (!inputDirtyFields[name].has(key) && typeof settings[key] === (["smoothing", "speed"].includes(key) ? "number" : "boolean")) profile[key] = settings[key];
    }
    if (activeInputProfile === name) renderInputProfile();
    syncOverviewInputSettings();
  },
  markSaved(message, name = activeInputProfile, fields = [...inputDirtyFields[name]], submitted = inputProfiles[name]) {
    fields.forEach((field) => {
      if (inputProfiles[name][field] === submitted[field]) inputDirtyFields[name].delete(field);
    });
    inputSettingsDirty = Object.values(inputDirtyFields).some((fields) => fields.size > 0);
    inputSaveStatus.textContent = message;
    inputSaveStatus.classList.toggle("is-dirty", inputSettingsDirty);
  },
  clearDirty() {
    Object.values(inputDirtyFields).forEach((fields) => fields.clear());
    Object.values(inputEditingFields).forEach((fields) => fields.clear());
    inputSettingsDirty = false;
  },
};

renderInputProfile();
syncOverviewInputSettings();

const connectionStatusChip = document.querySelector(".connection-status-chip");
const connectionDeviceStatus = document.querySelector(".connection-device-status");
const reconnectButton = document.querySelector(".reconnect-button");
const autoDiscoveryToggle = document.querySelector(".auto-discovery-toggle");
const discoveryState = document.querySelector(".discovery-state");
const moreButton = document.querySelector(".more-button");
const deviceMenu = document.querySelector(".device-menu");
const pairingModal = document.querySelector(".pairing-modal");
const pairingModeButtons = [...document.querySelectorAll("[data-pairing-mode]")];
const pairingPanels = [...document.querySelectorAll(".pairing-panel")];
const pairingSteps = [...document.querySelectorAll(".pairing-step")];
const discoveredDevice = document.querySelector(".discovered-device");
const manualAddress = document.querySelector("#manual-peer-address");
let discoveryTimer;
let pairingVerificationTimer;

function setConnectionState(state) {
  const connecting = state === "connecting";
  connectionStatusChip.classList.toggle("good", !connecting);
  connectionStatusChip.classList.toggle("pending", connecting);
  connectionStatusChip.querySelector("b").textContent = connecting ? "正在重新连接" : "安全连接正常";
  connectionDeviceStatus.classList.toggle("is-connecting", connecting);
  connectionDeviceStatus.querySelector("b").textContent = connecting ? "正在寻找可信设备…" : "已连接 · Wi‑Fi";
}

function setPairingStep(stepName) {
  pairingSteps.forEach((step) => step.classList.toggle("is-active", step.classList.contains(`pairing-step-${stepName}`)));
}

function setPairingMode(mode) {
  pairingModeButtons.forEach((button) => {
    const selected = button.dataset.pairingMode === mode;
    button.classList.toggle("is-selected", selected);
    button.setAttribute("aria-selected", String(selected));
  });
  pairingPanels.forEach((panel) => panel.classList.toggle("is-active", panel.classList.contains(`pairing-${mode}`)));
  if (mode === "manual") window.setTimeout(() => manualAddress.focus(), 0);
}

function startPairingDiscovery() {
  window.clearTimeout(discoveryTimer);
  discoveredDevice.hidden = true;
  discoveryTimer = window.setTimeout(() => {
    discoveredDevice.hidden = false;
    document.querySelector(".scan-status strong").textContent = "发现 1 台可配对设备";
    document.querySelector(".scan-status small").textContent = "已通过局域网广播验证设备响应";
  }, 650);
}

function openPairingModal() {
  window.clearTimeout(pairingVerificationTimer);
  pairingModal.hidden = false;
  setPairingStep("discover");
  setPairingMode("auto");
  manualAddress.value = "";
  document.querySelector(".field-error").hidden = true;
  document.querySelector(".scan-status strong").textContent = "正在查找附近设备…";
  document.querySelector(".scan-status small").textContent = "请确保另一台设备已打开 EdgeMouse";
  document.querySelector(".pairing-confirm-button").disabled = false;
  document.querySelector(".pairing-confirm-button").textContent = "配对码一致";
  startPairingDiscovery();
  document.querySelector(".modal-close").focus();
}

function closePairingModal() {
  window.clearTimeout(discoveryTimer);
  window.clearTimeout(pairingVerificationTimer);
  document.querySelector(".pairing-confirm-button").disabled = false;
  document.querySelector(".pairing-confirm-button").textContent = "配对码一致";
  pairingModal.hidden = true;
  document.querySelector(".pair-device-button").focus();
}

function showPairingCode(name, address, method) {
  document.querySelector(".pairing-target-name").textContent = name;
  document.querySelector(".pairing-target-address").textContent = `${method} · ${address}`;
  setPairingStep("code");
  document.querySelector(".pairing-confirm-button").focus();
}

function validIpv4(value) {
  const parts = value.trim().split(".");
  return parts.length === 4 && parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255);
}

reconnectButton?.addEventListener("click", () => {
  reconnectButton.disabled = true;
  reconnectButton.textContent = "正在重新连接…";
  setConnectionState("connecting");
  window.setTimeout(() => {
    setConnectionState("connected");
    reconnectButton.disabled = false;
    reconnectButton.textContent = "立即重新连接";
    document.querySelector(".peer-address").textContent = "自动获取 · 192.168.8.202";
    showToast("已通过自动发现重新连接可信设备");
  }, 900);
});

autoDiscoveryToggle?.addEventListener("click", () => {
  const enabled = autoDiscoveryToggle.classList.contains("is-on");
  discoveryState.classList.toggle("is-paused", !enabled);
  discoveryState.querySelector("strong").textContent = enabled ? "正在监听可信设备" : "自动发现已暂停";
  discoveryState.querySelector("small").textContent = enabled ? "UDP 43892 · 地址变化自动更新" : "将继续使用最后一次已知地址";
  document.querySelector(".discovery-detail").textContent = enabled ? "局域网自动发现" : "使用最后已知地址";
});

moreButton?.addEventListener("click", () => {
  const open = deviceMenu.hidden;
  deviceMenu.hidden = !open;
  moreButton.setAttribute("aria-expanded", String(open));
});

document.addEventListener("click", (event) => {
  if (!event.target.closest(".device-more")) {
    deviceMenu.hidden = true;
    moreButton?.setAttribute("aria-expanded", "false");
  }
});

document.querySelectorAll("[data-device-action]").forEach((button) => {
  button.addEventListener("click", () => {
    const messages = { copy: "连接信息已复制（原型演示）", verify: "可信证书验证通过", forget: "解除配对需要再次确认（原型演示）" };
    if (button.dataset.deviceAction === "verify") document.querySelector(".trust-detail").textContent = "刚刚重新验证";
    showToast(messages[button.dataset.deviceAction]);
    deviceMenu.hidden = true;
    moreButton.setAttribute("aria-expanded", "false");
  });
});

document.querySelector(".pair-device-button")?.addEventListener("click", openPairingModal);
document.querySelector(".modal-close")?.addEventListener("click", closePairingModal);
pairingModal?.addEventListener("click", (event) => {
  if (event.target === pairingModal) closePairingModal();
});
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !pairingModal.hidden) closePairingModal();
  else if (event.key === "Escape" && !diagnosticsModal?.hidden) closeDiagnosticsModal();
});

pairingModeButtons.forEach((button) => button.addEventListener("click", () => setPairingMode(button.dataset.pairingMode)));
discoveredDevice?.addEventListener("click", () => showPairingCode("MacBook Air", "192.168.8.189", "通过自动发现"));
document.querySelector(".manual-connect-button")?.addEventListener("click", () => {
  const address = manualAddress.value.trim();
  const error = document.querySelector(".field-error");
  error.hidden = validIpv4(address);
  if (!error.hidden) return;
  showPairingCode("手动地址设备", address, "通过手动地址");
});
document.querySelector(".pairing-back-button")?.addEventListener("click", () => setPairingStep("discover"));
document.querySelector(".pairing-confirm-button")?.addEventListener("click", (event) => {
  const button = event.currentTarget;
  button.disabled = true;
  button.textContent = "正在验证证书…";
  pairingVerificationTimer = window.setTimeout(() => {
    button.disabled = false;
    button.textContent = "配对码一致";
    setPairingStep("success");
    document.querySelector(".pairing-finish-button").focus();
  }, 750);
});
document.querySelector(".pairing-finish-button")?.addEventListener("click", () => {
  closePairingModal();
  showToast("安全配对完成，已保存可信证书");
});

const runDiagnosticsButton = document.querySelector(".run-diagnostics-button");
const diagnosticRows = [...document.querySelectorAll(".diagnostic-check")];
const diagnosticsOverall = document.querySelector(".diagnostics-overall");
const diagnosticLastRun = document.querySelector(".diagnostic-last-run");
const liveChart = document.querySelector(".live-chart");
const diagnosticsModal = document.querySelector(".diagnostics-modal");
const diagnosticsExportSteps = [...document.querySelectorAll(".diagnostics-export-step")];
let diagnosticsRunId = 0;
let exportTimer;

const diagnosticDefinitions = {
  certificate: { running: "正在验证双方证书…", complete: "双向验证正常", tag: "安全", log: "Trusted peer certificate verified" },
  discovery: { running: "正在测试 UDP 43892…", complete: "UDP 43892 可用", tag: "发现", log: "Discovery announcement and response succeeded" },
  permissions: { running: "正在检查捕获与注入…", complete: "捕获与注入可用", tag: "权限", log: "Input capture and injection permissions available" },
  recovery: { running: "正在模拟心跳中断…", complete: "心跳和紧急快捷键正常", tag: "恢复", log: "Local control recovery path passed" },
};

function diagnosticDelay(ms) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function setDiagnosticRow(row, state) {
  const icon = row.querySelector(".check");
  const detail = row.querySelector("small");
  const definition = diagnosticDefinitions[row.dataset.check];
  row.classList.toggle("is-running", state === "running");
  icon.classList.toggle("is-waiting", state === "waiting");
  icon.classList.toggle("is-running", state === "running");
  icon.textContent = state === "complete" ? "✓" : state === "running" ? "…" : "·";
  detail.textContent = state === "complete" ? definition.complete : state === "running" ? definition.running : "等待检查";
}

function appendDiagnosticLog(tag, message) {
  const line = document.createElement("p");
  line.className = "is-new";
  const timestamp = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  line.innerHTML = `<time>${timestamp}</time><span class="log-good">${tag}</span>${message}`;
  const logLines = document.querySelector(".log-lines");
  logLines.append(line);
  while (logLines.children.length > 8) logLines.firstElementChild.remove();
}

runDiagnosticsButton?.addEventListener("click", async () => {
  const currentRun = ++diagnosticsRunId;
  runDiagnosticsButton.disabled = true;
  diagnosticsOverall.classList.remove("good");
  diagnosticsOverall.classList.add("pending");
  diagnosticsOverall.textContent = "正在检查";
  liveChart.classList.add("is-testing");
  document.querySelectorAll(".metric-card").forEach((card) => card.classList.add("is-testing"));
  document.querySelector('[data-metric="connection"] strong').textContent = "检查中";
  diagnosticRows.forEach((row) => setDiagnosticRow(row, "waiting"));

  for (const [index, row] of diagnosticRows.entries()) {
    if (currentRun !== diagnosticsRunId) return;
    setDiagnosticRow(row, "running");
    runDiagnosticsButton.textContent = `检查中 ${index + 1} / ${diagnosticRows.length}`;
    await diagnosticDelay(420);
    if (currentRun !== diagnosticsRunId) return;
    setDiagnosticRow(row, "complete");
    const definition = diagnosticDefinitions[row.dataset.check];
    appendDiagnosticLog(definition.tag, definition.log);
  }

  await diagnosticDelay(220);
  if (currentRun !== diagnosticsRunId) return;
  runDiagnosticsButton.disabled = false;
  runDiagnosticsButton.textContent = "再次运行检查";
  diagnosticsOverall.classList.remove("pending");
  diagnosticsOverall.classList.add("good");
  diagnosticsOverall.textContent = "全部通过";
  liveChart.classList.remove("is-testing");
  document.querySelectorAll(".metric-card").forEach((card) => card.classList.remove("is-testing"));
  document.querySelector('[data-metric="connection"] strong').textContent = "正常";
  document.querySelector('[data-metric="latency"] strong').innerHTML = '16 <i>ms</i>';
  document.querySelector('[data-metric="jitter"] strong').innerHTML = '2.7 <i>ms</i>';
  document.querySelector(".chart-value b").textContent = "16 ms";
  diagnosticLastRun.textContent = "上次检查：刚刚";
  showToast("完整检查已通过");
});

function setDiagnosticsExportStep(stepName) {
  diagnosticsExportSteps.forEach((step) => step.classList.toggle("is-active", step.classList.contains(`diagnostics-export-${stepName}`)));
}

function updateGenerateButton() {
  document.querySelector(".generate-diagnostics-button").disabled = !document.querySelector('.export-options input:checked');
}

function openDiagnosticsModal() {
  window.clearTimeout(exportTimer);
  diagnosticsModal.hidden = false;
  setDiagnosticsExportStep("options");
  document.querySelectorAll(".export-options input").forEach((input) => { input.checked = true; });
  updateGenerateButton();
  document.querySelector(".diagnostics-modal-close").focus();
}

function closeDiagnosticsModal() {
  window.clearTimeout(exportTimer);
  diagnosticsModal.hidden = true;
  document.querySelector(".export-diagnostics-button").focus();
}

document.querySelector(".copy-diagnostics-button")?.addEventListener("click", () => showToast("诊断摘要已复制（原型演示）"));
document.querySelector(".open-logs-button")?.addEventListener("click", () => showToast("已打开日志文件夹（原型演示）"));
document.querySelector(".export-diagnostics-button")?.addEventListener("click", openDiagnosticsModal);
document.querySelector(".diagnostics-modal-close")?.addEventListener("click", closeDiagnosticsModal);
document.querySelector(".diagnostics-cancel-button")?.addEventListener("click", closeDiagnosticsModal);
diagnosticsModal?.addEventListener("click", (event) => {
  if (event.target === diagnosticsModal) closeDiagnosticsModal();
});
document.querySelectorAll(".export-options input").forEach((input) => input.addEventListener("change", updateGenerateButton));
document.querySelector(".generate-diagnostics-button")?.addEventListener("click", () => {
  setDiagnosticsExportStep("generating");
  const stamp = new Date().toISOString().slice(0, 10).replaceAll("-", "");
  document.querySelector(".export-file-name").textContent = `edgemouse-diagnostics-${stamp}.zip`;
  exportTimer = window.setTimeout(() => {
    setDiagnosticsExportStep("success");
    document.querySelector(".diagnostics-finish-button").focus();
  }, 1000);
});
document.querySelector(".diagnostics-finish-button")?.addEventListener("click", () => {
  closeDiagnosticsModal();
  showToast("诊断包已生成");
});

const settingsSaveStatus = document.querySelector(".settings-save-status");
const themeOptions = [...document.querySelectorAll(".theme-option")];
const languageSelect = document.querySelector("#language");
const updateChannelSelect = document.querySelector("#update-channel");
const resetSettingsModal = document.querySelector(".reset-settings-modal");
const infoModal = document.querySelector(".info-modal");
const defaultInputProfiles = JSON.parse(JSON.stringify(inputProfiles));
const systemColorPreference = window.matchMedia("(prefers-color-scheme: dark)");
let activeTheme = "system";
const desktopDirtyFields = new Set();

function markSettingsDirty(field) {
  desktopDirtyFields.add(field);
  settingsSaveStatus.textContent = "正在自动保存…";
  settingsSaveStatus.classList.add("is-dirty");
  settingsChanged("desktop");
}

function selectTheme(theme, shouldMarkDirty = true) {
  activeTheme = ["system", "light", "dark"].includes(theme) ? theme : "system";
  const resolvedTheme = activeTheme === "system" ? (systemColorPreference.matches ? "dark" : "light") : activeTheme;
  const labels = {
    system: `跟随系统 · ${resolvedTheme === "dark" ? "深色" : "浅色"}`,
    light: "始终使用浅色",
    dark: "始终使用深色",
  };
  themeOptions.forEach((button) => {
    const selected = button.dataset.theme === activeTheme;
    button.classList.toggle("is-selected", selected);
    button.setAttribute("aria-checked", String(selected));
  });
  document.documentElement.dataset.theme = resolvedTheme;
  document.querySelector('meta[name="color-scheme"]').content = resolvedTheme;
  document.querySelector(".theme-current").textContent = labels[activeTheme];
  window.localStorage.setItem("edgemouse-theme", activeTheme);
  if (shouldMarkDirty) markSettingsDirty("theme");
}

function applyLanguage(language, shouldMarkDirty = true) {
  const nextLanguage = language === "en" ? "en" : "zh-CN";
  languageSelect.value = nextLanguage;
  window.EdgeMouseI18n.apply(nextLanguage);
  window.localStorage.setItem("edgemouse-language", nextLanguage);
  if (shouldMarkDirty) markSettingsDirty("language");
}

document.querySelectorAll("[data-general-setting]").forEach((toggle) => {
  toggle.addEventListener("click", () => markSettingsDirty(toggle.dataset.generalSetting));
});

themeOptions.forEach((button) => {
  button.addEventListener("click", () => selectTheme(button.dataset.theme));
});

languageSelect?.addEventListener("change", () => {
  applyLanguage(languageSelect.value);
});

updateChannelSelect?.addEventListener("change", () => markSettingsDirty("updateChannel"));

async function checkForUpdates() {
  const buttons = [...document.querySelectorAll(".check-updates-button")];
  const updateStatus = document.querySelector(".update-status");
  buttons.forEach((button) => {
    button.disabled = true;
    button.dataset.originalLabel = button.textContent;
    button.textContent = "正在检查…";
  });
  updateStatus.classList.remove("good");
  updateStatus.classList.add("pending");
  updateStatus.textContent = "正在检查";
  await diagnosticDelay(850);
  buttons.forEach((button) => {
    button.disabled = false;
    button.textContent = button.dataset.originalLabel;
    delete button.dataset.originalLabel;
  });
  updateStatus.classList.remove("pending");
  updateStatus.classList.add("good");
  updateStatus.textContent = "已是最新版";
  document.querySelector(".update-last-check").textContent = "最后检查：刚刚";
  showToast(`EdgeMouse ${appVersion} 已是最新版`);
}

document.querySelectorAll(".check-updates-button").forEach((button) => {
  button.addEventListener("click", checkForUpdates);
});

function openResetSettingsModal() {
  resetSettingsModal.hidden = false;
  document.querySelector(".reset-modal-close").focus();
}

function closeResetSettingsModal() {
  resetSettingsModal.hidden = true;
  document.querySelector(".reset-settings-button").focus();
}

function restoreDefaultSettings() {
  window.localStorage.removeItem("edgemouse-preview-settings");
  window.EdgeMouseInputSettings.clearDirty();
  window.EdgeMouseDesktopSettings.clearDirty();
  document.querySelectorAll("[data-general-setting]").forEach((toggle) => setToggleState(toggle, true));
  Object.entries(defaultInputProfiles).forEach(([name, profile]) => Object.assign(inputProfiles[name], profile));
  activeInputProfile = "mac-to-windows";
  inputSettingsDirty = false;
  inputSaveStatus.textContent = "两个控制方向已恢复为推荐值";
  inputSaveStatus.classList.remove("is-dirty");
  renderInputProfile();
  syncOverviewInputSettings();
  selectTheme("system", false);
  applyLanguage("zh-CN", false);
  updateChannelSelect.value = "stable";
  setToggleState(autoDiscoveryToggle, true);
  discoveryState.classList.remove("is-paused");
  discoveryState.querySelector("strong").textContent = "正在监听可信设备";
  discoveryState.querySelector("small").textContent = "UDP 43892 · 地址变化自动更新";
  document.querySelector(".discovery-detail").textContent = "局域网自动发现";
  settingsSaveStatus.textContent = "默认设置已恢复并保存";
  settingsSaveStatus.classList.remove("is-dirty");
  closeResetSettingsModal();
  showToast("已恢复默认设置，设备证书保持不变");
}

systemColorPreference.addEventListener("change", () => {
  if (activeTheme === "system") selectTheme("system", false);
});

const savedTheme = window.localStorage.getItem("edgemouse-theme") ?? "system";
const savedLanguage = window.localStorage.getItem("edgemouse-language") ?? "zh-CN";
selectTheme(savedTheme, false);
applyLanguage(savedLanguage, false);
window.EdgeMouseI18n.startObserving();

window.EdgeMouseDesktopSettings = {
  apply(preferences) {
    if (!preferences) return;
    document.querySelectorAll("[data-general-setting]").forEach((toggle) => {
      const key = toggle.dataset.generalSetting;
      if (!desktopDirtyFields.has(key) && Object.hasOwn(preferences, key)) setToggleState(toggle, Boolean(preferences[key]));
    });
    if (!desktopDirtyFields.has("theme")) selectTheme(preferences.theme ?? "system", false);
    if (!desktopDirtyFields.has("language")) applyLanguage(preferences.language ?? "zh-CN", false);
    if (!desktopDirtyFields.has("updateChannel")) updateChannelSelect.value = preferences.updateChannel ?? "stable";
    if (!desktopDirtyFields.size) {
      settingsSaveStatus.textContent = "修改后自动保存，下次启动继续使用";
      settingsSaveStatus.classList.remove("is-dirty");
    }
  },
  get() {
    const toggleValue = (key) => document.querySelector(`[data-general-setting="${key}"]`)?.classList.contains("is-on") ?? false;
    return {
      autostart: toggleValue("autostart"),
      background: toggleValue("background"),
      clipboardSync: toggleValue("clipboardSync"),
      notifications: toggleValue("notifications"),
      theme: activeTheme,
      language: languageSelect.value,
      updateChannel: updateChannelSelect.value,
    };
  },
  getDirtyFields: () => [...desktopDirtyFields],
  clearDirty: () => desktopDirtyFields.clear(),
  markSaved(message = "设置已自动保存", submitted = this.get(), fields = [...desktopDirtyFields]) {
    const current = this.get();
    fields.forEach((field) => {
      if (current[field] === submitted[field]) desktopDirtyFields.delete(field);
    });
    settingsSaveStatus.textContent = message;
    settingsSaveStatus.classList.toggle("is-dirty", desktopDirtyFields.size > 0);
  },
};

document.querySelector(".reset-settings-button")?.addEventListener("click", openResetSettingsModal);
document.querySelector(".reset-modal-close")?.addEventListener("click", closeResetSettingsModal);
document.querySelector(".reset-cancel-button")?.addEventListener("click", closeResetSettingsModal);
document.querySelector(".confirm-reset-button")?.addEventListener("click", restoreDefaultSettings);
resetSettingsModal?.addEventListener("click", (event) => {
  if (event.target === resetSettingsModal) closeResetSettingsModal();
});

const infoModalContent = {
  license: {
    eyebrow: "开源信息",
    title: "MIT License",
    body: `<h3>自由使用，也保留署名</h3><p>EdgeMouse 采用 MIT 许可证。你可以使用、复制、修改、合并和分发软件，但需要保留原始版权与许可声明。</p><div class="info-note"><b>Copyright © 2026 EdgeMouse contributors</b><span>软件按“原样”提供，不附带任何形式的保证。</span></div>`,
  },
  dependencies: {
    eyebrow: "第三方组件",
    title: "核心依赖清单",
    body: `<h3>跨平台与安全传输</h3><ul><li><b>Tokio</b><span>异步运行时与任务调度</span></li><li><b>Quinn</b><span>低延迟 QUIC 连接</span></li><li><b>rustls</b><span>双向 TLS 与证书验证</span></li><li><b>Windows API</b><span>Windows 输入捕获与注入</span></li><li><b>Core Graphics</b><span>macOS 指针与键盘事件</span></li></ul>`,
  },
};

function openInfoModal(kind) {
  const info = infoModalContent[kind];
  if (!info) return;
  document.querySelector(".info-modal-eyebrow").textContent = info.eyebrow;
  document.querySelector("#info-modal-title").textContent = info.title;
  document.querySelector(".info-modal-content").innerHTML = info.body;
  infoModal.hidden = false;
  document.querySelector(".info-modal-close").focus();
}

function closeInfoModal() {
  infoModal.hidden = true;
}

document.querySelectorAll("[data-about-action]").forEach((button) => {
  button.addEventListener("click", () => {
    const action = button.dataset.aboutAction;
    if (action === "license" || action === "dependencies") {
      openInfoModal(action);
    } else if (action === "issues") {
      showToast("将在 GitHub Issues 中打开问题反馈（原型演示）");
    } else if (action === "version") {
      const versionInfo = `EdgeMouse ${appVersion} · QUIC · mutual TLS · Windows 11 / macOS 15+`;
      navigator.clipboard?.writeText(versionInfo).catch(() => {});
      showToast("版本与诊断信息已复制");
    }
  });
});

document.querySelector(".project-home-button")?.addEventListener("click", () => {
  showToast("将在浏览器中打开 EdgeMouse 项目主页（原型演示）");
});
document.querySelector(".info-modal-close")?.addEventListener("click", closeInfoModal);
document.querySelector(".info-modal-finish")?.addEventListener("click", closeInfoModal);
infoModal?.addEventListener("click", (event) => {
  if (event.target === infoModal) closeInfoModal();
});
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (!resetSettingsModal.hidden) closeResetSettingsModal();
  else if (!infoModal.hidden) closeInfoModal();
});

const layoutCanvas = document.querySelector(".layout-canvas");
const layoutDirectionButtons = [...document.querySelectorAll(".layout-direction button")];
let layoutDirty = false;

function syncOverviewLayout(edge) {
  const label = document.querySelector("[data-overview-layout]");
  if (!label) return;
  const direction = { left: "左侧", right: "右侧", top: "上方", bottom: "下方" }[edge] ?? "右侧";
  label.textContent = `Mac 位于 Windows ${direction}`;
}

function setLayoutEdge(edge, { dirty = false } = {}) {
  const changed = edge !== (layoutCanvas.dataset.edge ?? "right");
  layoutCanvas.dataset.edge = edge;
  syncOverviewLayout(edge);
  window.EdgeMouseDisplays?.setEdge(edge, dirty && changed);
  layoutDirectionButtons.forEach((button) => button.classList.toggle("is-selected", button.dataset.edge === edge));
  if (dirty && changed) markLayoutDirty();
}

function markLayoutDirty() {
  layoutDirty = true;
  document.querySelector(".layout-save-status").textContent = "正在自动保存…";
  document.querySelector(".layout-config-status").textContent = "正在应用";
  settingsChanged("layout");
}

window.EdgeMouseLayout = {
  getEdge: () => layoutCanvas.dataset.edge ?? "right",
  get() {
    return { peerOn: this.getEdge(), displayLayout: window.EdgeMouseDisplays?.get() ?? null, edgeProtection: document.querySelector('[data-layout-setting="edgeProtection"]').classList.contains("is-on") };
  },
  isDirty: () => layoutDirty,
  changed: markLayoutDirty,
  setEdge: (edge) => setLayoutEdge(edge),
  applySnapshot(edge, displayLayout = null) {
    if (!layoutDirty && !layoutCanvas.classList.contains("is-dragging")) {
      window.EdgeMouseDisplays?.apply(displayLayout);
      setLayoutEdge(displayLayout?.macOn ?? edge);
    }
  },
  clearDirty: () => { layoutDirty = false; },
  markSaved(message = "布局已自动保存", submitted = this.get()) {
    const current = this.get();
    if (current.peerOn !== submitted.peerOn || current.edgeProtection !== submitted.edgeProtection || JSON.stringify(current.displayLayout) !== JSON.stringify(submitted.displayLayout)) return;
    layoutDirty = false;
    const status = document.querySelector(".layout-save-status");
    if (status) status.textContent = message;
    const configStatus = document.querySelector(".layout-config-status");
    if (configStatus) configStatus.textContent = "正在同步";
  },
};

layoutDirectionButtons.forEach((button) => button.addEventListener("click", () => setLayoutEdge(button.dataset.edge, { dirty: true })));
document.querySelector('[data-layout-setting="edgeProtection"]')?.addEventListener("click", () => {
  markLayoutDirty();
});





document.querySelector(".detect-button")?.addEventListener("click", (event) => {
  if (window.__TAURI__?.core?.invoke) return;
  event.currentTarget.textContent = "检测中…";
  window.setTimeout(() => {
    event.currentTarget.textContent = "重新检测屏幕";
    showToast("已识别 3 个显示区域");
  }, 800);
});

document.querySelectorAll(".certificate-line button").forEach((button) => {
  button.addEventListener("click", () => {
    if (window.__TAURI__?.core?.invoke) return;
    showToast("内容已复制（网页预览）");
  });
});

// The standalone preview has no native service. Keep its settings in this
// browser so removing the demo's save buttons still leaves a working preview.
if (!window.__TAURI__?.core?.invoke) {
  const previewKey = "edgemouse-preview-settings";
  const previewStatus = "网页预览：设置仅保存在此浏览器";
  function savePreview(section) {
    const selectors = { input: [".input-save-status", ".overview-save-status"], desktop: [".settings-save-status"], layout: [".layout-save-status"], connection: [".connection-save-status"] };
    try {
      const saved = JSON.parse(localStorage.getItem(previewKey) ?? "{}");
      if (section === "input") {
        saved.input ??= {};
        for (const name of Object.keys(inputProfiles)) {
          const fields = window.EdgeMouseInputSettings.getDirtyFields(name);
          saved.input[name] = { ...saved.input[name], ...Object.fromEntries(fields.map((field) => [field, inputProfiles[name][field]])) };
        }
      }
      if (section === "desktop") saved.desktop = window.EdgeMouseDesktopSettings.get();
      if (section === "layout") saved.layout = window.EdgeMouseLayout.get();
      if (section === "connection") saved.autoReconnect = document.querySelector(".auto-reconnect-toggle").classList.contains("is-on");
      localStorage.setItem(previewKey, JSON.stringify(saved));
      if (section === "input") for (const name of Object.keys(inputProfiles)) window.EdgeMouseInputSettings.markSaved(previewStatus, name, window.EdgeMouseInputSettings.getDirtyFields(name));
      if (section === "desktop") window.EdgeMouseDesktopSettings.markSaved(previewStatus);
      if (section === "layout") window.EdgeMouseLayout.markSaved(previewStatus);
      for (const selector of selectors[section]) {
        const status = document.querySelector(selector);
        status.textContent = previewStatus;
        status.classList.remove("is-error", "is-dirty");
      }
      document.querySelectorAll(`[data-retry-settings="${section}"]`).forEach((button) => { button.hidden = true; });
    } catch {
      for (const selector of selectors[section]) {
        document.querySelector(selector).textContent = "自动保存失败，请重试";
        document.querySelector(selector).classList.add("is-error");
      }
      document.querySelectorAll(`[data-retry-settings="${section}"]`).forEach((button) => { button.hidden = false; });
    }
  }
  try {
    const saved = JSON.parse(localStorage.getItem(previewKey) ?? "{}");
    for (const [name, settings] of Object.entries(saved.input ?? {})) window.EdgeMouseInputSettings.applyLocalProfile(name, settings);
    if (saved.desktop) window.EdgeMouseDesktopSettings.apply(saved.desktop);
    if (saved.layout && ["left", "right", "top", "bottom"].includes(saved.layout.peerOn)) {
      window.EdgeMouseLayout.applySnapshot(saved.layout.peerOn, saved.layout.displayLayout ?? null);
      setToggleState(document.querySelector('[data-layout-setting="edgeProtection"]'), saved.layout.edgeProtection);
    }
    if (typeof saved.autoReconnect === "boolean") setToggleState(document.querySelector(".auto-reconnect-toggle"), saved.autoReconnect);
  } catch { /* A broken optional preview cache must not prevent opening the UI. */ }
  document.addEventListener("edgemouse:settings-change", ({ detail }) => {
    if (detail.commit) queueMicrotask(() => savePreview(detail.section));
  });
  document.querySelector(".auto-reconnect-toggle").addEventListener("click", () => savePreview("connection"));
  document.querySelectorAll("[data-retry-settings]").forEach((button) => button.addEventListener("click", () => savePreview(button.dataset.retrySettings)));
}
