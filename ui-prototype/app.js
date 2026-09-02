const navItems = [...document.querySelectorAll(".nav-item")];
const pages = [...document.querySelectorAll(".page")];
const toast = document.querySelector(".toast");
let toastTimer;

function showToast(message) {
  toast.querySelector("b").textContent = message;
  toast.classList.add("is-visible");
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 1800);
}

function showPage(name) {
  navItems.forEach((item) => item.classList.toggle("is-active", item.dataset.page === name));
  pages.forEach((page) => page.classList.toggle("is-active", page.id === `page-${name}`));
  const current = pages.find((page) => page.id === `page-${name}`);
  document.title = `EdgeMouse · ${current?.dataset.title ?? "UI 原型"}`;
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
let inputSettingsDirty = false;
const smoothingRange = document.querySelector("#pointer-smoothing");
const smoothingOutput = document.querySelector('output[for="pointer-smoothing"]');
const inputSaveStatus = document.querySelector(".input-save-status");

function smoothingLabel(value) {
  if (value < 34) return "跟手";
  if (value < 68) return "均衡";
  return "更平滑";
}

function setToggleState(toggle, enabled) {
  toggle.classList.toggle("is-on", enabled);
  toggle.setAttribute("aria-checked", String(enabled));
}

function markInputSettingsDirty() {
  inputSettingsDirty = true;
  inputSaveStatus.textContent = "有尚未保存的输入设置";
  inputSaveStatus.classList.add("is-dirty");
}

function syncOverviewInputSettings() {
  const profile = inputProfiles["mac-to-windows"];
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
    setToggleState(toggle, profile[toggle.dataset.inputSetting]);
  });
  document.querySelector('[data-input-description="horizontal"]').textContent = meta.horizontalDescription;
  smoothingRange.value = String(profile.smoothing);
  smoothingOutput.textContent = smoothingLabel(profile.smoothing);
  document.querySelectorAll("[data-map-source]").forEach((source) => {
    source.textContent = meta.sources[source.dataset.mapSource];
  });
  document.querySelectorAll("[data-input-map]").forEach((select) => {
    const key = select.dataset.inputMap;
    select.replaceChildren(...meta.choices[key].map((choice) => new Option(choice, choice)));
    select.value = profile[key];
    select.disabled = !profile.keyboard;
  });
  document.querySelectorAll('[data-input-choice="trigger"] button').forEach((button) => {
    button.classList.toggle("is-selected", button.dataset.value === profile.trigger);
  });
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
    markInputSettingsDirty();
    syncOverviewInputSettings();
    if (toggle.dataset.inputSetting === "keyboard") renderInputProfile();
  });
});

document.querySelectorAll("[data-overview-setting]").forEach((toggle) => {
  toggle.addEventListener("click", () => {
    inputProfiles["mac-to-windows"][toggle.dataset.overviewSetting] = toggle.classList.contains("is-on");
    markInputSettingsDirty();
    if (activeInputProfile === "mac-to-windows") renderInputProfile();
  });
});

smoothingRange.addEventListener("input", () => {
  inputProfiles[activeInputProfile].smoothing = Number(smoothingRange.value);
  smoothingOutput.textContent = smoothingLabel(Number(smoothingRange.value));
  markInputSettingsDirty();
});

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

document.querySelector(".input-save-button")?.addEventListener("click", () => {
  inputSettingsDirty = false;
  inputSaveStatus.textContent = "两个控制方向的设置已分别保存";
  inputSaveStatus.classList.remove("is-dirty");
  showToast("双向输入设置已保存（原型演示）");
});

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
  showToast("诊断包已生成（原型演示）");
});

const layoutCanvas = document.querySelector(".layout-canvas");
const layoutDirectionButtons = [...document.querySelectorAll(".layout-direction button")];
const dragBeam = layoutCanvas.querySelector(".drag-beam");

function setLayoutEdge(edge) {
  const win = layoutCanvas.querySelector(".screen-win");
  const mac = layoutCanvas.querySelector(".screen-mac");
  const beam = layoutCanvas.querySelector(".edge-glow");
  const hint = layoutCanvas.querySelector(".canvas-hint");
  const vertical = edge === "top" || edge === "bottom";
  const macComesFirst = edge === "left" || edge === "top";

  layoutCanvas.dataset.edge = edge;
  layoutCanvas.style.flexDirection = vertical ? "column" : "row";
  beam.style.width = vertical ? "160px" : "20px";
  beam.style.height = vertical ? "20px" : "160px";
  beam.querySelector("span").style.transform = vertical ? "none" : "rotate(-90deg)";

  const order = macComesFirst ? [mac, beam, win] : [win, beam, mac];
  order.forEach((element) => layoutCanvas.insertBefore(element, hint));
  layoutDirectionButtons.forEach((button) => button.classList.toggle("is-selected", button.dataset.edge === edge));
}

layoutDirectionButtons.forEach((button) => button.addEventListener("click", () => setLayoutEdge(button.dataset.edge)));

function edgeFromRects(macRect, winRect) {
  const deltaX = macRect.left + macRect.width / 2 - (winRect.left + winRect.width / 2);
  const deltaY = macRect.top + macRect.height / 2 - (winRect.top + winRect.height / 2);
  if (Math.abs(deltaX) >= Math.abs(deltaY)) return deltaX < 0 ? "left" : "right";
  return deltaY < 0 ? "top" : "bottom";
}

