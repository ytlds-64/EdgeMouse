# EdgeMouse

[简体中文](README.md) | **English**

EdgeMouse is a command-line software KVM that moves input between one
Windows user session and one macOS user session over a LAN. Crossing a configured
screen edge transfers mouse movement, buttons, scrolling, and the local keyboard;
crossing back restores local control.

This repository contains a functional mouse MVP, bidirectional keyboard
forwarding, automatic desktop geometry exchange, an integrated cross-platform
desktop application, signed installers, and in-app updates. It intentionally
excludes relay servers and elevated Windows desktops. Version 0.6.13
also includes optional text/image clipboard synchronization.

## Version 0.6.18

- Fixes disconnects during active use when reliable-stream retransmission delays heartbeats despite continuing mouse traffic.
- Updated peers exchange independent heartbeat datagrams every 250 ms, including when the movement queue is full; duplicate or late probes do not extend liveness.
- Preserves the default 1.5 s recovery deadline and input release. Older peers retain the existing reliable heartbeat.
- Update both Mac and Windows to 0.6.18 to enable the fix. A real network outage still triggers recovery; validation on the original physical computers and network remains pending.

## Version 0.6.17

- Adds timestamped connection diagnostics for disconnections during active use.
- Separates heartbeat receipt, input-loop processing and pending writes, with transport counters and complete underlying QUIC error causes.
- Records timing and counter metadata only, without keyboard, clipboard or screen contents.
- Heartbeat deadlines and local-input recovery behavior are unchanged. The intermittent disconnection is not yet confirmed fixed. Update both computers to 0.6.17 and export diagnostics from both shortly after the next occurrence.

## Version 0.6.16

Local-only previews and new automatic arrangements now center the displays, placing a narrower lower Mac screen directly beneath the upper one. Dragging near another display midpoint snaps to the centerline; releasing saves and synchronizes the actual positions.

Existing layouts are preserved. Click Arrange automatically while connected to replace an older edge-aligned arrangement with centered positions. Crossings still follow touching highlighted edges: narrower centered displays are inset, and users can manually align the side edges to connect both Mac screens to the same Windows edge. An existing service-stop regression test now waits for its asynchronous flag, preventing a scheduling race without changing application shutdown behavior.

## Version 0.6.15

Drag each monitor independently, as in Windows Display settings. Nearby edges snap into place, valid crossing segments are highlighted, and releasing the pointer saves and synchronizes the layout automatically. Windows can connect to the upper Mac display, the lower display, or both: touching edge overlap determines the crossing segments; gaps do not cross.

Includes keyboard adjustments, drag cancellation, collision protection and automatic arrangement. Disconnected or changed displays require rearrangement instead of silently reusing an obsolete mapping. This virtual layout controls crossings between computers and does not modify operating-system display settings. Update both computers to 0.6.15; older peers retain whole-desktop edge operation.

