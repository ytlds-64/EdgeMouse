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

document.querySelectorAll(".layout-direction button").forEach((button) => {
  button.addEventListener("click", () => {
    const canvas = document.querySelector(".layout-canvas");
    const win = canvas.querySelector(".screen-win");
    const mac = canvas.querySelector(".screen-mac");
    const beam = canvas.querySelector(".edge-glow");
    const edge = button.dataset.edge;
    canvas.style.flexDirection = edge === "top" || edge === "bottom" ? "column" : "row";
    beam.style.width = edge === "top" || edge === "bottom" ? "160px" : "20px";
    beam.style.height = edge === "top" || edge === "bottom" ? "20px" : "160px";
    beam.querySelector("span").style.transform = edge === "top" || edge === "bottom" ? "none" : "rotate(-90deg)";
    if (edge === "left" || edge === "top") canvas.insertBefore(mac, win);
    else canvas.insertBefore(win, mac);
    canvas.insertBefore(beam, mac === canvas.firstElementChild ? win : mac);
  });
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
