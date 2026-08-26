# EdgeMouse

EdgeMouse is a command-line software KVM that moves one mouse between one
Windows user session and one macOS user session over a LAN. Crossing a configured
screen edge transfers movement, buttons, and scrolling to the other machine;
crossing back restores local control.

This repository contains a functional mouse-only MVP. It intentionally excludes
keyboard input, clipboard sync, discovery, relay servers, multi-monitor setup,
tray UI, installers, and elevated Windows desktops.

## Implemented

- Left, right, top, or bottom edge switching with entry hysteresis.
- Primary, secondary, middle, back, and forward buttons.
- Vertical and horizontal scrolling.
- Coordinate mapping between differently sized Windows and macOS displays.
- Native macOS `CGEventTap` capture and marked `CGEventPost` injection.
- Native Windows `WH_MOUSE_LL` capture and marked `SendInput` injection.
- Mutually authenticated QUIC/TLS with one explicitly trusted peer certificate.
- Latest-position QUIC datagrams for movement, with reliable ordered delivery
  retained for clicks, scrolling, edge transitions, and final positions.
- Versioned, bounded binary frames with strict untrusted-input validation.
- 500 ms heartbeats, 1.5 s default timeout, local-pointer recovery, and forced
  synthetic-button release on disconnect.
- Automatic reconnection after an established link is interrupted, with local
  mouse control kept available while the peer or network is offline.
- Identity generation, configuration validation, diagnostics, and simulation
  commands.

Windows Raw Input capture remains a future high-polling-rate optimization. The
MVP uses the low-level hook and a fixed capture anchor while control is remote.

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

GitHub Actions repeats the formatting, static-analysis, test, and release-build
steps on both macOS and Windows after every push to `main`. A successful run also
publishes downloadable platform packages under the run's **Artifacts** section.

The scripts deliberately leave these machine-specific decisions to the user:
exchange only the public certificates, enter the peer LAN address and real screen
geometry, allow UDP port `43891` through Windows Firewall, and grant macOS
Accessibility permission.

## Pair the two machines

1. Generate a different identity on each machine:

   ```sh
   edgemouse identity ./identity
   ```

2. Copy only `certificate.der` to the other machine. Never copy or share
   `private-key.der`.
3. Copy and edit [the Windows example](examples/windows.toml) and
   [the macOS example](examples/macos.toml). The two files must use the same
   screen IDs and geometry, with opposite `layout.peer_on` values.
4. Set each `peer.address` to the other machine's LAN IP. Permit inbound UDP
   port `43891` in Windows Firewall and any host firewall in use.
5. Validate both files before connecting:

   ```sh
   edgemouse check-config ./edgemouse.toml
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

The certificate with the lower derived node ID initiates the connection; the
other side accepts it. Either process may be started first. After a successful
connection, a temporary network loss or peer restart restores local mouse control
and makes both agents retry automatically. Press Ctrl+C to release input and shut
down cleanly.

Screen coordinates must match the operating system's logical desktop coordinate
space. For the single-screen MVP the origin is normally `(0, 0)`. On a Retina
Mac use the logical resolution shown by macOS, not the doubled backing-pixel
resolution. Set each screen's `scale` to its OS display scale: for example,
Windows 200% scaling is `2.0`, while Windows 100% scaling is `1.0`.

Remote absolute movement is emitted every 4–12 ms according to current RTT,
always using the newest position. Stale movement is discarded during network
jitter instead of being replayed later. Buttons, wheels, enter, leave, and the
last position before each control event remain reliable and strictly ordered.
While movement is active, the agent prints a five-second link summary containing
QUIC RTT, the current movement interval, sent updates, skipped congested updates,
and merged updates. Windows requests 1 ms timer resolution while EdgeMouse is
running so the 4–12 ms movement schedule does not collapse to the default
roughly 15.6 ms system timer period.

Both computers must use protocol v2. Versions 0.1.5 through 0.1.9 use protocol
v2 and will intentionally refuse a connection to a 0.1.4 executable. For the
best movement behavior, install the latest version on both computers.

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