[Mac/Windows feature validation](https://github.com/ytlds-64/EdgeMouse/actions/runs/34551490878) and browser regression checks passed for upper/lower/both display connections, partial overlap, gaps, cancellation and restart persistence. Crossings between the user's two physical computers still need confirmation after updating.

## Version 0.6.14

Fixes image copies from WeChat and other applications being skipped when the clipboard also contains file references. Embedded image pixels now take precedence over file metadata; file-only copies and Mac concealed/transient contents remain excluded. Referenced files are not opened and file paths are not sent as text.

The failing macOS WeChat copy was confirmed to contain both image and file-reference formats. [Mac/Windows regression validation](https://github.com/ytlds-64/EdgeMouse/actions/runs/34447906300) reproduces the failure with released 0.6.13 code and passes with the fix, including Windows legacy bitmaps without a PNG representation. Update both computers to 0.6.14 and retest pasting in your applications.

## Version 0.6.13

The embedded Mac service now uses a background activation policy, without its own Dock icon. Dock reopening and secondary launches focus the existing window; application quit cleans up the service, including macOS termination paths.

Settings → General adds an automatically saved, default-on clipboard toggle. Both paired devices must support and enable it. Newly copied text/images sync while connected; pre-connection, disconnected and disabled-period contents are not replayed. Limits are 1 MiB UTF-8 text, 16 million image pixels (maximum side 8192), and 16 MiB encoded PNG. File lists and Mac concealed/transient content are skipped. Clipboard payloads are never logged or persisted and use a separate low-priority reliable stream over the existing authenticated QUIC connection. Old peers retain input interoperability without the clipboard extension.

[Mac/Windows CI validation](https://github.com/ytlds-64/EdgeMouse/actions/runs/34332463000) passes, including native text/image clipboard roundtrips on both systems and Windows mouse injection regression. An isolated Mac app also passed close/reopen, minimize/restore, Cmd-Q service cleanup, and full exit with background running disabled. Clipboard synchronization has not yet been tested between the user’s two physical computers. Update both computers to 0.6.13 to use clipboard synchronization.

## Download

Download the latest signed packages from
[GitHub Releases](https://github.com/ytlds-64/EdgeMouse/releases/latest):

- Windows: use the `.exe` installer. It supports Simplified Chinese and English.
- macOS: use the universal `.dmg`, which supports both Apple silicon and Intel Macs.

Since 0.6.12, Overview, Input, Layout, and desktop preferences save
automatically. Toggles and selections apply immediately; speed and smoothing
sliders apply when adjustment ends. Layout dragging commits only on release,
and cancellation keeps the previous layout. Settings persist between launches
and synchronize when connected. Failed saves show an explicit retry action.

The Windows receiver now uses the low-level hook's injected-event flag to decide
whether movement may reclaim local control. Unmarked synthetic Raw Input packets
cannot trigger a receiver reclaim; outgoing Raw Input remains enabled. Pointer
warps during handoffs also use marked injection. Reclaim logs include the
triggering movement and receiver coordinates.

The current stable release is 0.6.18. Future signed releases can also be checked
and installed from the Settings page in the desktop application.

macOS packages currently use ad-hoc signing, without Developer ID signing or
Apple notarization. Updates may require granting Accessibility access again.
Starting with 0.6.7, the service stays on while waiting for the OS to report
permission granted, then continues automatically. Updater signature verification
is separate from macOS application identity signing; no privacy permissions are
bypassed or reset.

Version 0.6.11 restores local control immediately when the receiving computer's
physical mouse moves, before waiting for the authenticated peer acknowledgement.
Late remote input is fenced until confirmation, while local dragging stays safe.
Mac handback reconciles released keys, preserves genuine held-key suppression,
and cancels buffered remote motion. An existing editable responder can be
reasserted only in the unchanged foreground app/window; another input or app is
never activated, no click is synthesized, and no text content is read. Apps that
do not expose the required accessibility focus attributes are safely skipped.
The affected applications and input methods still need a real-device retest.
Update both computers for all fixes.

Version 0.6.10 filters EdgeMouse's own injected packets in Windows Raw Input,
preventing remote motion from entering the physical-mouse reclaim path. Real
mouse input and precision touchpads with null device handles remain supported.
A native Windows regression test exercises SendInput, Raw Input and the fallback
hook together and checks the visible cursor position.

Version 0.6.9 routes the pointer inside real monitor regions and maps handoffs only
to populated outer-edge segments. This prevents virtual movement through empty
areas beside landscape/portrait displays from diverging from the visible OS cursor.
Input now includes **Pointer speed** (25%–300%, default 100%), independently saved
for both directions and synchronized when both computers run 0.6.9+. Only outgoing
remote motion is scaled; local OS speed, scrolling and keyboard input are unchanged.
Older peers can still connect and exchange existing settings without receiving
unsupported speed fields. Diagnostic logs include individual peer monitor geometry.

After installation, complete secure pairing once from the Connection page. If
you previously used a source or command-line build, choose **Import previous
pairing** in the pairing window and select the old `edgemouse.toml`. EdgeMouse
copies the existing identity and trusted certificate into the per-user app data
directory, keeps a backup of the previous app config, and preserves the pairing
across future updates.

## Implemented

- Left, right, top, or bottom edge switching with entry hysteresis.
- Primary, secondary, middle, back, and forward buttons.
- Vertical and horizontal scrolling.
- Independent horizontal and wheel/vertical scroll reversal for each control
  direction, editable on either device and synchronized to the owning computer.
- Bidirectional keyboard forwarding while the local mouse owns the peer,
  including modifiers, navigation, function keys, numpad keys, and key repeat.
- Cross-platform shortcut mapping: Windows `Ctrl` becomes Mac `Command` and Mac
  `Command` becomes Windows `Ctrl`; the remaining Control/Windows and
  Alt/Option keys stay available in the corresponding platform roles.
- Ordered keyboard delivery, forced key release on handback/disconnect, held-key
  transition safety, and `Ctrl+Alt+Shift+Esc` emergency local recovery.
- Coordinate mapping between differently sized Windows and macOS displays.
- Automatic resolution, scaling, rotation, and active multi-display desktop
  bounds detection at startup and after reconnecting.
- Authenticated desktop-geometry exchange, so neither configuration duplicates
  the peer's current width, height, origin, orientation, or scale.
- Native macOS `CGEventTap` capture and marked `CGEventPost` injection.
- Native Windows `WH_MOUSE_LL` capture and marked `SendInput` injection.
- Windows Raw Input movement capture after handoff, retaining `WH_MOUSE_LL` as
  the fail-open suppression and automatic fallback layer.
- Mutually authenticated QUIC/TLS with one explicitly trusted peer certificate.
- One-time 8-digit short-code pairing that securely exchanges public certificates
  while keeping both private keys on their original machines.
- Latest-position QUIC datagrams for movement, with reliable ordered delivery
  retained for clicks, scrolling, edge transitions, and final positions.
- Versioned, bounded binary frames with strict untrusted-input validation.
- 250 ms independent heartbeats between updated peers (500 ms reliable heartbeats
  for older peers), 1.5 s default timeout, local-pointer recovery, and forced
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
- A Tauri 2 desktop application that reuses the EdgeMouse UI on Windows and
  macOS and reads the real agent version, local process state, configuration,
  trusted node summary, platform permission state, detected desktop geometry,
  current connection phase, reconnect count, latency, jitter, and input counters.
- Live desktop scroll-direction controls that persist to `edgemouse.toml` and
  update an already-running background agent without reconnecting.
- A persistent Screen layout editor that converts the shared Windows-to-Mac
  view into each computer's local edge, synchronizes the opposite edge over the
  trusted connection, and reconnects both agents automatically.
- Working light/dark/system themes and Simplified Chinese/English UI switching.

Windows uses a fixed mouse capture anchor while control is remote. Raw Input
preserves high-polling-rate physical movement in that state, while the low-level
hook continues to suppress local legacy events and takes over automatically if
Raw Input is unavailable. Both platforms fail open if capture cannot keep up,
and keys held before a handoff remain local until physically released.
Set `session.windows_raw_input = false` on Windows to force the established
low-level-hook movement path; the default is `true`.

## Build

Install a stable Rust toolchain (Rust 1.89 or newer), then build on each target
machine:

```sh
cargo build --release -p edgemouse-agent
```

The executable is `target/release/edgemouse` on macOS and
`target\release\edgemouse.exe` on Windows.

### Desktop application

The desktop application keeps the proven background agent separate
from the window. This means opening or closing the window does not interrupt an
active mouse/keyboard session. Connection state and the diagnostics quality chart
are live. The horizontal and wheel-direction switches on the Input page are
connected to the local background agent. Dragging or choosing a direction on
the Screen layout page applies automatically when direction selection or dragging ends; configuration controls that
still say “prototype” remain demonstrations until their Rust commands are
connected in the next stages.

Build and open the desktop window on macOS from the repository root:

```sh
cargo build -p edgemouse-desktop
./target/debug/edgemouse-desktop --config ./edgemouse.toml
```

On Windows, after pulling the latest `main`, use the included PowerShell script:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-run-desktop-windows.ps1
```

It safely restarts and rebuilds both the background service and desktop app,
validates the existing `edgemouse.toml`, starts the updated background service,
then opens the desktop window. Windows 11 already includes the WebView2 runtime
used by Tauri; older Windows installations may need the current WebView2 Runtime
installed first.

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
rendered on a stable 4 ms cadence through a fixed 12 ms jitter buffer.
Positions between received samples are interpolated instead of jumping from one
network packet to the next. The renderer never extrapolates beyond the newest
real position, so stopping cannot produce prediction overshoot followed by a
visible correction. Buttons, wheels, leave events, and drag transitions flush
the newest real position immediately, so buffering never changes control-event
ordering or click accuracy.
With Local mouse priority enabled, moving this computer's real physical mouse
immediately restores its local control. Synthetic cursor movement is excluded.
Local input does not wait for the peer acknowledgement. Late remote events are
discarded until Ack or an ordered Leave, and no reverse crossing is forced.
If confirmation does not arrive within 1.5 seconds, the link reconnects while
local input remains available. Disable this option if incidental movement of
the receiving computer's mouse should not interrupt remote control.
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
EdgeMouse 0.3.5 removes prediction and stabilizes the receive timeline at 12 ms
after real-world testing exposed stop-time correction jumps. It also resamples
and seeds the Windows pointer after connection setup, so the first outward edge
movement is preserved even when the program becomes ready with the cursor
already at the screen boundary. Protocol v5 is unchanged.
EdgeMouse 0.4.0 adds native macOS keyboard capture and completes bidirectional
keyboard following. Mac `Command` shortcuts map to Windows `Control`, keys held
before a handoff remain local, and all captured keys are released during
handback, emergency recovery, disconnect, or shutdown. Protocol v5 remains
unchanged, but both computers should run 0.4.0 when testing reverse keyboard
control.
EdgeMouse 0.5.0 adds Windows Raw Input movement and ordered raw button/wheel
capture after handoff. The existing low-level hook remains active as the safety
suppression and automatic fallback layer. Protocol v5 is unchanged.
EdgeMouse 0.5.1 adds live local telemetry for the desktop app: connection phase,
reconnect count, five-second RTT and arrival-jitter samples, and recent movement
counters. The UI plots the latest 60 seconds without parsing runtime log text.
EdgeMouse 0.5.2 reports smoothed RTT variation consistently in both control
directions, clarifies that Windows needs no macOS-style input permission, and
makes the Input page's horizontal and wheel-direction switches persistent and
live. Each computer stores the preferences for the physical input attached to
that computer; existing configurations default both switches to off.
EdgeMouse 0.5.3 makes the Screen layout page persistent. Saving writes the
local `layout.peer_on`, sends the corresponding edge over the authenticated
connection, and makes the trusted peer store the opposite edge before both
agents reconnect automatically. This layout update and acknowledgement use
protocol v6, so both computers must update to 0.5.3 before using this release.
EdgeMouse 0.5.4 gives macOS its native decorated window and localized system
menus while retaining the custom Windows title bar. The desktop status panel
also keeps the last confirmed agent state across two isolated local-control
poll misses, so a brief status-channel delay is shown as confirmation in
progress instead of incorrectly claiming that the background service stopped.
Protocol v6 is unchanged and remains connection-compatible with 0.5.3.
EdgeMouse 0.5.5 gives the frameless Windows desktop window rounded transparent
corners and regenerates the macOS application icon with a real alpha channel,
removing the unwanted white square shown behind the icon in the Dock. Protocol
v6 is unchanged and remains connection-compatible with 0.5.3.
EdgeMouse 0.5.6 makes the Windows webview slightly overscan its transparent
host so display scaling cannot expose a dark seam around the rounded corners.
It also replaces the improvised macOS device mark with a crisp vector Apple
silhouette. Protocol v6 is unchanged and remains connection-compatible with
0.5.3.
EdgeMouse 0.5.7 replaces that approximate device mark with the standardized
Simple Icons Apple vector and starts the Windows background agent through
Windows Script Host without opening a PowerShell window. Runtime output is
still written to the current and archived log files, while the visible runner
remains available for manual diagnostics. Protocol v6 is unchanged and remains
connection-compatible with 0.5.3.
EdgeMouse 0.5.8 upgrades the connection protocol to v7 and exchanges the full
per-display topology after authentication. The desktop app now draws every
Windows and macOS monitor at its real relative position, labels its physical
pixel resolution and primary-display status, and derives both layout summaries
from live device data. Both computers must update to 0.5.8 before connecting.
EdgeMouse 0.5.9 makes the Overview layout label follow the same canonical
Mac-relative-to-Windows direction used by the Screen layout page. It also turns
the Overview power switch into a live local-service control: the desktop app
starts only a matching agent version with file logging, suppresses the Windows
console window, and uses the safe control channel when stopping. Protocol v7 is
unchanged and remains connection-compatible with 0.5.8.
EdgeMouse 0.6.0 connects the remaining desktop controls to the native service,
adds signed Windows and universal macOS installers, and enables signed in-app
updates. Protocol v7 is unchanged and remains connection-compatible with 0.5.8.
EdgeMouse 0.6.1 unifies the desktop application version shown throughout the UI,
adds Simplified Chinese and English Windows installer interfaces, and provides a
Chinese-first bilingual project home page. Protocol v7 is unchanged.
EdgeMouse 0.6.2 fixes startup when packaged installations have not yet imported
their trusted peer certificate, adds one-time import of a previous
`edgemouse.toml` pairing, and corrects error-toast icon sizing and long messages.
Protocol v7 is unchanged.
EdgeMouse 0.6.3 adds live download progress, transferred size, and installation
stages to in-app updates. Diagnostics can now repair pairing, discovery, and the
background service directly, open the correct macOS permission page, and recheck
permission status automatically when the user returns. Protocol v7 is unchanged.
EdgeMouse 0.6.4 fixes macOS Accessibility authorization for the packaged input
service, adds a Show in folder action for diagnostics exports and a connection
shortcut on Overview, uses live connection state on Screen layout, and replaces
the Windows icon with transparent multi-resolution artwork. Protocol v7 is
unchanged.
EdgeMouse 0.6.5 fixes a race between normal handback and physical-mouse reclaim
that could terminate the input service. Session failures release input before
following the reconnect setting. Windows now uses full-resolution window and
tray icons, and installation asks the shell to refresh cached icons. Diagnostic
exports include background stderr and preserve Unicode log text. Protocol v7 is
unchanged.
EdgeMouse 0.6.6 defaults manual Windows installation to Simplified Chinese with
an English option, stops the background service before replacement, and verifies
the installed component version. Layout edits saved on either device propagate
automatically; offline edits survive restarts and initial connections reconcile
legacy layout differences. Input settings for both directions, edge protection
and automatic reconnect merge per field using logical revisions and a device-ID
tie-break. Saved edits outrank initial defaults; offline edits survive restarts.
Theme, language, sidebar expansion, startup settings, identities and permissions
remain local. Both devices need 0.6.6 for full synchronization. The optional
extension is capability-negotiated on protocol v7, retaining legacy connections.
Input cards have improved spacing and aligned rows; the sidebar can collapse to
icons and remembers its state locally. Fixed key mappings and dwell switching
remain non-editable.

## Verify the source tree

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p edgemouse-agent -- doctor
cargo run -p edgemouse-agent -- demo
```

Run `node scripts/test-native-service.cjs` for native UI event regressions.
`node scripts/test-settings-autosave.cjs` additionally requires Playwright and
a browser (`EDGEMOUSE_TEST_BROWSER` can specify the executable). It exercises
rapid edits, retries, stale snapshots, drag cancellation, reopening, and reset
ordering against simulated IPC without controlling real devices.

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
