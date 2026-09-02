(() => {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;

  document.documentElement.dataset.nativeApp = "true";

  document.querySelectorAll("[data-window-action]").forEach((element) => {
    element.addEventListener("pointerdown", (event) => {
      if (element.dataset.windowAction !== "drag" || event.button !== 0) return;
      invoke("window_action", { action: "drag" }).catch(console.error);
    });
    if (element.dataset.windowAction !== "drag") {
      element.addEventListener("click", () => {
        invoke("window_action", { action: element.dataset.windowAction }).catch(console.error);
      });
    }
  });

  const setText = (selector, value) => {
    if (value === undefined || value === null || value === "") return;
    const element = document.querySelector(selector);
    if (element) element.textContent = value;
  };

  const setChip = (selector, healthy, message) => {
    const chip = document.querySelector(selector);
    if (!chip) return;
    chip.classList.toggle("good", healthy);
    chip.classList.toggle("pending", !healthy);
    const label = chip.querySelector("b") ?? chip;
    label.textContent = message;
  };

  const setOnlineLabel = (selector, message) => {
    const element = document.querySelector(selector);
    if (!element) return;
    const dot = element.querySelector("b");
    element.replaceChildren(...(dot ? [dot, message] : [message]));
  };

  const groupedNode = (node) => {
    if (!node) return "—";
    const groups = node.match(/.{1,4}/g) ?? [node];
    return `${groups.slice(0, 4).join(" ")} ··· ${groups.slice(-3).join(" ")}`;
  };

  const oppositeEdge = { left: "right", right: "left", top: "bottom", bottom: "top" };

  function updateDeviceCards(snapshot) {
    const windowsLocal = snapshot.platform.operatingSystem.toLowerCase().includes("windows");
    const localSelector = windowsLocal ? ".windows-device" : ".mac-device";
    const peerSelector = windowsLocal ? ".mac-device" : ".windows-device";
    const localLayoutSelector = windowsLocal ? ".screen-win" : ".screen-mac";

    setText(`${localSelector} h2`, snapshot.config.localName);
    setText(`${peerSelector} h2`, snapshot.config.peerScreenName);
    setOnlineLabel(`${localSelector} .online`, snapshot.agent.running ? "本机运行中" : "本机服务未启动");
    setOnlineLabel(`${peerSelector} .online`, snapshot.agent.running ? "可信设备待连接" : "等待后台服务");

    const width = snapshot.platform.desktopWidth;
    const height = snapshot.platform.desktopHeight;
    if (width && height) {
      const displayCount = snapshot.platform.displayCount ?? 1;
      setText(`${localLayoutSelector} small`, `${Math.round(width)} × ${Math.round(height)} · ${displayCount} 个屏幕`);
    }

    if (snapshot.config.peerOn && typeof window.setLayoutEdge === "function") {
      const uiEdge = windowsLocal ? snapshot.config.peerOn : oppositeEdge[snapshot.config.peerOn];
      if (uiEdge) window.setLayoutEdge(uiEdge);
    }
  }

  function applySnapshot(snapshot) {
    const running = snapshot.agent.running;
    const configValid = snapshot.config.valid;
    window.setEdgeMouseAppVersion?.(running ? snapshot.agent.version : snapshot.desktopVersion);
    setText("[data-native-mode]", "桌面应用 · 实时状态");

    setChip(
      ".connection-status-chip",
      running,
      running ? "后台服务运行中" : "后台服务未启动",
    );
    setText(
      ".connection-device-status b",
      running ? "自动发现与重连已启动" : "等待 EdgeMouse 后台服务",
    );
    setText(
      ".peer-address",
      snapshot.config.peerAddress?.startsWith("auto") ? "自动获取 · UDP 43892" : snapshot.config.peerAddress,
    );
    setText(
      ".discovery-detail",
      snapshot.config.peerAddress?.startsWith("auto") ? "局域网自动发现" : "固定设备地址",
    );
    setText(".trust-detail", configValid ? "可信证书已载入" : "配置需要检查");
    setText(".certificate-line code", groupedNode(snapshot.config.peerNode));

    const diagnostics = document.querySelector(".diagnostics-overall");
    if (diagnostics) {
      diagnostics.classList.toggle("good", running && configValid);
      diagnostics.classList.toggle("pending", !(running && configValid));
      diagnostics.textContent = running && configValid ? "基础检查通过" : "需要处理";
      diagnostics.title = snapshot.agent.error ?? snapshot.config.error ?? "";
    }

    const serviceChip = document.querySelector("#page-settings .setting-panel .status-chip");
    if (serviceChip) {
      serviceChip.classList.toggle("good", running);
      serviceChip.classList.toggle("pending", !running);
      serviceChip.textContent = running ? `后台服务正常 · PID ${snapshot.agent.processId}` : "后台服务未运行";
    }

    const screenFacts = document.querySelectorAll(".screen-facts b");
    if (screenFacts[0] && snapshot.platform.desktopWidth) {
      const orientation = snapshot.platform.desktopWidth >= snapshot.platform.desktopHeight ? "横向" : "纵向";
      screenFacts[0].textContent = `${snapshot.platform.displayCount ?? 1} 个屏幕 · ${orientation}`;
    }
    if (screenFacts[2]) {
      screenFacts[2].textContent = configValid ? "配置已读取" : "配置读取失败";
      screenFacts[2].title = snapshot.config.error ?? snapshot.config.path ?? "";
    }

    updateDeviceCards(snapshot);
  }

  let refreshing = false;
  async function refreshSnapshot() {
    if (refreshing) return;
    refreshing = true;
    try {
      applySnapshot(await invoke("get_app_snapshot"));
    } catch (error) {
      console.error("Unable to load EdgeMouse desktop status", error);
      setChip(".connection-status-chip", false, "无法读取后台状态");
    } finally {
      refreshing = false;
    }
  }

  refreshSnapshot();
  window.setInterval(refreshSnapshot, 3000);
})();
