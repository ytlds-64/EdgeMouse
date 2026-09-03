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
  const qualityHistory = [];
  const chartWidth = 600;
  const chartHeight = 150;
  const chartPointCount = 60;
  let lastQualitySecond;

  const connectionLabels = {
    starting: "正在启动",
    connecting: "正在连接",
    connected: "已连接",
    reconnecting: "正在重连",
    stopped: "未运行",
  };

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
    if (empty) empty.hidden = !values.length;
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
    setText('[data-metric="jitter"] p', jitter === null ? "等待真实链路数据" : `最近 5 秒 · ${Number(connection?.staleMoves ?? 0)} 个过期事件`);
    setMetricHealth("jitter", connected && jitter !== null);

    const permission = snapshot.platform.permissionGranted;
    setMetricValue("permissions", permission === true ? "已授权" : permission === false ? "未授权" : "待确认");
    setText('[data-metric="permissions"] p', permission === false ? "请检查系统输入权限" : "捕获与注入权限");
    setMetricHealth("permissions", permission !== false);

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
    const localLayoutSelector = windowsLocal ? ".screen-win" : ".screen-mac";
    const state = connectionState(snapshot);
    const connected = state === "connected";

    setText(`${localSelector} h2`, snapshot.config.localName);
    setText(`${peerSelector} h2`, snapshot.config.peerScreenName);
    setOnlineLabel(`${localSelector} .online`, snapshot.agent.running ? "本机运行中" : "本机服务未启动");
    setOnlineLabel(`${peerSelector} .online`, connected ? "可信设备已连接" : connectionLabels[state] ?? "可信设备待连接");

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
    const state = connectionState(snapshot);
    const connected = state === "connected";
    const peerName = snapshot.agent.connection?.peerName ?? snapshot.config.peerScreenName ?? "可信设备";
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

  refreshSnapshot();
  window.setInterval(refreshSnapshot, 1000);
})();