function setDropPreview(edge) {
  layoutDirectionButtons.forEach((button) => button.classList.toggle("is-drop-preview", button.dataset.edge === edge));
}

function cardCenter(rect) {
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
}

function pointOnCardEdge(rect, deltaX, deltaY) {
  const center = cardCenter(rect);
  const horizontalScale = Math.abs(deltaX) < 0.001 ? Number.POSITIVE_INFINITY : rect.width / 2 / Math.abs(deltaX);
  const verticalScale = Math.abs(deltaY) < 0.001 ? Number.POSITIVE_INFINITY : rect.height / 2 / Math.abs(deltaY);
  const scale = Math.min(horizontalScale, verticalScale);
  return { x: center.x + deltaX * scale, y: center.y + deltaY * scale };
}

function updateDragBeam() {
  const canvasRect = layoutCanvas.getBoundingClientRect();
  const winRect = layoutCanvas.querySelector(".screen-win").getBoundingClientRect();
  const macRect = layoutCanvas.querySelector(".screen-mac").getBoundingClientRect();
  const cardsOverlap = winRect.left < macRect.right && winRect.right > macRect.left && winRect.top < macRect.bottom && winRect.bottom > macRect.top;
  if (cardsOverlap) {
    dragBeam.classList.remove("is-visible");
    return;
  }
  const winCenter = cardCenter(winRect);
  const macCenter = cardCenter(macRect);
  const deltaX = macCenter.x - winCenter.x;
  const deltaY = macCenter.y - winCenter.y;
  const start = pointOnCardEdge(winRect, deltaX, deltaY);
  const end = pointOnCardEdge(macRect, -deltaX, -deltaY);
  const lineX = end.x - start.x;
  const lineY = end.y - start.y;
  const angle = Math.atan2(lineY, lineX);

  dragBeam.style.left = `${start.x - canvasRect.left}px`;
  dragBeam.style.top = `${start.y - canvasRect.top - 11}px`;
  dragBeam.style.width = `${Math.hypot(lineX, lineY)}px`;
  dragBeam.style.transform = `rotate(${angle}rad)`;
  dragBeam.querySelector("span").style.transform = `translate(-50%, -50%) rotate(${-angle}rad)`;
  dragBeam.classList.add("is-visible");
}

function hideDragBeam() {
  dragBeam.classList.remove("is-visible");
  dragBeam.removeAttribute("style");
  dragBeam.querySelector("span").removeAttribute("style");
}

document.querySelectorAll(".mini-screen").forEach((screen) => {
  let drag;

  screen.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    if (drag) {
      finishDrag({ pointerId: drag.pointerId });
      return;
    }
    drag = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, moved: false };
    screen.classList.add("is-dragging");
    layoutCanvas.classList.add("is-dragging");
    updateDragBeam();
  });

  window.addEventListener("pointermove", (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const deltaX = event.clientX - drag.startX;
    const deltaY = event.clientY - drag.startY;
    drag.moved ||= Math.hypot(deltaX, deltaY) > 8;
    screen.style.transform = `translate3d(${deltaX}px, ${deltaY}px, 0) scale(1.02)`;
    updateDragBeam();
    const other = layoutCanvas.querySelector(screen.classList.contains("screen-mac") ? ".screen-win" : ".screen-mac");
    const macRect = screen.classList.contains("screen-mac") ? screen.getBoundingClientRect() : other.getBoundingClientRect();
    const winRect = screen.classList.contains("screen-win") ? screen.getBoundingClientRect() : other.getBoundingClientRect();
    setDropPreview(edgeFromRects(macRect, winRect));
  });

  const finishDrag = (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const other = layoutCanvas.querySelector(screen.classList.contains("screen-mac") ? ".screen-win" : ".screen-mac");
    const macRect = screen.classList.contains("screen-mac") ? screen.getBoundingClientRect() : other.getBoundingClientRect();
    const winRect = screen.classList.contains("screen-win") ? screen.getBoundingClientRect() : other.getBoundingClientRect();
    const edge = edgeFromRects(macRect, winRect);
    const moved = drag.moved;

    screen.style.transform = "";
    screen.classList.remove("is-dragging");
    layoutCanvas.classList.remove("is-dragging");
    hideDragBeam();
    setDropPreview();
    drag = undefined;
    if (moved) {
      setLayoutEdge(edge);
      const labels = { left: "左侧", right: "右侧", top: "上方", bottom: "下方" };
      showToast(`屏幕布局已调整为 ${labels[edge]}`);
    }
  };

  window.addEventListener("pointerup", finishDrag, true);
  window.addEventListener("pointercancel", finishDrag, true);
  window.addEventListener("mouseup", () => {
    if (drag) finishDrag({ pointerId: drag.pointerId });
  }, true);
});

document.querySelectorAll(".save-button:not(.input-save-button)").forEach((button) => button.addEventListener("click", () => showToast("设置已保存（原型演示）")));
document.querySelectorAll(".action-button").forEach((button) => button.addEventListener("click", () => showToast(`${button.textContent.trim()}（原型演示）`)));
document.querySelector(".detect-button")?.addEventListener("click", (event) => {
  event.currentTarget.textContent = "检测中…";
  window.setTimeout(() => {
    event.currentTarget.textContent = "重新检测屏幕";
    showToast("已识别 3 个显示区域");
  }, 800);
});

document.querySelectorAll(".about-links button, .certificate-line button").forEach((button) => {
  button.addEventListener("click", () => showToast("内容已复制（原型演示）"));
});
