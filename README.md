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
- Versioned, bounded binary frames with strict untrusted-input validation.
- 500 ms heartbeats, 1.5 s default timeout, local-pointer recovery, and forced
  synthetic-button release on disconnect.
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

The certificate with the lower derived node ID initiates the connection; the
other side accepts it. Either process may be started first. Press Ctrl+C to
release input and shut down cleanly.

Screen coordinates must match the operating system's logical desktop coordinate
space. For the single-screen MVP the origin is normally `(0, 0)`. On a Retina
Mac use the logical resolution shown by macOS, not the doubled backing-pixel
resolution.

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
