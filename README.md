# EdgeMouse

EdgeMouse is a command-line software KVM that moves input between one
Windows user session and one macOS user session over a LAN. Crossing a configured
screen edge transfers mouse movement, buttons, scrolling, and the Windows
keyboard to macOS; crossing back restores local control.

This repository contains a functional mouse MVP, Windows-to-macOS keyboard
forwarding, and automatic desktop geometry exchange. It intentionally excludes
clipboard sync, relay servers, tray UI, installers, and elevated Windows desktops.

## Implemented

- Left, right, top, or bottom edge switching with entry hysteresis.
- Primary, secondary, middle, back, and forward buttons.
- Vertical and horizontal scrolling.
- Windows keyboard forwarding to macOS while the Windows mouse owns the Mac,
  including modifiers, navigation, function keys, numpad keys, and key repeat.
- Windows-style shortcut mapping on macOS: Windows `Ctrl` becomes Mac `Command`,
  the Windows key becomes Mac `Control`, and `Alt` remains Mac `Option`.
- Ordered keyboard delivery, forced key release on handback/disconnect, held-key
  transition safety, and `Ctrl+Alt+Shift+Esc` emergency local recovery.
- Coordinate mapping between differently sized Windows and macOS displays.
- Automatic resolution, scaling, rotation, and active multi-display desktop
  bounds detection at startup and after reconnecting.
- Authenticated desktop-geometry exchange, so neither configuration duplicates
  the peer's current width, height, origin, orientation, or scale.
- Native macOS `CGEventTap` capture and marked `CGEventPost` injection.
- Native Windows `WH_MOUSE_LL` capture and marked `SendInput` injection.
- Mutually authenticated QUIC/TLS with one explicitly trusted peer certificate.
- One-time 8-digit short-code pairing that securely exchanges public certificates
  while keeping both private keys on their original machines.
- Latest-position QUIC datagrams for movement, with reliable ordered delivery
  retained for clicks, scrolling, edge transitions, and final positions.
- Versioned, bounded binary frames with strict untrusted-input validation.
- 500 ms heartbeats, 1.5 s default timeout, local-pointer recovery, and forced
  synthetic-button release on disconnect.
- Automatic reconnection after an established link is interrupted, with local
  mouse control kept available while the peer or network is offline.
- Persistent startup retry when the peer or local network is unavailable during
  login, without capturing the local mouse between attempts.
- Automatic IPv4 LAN discovery of the configured certificate-pinned peer, both
  at startup and during reconnection; static peer addresses remain supported.
- Single-instance protection plus local `status` and graceful `stop` commands.
- Optional per-user login startup on macOS and Windows, with persistent logs.
- Identity generation, configuration validation, diagnostics, and simulation
  commands.

Windows Raw Input capture remains a future high-polling-rate optimization. The
MVP uses low-level mouse and keyboard hooks and a fixed mouse capture anchor
while control is remote. Keyboard capture in the reverse macOS-to-Windows
direction is not enabled yet.

## Build

Install a stable Rust toolchain (Rust 1.85 or newer), then build on each target
machine:

```sh
cargo build --release -p edgemouse-agent
```

The executable is `target/release/edgemouse` on macOS and
`target\release\edgemouse.exe` on Windows.

## One-command preparation

The preparation scripts check the Rust installation, run formatting and static
analysis, execute all tests, create a release build, run platform diagnostics,
generate a local identity if needed, and copy the correct configuration template
to `edgemouse.toml`. Existing identity and configuration files are never
overwritten.

On macOS:

```sh
./scripts/bootstrap-macos.sh
```

On Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows.ps1
```

Use `--verify-only` on macOS or `-VerifyOnly` on Windows to run the full source
verification without creating an identity or configuration. Generated private
keys, certificates, and `edgemouse.toml` are ignored by Git.

After the first Windows setup, this single command checks that the tracked source
tree is clean, fast-forwards `main` from GitHub, builds the release executable,
prints its version, then starts EdgeMouse with current and timestamped logs:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\update-build-run-windows.ps1
```

Machine-specific configuration, certificates, private keys, and logs are ignored
by Git and are not overwritten. The script deliberately stops instead of stashing
or discarding tracked local source changes. It retries a temporarily unavailable
GitHub connection three times. `-SkipUpdate` builds and starts the source already
present on disk, while `run-windows-with-log.ps1` starts the existing release
executable without contacting GitHub.

GitHub Actions repeats the formatting, static-analysis, test, and release-build
steps on both macOS and Windows after every push to `main`. A successful run also
publishes downloadable platform packages under the run's **Artifacts** section.

The scripts deliberately leave firewall and permission decisions to the user:
allow inbound UDP ports `43891` and `43892` plus TCP port `43893` through Windows
Firewall, and grant macOS Accessibility permission. Screen geometry is detected
automatically when `[local.screen]` contains `auto = true`.

## Pair the two machines

1. Generate a different identity on each machine and create its configuration.
   The preparation scripts do both without overwriting existing files:

   ```powershell
   # Windows
   powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-windows.ps1
   ```

   ```sh
   # macOS
   ./scripts/bootstrap-macos.sh
   ```

