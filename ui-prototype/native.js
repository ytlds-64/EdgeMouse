(() => {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;

  document.documentElement.dataset.nativeApp = "true";
  const isMacOS = /Macintosh|Mac OS X/.test(navigator.userAgent);
  document.documentElement.dataset.desktopPlatform = isMacOS ? "macos" : "windows";

  if (isMacOS) {
    const syncNativeMenuLanguage = () => {
      const language = document.querySelector("#language")?.value
        ?? window.localStorage.getItem("edgemouse-language")
        ?? "zh-CN";
      invoke("set_menu_language", { language }).catch(console.error);
    };
    syncNativeMenuLanguage();
    document.querySelector("#language")?.addEventListener("change", syncNativeMenuLanguage);
  }

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
  const qualityHistory = [];
  const chartWidth = 600;
  const chartHeight = 150;
  const chartPointCount = 60;
  let lastQualitySecond;
  let latestSnapshot;

  const connectionLabels = {
    starting: "正在启动",
    connecting: "正在连接",
    connected: "已连接",
    reconnecting: "正在重连",
    stopped: "未运行",
  };

  function desktopFromLegacy(width, height, scaleFactor, displayCount) {
    if (!Number.isFinite(Number(width)) || !Number.isFinite(Number(height))) return null;
    const desktopWidth = Number(width);
    const desktopHeight = Number(height);
    const scale = Number(scaleFactor) || 1;
    return {
      originX: 0,
      originY: 0,
      width: desktopWidth,
      height: desktopHeight,
      scaleFactor: scale,
      displays: [{
        originX: 0,
        originY: 0,
        width: desktopWidth,
        height: desktopHeight,
        pixelWidth: Math.round(desktopWidth * scale),
        pixelHeight: Math.round(desktopHeight * scale),
        scaleFactor: scale,
        primary: true,
      }],
      reportedDisplayCount: Number(displayCount) || 1,
    };
  }

  function usableDisplays(desktop) {
    if (!desktop?.displays?.length) return [];
    return desktop.displays.filter((display) => [
      display.originX,
      display.originY,
      display.width,
      display.height,
      display.pixelWidth,
      display.pixelHeight,
    ].every((value) => Number.isFinite(Number(value))) && Number(display.width) > 0 && Number(display.height) > 0);
  }

  function desktopLayoutLabel(desktop) {
    const displays = usableDisplays(desktop);
    const count = displays.length || Number(desktop?.reportedDisplayCount) || 0;
    if (!count) return "等待设备数据";
    if (count === 1) {
      const display = displays[0];
      const portrait = display
        ? Number(display.pixelHeight) > Number(display.pixelWidth)
        : Number(desktop.height) > Number(desktop.width);
      return `1 个屏幕 · ${portrait ? "纵向" : "横向"}`;
    }
    const centers = displays.map((display) => ({
      x: Number(display.originX) + Number(display.width) / 2,
      y: Number(display.originY) + Number(display.height) / 2,
    }));
    const xSpread = centers.length ? Math.max(...centers.map((point) => point.x)) - Math.min(...centers.map((point) => point.x)) : 0;
    const ySpread = centers.length ? Math.max(...centers.map((point) => point.y)) - Math.min(...centers.map((point) => point.y)) : 0;
    const arrangement = Math.max(xSpread, ySpread) === 0
      ? "自动布局"
      : xSpread > ySpread * 1.35
        ? "横向排列"
        : ySpread > xSpread * 1.35
          ? "纵向排列"
          : "混合布局";
    return `${count} 个屏幕 · ${arrangement}`;
  }

  function desktopSummary(desktop) {
    const displays = usableDisplays(desktop);
    const count = displays.length || Number(desktop?.reportedDisplayCount) || 0;
    if (!count) return "等待连接后读取屏幕";
    const width = Math.round(Number(desktop.width) || 0);
    const height = Math.round(Number(desktop.height) || 0);
    return `${count} 个屏幕 · ${width} × ${height} 桌面`;
  }

  function renderDesktopMap(selector, desktop, platformName) {
    const card = document.querySelector(selector);
    const map = card?.querySelector(".desktop-map");
    if (!card || !map) return;
    const displays = usableDisplays(desktop);
    card.querySelector(".desktop-summary").textContent = desktopSummary(desktop);
    map.replaceChildren();
    map.classList.toggle("is-empty", displays.length === 0);
    if (!displays.length) {
      const empty = document.createElement("span");
      empty.className = "display-map-empty";
      empty.textContent = "连接后显示真实屏幕排列";
      map.append(empty);
      return;
    }

    const originX = Number(desktop.originX);
    const originY = Number(desktop.originY);
    const desktopWidth = Number(desktop.width);
    const desktopHeight = Number(desktop.height);
    const availableWidth = Math.max(1, map.clientWidth - 20);
    const availableHeight = Math.max(1, map.clientHeight - 20);
    const scale = Math.min(availableWidth / desktopWidth, availableHeight / desktopHeight);
    const drawnWidth = desktopWidth * scale;
    const drawnHeight = desktopHeight * scale;
    const offsetX = (map.clientWidth - drawnWidth) / 2;
    const offsetY = (map.clientHeight - drawnHeight) / 2;

    displays.forEach((display, index) => {
      const tile = document.createElement("div");
      tile.className = `display-tile ${platformName === "Windows" ? "windows-wallpaper" : "mac-wallpaper"}`;
      tile.classList.toggle("is-primary", Boolean(display.primary));
      tile.style.left = `${offsetX + (Number(display.originX) - originX) * scale}px`;
      tile.style.top = `${offsetY + (Number(display.originY) - originY) * scale}px`;
      tile.style.width = `${Math.max(34, Number(display.width) * scale)}px`;
      tile.style.height = `${Math.max(34, Number(display.height) * scale)}px`;
      const number = document.createElement("b");
      number.textContent = display.primary ? "主屏" : String(index + 1);
      const resolution = document.createElement("small");
      resolution.textContent = `${Math.round(Number(display.pixelWidth))} × ${Math.round(Number(display.pixelHeight))}`;
      tile.title = `${platformName} ${display.primary ? "主屏" : `屏幕 ${index + 1}`} · ${resolution.textContent} · 位置 (${Math.round(Number(display.originX))}, ${Math.round(Number(display.originY))})`;
      tile.append(number, resolution);
      map.append(tile);
    });
  }

  const numberOrNull = (value) => {
    if (value === null || value === undefined || value === "") return null;
    const number = Number(value);
    return Number.isFinite(number) ? Math.max(0, number) : null;
  };

  function connectionState(snapshot) {
    if (!snapshot.agent.running) return "stopped";
    return snapshot.agent.connection?.state ?? "starting";
  }

  function formatDuration(startedAt) {
    if (!startedAt) return "刚刚连接";
    const seconds = Math.max(0, Math.floor((Date.now() - Number(startedAt)) / 1000));
    if (seconds < 60) return `已持续连接 ${seconds} 秒`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `已持续连接 ${minutes} 分钟`;
    const hours = Math.floor(minutes / 60);
    return `已持续连接 ${hours} 小时 ${minutes % 60} 分钟`;
  }

  function setMetricValue(metric, value, unit = "") {
    const strong = document.querySelector(`[data-metric="${metric}"] strong`);
    if (!strong) return;
    strong.replaceChildren(document.createTextNode(String(value)));
    if (unit) {
      const suffix = document.createElement("i");
      suffix.textContent = unit;
      strong.append(" ", suffix);
    }
  }

  function setMetricHealth(metric, healthy) {
    const card = document.querySelector(`[data-metric="${metric}"]`);
    if (!card) return;
    card.classList.toggle("is-offline", !healthy);
    const icon = card.querySelector(".metric-icon");
    icon?.classList.toggle("good", healthy);
    icon?.classList.toggle("pending", !healthy);
  }

  function chartX(index) {
    const offset = chartPointCount - qualityHistory.length;
    return ((offset + index) / (chartPointCount - 1)) * chartWidth;
  }

  function chartY(value, maximum) {
    const padding = 8;
    return chartHeight - padding - (value / maximum) * (chartHeight - padding * 2);
  }

  function chartLinePath(key, maximum) {
    let path = "";
    let drawing = false;
    qualityHistory.forEach((sample, index) => {
      const value = sample?.[key];
      if (value === null || value === undefined) {
        drawing = false;
        return;
      }
      const point = `${chartX(index).toFixed(2)} ${chartY(value, maximum).toFixed(2)}`;
      path += `${drawing ? " L" : "M"} ${point}`;
      drawing = true;
    });
    return path;
  }

  function chartFillPath(maximum) {
    const segments = [];
    let segment = [];
    qualityHistory.forEach((sample, index) => {
      if (sample?.latency === null || sample?.latency === undefined) {
        if (segment.length) segments.push(segment);
        segment = [];
        return;
      }
      segment.push({ x: chartX(index), y: chartY(sample.latency, maximum) });
    });
    if (segment.length) segments.push(segment);
    return segments.map((points) => {
      const first = points[0];
      const last = points[points.length - 1];
      const line = points.map((point) => `L ${point.x.toFixed(2)} ${point.y.toFixed(2)}`).join(" ");
      return `M ${first.x.toFixed(2)} ${chartHeight} ${line} L ${last.x.toFixed(2)} ${chartHeight} Z`;
    }).join(" ");
  }

  function renderQualityChart() {
    const values = qualityHistory.flatMap((sample) => sample ? [sample.latency, sample.jitter] : []).filter(Number.isFinite);
    const maximum = Math.max(20, Math.ceil((Math.max(0, ...values) * 1.2) / 10) * 10);
    document.querySelector("[data-chart-latency]")?.setAttribute("d", chartLinePath("latency", maximum));
    document.querySelector("[data-chart-jitter]")?.setAttribute("d", chartLinePath("jitter", maximum));
    document.querySelector("[data-chart-fill]")?.setAttribute("d", chartFillPath(maximum));
    const empty = document.querySelector(".chart-empty");
    if (empty) {
      const hasValues = values.length > 0;
      empty.hidden = hasValues;
      empty.style.display = hasValues ? "none" : "";
      empty.setAttribute("aria-hidden", String(hasValues));
    }
  }

  function recordQualitySample(connection, connected) {
    const currentSecond = Math.floor(Date.now() / 1000);
    if (lastQualitySecond === currentSecond) return;
    if (lastQualitySecond !== undefined) {
      const missing = Math.min(chartPointCount, Math.max(0, currentSecond - lastQualitySecond - 1));
      for (let index = 0; index < missing; index += 1) qualityHistory.push(null);
    }
    lastQualitySecond = currentSecond;
    const metricAge = connection?.metricsUpdatedUnixMs
      ? Date.now() - Number(connection.metricsUpdatedUnixMs)
      : Number.POSITIVE_INFINITY;
    const metricsFresh = connected && metricAge <= 15_000;
    qualityHistory.push(metricsFresh ? {
      latency: numberOrNull(connection.rttMs),
      jitter: numberOrNull(connection.jitterMs),
    } : null);
    while (qualityHistory.length > chartPointCount) qualityHistory.shift();
    renderQualityChart();
  }

  function updateDiagnostics(snapshot) {
    const connection = snapshot.agent.connection;
    const state = connectionState(snapshot);
    const connected = state === "connected";
    const rtt = numberOrNull(connection?.rttMs);
    const jitter = numberOrNull(connection?.jitterMs);
    const reconnects = Number(connection?.reconnectCount ?? 0);

    setMetricValue("connection", connectionLabels[state] ?? "未知");
    setText(
      '[data-metric="connection"] p',
      connected ? `${formatDuration(connection?.connectedSinceUnixMs)} · 重连 ${reconnects} 次` : "本机控制保持可用",
    );
    setMetricHealth("connection", connected);

    setMetricValue("latency", rtt === null ? "—" : rtt.toFixed(1), rtt === null ? "" : "ms");
    setText('[data-metric="latency"] p', rtt === null ? "等待真实链路数据" : rtt <= 30 ? "连接质量良好" : rtt <= 60 ? "连接质量一般" : "延迟较高");
    setMetricHealth("latency", connected && rtt !== null);

    setMetricValue("jitter", jitter === null ? "—" : jitter.toFixed(1), jitter === null ? "" : "ms");
    setText('[data-metric="jitter"] p', jitter === null ? "等待真实链路数据" : `RTT 变化 · ${Number(connection?.staleMoves ?? 0)} 个过期事件`);
    setMetricHealth("jitter", connected && jitter !== null);

    const permission = snapshot.platform.permissionGranted;
    const windowsLocal = snapshot.platform.operatingSystem.toLowerCase().includes("windows");
    if (windowsLocal) {
      setMetricValue("permissions", "无需授权");
      setText('[data-metric="permissions"] p', "Windows 输入接口可用");
      setText('[data-check="permissions"] small', "Windows 输入接口可用");
      setMetricHealth("permissions", true);
    } else {
      setMetricValue("permissions", permission === true ? "已授权" : permission === false ? "未授权" : "待确认");
      setText('[data-metric="permissions"] p', permission === false ? "请检查系统输入权限" : "辅助功能与输入监控");
      setText('[data-check="permissions"] small', permission === false ? "请检查系统输入权限" : "捕获与注入可用");
      setMetricHealth("permissions", permission !== false);
    }

    setText(".chart-caption", "最近 60 秒 · 每秒刷新");
    setText(".chart-value b", rtt === null ? "—" : `${rtt.toFixed(1)} ms`);
    setText(".diagnostic-last-run", `实时监控 · 重连 ${reconnects} 次`);
    setText(
      '.summary-card[data-go="diagnostics"] small',
      rtt === null ? connectionLabels[state] ?? "等待数据" : `${rtt.toFixed(1)} ms · ${rtt <= 30 ? "良好" : rtt <= 60 ? "一般" : "较高"}`,
    );
    recordQualitySample(connection, connected);
  }

  function updateDeviceCards(snapshot) {
    const windowsLocal = snapshot.platform.operatingSystem.toLowerCase().includes("windows");
    const localSelector = windowsLocal ? ".windows-device" : ".mac-device";
    const peerSelector = windowsLocal ? ".mac-device" : ".windows-device";
    const state = connectionState(snapshot);
    const connected = state === "connected";

    setText(`${localSelector} h2`, snapshot.config.localName);
    setText(`${peerSelector} h2`, snapshot.config.peerScreenName);
    const localStatus = snapshot.agent.running
      ? snapshot.agent.statusFresh === false ? "本机状态确认中" : "本机运行中"
      : "本机服务未启动";
    setOnlineLabel(`${localSelector} .online`, localStatus);
    setOnlineLabel(`${peerSelector} .online`, connected ? "可信设备已连接" : connectionLabels[state] ?? "可信设备待连接");

    const localDesktop = snapshot.platform.desktop ?? desktopFromLegacy(
      snapshot.platform.desktopWidth,
      snapshot.platform.desktopHeight,
      snapshot.platform.scaleFactor,
      snapshot.platform.displayCount,
    );
    const peerDesktop = snapshot.agent.connection?.peerDesktop ?? null;
    const windowsDesktop = windowsLocal ? localDesktop : peerDesktop;
    const macDesktop = windowsLocal ? peerDesktop : localDesktop;
    renderDesktopMap(".screen-win", windowsDesktop, "Windows");
    renderDesktopMap(".screen-mac", macDesktop, "macOS");
    setText('[data-screen-fact="windows"]', desktopLayoutLabel(windowsDesktop));
    setText('[data-screen-fact="macos"]', desktopLayoutLabel(macDesktop));

    if (snapshot.config.peerOn && window.EdgeMouseLayout) {
      const uiEdge = windowsLocal ? snapshot.config.peerOn : oppositeEdge[snapshot.config.peerOn];
      if (uiEdge) window.EdgeMouseLayout.applySnapshot(uiEdge);
    }
  }

  function applySnapshot(snapshot) {
    latestSnapshot = snapshot;
    const running = snapshot.agent.running;
    const configValid = snapshot.config.valid;
    const state = connectionState(snapshot);
    const connected = state === "connected";
    const peerName = snapshot.agent.connection?.peerName ?? snapshot.config.peerScreenName ?? "可信设备";
    const localInputProfile = snapshot.platform.operatingSystem.toLowerCase().includes("windows")
      ? "windows-to-mac"
      : "mac-to-windows";
    window.EdgeMouseInputSettings?.applyLocalProfile(localInputProfile, {
      horizontal: snapshot.config.reverseScrollHorizontal,
      vertical: snapshot.config.reverseScrollVertical,
    });
    window.setEdgeMouseAppVersion?.(running ? snapshot.agent.version : snapshot.desktopVersion);
    setText("[data-native-mode]", "桌面应用 · 实时状态");

    setChip(
      ".connection-status-chip",
      connected,
      connected ? `已连接 ${peerName}` : connectionLabels[state] ?? "等待连接",
    );
    setText(
      ".connection-device-status b",
      connected ? `${peerName} · 双向 TLS` : running ? "自动发现与重连已启动" : "等待 EdgeMouse 后台服务",
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
      diagnostics.classList.toggle("good", connected && configValid);
      diagnostics.classList.toggle("pending", !(connected && configValid));
      diagnostics.textContent = connected && configValid ? "实时监控正常" : running ? "等待连接" : "需要处理";
      diagnostics.title = snapshot.agent.error ?? snapshot.config.error ?? "";
    }

    const serviceChip = document.querySelector("#page-settings .setting-panel .status-chip");
    if (serviceChip) {
      serviceChip.classList.toggle("good", running);
      serviceChip.classList.toggle("pending", !running);
      serviceChip.textContent = running
        ? snapshot.agent.statusFresh === false
          ? `正在确认后台状态 · PID ${snapshot.agent.processId}`
          : `后台服务正常 · PID ${snapshot.agent.processId}`
        : "后台服务未运行";
      serviceChip.title = snapshot.agent.error ?? "";
    }

    const configStatus = document.querySelector(".layout-config-status");
    if (configStatus) {
      configStatus.textContent = window.EdgeMouseLayout?.isDirty()
        ? "尚未保存"
        : configValid && connected
          ? "两端一致"
          : configValid
            ? "等待连接同步"
            : "配置读取失败";
      configStatus.title = snapshot.config.error ?? snapshot.config.path ?? "";
    }

    const layoutSaveStatus = document.querySelector(".layout-save-status");
    if (layoutSaveStatus && !window.EdgeMouseLayout?.isDirty() && connected) {
      layoutSaveStatus.textContent = "布局已保存，两端配置一致";
    }

    updateDeviceCards(snapshot);
    updateDiagnostics(snapshot);
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

  document.querySelector(".input-save-button")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    if (!latestSnapshot) {
      window.showEdgeMouseToast?.("正在读取本机配置，请稍后重试");
      return;
    }
    const localProfile = latestSnapshot.platform.operatingSystem.toLowerCase().includes("windows")
      ? "windows-to-mac"
      : "mac-to-windows";
    const settingsApi = window.EdgeMouseInputSettings;
    if (settingsApi?.getActiveProfile() !== localProfile) {
      window.showEdgeMouseToast?.("请在另一台设备上设置这个控制方向");
      return;
    }
    const settings = settingsApi?.getProfile(localProfile);
    if (!settings) return;
    button.disabled = true;
    try {
      const result = await invoke("save_scroll_settings", {
        reverseHorizontal: Boolean(settings.horizontal),
        reverseVertical: Boolean(settings.vertical),
      });
      const message = result.warning ?? "滚动方向已保存并立即生效";
      settingsApi.markSaved(result.warning ? message : "滚动方向已保存；其他输入选项仍为界面预览");
      window.showEdgeMouseToast?.(message);
      await refreshSnapshot();
    } catch (error) {
      console.error("Unable to save scroll settings", error);
      window.showEdgeMouseToast?.(`无法保存滚动方向：${error}`);
    } finally {
      button.disabled = false;
    }
  });

  document.querySelector(".layout-save-button")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    const peerOn = window.EdgeMouseLayout?.getEdge();
    if (!latestSnapshot || !peerOn) {
      window.showEdgeMouseToast?.("正在读取本机配置，请稍后重试");
      return;
    }
    button.disabled = true;
    try {
      const result = await invoke("save_layout", { peerOn });
      const message = result.warning ?? "布局已保存，正在同步并重新连接";
      window.EdgeMouseLayout?.markSaved(message);
      window.showEdgeMouseToast?.(message);
      await refreshSnapshot();
    } catch (error) {
      console.error("Unable to save screen layout", error);
      window.showEdgeMouseToast?.(`无法保存屏幕布局：${error}`);
    } finally {
      button.disabled = false;
    }
  });

  refreshSnapshot();
  window.setInterval(refreshSnapshot, 1000);
  window.addEventListener("resize", () => {
    if (latestSnapshot) updateDeviceCards(latestSnapshot);
  });
})();
