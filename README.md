# EdgeMouse

**简体中文** | [English](README.en.md)

EdgeMouse 是一款开源的跨平台软件 KVM，让一套鼠标和键盘在 Windows 与
macOS 之间自然切换。把指针推过设定的屏幕边缘，鼠标移动、按键、滚轮和键盘
便会跟随到另一台电脑；推回边缘即可恢复本机控制。

项目通过局域网直接连接两台设备，使用双向 TLS 验证已配对的设备，不依赖云端
中继。桌面应用、后台服务、签名安装包和应用内更新已经集成。

## 下载与安装

请从 [GitHub Releases](https://github.com/ytlds-64/EdgeMouse/releases/latest)
下载最新稳定版：

- Windows：下载 `.exe` 安装程序。
- macOS：下载通用版 `.dmg`，同时支持 Apple 芯片和 Intel Mac。

Windows 安装程序会根据系统语言显示简体中文或英文，也可以在安装开始前手动
选择语言。macOS 第一次使用时，需要在 **系统设置 → 隐私与安全性** 中允许
EdgeMouse 使用 **辅助功能** 和 **本地网络**。

安装后请在“连接”页面完成一次安全配对。以前使用过源码版或命令行版时，可以在
配对窗口选择“导入旧配对配置”，选中原来的 `edgemouse.toml`；应用会将设备身份
和可信证书安全复制到当前用户目录并保留原配置备份，后续升级无需再次配对。

> 当前稳定版：0.6.3。应用的“设置”页面可以检查并安装后续签名更新。

## 已实现功能

- 支持从左、右、上、下任一屏幕边缘切换，并带有防误触迟滞。
- 双向传递鼠标移动、左键、右键、中键、前进/后退键和滚轮。
- 双向键盘跟随，支持组合键、修饰键、方向键、功能键、小键盘和按键重复。
- 跨平台快捷键映射：Windows `Ctrl` 对应 Mac `Command`，Mac `Command`
  对应 Windows `Ctrl`；Control/Windows 与 Alt/Option 的平台角色仍然保留。
- Windows → Mac 与 Mac → Windows 可分别设置横向、纵向滚动反转。
- 自动识别分辨率、缩放、旋转方向、负坐标和多显示器排列。
- 在可信连接中交换两端完整屏幕拓扑，不需要手动填写对方分辨率。
- 屏幕布局页按真实数量、相对位置和分辨率绘制两台电脑的显示器。
- 通过拖动设备卡片或选择方向设置穿越边缘，并将相反方向同步到另一端。
- 自动发现局域网内已经配对的设备，DHCP 地址变化后无需修改 IP 或重新配对。
- 断线自动恢复、启动时持续重试、心跳超时保护和本机指针自动取回。
- 支持由接收端物理鼠标主动取回控制，避免输入卡在另一台电脑。
- Windows Raw Input 与低级钩子协作，macOS 使用原生 CoreGraphics 捕获和注入。
- 鼠标移动使用低延迟 QUIC 数据报；点击、滚轮、切换与最终位置可靠有序传输。
- macOS 接收端使用稳定的抖动缓冲和插值，减少高频鼠标移动时的卡顿与跳动。
- 一次性 8 位短码安全配对，私钥始终只保存在各自电脑上。
- 单实例保护、安全停止、本机状态查询和紧急恢复快捷键
  `Ctrl+Alt+Shift+Esc`。
- Windows 和 macOS 登录后自动启动，并在后台保存当前及历史日志。
- 桌面应用提供实时连接状态、延迟、抖动、重连次数、输入计数和诊断导出。
- 支持浅色、深色、跟随系统主题，以及简体中文/英文界面。
- 支持签名安装包和应用内检查更新。

## 快速开始

### 1. 准备两台电脑

两台电脑应连接到同一个局域网。Windows 防火墙需要允许以下入站端口：

- UDP `43891`：QUIC 鼠标和键盘数据
- UDP `43892`：局域网发现与配对广播
- TCP `43893`：一次性安全配对

macOS 需要为 EdgeMouse 开启“辅助功能”和“本地网络”权限。Windows 不需要
macOS 式的输入权限；如果防火墙提示是否允许网络访问，请选择允许专用网络。

### 2. 生成配置与身份

从源码运行时，可以使用项目自带脚本。已有身份、证书和 `edgemouse.toml`
不会被覆盖。

macOS：

```sh
./scripts/bootstrap-macos.sh
```

Windows PowerShell：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows.ps1
```

两端配置中的 `peer.address` 建议保持为 `"auto"`，这样电脑重启或路由器重新
分配 IP 后仍会自动发现可信设备。`local.screen.auto = true` 会启用自动屏幕
识别。

### 3. 安全配对

先停止两端正在运行的 EdgeMouse。推荐让 Windows 显示一次性配对码：

```powershell
.\target\release\edgemouse.exe pair host .\edgemouse.toml
```

然后在 Mac 输入该配对码：

```sh
./target/release/edgemouse pair join ./edgemouse.toml 1234-5678
```

如果有线与 Wi-Fi 之间不转发 UDP 广播，可以在命令末尾临时追加 Windows IP：

```sh
./target/release/edgemouse pair join ./edgemouse.toml 1234-5678 192.168.8.202
```

配对码 5 分钟后失效，最多允许 3 次尝试。配对只交换公开证书，私钥不会离开
本机。以后 IP 地址变化不需要重新配对。

### 4. 检查并启动

在两端检查配置：

```sh
edgemouse check-config ./edgemouse.toml
```

只测试自动发现而不接管鼠标时，可在两台电脑上同时运行：

```sh
edgemouse discover ./edgemouse.toml
```

启动后台服务：

```sh
edgemouse run ./edgemouse.toml
```

也可以直接使用桌面应用概览页的“本机服务”开关启动或停止服务。关闭窗口后
是否继续在后台运行、是否登录时启动，都可以在设置页调整。

## 登录后自动运行

确认手动运行正常后，可以安装当前用户的登录启动项，不需要系统服务或管理员
权限。

macOS：

```sh
./scripts/manage-autostart-macos.sh install ./edgemouse.toml
```

Windows PowerShell：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\manage-autostart-windows.ps1 Install .\edgemouse.toml
```

把 `install` / `Install` 换成 `status`、`start`、`stop` 或 `uninstall`，可以
查看状态、启动、停止或移除登录启动项。后台日志保存在：

- macOS：`logs/mac-autostart.out.log` 和 `logs/mac-autostart.err.log`
- Windows：`windows-current.log` 以及 `logs/` 中按时间保存的历史日志

本机状态/停止通道只监听 `127.0.0.1:43894`，局域网中的其他设备无法访问，
也不需要添加防火墙规则。

## 自动屏幕与布局

启用 `local.screen.auto = true` 后，Windows 会读取完整的虚拟桌面，macOS 会
读取所有活动 CoreGraphics 显示器的并集。旋转、Retina/Windows 缩放、辅助
屏幕负坐标和分辨率变化会在启动及重连后自动更新。

认证完成后，两端会交换完整显示器拓扑。屏幕布局页显示每块屏幕的相对位置、
物理像素分辨率和主屏标记。保存“Mac 位于 Windows 的哪一侧”时，另一台电脑
会自动保存相反的本机边缘并重新连接。

旧版手动字段 `origin_x`、`origin_y`、`width`、`height` 和 `scale` 仍然可用，
但必须明确设置 `auto = false`。

## 输入与安全恢复

控制权在另一台电脑时，EdgeMouse 会按顺序传递键盘事件，并在归还、断线、
紧急恢复和退出时强制释放所有已按下的按键与鼠标按钮，避免修饰键“粘住”。
切换发生前已经按住的键会继续留在本机，直到物理释放。

如果输入意外卡在远端，可按 `Ctrl+Alt+Shift+Esc` 立即恢复本机控制。接收端也
可以把自己的物理鼠标坚定地推向已配置的边缘，请求经过认证的控制权交接；若
1.5 秒内没有收到确认，会恢复本机输入并重新连接。

Windows 远程控制期间使用固定捕获锚点和 Raw Input 保留高轮询率移动，低级
钩子负责阻止本机遗留事件，并在 Raw Input 不可用时自动回退。需要强制使用旧
路径时，可在 Windows 配置中设置：

```toml
[session]
windows_raw_input = false
```

## 连接与安全设计

- 正常会话使用 QUIC 和双向 TLS，只接受配置中证书完全匹配的可信设备。
- 自动发现广播只包含节点 ID、设备名和 QUIC 端口；来源地址只作为连接提示。
- 伪造的局域网广播仍然无法通过固定证书验证，不能冒充已经配对的设备。
- 配对使用随机一次性会话、SPAKE2 派生密钥和完整握手记录认证。
- 短码及其哈希不会通过广播发送，公开证书可以交换，私钥永不离开本机。
- 移动数据采用有界、带版本的二进制帧，并严格校验所有不可信输入。
- 500 ms 心跳和默认 1.5 秒超时会在断线时恢复本机指针并释放合成按键。

## 从源码构建

需要 Rust 1.85 或更高版本。

只构建后台服务：

```sh
cargo build --release -p edgemouse-agent
```

可执行文件位于 macOS 的 `target/release/edgemouse` 或 Windows 的
`target\release\edgemouse.exe`。

构建桌面应用：

```sh
cargo build -p edgemouse-desktop
./target/debug/edgemouse-desktop --config ./edgemouse.toml
```

Windows 可以使用一条命令更新源码、构建后台服务和桌面应用并启动：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-run-desktop-windows.ps1
```

开发期间更新、构建并带日志启动后台服务：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\update-build-run-windows.ps1
```

这些脚本不会覆盖设备身份、配置、证书、私钥或日志，也不会自动丢弃本地源码
改动。

## 验证源码

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p edgemouse-agent -- doctor
cargo run -p edgemouse-agent -- demo
```

每次推送到 `main` 后，GitHub Actions 会在 Windows 和 macOS 上重复格式检查、
静态分析、测试和发布构建。正式版本会附带签名安装包、通用 macOS 应用和应用内
更新所需的签名文件。

## 工作区结构

- `edgemouse-core`：几何、屏幕拓扑、路由、安全状态机和平台接口。
- `edgemouse-protocol`：二进制消息序列化和严格校验。
- `edgemouse-transport`：固定可信证书的双向 TLS、QUIC、组帧和身份材料。
- `edgemouse-platform-macos`：macOS CoreGraphics 捕获与注入。
- `edgemouse-platform-windows`：Windows Win32 捕获与注入。
- `edgemouse-agent`：命令行、TOML 配置、网络、心跳和运行时协调。
- `edgemouse-desktop`：Windows 与 macOS 桌面应用、后台控制、诊断和更新。

## 兼容性与版本记录

- 0.3.x：完成自动发现、断线恢复、物理鼠标取回和 macOS 移动平滑。
- 0.4.0：加入 macOS 原生键盘捕获，完成双向键盘跟随。
- 0.5.0–0.5.2：加入 Windows Raw Input、实时诊断和双向滚动设置。
- 0.5.3–0.5.7：加入布局同步、原生 macOS 窗口、圆角、正式图标和隐藏式后台启动。
- 0.5.8：协议升级到 v7，交换并显示完整的多显示器拓扑。
- 0.5.9：统一概览与布局方向，并把概览电源开关接入真实后台服务。
- 0.6.0：绑定桌面应用功能，提供签名安装包、通用 macOS 构建和应用内更新。
- 0.6.1：统一桌面应用版本显示，加入简体中文/英文 Windows 安装界面，并提供
  中文默认、英文可切换的项目首页。
- 0.6.2：修复安装版首次运行时配对证书未迁移导致无法启动的问题，支持从旧版
  `edgemouse.toml` 一次性导入配对身份，并修正错误提示图标和长文本显示。
- 0.6.3：应用内更新加入实时下载进度、文件大小和安装阶段提示；诊断问题可直接
  修复配对、自动发现与后台服务，macOS 权限可一键打开系统设置并自动复检。

协议 v7 从 0.5.8 开始使用。两台电脑都应更新到兼容版本后再连接。

## 开源许可

EdgeMouse 使用 [MIT License](LICENSE)。项目不包含从 Deskflow、Barrier、
Input Leap 或 Lan Mouse 复制的 GPL 实现代码。

详细英文说明请查看 [README.en.md](README.en.md)。
