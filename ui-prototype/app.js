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

document.querySelectorAll(".save-button").forEach((button) => button.addEventListener("click", () => showToast("设置已保存（原型演示）")));
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
