(() => {
  const zhToEn = {
    "主导航": "Main navigation",
    "窗口标题栏": "Window title bar",
    "最小化": "Minimize",
    "最大化": "Maximize",
    "关闭": "Close",
    "概览": "Overview",
    "屏幕布局": "Screen layout",
    "输入": "Input",
    "连接": "Connection",
    "诊断": "Diagnostics",
    "设置": "Settings",
    "关于": "About",
    "查看设备连接状态与关键设置": "View device status and essential settings",
    "外观原型": "Visual prototype",
    "已连接": "Connected",
    "自动发现 · 有线网络": "Auto discovery · Ethernet",
    "自动发现 · Wi‑Fi": "Auto discovery · Wi-Fi",
    "双向连接": "Bidirectional connection",
    "将鼠标推过屏幕边缘即可切换控制": "Push the pointer across a screen edge to switch control",
    "开机自动连接": "Connect at startup",
    "已启用": "Enabled",
    "边缘切换": "Edge switching",
    "Windows 右侧": "Right of Windows",
    "网络延迟": "Network latency",
    "18 ms · 良好": "18 ms · Good",
    "反转横向滚动": "Reverse horizontal scrolling",
    "反转纵向滚动": "Reverse vertical scrolling",
    "设置会在两台设备下次连接时同步": "Settings sync the next time both devices connect",
    "保存设置": "Save settings",
    "按实际摆放位置排列设备，边缘切换会自动匹配": "Arrange devices to match their physical positions",
    "重新检测屏幕": "Detect displays again",
    "拖动 Windows PC 调整屏幕位置": "Drag Windows PC to adjust its position",
    "拖动 MacBook Air 调整屏幕位置": "Drag MacBook Air to adjust its position",
    "主设备": "Primary device",
    "穿越边缘": "Crossing edge",
    "按住设备卡片，拖到另一台设备的左、右、上或下方": "Drag a device card to the left, right, top, or bottom of the other device",
    "布局设置": "Layout settings",
    "自动识别": "Auto detected",
    "Mac 位于 Windows 的": "Mac is positioned",
    "左侧": "Left",
    "右侧": "Right",
    "上方": "Above",
    "下方": "Below",
    "自动检测分辨率": "Detect display geometry automatically",
    "包含缩放、旋转和多显示器": "Includes scaling, rotation, and multiple displays",
    "防止边缘误触": "Prevent accidental edge crossing",
    "越过边缘 8 px 后切换": "Switch after crossing the edge by 8 px",
    "Windows 桌面": "Windows desktop",
    "1 个屏幕 · 横向": "1 display · Landscape",
    "macOS 桌面": "macOS desktop",
    "2 个屏幕 · 自动布局": "2 displays · Automatic layout",
    "配置状态": "Configuration",
    "两端一致": "Synchronized",
    "屏幕变化会在重新连接后自动更新": "Display changes update automatically after reconnecting",
    "保存布局": "Save layout",
    "设置鼠标、触控板和键盘在另一台设备上的使用方式": "Configure mouse, trackpad, and keyboard behavior on the other device",
    "分别设置两个控制方向": "Configure each control direction separately",
    "切换方向不会影响另一侧已经保存的偏好": "Switching direction does not alter saved settings for the other side",
    "输入控制方向": "Input direction",
    "触控板与 Mac 键盘": "Trackpad and Mac keyboard",
    "鼠标与 Windows 键盘": "Mouse and Windows keyboard",
    "鼠标与触控板": "Mouse and trackpad",
    "两台设备可分别使用不同方向": "Each direction can use different settings",
    "修正 macOS 触控板控制 Windows 时的方向": "Correct trackpad direction when controlling Windows",
    "单独调整 Windows 鼠标在 macOS 中的滚动方向": "Adjust Windows mouse scrolling on macOS separately",
    "按当前控制方向单独保存": "Saved separately for this control direction",
    "远程指针平滑度": "Remote pointer smoothing",
    "平衡跟手性与网络抖动": "Balance responsiveness and network jitter",
    "跟手": "Responsive",
    "均衡": "Balanced",
    "更平滑": "Smoother",
    "键盘": "Keyboard",
    "键盘随当前鼠标一起切换": "Keyboard follows the active pointer",
    "键盘跟随鼠标": "Keyboard follows pointer",
    "进入另一台设备时自动转发按键": "Forward keys automatically on the other device",
    "键盘映射": "Keyboard mapping",
    "主修饰键映射": "Primary modifier mapping",
    "次修饰键映射": "Secondary modifier mapping",
    "输入法切换键映射": "Input language shortcut mapping",
    "Windows 键": "Windows key",
    "保持原键": "Keep original key",
    "Windows Ctrl": "Windows Ctrl",
    "Windows Alt": "Windows Alt",
    "Windows Shift + Space": "Windows Shift + Space",
    "Windows 键 + Space": "Windows key + Space",
    "不映射": "Do not map",
    "Mac Control": "Mac Control",
    "Mac Command": "Mac Command",
    "Mac Option": "Mac Option",
    "Mac 中 / 英": "Mac Input Source",
    "Mac Control + Space": "Mac Control + Space",
    "控制权切换": "Control switching",
    "决定何时把输入交给另一台设备": "Choose when input moves to the other device",
    "边缘触发方式": "Edge trigger",
    "推动穿越": "Push across",
    "停留后切换": "Switch after dwell",
    "本机反向抢回": "Reverse-edge local reclaim",
    "鼠标卡住时可从目标边缘抢回控制": "Reclaim control from the target edge if the pointer gets stuck",
    "拖拽时禁止切换": "Block switching while dragging",
    "避免拖动文件时误穿越屏幕": "Prevents accidental crossing while dragging files",
    "紧急恢复": "Emergency recovery",
    "如果输入失去响应，同时按下以下组合键即可立即恢复本机控制。": "If input stops responding, press this shortcut to restore local control immediately.",
    "修改输入设置不会改变系统自身的鼠标设置": "Input settings do not change the operating system's own mouse settings",
    "保存输入设置": "Save input settings",
    "管理自动发现、可信设备与安全配对": "Manage discovery, trusted devices, and secure pairing",
    "安全连接正常": "Secure connection healthy",
    "已配对设备": "Paired device",
    "已连接 · Wi‑Fi": "Connected · Wi-Fi",
    "更多操作": "More actions",
    "复制连接信息": "Copy connection information",
    "重新验证证书": "Verify certificate again",
    "解除配对…": "Unpair…",
    "查找方式": "Discovery method",
    "局域网自动发现": "Local network discovery",
    "当前地址": "Current address",
    "自动获取 · 192.168.8.189": "Automatic · 192.168.8.189",
    "加密方式": "Encryption",
    "双向 TLS": "Mutual TLS",
    "可信状态": "Trust status",
    "证书已验证": "Certificate verified",
    "设备指纹": "Device fingerprint",
    "复制": "Copy",
    "连接选项": "Connection options",
    "自动发现可信设备": "Discover trusted devices automatically",
    "IP 变化后无需重新配对": "No pairing required after IP changes",
    "断开后自动重连": "Reconnect automatically",
    "网络恢复后自动继续连接": "Resume when the network returns",
    "正在监听可信设备": "Listening for trusted devices",
    "UDP 43892 · 地址变化自动更新": "UDP 43892 · Address changes update automatically",
    "立即重新连接": "Reconnect now",
    "配对新设备": "Pair a new device",
    "私钥始终保留在本机": "Private keys always stay on this device",
    "设备之间仅交换公开证书；局域网发现结果仍需通过已保存证书完成双向验证。": "Devices exchange public certificates only; discovered peers must still pass mutual verification with saved certificates.",
    "了解安全设计 ›": "Learn about security ›",
    "检查连接质量、系统权限与最近运行情况": "Check connection quality, system permissions, and recent activity",
    "桌面应用 · 实时状态": "Desktop app · Live status",
    "正在启动": "Starting",
    "正在连接": "Connecting",
    "正在重连": "Reconnecting",
    "未运行": "Not running",
    "未知": "Unknown",
    "本机控制保持可用": "Local control remains available",
    "等待真实链路数据": "Waiting for live link data",
    "连接质量良好": "Connection quality is good",
    "连接质量一般": "Connection quality is fair",
    "延迟较高": "Latency is high",
    "请检查系统输入权限": "Check system input permissions",
    "捕获与注入权限": "Capture and injection permissions",
    "最近 60 秒 · 每秒刷新": "Last 60 seconds · Updated every second",
    "等待连接质量数据…": "Waiting for connection quality data…",
    "实时监控正常": "Live monitoring healthy",
    "等待连接": "Waiting to connect",
    "等待数据": "Waiting for data",
    "可信设备已连接": "Trusted device connected",
    "本机运行中": "Running locally",
    "本机服务未启动": "Local service is not running",
    "后台服务未启动": "Background service is not running",
    "自动发现与重连已启动": "Auto discovery and reconnection are active",
    "等待 EdgeMouse 后台服务": "Waiting for the EdgeMouse background service",
    "上次检查：18 分钟前": "Last check: 18 minutes ago",
    "运行完整检查": "Run full check",
    "连接状态": "Connection status",
    "正常": "Healthy",
    "已持续连接 18 分钟": "Connected for 18 minutes",
    "当前延迟": "Current latency",
    "适合流畅控制": "Suitable for smooth control",
    "抖动": "Jitter",
    "最近 5 秒平均值": "5-second average",
    "系统权限": "System permissions",
    "已授权": "Authorized",
    "辅助功能与本地网络": "Accessibility and local network",
    "连接质量": "Connection quality",
    "最近 60 秒": "Last 60 seconds",
    "延迟": "Latency",
    "当前": "Current",
    "60 秒前": "60 seconds ago",
    "现在": "Now",
    "运行检查": "Checks",
    "全部通过": "All passed",
    "双向验证正常": "Mutual verification healthy",
    "可信证书": "Trusted certificate",
    "输入权限": "Input permissions",
    "自动恢复": "Automatic recovery",
    "UDP 43892 可用": "UDP 43892 available",
    "捕获与注入可用": "Capture and injection available",
    "心跳和紧急快捷键正常": "Heartbeat and emergency shortcut healthy",
    "最近日志": "Recent log",
    "网络": "Network",
    "敏感字段会在导出时自动隐藏": "Sensitive fields are hidden automatically during export",
    "复制摘要": "Copy summary",
    "打开日志文件夹": "Open log folder",
    "导出诊断包": "Export diagnostics",
    "调整 EdgeMouse 的启动、外观和后台行为": "Configure startup, appearance, and background behavior",
    "常规": "General",
    "后台服务正常": "Background service healthy",
    "登录时启动 EdgeMouse": "Launch EdgeMouse at login",
    "进入桌面后自动连接可信设备": "Connect to trusted devices after login",
    "关闭窗口后继续运行": "Keep running after closing the window",
    "从菜单栏或系统托盘再次打开": "Reopen from the menu bar or system tray",
    "显示连接通知": "Show connection notifications",
    "连接、断开和控制权变化时提醒": "Notify on connect, disconnect, and control changes",
    "外观": "Appearance",
    "跟随系统 · 浅色": "Follow system · Light",
    "跟随系统 · 深色": "Follow system · Dark",
    "界面主题": "Interface theme",
    "跟随系统": "Follow system",
    "当前为浅色": "Currently light",
    "当前为深色": "Currently dark",
    "浅色": "Light",
    "始终使用浅色": "Always use light",
    "深色": "Dark",
    "始终使用深色": "Always use dark",
    "适合暗光环境": "Best for dim environments",
    "界面语言": "Interface language",
    "简体中文": "Chinese (Simplified)",
    "更新": "Updates",
    "已是最新版": "Up to date",
    "最后检查：刚刚": "Last checked: just now",
    "更新通道": "Update channel",
    "稳定版": "Stable",
    "预览版": "Preview",
    "检查更新": "Check for updates",
    "重置": "Reset",
    "保留证书": "Certificates kept",
    "恢复界面、输入和连接偏好，不会删除设备身份、私钥或可信证书。": "Restore appearance, input, and connection preferences without deleting identities, private keys, or trusted certificates.",
    "恢复默认设置…": "Restore defaults…",
    "主题和语言会立即预览，并在下次启动继续使用": "Theme and language preview instantly and persist for the next launch",
    "EdgeMouse 版本、开源许可和产品信息": "EdgeMouse version, licenses, and product information",
    "版本": "Version",
    "· 稳定版": "· Stable",
    "让 Windows 与 macOS 像一张连续的桌面一样自然协作。": "Make Windows and macOS work together like one continuous desktop.",
    "查看项目主页": "View project page",
    "当前能力": "Current capabilities",
    "MVP 已就绪": "MVP ready",
    "双向鼠标": "Bidirectional pointer",
    "键盘跟随": "Keyboard follows pointer",
    "自动屏幕识别": "Automatic display detection",
    "局域网发现": "Local network discovery",
    "安全配对": "Secure pairing",
    "自动重连": "Automatic reconnect",
    "诊断导出": "Diagnostics export",
    "构建信息": "Build information",
    "平台": "Platforms",
    "传输": "Transport",
    "QUIC · 双向 TLS": "QUIC · Mutual TLS",
    "项目": "Project",
    "开源 · MIT License": "Open source · MIT License",
    "开源许可": "Open-source license",
    "第三方组件": "Third-party components",
    "查看清单 ›": "View list ›",
    "问题反馈": "Report an issue",
    "诊断信息": "Diagnostic information",
    "复制版本信息 ›": "Copy version information ›",
    "© 2026 EdgeMouse contributors · 为跨平台桌面协作而设计": "© 2026 EdgeMouse contributors · Designed for cross-platform desktop collaboration",
    "关闭配对窗口": "Close pairing window",
    "查找设备方式": "Device discovery method",
    "自动发现": "Automatic discovery",
    "手动地址": "Manual address",
    "正在查找附近设备…": "Looking for nearby devices…",
    "请确保另一台设备已打开 EdgeMouse": "Make sure EdgeMouse is open on the other device",
    "自动发现 ·": "Auto discovery ·",
    "配对 ›": "Pair ›",
    "另一台设备的局域网地址": "Local network address of the other device",
    "例如 192.168.8.189": "For example, 192.168.8.189",
    "自动发现不可用时，可输入对方当前 IP；配对完成后仍会自动追踪地址变化。": "If discovery is unavailable, enter the current IP. Address changes are tracked after pairing.",
    "请输入正确的 IPv4 地址": "Enter a valid IPv4 address",
    "连接此设备": "Connect to this device",
    "确认两台设备显示相同配对码": "Confirm that both devices show the same pairing code",
    "这可以防止局域网中的其他设备冒充你的电脑。": "This prevents other devices on the network from impersonating your computer.",
    "证书待确认": "Certificate pending",
    "返回": "Back",
    "配对码一致": "Codes match",
    "安全配对完成": "Secure pairing complete",
    "证书已经保存。以后 IP 地址发生变化时，EdgeMouse 仍会自动找到并验证这台设备。": "The certificate is saved. EdgeMouse will continue to find and verify this device after its IP changes.",
    "完成": "Done",
    "故障排查": "Troubleshooting",
    "关闭诊断包窗口": "Close diagnostics window",
    "选择要打包的信息。私钥、配对码、完整证书和键盘内容始终不会导出。": "Choose what to include. Private keys, pairing codes, full certificates, and keyboard content are never exported.",
    "最近运行日志": "Recent runtime logs",
    "连接、切换和错误事件，最多 2 MB": "Connection, switching, and error events, up to 2 MB",
    "已脱敏": "Redacted",
    "连接质量记录": "Connection quality history",
    "最近 5 分钟延迟、抖动与重连次数": "Latency, jitter, and reconnects from the last 5 minutes",
    "推荐": "Recommended",
    "系统与配置摘要": "System and configuration summary",
    "版本、屏幕布局、权限状态和设备类型": "Version, display layout, permissions, and device types",
    "无密钥": "No secrets",
    "导出前自动保护隐私": "Privacy protected before export",
    "IP 地址只保留网段，设备指纹仅保留首尾各 4 位。": "IP addresses keep only the subnet; fingerprints keep the first and last four digits.",
    "取消": "Cancel",
    "生成诊断包": "Generate diagnostics",
    "正在整理诊断信息": "Preparing diagnostics",
    "正在脱敏日志并汇总连接质量…": "Redacting logs and summarizing connection quality…",
    "诊断包已生成": "Diagnostics generated",
    "文件已保存在下载目录，可以直接发给开发人员分析。": "The file is saved in Downloads and is ready to send for analysis.",
    "约 186 KB · 已脱敏": "About 186 KB · Redacted",
    "恢复默认值": "Restore defaults",
    "确认重置设置？": "Reset all settings?",
    "关闭重置窗口": "Close reset window",
    "以下偏好会恢复为推荐值：": "The following preferences will return to recommended values:",
    "界面主题与语言": "Theme and language",
    "双向鼠标、键盘映射与平滑度": "Bidirectional pointer, keyboard mappings, and smoothing",
    "自动发现、重连和通知选项": "Discovery, reconnect, and notification options",
    "设备证书不会删除": "Device certificates will not be deleted",
    "完成后无需重新配对 Windows 与 Mac。": "Windows and Mac will not need to be paired again.",
    "恢复默认设置": "Restore default settings",
    "开源信息": "Open-source information",
    "自由使用，也保留署名": "Free to use with attribution",
    "EdgeMouse 采用 MIT 许可证。你可以使用、复制、修改、合并和分发软件，但需要保留原始版权与许可声明。": "EdgeMouse uses the MIT License. You may use, copy, modify, merge, and distribute the software while retaining the original copyright and license notice.",
    "软件按“原样”提供，不附带任何形式的保证。": "The software is provided as-is, without warranty of any kind.",
    "核心依赖清单": "Core dependencies",
    "跨平台与安全传输": "Cross-platform and secure transport",
    "异步运行时与任务调度": "Asynchronous runtime and task scheduling",
    "低延迟 QUIC 连接": "Low-latency QUIC connections",
    "双向 TLS 与证书验证": "Mutual TLS and certificate verification",
    "Windows 输入捕获与注入": "Windows input capture and injection",
    "macOS 指针与键盘事件": "macOS pointer and keyboard events",
    "关闭信息窗口": "Close information window",
    "原型操作已完成": "Prototype action completed",
    "有尚未保存的设置": "There are unsaved settings",
    "界面语言已切换，保存后会在下次启动继续使用": "Language changed and will persist for the next launch",
    "所有设置均已保存": "All settings saved",
    "默认设置已恢复并保存": "Default settings restored and saved",
    "有尚未保存的输入设置": "There are unsaved input settings",
    "两个控制方向的设置已分别保存": "Both control directions have been saved",
    "两个控制方向已恢复为推荐值": "Both control directions restored to recommended values",
    "等待检查": "Waiting",
    "检查中": "Checking",
    "正在检查": "Checking",
    "正在检查…": "Checking…",
    "再次运行检查": "Run check again",
    "上次检查：刚刚": "Last check: just now",
    "正在重新连接": "Reconnecting",
    "正在寻找可信设备…": "Looking for a trusted device…",
    "自动发现已暂停": "Automatic discovery paused",
    "将继续使用最后一次已知地址": "The last known address will continue to be used",
    "使用最后已知地址": "Using last known address",
    "正在重新连接…": "Reconnecting…",
    "自动获取 · 192.168.8.202": "Automatic · 192.168.8.202",
    "正在验证证书…": "Verifying certificate…",
    "发现 1 台可配对设备": "Found 1 device available for pairing",
    "已通过局域网广播验证设备响应": "Device response verified through local discovery",
    "通过手动地址": "Via manual address",
    "手动地址设备": "Manual-address device",
    "正在验证双方证书…": "Verifying both certificates…",
    "正在测试 UDP 43892…": "Testing UDP 43892…",
    "正在检查捕获与注入…": "Checking capture and injection…",
    "正在模拟心跳中断…": "Simulating a heartbeat interruption…",
    "安全": "Security",
    "发现": "Discovery",
    "权限": "Permissions",
    "恢复": "Recovery",
    "双向输入设置已保存": "Bidirectional input settings saved",
    "已通过自动发现重新连接可信设备": "Trusted device reconnected through automatic discovery",
    "连接信息已复制": "Connection information copied",
    "可信证书验证通过": "Trusted certificate verified",
    "解除配对需要再次确认": "Unpairing requires confirmation",
    "刚刚重新验证": "Verified again just now",
    "安全配对完成，已保存可信证书": "Secure pairing complete and trusted certificate saved",
    "完整检查已通过": "Full check passed",
    "诊断摘要已复制": "Diagnostic summary copied",
    "已打开日志文件夹": "Log folder opened",
    "诊断包已生成": "Diagnostics package generated",
    "设置已保存": "Settings saved",
    "已恢复默认设置，设备证书保持不变": "Defaults restored; device certificates were kept",
    "将在 GitHub Issues 中打开问题反馈": "Issue reporting will open in GitHub Issues",
    "版本与诊断信息已复制": "Version and diagnostic information copied",
    "将在浏览器中打开 EdgeMouse 项目主页": "The EdgeMouse project page will open in the browser",
    "内容已复制": "Content copied",
  };

  const originalText = new WeakMap();
  const originalAttributes = new WeakMap();
  const translatedAttributes = ["aria-label", "placeholder", "data-title", "title"];
  let currentLanguage = "zh-CN";
  let observer;

  function translateDynamic(value) {
    const checks = value.match(/^检查中 (\d+) \/ (\d+)$/);
    if (checks) return `Checking ${checks[1]} / ${checks[2]}`;
    const connectedTo = value.match(/^已连接 (.+)$/);
    if (connectedTo) return `Connected to ${connectedTo[1]}`;
    const justConnected = value.match(/^刚刚连接 · 重连 (\d+) 次$/);
    if (justConnected) return `Just connected · ${justConnected[1]} reconnects`;
    const securePeer = value.match(/^(.+) · 双向 TLS$/);
    if (securePeer) return `${securePeer[1]} · Mutual TLS`;
    const connectedSeconds = value.match(/^已持续连接 (\d+) 秒 · 重连 (\d+) 次$/);
    if (connectedSeconds) return `Connected for ${connectedSeconds[1]} seconds · ${connectedSeconds[2]} reconnects`;
    const connectedMinutes = value.match(/^已持续连接 (\d+) 分钟 · 重连 (\d+) 次$/);
    if (connectedMinutes) return `Connected for ${connectedMinutes[1]} minutes · ${connectedMinutes[2]} reconnects`;
    const connectedHours = value.match(/^已持续连接 (\d+) 小时 (\d+) 分钟 · 重连 (\d+) 次$/);
    if (connectedHours) return `Connected for ${connectedHours[1]}h ${connectedHours[2]}m · ${connectedHours[3]} reconnects`;
    const recentStale = value.match(/^最近 5 秒 · (\d+) 个过期事件$/);
    if (recentStale) return `Last 5 seconds · ${recentStale[1]} stale events`;
    const liveReconnects = value.match(/^实时监控 · 重连 (\d+) 次$/);
    if (liveReconnects) return `Live monitoring · ${liveReconnects[1]} reconnects`;
    const quality = value.match(/^([\d.]+) ms · (良好|一般|较高)$/);
    if (quality) return `${quality[1]} ms · ${{ 良好: "Good", 一般: "Fair", 较高: "High" }[quality[2]]}`;
    const found = value.match(/^已识别 (\d+) 个显示区域$/);
    if (found) return `${found[1]} display regions detected`;
    const layout = value.match(/^屏幕布局已调整为 (.+)$/);
    if (layout) return `Screen layout changed to ${zhToEn[layout[1]] ?? layout[1]}`;
    const paired = value.match(/^通过自动发现 · (.+)$/);
    if (paired) return `Via automatic discovery · ${paired[1]}`;
    const latest = value.match(/^EdgeMouse (.+) 已是最新版$/);
    if (latest) return `EdgeMouse ${latest[1]} is up to date`;
    const prototype = value.match(/^(.+)（原型演示）$/);
    if (prototype) return `${zhToEn[prototype[1]] ?? prototype[1]} (prototype)`;
    return value;
  }

  function translatedValue(value) {
    const leading = value.match(/^\s*/)?.[0] ?? "";
    const trailing = value.match(/\s*$/)?.[0] ?? "";
    const core = value.trim();
    if (!core) return value;
    return `${leading}${zhToEn[core] ?? translateDynamic(core)}${trailing}`;
  }

  function translateTextNode(node) {
    if (!originalText.has(node) || /[\u3400-\u9fff]/.test(node.nodeValue)) originalText.set(node, node.nodeValue);
    const original = originalText.get(node);
    const nextValue = currentLanguage === "en" ? translatedValue(original) : original;
    if (node.nodeValue !== nextValue) node.nodeValue = nextValue;
  }

  function translateAttributes(element) {
    let originals = originalAttributes.get(element);
    if (!originals) {
      originals = new Map();
      originalAttributes.set(element, originals);
    }
    translatedAttributes.forEach((name) => {
      if (!element.hasAttribute?.(name)) return;
      const value = element.getAttribute(name);
      if (!originals.has(name) || /[\u3400-\u9fff]/.test(value)) originals.set(name, value);
      const original = originals.get(name);
      element.setAttribute(name, currentLanguage === "en" ? translatedValue(original) : original);
    });
  }

  function translateSubtree(root) {
    if (root.nodeType === Node.TEXT_NODE) {
      translateTextNode(root);
      return;
    }
    if (root.nodeType !== Node.ELEMENT_NODE && root.nodeType !== Node.DOCUMENT_NODE) return;
    if (root.nodeType === Node.ELEMENT_NODE) translateAttributes(root);
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (node.nodeType === Node.TEXT_NODE) translateTextNode(node);
      else translateAttributes(node);
    }
  }

  function apply(language) {
    currentLanguage = language === "en" ? "en" : "zh-CN";
    document.documentElement.lang = currentLanguage;
    translateSubtree(document.documentElement);
    const activeTitle = document.querySelector(".page.is-active")?.dataset.title ?? (currentLanguage === "en" ? "UI Prototype" : "UI 原型");
    document.title = `EdgeMouse · ${activeTitle}`;
  }

  function startObserving() {
    if (observer) return;
    observer = new MutationObserver((records) => {
      if (currentLanguage !== "en") return;
      records.forEach((record) => {
        if (record.type === "characterData") translateTextNode(record.target);
        record.addedNodes?.forEach((node) => translateSubtree(node));
      });
    });
    observer.observe(document.documentElement, { childList: true, characterData: true, subtree: true });
  }

  window.EdgeMouseI18n = {
    apply,
    startObserving,
    translate: (value) => (currentLanguage === "en" ? translatedValue(value) : value),
    current: () => currentLanguage,
  };
})();