2. Edit [the Windows example](examples/windows.toml) and
   [the macOS example](examples/macos.toml). The two files must use matching
   screen IDs with opposite `layout.peer_on` values. Keep `auto = true` under
   each local screen; the peer sends its current desktop geometry after the
   authenticated connection. The peer certificate named in each config does not
   need to exist before pairing.
3. Keep `peer.address = "auto"` on both computers to follow DHCP address changes
   automatically; no IP edit or re-pairing is needed after a reboot. Alternatively,
   set it to the other machine's static LAN address, such as
   `192.168.8.202:43891`. Permit inbound UDP `43891` (QUIC mouse traffic), UDP
   `43892` (discovery/pairing offer), and TCP `43893` (one-time pairing) in
   Windows Firewall.
4. Stop any running EdgeMouse agents. On Windows, display a one-time code:

   ```powershell
   .\target\release\edgemouse.exe pair host .\edgemouse.toml
   ```

   On the Mac, enter that code exactly as displayed:

   ```sh
   ./target/release/edgemouse pair join ./edgemouse.toml 1234-5678
   ```

   If UDP broadcast does not cross the wired/Wi-Fi network, append the Windows
   IP address to bypass discovery while keeping the same authenticated pairing:

   ```sh
   ./target/release/edgemouse pair join ./edgemouse.toml 1234-5678 192.168.8.202
   ```

   Either platform can technically host, but Windows-host/Mac-join avoids adding
   a new inbound TCP rule to macOS. The code expires after five minutes and the
   host stops after three rejected attempts. Existing identical peer
   certificates are kept; a different existing certificate is never replaced
   silently.
5. Validate both files after pairing:

   ```sh
   edgemouse check-config ./edgemouse.toml
   ```

   To test discovery without capturing either mouse, run this command on both
   computers at about the same time:

   ```sh
   edgemouse discover ./edgemouse.toml
   ```

6. On macOS, enable EdgeMouse under **System Settings → Privacy & Security →
   Accessibility**. Check status with `edgemouse doctor`.
7. Start both sides:

   ```sh
   edgemouse run ./edgemouse.toml
   ```

   On macOS, the included launcher can start the release executable while showing
   and saving the complete terminal output:

   ```sh
   ./scripts/run-macos-with-log.sh
   ```

   The newest run is written to `mac-current.log`, and timestamped copies are
   kept under `logs/`. An alternative configuration path may be passed as the
   script's only argument.

   Windows has an equivalent PowerShell launcher:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\run-windows-with-log.ps1
   ```

   It writes the newest run to `windows-current.log` and preserves timestamped
   copies under `logs/`, so restarting the agent does not destroy the prior log.

## Run automatically after login

After manual operation has been verified, install the per-user login agent. This
does not require a system service or administrator privileges.

On macOS:

```sh
./scripts/manage-autostart-macos.sh install ./edgemouse.toml
```

The installer creates a background-only `~/Applications/EdgeMouse.app` and
associates the login agent with its stable `com.edgemouse.agent` identity. On
first install, allow EdgeMouse in **System Settings > Privacy & Security > Local
Network** and **Accessibility**. The Local Network permission lets EdgeMouse
automatically discover its trusted peer after either computer receives a new
DHCP address; Accessibility permits mouse capture and injection. These
permissions are attributed to EdgeMouse instead of Terminal.

For repeated local development builds, create a fixed signing identity once
before installing the login agent:

```sh
./scripts/setup-macos-local-signing.sh install
./scripts/manage-autostart-macos.sh install ./edgemouse.toml
```

The signing certificate and private key remain in the current user's login
keychain and are used only for code signing. The certificate is valid for ten
years. Switching from an older ad-hoc build to this fixed identity requires one
final removal and re-addition of EdgeMouse under Accessibility. Later EdgeMouse
upgrades signed with the same identity retain that permission. Without the
fixed identity, the installer still works but warns that an ad-hoc signature
may require Accessibility permission to be renewed after an upgrade.

On Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\manage-autostart-windows.ps1 Install .\edgemouse.toml
```

Replace `install`/`Install` with `status`, `start`, `stop`, or `uninstall` as
needed. The commands are case-insensitive on macOS and PowerShell accepts the
capitalized forms shown above. `stop` asks the running agent to release captured
input and injected buttons before it exits. Starting a second agent is refused,
so a manual launcher cannot accidentally compete with the login agent.

The macOS login agent writes `logs/mac-autostart.out.log` and
`logs/mac-autostart.err.log`. The Windows login shortcut uses the normal logging
launcher and writes `windows-current.log` plus timestamped files under `logs/`.
The local status/stop channel binds only `127.0.0.1:43894`; it is not reachable
from the LAN and needs no firewall rule.

The certificate with the lower derived node ID initiates the connection; the
other side accepts it. Either process may be started first. After a successful
connection, a temporary network loss or peer restart restores local mouse control
and makes both agents retry automatically. Press Ctrl+C while input is still
local to shut down cleanly; while Windows input belongs to macOS, Ctrl+C is
forwarded as Command+C. `edgemouse status` reports the local process and version;
`edgemouse stop` performs the same safe shutdown when the agent is running in the
background.

