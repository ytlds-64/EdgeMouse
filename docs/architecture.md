# EdgeMouse MVP architecture

## Product boundary

The MVP connects one Windows user session and one macOS user session on the
same LAN. Moving through a configured edge transfers the physical mouse to the
paired logical screen; moving back through the opposite edge restores it.

Keyboard, clipboard, file transfer, peer discovery, relay servers, multi-monitor
topologies, tray UI, installers, lock screens, and Windows UAC secure desktops
are outside this milestone.

## Runtime

Each machine runs the same per-user agent. Input APIs and permissions belong to
the interactive session, so the MVP does not run capture inside a system service.

The process has three execution contexts:

1. A native OS callback thread captures physical input and only normalizes and
   enqueues events. It never performs network or UI work.
2. The main coordinator polls captured and network events, advances the pure
   `edgemouse-core` state machine, and applies capture/injection effects.
3. A Tokio worker owns the QUIC endpoint, the reliable bidirectional stream, and
   heartbeat scheduling.

The native queues and the network command queue are bounded by failure behavior:
if the 1,024-message outbound queue fills, the coordinator treats transport as
unavailable and restores local input rather than silently losing button events.

## Control state and safety

- `Local`: physical events pass through on this machine.
- `Remote`: the local cursor is hidden and anchored; physical events are
  suppressed, routed, and injected on the peer.
- `Recovering`: the link timed out or failed and the saved local position is
  restored before returning to `Local`.

Safety invariants:

- Synthetic events carry an OS-specific marker and are never retransmitted.
- Screen transition is blocked while any button is held.
- An entry guard prevents immediately bouncing back across the entry edge.
- A 1.5-second heartbeat timeout cannot leave the source pointer captured.
- Receiver timeout/disconnect releases every tracked synthetic button.
- The receiver rejects input before `Enter`, stale sequence numbers, zero
  session/sequence values, and events addressed to another screen.
- Protocol numbers must be finite and frames are capped at 64 KiB.
- Ctrl+C restores local capture, releases injected buttons, and sends `Goodbye`.

## Transport and trust

Each node generates a self-signed certificate and PKCS#8 private key. The first
128 bits of SHA-256 over the certificate form its node ID. Configuration contains
exactly one peer certificate; both rustls client and server configurations trust
only that certificate and require client authentication. This provides mutual
authentication without a public CA or an insecure certificate-verification
bypass.

Both agents bind a QUIC UDP endpoint. The lower node ID initiates and retries;
the higher node ID accepts, eliminating duplicate-connection races. TLS uses
ALPN `edgemouse/1`. After TLS, both sides exchange and validate a protocol
`Hello`, then use one reliable bidirectional stream for ordered mouse and control
frames. A heartbeat is sent every 500 ms.

The locked dependency graph resolves `quinn-proto` to 0.11.17, beyond the
0.11.14 fix for the malformed transport-parameter denial-of-service advisory.

## Platform adapters

### Windows

Capture runs a `WH_MOUSE_LL` hook on a dedicated message-loop thread. Local
movement uses consecutive hook coordinates. During remote control, movement is
calculated against the fixed capture anchor because suppressed input does not
advance the real cursor. `SendInput` performs absolute virtual-desktop movement,
buttons, and wheel injection. `dwExtraInfo` plus the injected flag prevent
feedback loops. Raw Input can later replace the movement source for high-rate
devices while the hook remains responsible for suppression.

### macOS

An active session `CGEventTap` captures and suppresses mouse events on a
dedicated `CFRunLoop`. `CGEventPost` injects absolute movement, drag variants,
buttons, and pixel scrolling with a dedicated event-source marker. Startup
checks Accessibility permission; cursor hide/show and warp calls are balanced
during transitions and teardown.

## Deliberate next steps

1. Test on physical Windows and macOS machines, including 125–1000 Hz mice,
   Retina scaling, horizontal scroll, sleep/wake, and Wi-Fi loss.
2. Add Windows Raw Input and movement coalescing based on latency measurements.
3. Add automatic LAN discovery plus a short-code/fingerprint pairing UX.
4. Add a tray/settings UI, signed installers, launch-at-login, and diagnostics.
5. Only then consider keyboard and clipboard channels.
