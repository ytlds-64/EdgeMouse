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
