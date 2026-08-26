# EdgeMouse MVP architecture

## Product boundary

The MVP connects one Windows user session and one macOS user session on the
same LAN. Moving through a configured edge transfers the physical mouse to the
paired logical screen; moving back through the opposite edge restores it.

Keyboard, clipboard, file transfer, automatic certificate exchange, relay servers, multi-monitor
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
3. A Tokio worker owns the QUIC endpoint, the reliable bidirectional stream,
   unreliable movement datagrams, and heartbeat scheduling.

The native queues and the network command queue are bounded by failure behavior.
Absolute movement uses latest-value slots on both sender and receiver and is
flushed every 4–12 ms according to the smoothed RTT. Old movement datagrams may
be discarded instead of retransmitted, preventing Wi-Fi jitter or a
high-polling-rate mouse from building a stale network backlog. Before queuing a
new QUIC Datagram, the sender checks that it fits without evicting an older
packet; a congested absolute position is skipped because the next position
supersedes it. Before a button, wheel, enter, leave, or release event is queued,
any pending movement is flushed reliably first so control ordering remains
exact. If the 1,024-message control queue fills, the coordinator treats
transport as unavailable and restores local input rather than silently losing
button events. On Windows, a balanced `timeBeginPeriod(1)`/`timeEndPeriod(1)`
request keeps the movement timer from being rounded to the default scheduler
period while the agent is running.

## Control state and safety

- `Local`: physical events pass through on this machine.
- `Remote`: the local cursor is hidden and anchored; physical events are
  suppressed, routed, and injected on the peer.
- `ReceivingRemote`: the peer cursor stays visible while this machine's physical
  mouse is temporarily prevented from competing for the same OS cursor.
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
- Finite positions that fall outside the configured target bounds are clamped
  before injection and recovery, so a boundary mismatch cannot terminate the
  agent or prevent reconnection.
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
ALPN `edgemouse/2`. After TLS, both sides exchange and validate a protocol
`Hello`. Enter/leave, buttons, wheels, final positions, releases, and heartbeats
use one reliable bidirectional stream. Ordinary absolute movement uses QUIC
Datagram frames, which are encrypted and authenticated but intentionally
unreliable and unordered. Each movement carries a reliable-event watermark, so
the receiver cannot apply movement after a click until that click has been
processed. Sequence numbers reject late datagrams, and latest-value receive
coalescing prevents application-level backlog. A heartbeat is sent every 500 ms.
The receive slot compares event sequence numbers rather than packet arrival
order, so a delayed older datagram cannot overwrite a newer position when the
pointer reverses direction.

When `peer.address` is `auto`, each agent also binds IPv4 UDP port 43892 and
broadcasts a bounded discovery announcement containing its node ID, display
name, and QUIC port. Receivers reject malformed, oversized, self-originated, and
unexpected-node announcements. The advertised data is an untrusted locator, not
an authentication mechanism: the IP comes from the datagram source and the
subsequent QUIC connection still requires the configured peer certificate and
mutual TLS. Discovery runs again on reconnect so a DHCP address change does not
require editing the configuration. A static `host:port` remains available for
networks that block IPv4 broadcast.

The locked dependency graph pins `quinn-proto` to 0.11.16, beyond the 0.11.14
fix for the malformed transport-parameter denial-of-service advisory. Version
0.11.17 is intentionally excluded because its refactored Datagram buffer can
double-decrement queued payload bytes when the drop-oldest path is exercised.

## Platform adapters

### Windows

Capture runs a `WH_MOUSE_LL` hook on a dedicated message-loop thread. Local
movement uses consecutive hook coordinates. The hook's per-monitor-aware
coordinates are divided by the configured display scale as floating-point
values before they enter the logical screen topology, preserving half-point
motion at 200% scaling. During remote control, movement is calculated against a
fixed anchor at the local screen center because suppressed input does not
advance the real cursor. Mode changes temporarily ignore warp-generated mouse
moves and discard queued pre-transition movement, so an edge handoff cannot
replace the new relative-motion reference with an old position. `SendInput`
performs absolute virtual-desktop movement, buttons, and wheel injection.
`dwExtraInfo` plus the injected flag prevent feedback loops. Raw Input can later
replace the movement source for high-rate devices while the hook remains
responsible for suppression.

### macOS

An active session `CGEventTap` captures and suppresses mouse events on a
dedicated `CFRunLoop`. `CGEventPost` injects absolute movement, drag variants,
buttons, and pixel scrolling with a dedicated event-source marker. Startup
checks Accessibility permission. During remote send or receive,
`CGAssociateMouseAndMouseCursorPosition` prevents local hardware movement from
dragging the OS cursor; association, cursor hide/show, and warp calls are
restored during transitions and teardown.

## Deliberate next steps

1. Test on physical Windows and macOS machines, including 125–1000 Hz mice,
   long mixed Ethernet/Wi-Fi sessions, horizontal scroll, sleep/wake, and the
   automatic recovery path during Wi-Fi loss.
2. Add Windows Raw Input based on latency measurements.
3. Add a short-code/fingerprint pairing UX for securely exchanging certificates.
4. Add a tray/settings UI, signed installers, launch-at-login, and diagnostics.
5. Only then consider keyboard and clipboard channels.