Discovery packets contain only the node ID, device name, and QUIC port. Their IP
address is taken from the UDP source and they are treated only as connection
hints: a discovered endpoint must still present the exact certificate already
configured for that peer and complete mutual TLS. A forged LAN broadcast cannot
become a trusted EdgeMouse peer.

Pairing offers contain only a random one-time session ID, device name, and TCP
port; the short code and its hash are never broadcast. If a direct host IP is
provided, the same fresh offer is sent over TCP instead. SPAKE2 derives a session
key from the code without sending the code itself. Both certificate records and
the final confirmation are authenticated over the complete handshake transcript
before either certificate is saved. Certificates are public; private keys never
leave their machine. The saved certificate remains the trust anchor for normal
mutual TLS connections after pairing.

With `local.screen.auto = true`, Windows reads the complete per-monitor-aware
virtual desktop and macOS reads the union of active CoreGraphics displays.
Rotation, negative secondary-display origins, Retina/Windows scaling, and
resolution changes are therefore reflected automatically on startup and after a
reconnect. The authenticated `Hello` exchange supplies that result to the peer.
Older manual `origin_x`, `origin_y`, `width`, `height`, and `scale` fields remain
supported when `auto = false` is set explicitly. In 0.3.0, omitting `auto`
selects automatic detection so an existing machine configuration upgrades
without requiring a manual geometry edit.

Remote absolute movement is emitted every 4–12 ms according to current RTT,
always using the newest position. Stale movement is discarded during network
jitter instead of being replayed later. Buttons, wheels, enter, leave, and the
last position before each control event remain reliable and strictly ordered.
On macOS, received movement keeps a short arrival-timestamped history and is
rendered on a stable 4 ms cadence through an adaptive 8–12 ms jitter buffer.
Positions between received samples are interpolated instead of jumping from one
network packet to the next. When measured arrival jitter is high, a genuine
packet gap may use at most 12 ms and 24 pixels of bounded prediction; the real
position always replaces it as soon as a sample arrives. Buttons, wheels, leave
events, and drag transitions flush the newest real position immediately, so
buffering never changes control-event ordering or click accuracy.
If the peer's physical mouse becomes unresponsive while it controls this
computer, deliberately pushing this computer's physical mouse toward the
configured peer edge requests an authenticated control handoff. The detector
models the distance from the current remote pointer to that edge and requires a
firm overshoot, so ordinary trackpad movement does not steal control. Synthetic
cursor movement is excluded. The original sender releases held buttons and keys
before acknowledging; if it cannot acknowledge within 1.5 seconds, the receiver
restores local input and reconnects instead of leaving the pointer trapped.
While movement is active, the agent prints a five-second link summary containing
QUIC RTT, the current movement interval, sent updates, skipped congested updates,
merged updates, receive-side arrival jitter, and the largest active-movement
arrival gap. Windows requests 1 ms timer resolution while EdgeMouse is running
so the 4–12 ms movement schedule does not collapse to the default roughly
15.6 ms system timer period.

The authenticated physical-mouse reclaim handshake uses protocol v5 in
EdgeMouse 0.3.1. Both computers must run 0.3.1 or newer; earlier builds
intentionally refuse this connection instead of silently using incompatible
control messages. EdgeMouse 0.3.2 fixes physical-versus-synthetic movement
classification during that handoff and keeps the Windows takeover reference
synchronized with the currently injected pointer.
EdgeMouse 0.3.3 adds low-latency macOS receive smoothing and arrival-jitter
diagnostics without changing protocol v5, so it remains connection-compatible
with 0.3.1 and 0.3.2 during a staged upgrade.
EdgeMouse 0.3.4 replaces the fixed macOS receive filter with an adaptive 8–12 ms
jitter buffer, arrival-time interpolation, and short bounded prediction. It
still uses protocol v5 and remains connection-compatible with 0.3.1–0.3.3.

## Verify the source tree

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p edgemouse-agent -- doctor
cargo run -p edgemouse-agent -- demo
```

The transport test binds two loopback UDP sockets and performs a real mutual-TLS
handshake. Some sandboxes require permission for that test.

## Workspace

- `edgemouse-core`: geometry, topology, routing, safety state machine, and
  platform adapter traits.
- `edgemouse-protocol`: binary message serialization and strict validation.
- `edgemouse-transport`: pinned-peer mutual TLS, QUIC connection, framing, and
  identity material.
- `edgemouse-platform-macos`: CoreGraphics capture/injection adapter.
- `edgemouse-platform-windows`: Win32 capture/injection adapter.
- `edgemouse-agent`: CLI, TOML configuration, network worker, heartbeats, and
  runtime coordination.

The provisional project name can be changed before packaging. The code is MIT
licensed and contains no copied GPL implementation code from Deskflow, Barrier,
Input Leap, or Lan Mouse.
