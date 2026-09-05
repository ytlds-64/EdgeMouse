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
  let desktopPreferences;
  let lastNotificationState;
  let pairingPollTimer;
  let pairingHostActive = false;
  let serviceActionPending = false;
  let pendingPermissionRepair = false;
  let updateProgressHideTimer;
  let lastDiagnosticExportPath;

  function formatBytes(bytes) {
    const value = Number(bytes);
    if (!Number.isFinite(value) || value < 0) return "";
    if (value < 1024) return `${Math.round(value)} B`;
    if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KB`;
    return `${(value / (1024 ** 2)).toFixed(1)} MB`;
  }

  function hideUpdateProgress(delay = 0) {
    window.clearTimeout(updateProgressHideTimer);
    updateProgressHideTimer = window.setTimeout(() => {
      const popover = document.querySelector(".update-progress-popover");
      if (popover) popover.hidden = true;
      document.body.classList.remove("is-updating");
    }, delay);
  }

  function renderUpdateProgress(progress) {
    const popover = document.querySelector(".update-progress-popover");
    if (!popover) return;
    window.clearTimeout(updateProgressHideTimer);
    const phase = progress?.phase ?? "downloading";
    const percent = Number.isFinite(Number(progress?.percent)) ? Math.max(0, Math.min(100, Number(progress.percent))) : null;
    const downloaded = Number(progress?.downloaded) || 0;
    const total = Number(progress?.total) || 0;
    const version = progress?.version ? ` ${progress.version}` : "";
    const titles = {
      downloading: `正在下载 EdgeMouse${version}`,
      installing: `正在安装 EdgeMouse${version}`,
      restarting: `EdgeMouse${version} 安装完成`,
      error: "更新失败",
    };
    const icons = { downloading: "↓", installing: "…", restarting: "✓", error: "!" };
    const detail = phase === "downloading" && downloaded > 0
      ? `${formatBytes(downloaded)}${total > 0 ? ` / ${formatBytes(total)}` : ""}`
      : progress?.message ?? "正在连接更新服务器…";
    popover.hidden = false;
    popover.classList.toggle("is-indeterminate", phase === "downloading" && percent === null);
    popover.classList.toggle("is-error", phase === "error");
    setText(".update-progress-title", titles[phase] ?? "正在更新 EdgeMouse");
    setText(".update-progress-detail", detail);
    setText(".update-progress-percent", percent === null ? "—" : `${Math.round(percent)}%`);
    setText(".update-progress-icon", icons[phase] ?? "↓");
    const bar = popover.querySelector(".update-progress-track i");
    if (bar) bar.style.width = `${percent ?? 0}%`;
    document.body.classList.add("is-updating");
  }

  const updateProgressSubscription = window.__TAURI__?.event?.listen?.("update-progress", (event) => {
    renderUpdateProgress(event.payload);
  });
  updateProgressSubscription?.catch((error) => console.error("Unable to subscribe to update progress", error));

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
      setText('[data-metric="permissions"] p', permission === false ? "请检查系统输入权限" : "辅助功能权限");
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
    const localScreenSelector = windowsLocal ? ".screen-win" : ".screen-mac";
    const peerScreenSelector = windowsLocal ? ".screen-mac" : ".screen-win";
    const state = connectionState(snapshot);
    const connected = state === "connected";

    setText(`${localSelector} h2`, snapshot.config.localName);
    setText(`${peerSelector} h2`, snapshot.config.peerScreenName);
    const localStatus = snapshot.agent.running
      ? snapshot.agent.statusFresh === false ? "本机状态确认中" : "本机运行中"
      : "本机服务未启动";
    setOnlineLabel(`${localSelector} .online`, localStatus);
    setOnlineLabel(`${peerSelector} .online`, connected ? "可信设备已连接" : connectionLabels[state] ?? "可信设备待连接");
    const localScreenBadge = document.querySelector(`${localScreenSelector} .screen-badge`);
    const peerScreenBadge = document.querySelector(`${peerScreenSelector} .screen-badge`);
    if (localScreenBadge) {
      localScreenBadge.textContent = "本机";
      localScreenBadge.classList.remove("connected");
    }
    if (peerScreenBadge) {
      peerScreenBadge.textContent = connected ? "已连接" : connectionLabels[state] ?? "未连接";
      peerScreenBadge.classList.toggle("connected", connected);
    }

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
    const pairingRequired = snapshot.config.pairingRequired === true;
    const state = connectionState(snapshot);
    const connected = state === "connected";
    const peerName = snapshot.agent.connection?.peerName ?? snapshot.config.peerScreenName ?? "可信设备";
    const windowsLocal = snapshot.platform.operatingSystem.toLowerCase().includes("windows");
    const localInputProfile = windowsLocal
      ? "windows-to-mac"
      : "mac-to-windows";
    const incomingInputProfile = windowsLocal ? "mac-to-windows" : "windows-to-mac";
    window.EdgeMouseInputSettings?.setLocalPlatform(windowsLocal ? "windows" : "macos");
    window.EdgeMouseInputSettings?.setOverviewProfile(localInputProfile);
    window.EdgeMouseInputSettings?.applyLocalProfile(localInputProfile, {
      horizontal: snapshot.config.reverseScrollHorizontal,
      vertical: snapshot.config.reverseScrollVertical,
      keyboard: snapshot.config.keyboardEnabled,
      dragLock: snapshot.config.blockSwitchWhileDragging,
    });
    window.EdgeMouseInputSettings?.applyLocalProfile(incomingInputProfile, {
      smoothing: snapshot.config.pointerSmoothing,
      reclaim: snapshot.config.reclaimEnabled,
    });
    // The About and Update pages describe the desktop application itself. The
    // background agent can temporarily be an older version during an upgrade,
    // so using its version here makes the title bar and About page disagree.
    window.setEdgeMouseAppVersion?.(snapshot.desktopVersion);
    setText("[data-native-mode]", "桌面应用 · 实时状态");

    const overviewConnectButton = document.querySelector(".overview-connect-button");
    if (overviewConnectButton && !overviewConnectButton.dataset.pending) {
      overviewConnectButton.disabled = false;
      overviewConnectButton.textContent = running ? "重新连接" : "开始连接";
    }

    if (!serviceActionPending) {
      const serviceToggle = document.querySelector("[data-service-toggle]");
      if (serviceToggle) {
        serviceToggle.classList.toggle("is-on", running);
        serviceToggle.setAttribute("aria-checked", String(running));
        serviceToggle.title = snapshot.agent.error ?? "";
      }
      setText(
        ".service-state-label",
        running ? "本机服务运行中" : pairingRequired ? "尚未配对 · 点击设置" : "本机服务未启动",
      );
    }

    setChip(
      ".connection-status-chip",
      connected,
      connected ? `已连接 ${peerName}` : pairingRequired ? "需要完成配对" : connectionLabels[state] ?? "等待连接",
    );
    setText(
      ".connection-device-status b",
      connected ? `${peerName} · 双向 TLS` : pairingRequired ? "请配对新设备或导入旧配对配置" : running ? "自动发现与重连已启动" : "等待 EdgeMouse 后台服务",
    );
    setText(
      ".peer-address",
      snapshot.config.peerAddress?.startsWith("auto") ? "自动获取 · UDP 43892" : snapshot.config.peerAddress,
    );
    setText(
      ".discovery-detail",
      snapshot.config.peerAddress?.startsWith("auto") ? "局域网自动发现" : "固定设备地址",
    );
    setText(".trust-detail", configValid ? "可信证书已载入" : pairingRequired ? "尚未选择可信设备" : "配置需要检查");
    setText(".certificate-line code", groupedNode(snapshot.config.peerNode));
    const autoReconnectToggle = document.querySelector(".auto-reconnect-toggle");
    if (autoReconnectToggle) {
      const enabled = snapshot.config.autoReconnect !== false;
      autoReconnectToggle.classList.toggle("is-on", enabled);
      autoReconnectToggle.setAttribute("aria-checked", String(enabled));
    }
    const edgeProtectionToggle = document.querySelector('[data-layout-setting="edgeProtection"]');
    if (edgeProtectionToggle && !window.EdgeMouseLayout?.isDirty()) {
      const enabled = Number(snapshot.config.entryHysteresis ?? 0) > 0;
      edgeProtectionToggle.classList.toggle("is-on", enabled);
      edgeProtectionToggle.setAttribute("aria-checked", String(enabled));
    }

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

    if (lastNotificationState !== undefined
      && desktopPreferences?.notifications
      && state !== lastNotificationState
      && ["connected", "reconnecting", "stopped"].includes(state)) {
      const notification = {
        connected: ["EdgeMouse 已连接", `已安全连接到 ${peerName}`],
        reconnecting: ["EdgeMouse 正在重连", "网络发生变化，正在自动恢复连接"],
        stopped: ["EdgeMouse 已停止", "本机鼠标和键盘控制保持可用"],
      }[state];
      invoke("show_system_notification", { title: notification[0], body: notification[1] }).catch(console.error);
    }
    lastNotificationState = state;
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

  async function refreshDesktopPreferences() {
    try {
      desktopPreferences = await invoke("get_desktop_preferences");
      window.EdgeMouseDesktopSettings?.apply(desktopPreferences);
    } catch (error) {
      console.error("Unable to load desktop settings", error);
    }
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (_error) {
      const input = document.createElement("textarea");
      input.value = text;
      input.style.position = "fixed";
      input.style.opacity = "0";
      document.body.append(input);
      input.select();
      document.execCommand("copy");
      input.remove();
    }
  }

  function renderDiagnosticReport(report) {
    let passed = 0;
    report.checks.forEach((check) => {
      const row = document.querySelector(`[data-check="${check.key}"]`);
      if (!row) return;
      passed += check.passed ? 1 : 0;
      row.classList.remove("is-running");
      row.classList.toggle("is-failed", !check.passed);
      const icon = row.querySelector(".check");
      if (icon) icon.textContent = check.passed ? "✓" : "×";
      const detail = row.querySelector("small");
      if (detail) detail.textContent = check.detail;
      row.querySelector(".diagnostic-repair-button")?.remove();
      if (!check.passed && check.repairAction && check.repairLabel) {
        const repair = document.createElement("button");
        repair.type = "button";
        repair.className = "diagnostic-repair-button";
        repair.dataset.repairAction = check.repairAction;
        repair.textContent = check.repairLabel;
        row.append(repair);
      }
    });
    const overall = document.querySelector(".diagnostics-overall");
    if (overall) {
      const allPassed = passed === report.checks.length;
      overall.classList.toggle("good", allPassed);
      overall.classList.toggle("pending", !allPassed);
      overall.textContent = allPassed ? "全部通过" : `${passed}/${report.checks.length} 项通过`;
    }
    const logContainer = document.querySelector(".log-lines");
    if (logContainer) {
      logContainer.replaceChildren();
      const lines = report.logLines.length ? report.logLines : ["暂时没有可显示的后台日志"];
      lines.forEach((message) => {
        const line = document.createElement("p");
        const time = document.createElement("time");
        time.textContent = new Date().toLocaleTimeString("zh-CN", { hour12: false });
        const tag = document.createElement("span");
        tag.textContent = "本机";
        line.append(time, tag, document.createTextNode(message));
        logContainer.append(line);
      });
    }
    const lastRun = document.querySelector(".diagnostic-last-run");
    if (lastRun) lastRun.textContent = "上次检查：刚刚";
  }

  async function readDiagnosticReport() {
    const report = await invoke("run_diagnostics");
    renderDiagnosticReport(report);
    return report;
  }

  document.querySelector(".checklist")?.addEventListener("click", async (event) => {
    const button = event.target.closest(".diagnostic-repair-button");
    if (!button) return;
    event.stopImmediatePropagation();
    const action = button.dataset.repairAction;
    if (action === "pair") {
      document.querySelector('.nav-item[data-page="connection"]')?.click();
      openRealPairingModal();
      window.showEdgeMouseToast?.("请完成安全配对；完成后诊断会自动恢复");
      return;
    }
    const originalLabel = button.textContent;
    button.disabled = true;
    button.textContent = "正在修复…";
    try {
      const result = await invoke("repair_diagnostic", { action });
      pendingPermissionRepair = Boolean(result.requiresUserAction);
      window.showEdgeMouseToast?.(result.message);
      await new Promise((resolve) => window.setTimeout(resolve, result.requiresUserAction ? 250 : 900));
      await refreshSnapshot();
      await readDiagnosticReport();
    } catch (error) {
      window.showEdgeMouseToast?.(`修复失败：${error}`, "error");
      button.disabled = false;
      button.textContent = originalLabel;
    }
  }, true);

  document.querySelector(".run-diagnostics-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    button.disabled = true;
    button.textContent = "正在检查…";
    document.querySelectorAll(".diagnostic-check").forEach((row) => {
      row.classList.remove("is-failed");
      row.classList.add("is-running");
      const icon = row.querySelector(".check");
      if (icon) icon.textContent = "…";
    });
    try {
      const report = await readDiagnosticReport();
      const passed = report.checks.filter((check) => check.passed).length;
      const failed = report.checks.length - passed;
      window.showEdgeMouseToast?.(failed === 0 ? "完整检查已通过" : `${failed} 项需要处理，可点击对应的修复按钮`);
    } catch (error) {
      window.showEdgeMouseToast?.(`诊断失败：${error}`);
    } finally {
      button.disabled = false;
      button.textContent = "再次运行检查";
    }
  }, true);

  document.querySelector(".copy-diagnostics-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    try {
      const report = await readDiagnosticReport();
      await copyText(report.summary);
      window.showEdgeMouseToast?.("真实诊断摘要已复制");
    } catch (error) {
      window.showEdgeMouseToast?.(`无法复制诊断摘要：${error}`);
    }
  }, true);

  document.querySelector(".open-logs-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    try {
      const result = await invoke("open_logs_folder");
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      window.showEdgeMouseToast?.(`无法打开日志文件夹：${error}`);
    }
  }, true);

  document.querySelector(".generate-diagnostics-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    const checked = new Set([...document.querySelectorAll(".export-options input:checked")].map((input) => input.value));
    const steps = [...document.querySelectorAll(".diagnostics-export-step")];
    const showStep = (name) => steps.forEach((step) => step.classList.toggle("is-active", step.classList.contains(`diagnostics-export-${name}`)));
    button.disabled = true;
    showStep("generating");
    try {
      const result = await invoke("export_diagnostics", {
        includeLogs: checked.has("logs"),
        includeConfig: checked.has("system"),
        includeSystem: checked.has("system") || checked.has("network"),
      });
      const name = result.path.split(/[\\/]/).pop();
      lastDiagnosticExportPath = result.path;
      setText(".export-file-name", name);
      const detail = document.querySelector(".export-file small");
      if (detail) detail.textContent = "已脱敏 · 下载目录";
      showStep("success");
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      showStep("options");
      window.showEdgeMouseToast?.(`无法生成诊断包：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelector(".reveal-diagnostics-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    if (!lastDiagnosticExportPath) {
      window.showEdgeMouseToast?.("请先生成诊断包");
      return;
    }
    button.disabled = true;
    try {
      const result = await invoke("reveal_file", { path: lastDiagnosticExportPath });
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      window.showEdgeMouseToast?.(`无法打开文件位置：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelectorAll("[data-device-action]").forEach((button) => {
    button.addEventListener("click", async (event) => {
      event.stopImmediatePropagation();
      const action = button.dataset.deviceAction;
      try {
        if (action === "copy") {
          const config = latestSnapshot?.config;
          await copyText([
            `EdgeMouse ${latestSnapshot?.desktopVersion ?? ""}`,
            `本机：${config?.localName ?? "—"}`,
            `可信设备：${config?.peerScreenName ?? "—"}`,
            `设备指纹：${config?.peerNode ?? "—"}`,
            `连接地址：${config?.peerAddress ?? "—"}`,
          ].join("\n"));
          window.showEdgeMouseToast?.("真实连接信息已复制");
        } else if (action === "verify") {
          const message = await invoke("verify_trusted_device");
          setText(".trust-detail", "刚刚重新验证");
          window.showEdgeMouseToast?.(message);
        } else if (action === "forget") {
          if (!window.confirm("确定解除当前可信设备吗？原证书会备份，后台连接将停止。")) return;
          const message = await invoke("forget_trusted_device");
          window.showEdgeMouseToast?.(message);
          await refreshSnapshot();
        }
      } catch (error) {
        window.showEdgeMouseToast?.(`设备操作失败：${error}`);
      }
    }, true);
  });

  const pairingModal = document.querySelector(".pairing-modal");
  const pairingSteps = [...document.querySelectorAll(".pairing-step")];
  const showPairingStep = (name) => pairingSteps.forEach((step) => step.classList.toggle("is-active", step.classList.contains(`pairing-step-${name}`)));

  function stopPairingPoll() {
    window.clearInterval(pairingPollTimer);
    pairingPollTimer = undefined;
  }

  function showPairingSuccess(status) {
    pairingHostActive = false;
    stopPairingPoll();
    setText(".pairing-success-message", status.message);
    showPairingStep("success");
    refreshSnapshot();
  }

  async function pollPairingStatus() {
    try {
      const status = await invoke("get_pairing_status");
      if (status.phase === "complete") {
        showPairingSuccess(status);
      } else if (status.phase === "failed") {
        pairingHostActive = false;
        stopPairingPoll();
        showPairingStep("discover");
        window.showEdgeMouseToast?.(status.message);
      } else if (status.phase === "hosting") {
        setText(".pairing-host-status", status.message);
      }
    } catch (error) {
      console.error("Unable to read pairing status", error);
    }
  }

  function openRealPairingModal() {
    stopPairingPoll();
    pairingHostActive = false;
    pairingModal.hidden = false;
    showPairingStep("discover");
    const code = document.querySelector("#pairing-code-input");
    const address = document.querySelector("#manual-peer-address");
    if (code) code.value = "";
    if (address) address.value = "";
    const error = pairingModal.querySelector(".field-error");
    if (error) error.hidden = true;
  }

  async function closeRealPairingModal() {
    stopPairingPoll();
    if (pairingHostActive) {
      pairingHostActive = false;
      await invoke("cancel_pairing").catch(console.error);
    }
    pairingModal.hidden = true;
    refreshSnapshot();
  }

  document.querySelector(".pair-device-button")?.addEventListener("click", (event) => {
    event.stopImmediatePropagation();
    openRealPairingModal();
  }, true);

  document.querySelector(".start-pairing-host-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    button.disabled = true;
    button.textContent = "正在生成…";
    try {
      const status = await invoke("start_pairing_host");
      pairingHostActive = true;
      setText(".pairing-code", status.code?.replace("-", " – "));
      setText(".pairing-host-status", status.message);
      showPairingStep("code");
      pairingPollTimer = window.setInterval(pollPairingStatus, 500);
    } catch (error) {
      window.showEdgeMouseToast?.(`无法创建配对码：${error}`);
    } finally {
      button.disabled = false;
      button.textContent = "生成一次性配对码";
    }
  }, true);

  document.querySelector(".join-pairing-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    const codeInput = document.querySelector("#pairing-code-input");
    const hostInput = document.querySelector("#manual-peer-address");
    const normalized = codeInput.value.replace(/\D/g, "");
    const errorLabel = pairingModal.querySelector(".field-error");
    if (normalized.length !== 8) {
      errorLabel.hidden = false;
      return;
    }
    errorLabel.hidden = true;
    button.disabled = true;
    button.textContent = "正在安全配对…";
    try {
      const status = await invoke("join_pairing", {
        code: `${normalized.slice(0, 4)}-${normalized.slice(4)}`,
        host: hostInput.value.trim() || null,
      });
      showPairingSuccess(status);
    } catch (error) {
      window.showEdgeMouseToast?.(`${error}`);
    } finally {
      button.disabled = false;
      button.textContent = "安全加入";
    }
  }, true);

  document.querySelector(".import-pairing-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    button.disabled = true;
    button.textContent = "正在导入…";
    try {
      const result = await invoke("import_existing_pairing");
      if (result.running) pairingModal.hidden = true;
      window.showEdgeMouseToast?.(result.message);
      await refreshSnapshot();
    } catch (error) {
      window.showEdgeMouseToast?.(`无法导入旧配对配置：${error}`, "error");
    } finally {
      button.disabled = false;
      button.textContent = "导入旧配对配置…";
    }
  }, true);

  document.querySelector(".pairing-cancel-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    await closeRealPairingModal();
  }, true);

  document.querySelector(".pairing-modal-close")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    await closeRealPairingModal();
  }, true);

  document.querySelector(".pairing-finish-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    await closeRealPairingModal();
    window.showEdgeMouseToast?.("安全配对完成，可信证书已保存");
  }, true);

  pairingModal?.addEventListener("click", async (event) => {
    if (event.target !== pairingModal) return;
    event.stopImmediatePropagation();
    await closeRealPairingModal();
  }, true);

  window.addEventListener("keydown", async (event) => {
    if (event.key !== "Escape" || pairingModal?.hidden) return;
    event.stopImmediatePropagation();
    await closeRealPairingModal();
  }, true);

  document.querySelector(".project-home-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    try {
      await invoke("open_external_url", { url: "https://github.com/ytlds-64/EdgeMouse" });
    } catch (error) {
      window.showEdgeMouseToast?.(`无法打开项目主页：${error}`);
    }
  }, true);

  document.querySelector('[data-about-action="issues"]')?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    try {
      await invoke("open_external_url", { url: "https://github.com/ytlds-64/EdgeMouse/issues" });
    } catch (error) {
      window.showEdgeMouseToast?.(`无法打开问题反馈：${error}`);
    }
  }, true);

  document.querySelector('[data-about-action="version"]')?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const snapshot = latestSnapshot;
    const text = `EdgeMouse ${snapshot?.desktopVersion ?? ""} · ${snapshot?.platform?.operatingSystem ?? ""} · QUIC · mutual TLS`;
    await copyText(text);
    window.showEdgeMouseToast?.("真实版本与平台信息已复制");
  }, true);

  document.querySelectorAll(".check-updates-button").forEach((button) => {
    button.addEventListener("click", async (event) => {
      event.stopImmediatePropagation();
      const install = button.dataset.installUpdate === "true";
      const updateVersion = button.dataset.updateVersion ?? "";
      const buttons = [...document.querySelectorAll(".check-updates-button")];
      buttons.forEach((item) => {
        item.disabled = true;
        item.textContent = install ? "正在下载并安装…" : "正在检查…";
      });
      const status = document.querySelector(".update-status");
      status?.classList.remove("good");
      status?.classList.add("pending");
      if (status) status.textContent = install ? "正在安装" : "正在检查";
      if (install) {
        renderUpdateProgress({ phase: "downloading", version: updateVersion, downloaded: 0, total: null, percent: null, message: "正在连接更新服务器…" });
      }
      try {
        const result = await invoke("check_for_updates", { install });
        setText(".update-last-check", "最后检查：刚刚");
        if (result.available) {
          if (status) status.textContent = `可更新至 ${result.version}`;
          buttons.forEach((item) => {
            item.dataset.installUpdate = "true";
            item.dataset.updateVersion = result.version;
            item.textContent = `下载并安装 ${result.version}`;
          });
        } else {
          status?.classList.remove("pending");
          status?.classList.add("good");
          if (status) status.textContent = "已是最新版";
          buttons.forEach((item) => {
            delete item.dataset.installUpdate;
            delete item.dataset.updateVersion;
            item.textContent = "再次检查更新";
          });
        }
        window.showEdgeMouseToast?.(result.message);
      } catch (error) {
        if (status) status.textContent = "检查失败";
        buttons.forEach((item) => {
          item.textContent = "重新检查更新";
        });
        if (install) {
          renderUpdateProgress({ phase: "error", downloaded: 0, total: null, percent: 0, message: `${error}` });
          hideUpdateProgress(8000);
        }
        window.showEdgeMouseToast?.(`${error}`, "error");
      } finally {
        buttons.forEach((item) => { item.disabled = false; });
      }
    }, true);
  });

  document.querySelector(".settings-save-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    const settings = window.EdgeMouseDesktopSettings?.get();
    if (!settings) return;
    button.disabled = true;
    try {
      const result = await invoke("save_desktop_preferences", settings);
      desktopPreferences = result.preferences;
      window.EdgeMouseDesktopSettings?.apply(result.preferences);
      window.EdgeMouseDesktopSettings?.markSaved(result.message);
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      window.showEdgeMouseToast?.(`无法保存桌面设置：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelector(".confirm-reset-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    button.disabled = true;
    try {
      const result = await invoke("reset_preferences");
      desktopPreferences = result.preferences;
      window.EdgeMouseDesktopSettings?.apply(result.preferences);
      document.querySelector(".reset-settings-modal").hidden = true;
      window.showEdgeMouseToast?.(result.message);
      await refreshSnapshot();
    } catch (error) {
      window.showEdgeMouseToast?.(`无法恢复默认设置：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelector("[data-service-toggle]")?.addEventListener("click", async (event) => {
    const toggle = event.currentTarget;
    const shouldRun = toggle.classList.contains("is-on");
    if (shouldRun && latestSnapshot?.config?.pairingRequired) {
      toggle.classList.remove("is-on");
      toggle.setAttribute("aria-checked", "false");
      document.querySelector('[data-page="connection"]')?.click();
      openRealPairingModal();
      window.showEdgeMouseToast?.("请先完成一次安全配对，或导入以前的配对配置");
      return;
    }
    serviceActionPending = true;
    toggle.disabled = true;
    toggle.setAttribute("aria-busy", "true");
    setText(".service-state-label", shouldRun ? "正在启动…" : "正在停止…");
    try {
      const result = await invoke("set_agent_running", { running: shouldRun });
      toggle.classList.toggle("is-on", result.running);
      toggle.setAttribute("aria-checked", String(result.running));
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      toggle.classList.toggle("is-on", !shouldRun);
      toggle.setAttribute("aria-checked", String(!shouldRun));
      console.error("Unable to change EdgeMouse service state", error);
      window.showEdgeMouseToast?.(`${shouldRun ? "无法启动" : "无法停止"} EdgeMouse：${error}`);
    } finally {
      serviceActionPending = false;
      toggle.disabled = false;
      toggle.removeAttribute("aria-busy");
      await refreshSnapshot();
    }
  });

  document.querySelector(".input-save-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    if (!latestSnapshot) {
      window.showEdgeMouseToast?.("正在读取本机配置，请稍后重试");
      return;
    }
    const settingsApi = window.EdgeMouseInputSettings;
    const profile = settingsApi?.getActiveProfile();
    const settings = settingsApi?.getProfile(profile);
    if (!settings) return;
    button.disabled = true;
    try {
      const result = await invoke("save_input_settings", {
        profile,
        reverseHorizontal: Boolean(settings.horizontal),
        reverseVertical: Boolean(settings.vertical),
        pointerSmoothing: Number(settings.smoothing),
        keyboardEnabled: Boolean(settings.keyboard),
        reclaimEnabled: Boolean(settings.reclaim),
        dragLock: Boolean(settings.dragLock),
      });
      const message = result.warning ?? (result.restarted ? "输入设置已保存，后台服务已重新连接" : "输入设置已保存");
      settingsApi.markSaved(message);
      window.showEdgeMouseToast?.(message);
      await refreshSnapshot();
    } catch (error) {
      console.error("Unable to save scroll settings", error);
      window.showEdgeMouseToast?.(`无法保存滚动方向：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelector(".overview-save-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    const settingsApi = window.EdgeMouseInputSettings;
    const profile = settingsApi?.getOverviewProfile();
    const settings = settingsApi?.getProfile(profile);
    if (!latestSnapshot || !settings) {
      window.showEdgeMouseToast?.("正在读取本机配置，请稍后重试");
      return;
    }
    button.disabled = true;
    try {
      const result = await invoke("save_input_settings", {
        profile,
        reverseHorizontal: Boolean(settings.horizontal),
        reverseVertical: Boolean(settings.vertical),
        pointerSmoothing: Number(settings.smoothing),
        keyboardEnabled: Boolean(settings.keyboard),
        reclaimEnabled: Boolean(settings.reclaim),
        dragLock: Boolean(settings.dragLock),
      });
      const message = result.warning ?? (result.restarted ? "滚轮方向已保存，后台服务已重新连接" : "滚轮方向已保存");
      settingsApi.markSaved(message);
      setText(".overview-save-status", message);
      window.showEdgeMouseToast?.(message);
      await refreshSnapshot();
    } catch (error) {
      window.showEdgeMouseToast?.(`无法保存滚轮方向：${error}`);
    } finally {
      button.disabled = false;
    }
  }, true);

  document.querySelector(".detect-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const button = event.currentTarget;
    button.disabled = true;
    button.textContent = "正在检测…";
    try {
      await refreshSnapshot();
      const localDesktop = latestSnapshot?.platform?.desktop;
      const count = usableDisplays(localDesktop).length || Number(localDesktop?.reportedDisplayCount) || 0;
      window.showEdgeMouseToast?.(`已重新读取本机 ${count} 个屏幕；另一台电脑会在连接后自动同步`);
    } catch (error) {
      window.showEdgeMouseToast?.(`屏幕检测失败：${error}`);
    } finally {
      button.disabled = false;
      button.textContent = "重新检测屏幕";
    }
  }, true);

  document.querySelector(".auto-reconnect-toggle")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    const toggle = event.currentTarget;
    const enabled = !toggle.classList.contains("is-on");
    toggle.classList.toggle("is-on", enabled);
    toggle.setAttribute("aria-checked", String(enabled));
    toggle.disabled = true;
    try {
      const result = await invoke("save_connection_settings", { autoReconnect: enabled });
      window.showEdgeMouseToast?.(result.warning ?? (enabled ? "自动重连已启用" : "自动重连已关闭"));
      await refreshSnapshot();
    } catch (error) {
      toggle.classList.toggle("is-on", !enabled);
      toggle.setAttribute("aria-checked", String(!enabled));
      window.showEdgeMouseToast?.(`无法保存连接设置：${error}`);
    } finally {
      toggle.disabled = false;
    }
  }, true);

  async function reconnectFrom(button, overview = false) {
    if (latestSnapshot?.config?.pairingRequired) {
      document.querySelector('[data-page="connection"]')?.click();
      openRealPairingModal();
      window.showEdgeMouseToast?.("请先完成一次安全配对，或导入以前的配对配置");
      return;
    }
    button.dataset.pending = "true";
    button.disabled = true;
    button.textContent = overview ? "连接中…" : "正在重新连接…";
    try {
      const result = await invoke("reconnect_agent");
      window.showEdgeMouseToast?.(result.message);
    } catch (error) {
      window.showEdgeMouseToast?.(`重新连接失败：${error}`);
    } finally {
      delete button.dataset.pending;
      button.disabled = false;
      if (!overview) button.textContent = "立即重新连接";
      await refreshSnapshot();
    }
  }

  document.querySelector(".reconnect-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    await reconnectFrom(event.currentTarget);
  }, true);

  document.querySelector(".overview-connect-button")?.addEventListener("click", async (event) => {
    event.stopImmediatePropagation();
    await reconnectFrom(event.currentTarget, true);
  }, true);

  document.querySelector(".layout-save-button")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    const peerOn = window.EdgeMouseLayout?.getEdge();
    if (!latestSnapshot || !peerOn) {
      window.showEdgeMouseToast?.("正在读取本机配置，请稍后重试");
      return;
    }
    button.disabled = true;
    try {
      const edgeProtection = document.querySelector('[data-layout-setting="edgeProtection"]')?.classList.contains("is-on") ?? true;
      const result = await invoke("save_layout", { peerOn, edgeProtection });
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

  refreshDesktopPreferences();
  refreshSnapshot();
  window.setInterval(refreshSnapshot, 1000);
  window.addEventListener("focus", () => {
    if (!pendingPermissionRepair) return;
    window.setTimeout(async () => {
      try {
        const report = await readDiagnosticReport();
        pendingPermissionRepair = report.checks.some((check) => check.key === "permissions" && !check.passed);
        if (!pendingPermissionRepair) window.showEdgeMouseToast?.("输入权限已生效，诊断已自动复检通过");
      } catch (error) {
        console.error("Unable to recheck permissions", error);
      }
    }, 500);
  });
  window.addEventListener("resize", () => {
    if (latestSnapshot) updateDeviceCards(latestSnapshot);
  });
})();
