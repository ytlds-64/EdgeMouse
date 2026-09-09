//! Clipboard I/O and image conversion run off the input and network loops.
//! Only newly copied text/images are sent; contents never enter logs or disk.
use edgemouse_core::NodeId;
use edgemouse_protocol::clipboard::{
    ClipboardContent, ClipboardPacket, MAX_IMAGE_BYTES, MAX_TEXT_BYTES,
};
use edgemouse_transport::ClipboardLink;
use image::{ImageEncoder, ImageFormat, ImageReader, Limits};
use ring::digest::{SHA256, digest};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(150);
const MAX_PIXELS: usize = 16 * 1024 * 1024;
type Slot = Arc<Mutex<Option<ClipboardPacket>>>;

#[derive(Default)]
struct Revisions {
    clock: u64,
    accepted: (u64, u128),
}
impl Revisions {
    fn local(&mut self, node: NodeId) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.accepted = (self.clock, node.0);
        self.clock
    }
    fn receive(&mut self, revision: u64, node: NodeId) -> bool {
        self.clock = self.clock.max(revision);
        (revision, node.0) > self.accepted
    }
    fn applied(&mut self, revision: u64, node: NodeId) {
        self.accepted = (revision, node.0);
    }
}

/// Missing preferences preserve the product default. Unreadable/malformed
/// preferences fail closed while an atomic settings write is being completed.
fn enabled(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(source) => toml::from_str::<toml::Value>(&source)
            .ok()
            .map(|value| {
                value
                    .get("clipboardSync")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(true)
            })
            .unwrap_or(false),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn image_dimensions(width: usize, height: usize) -> Result<(), &'static str> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || width
            .checked_mul(height)
            .is_none_or(|pixels| pixels > MAX_PIXELS)
    {
        Err("clipboard image exceeds supported dimensions")
    } else {
        Ok(())
    }
}

fn encode_image(image: arboard::ImageData<'_>) -> Result<ClipboardContent, &'static str> {
    image_dimensions(image.width, image.height)?;
    if image.bytes.len() != image.width * image.height * 4 {
        return Err("invalid clipboard pixels");
    }
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &image.bytes,
            image.width as u32,
            image.height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "clipboard image conversion failed")?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("clipboard image exceeds transfer limit");
    }
    Ok(ClipboardContent::Png(bytes))
}

fn decode_image(bytes: &[u8]) -> Result<arboard::ImageData<'static>, &'static str> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("clipboard image exceeds transfer limit");
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some((MAX_PIXELS * 4) as u64);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "invalid clipboard image")?
        .into_rgba8();
    image_dimensions(image.width() as usize, image.height() as usize)?;
    Ok(arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: Cow::Owned(image.into_raw()),
    })
}

trait ClipboardBackend {
    fn revision(&self) -> u64;
    fn read(&mut self) -> Result<Option<ClipboardContent>, &'static str>;
    fn write(&mut self, content: &ClipboardContent) -> Result<(), &'static str>;
}

struct NativeClipboard(arboard::Clipboard);
impl ClipboardBackend for NativeClipboard {
    fn revision(&self) -> u64 {
        native_revision()
    }
    fn read(&mut self) -> Result<Option<ClipboardContent>, &'static str> {
        if excluded_content() {
            return Ok(None);
        }
        #[cfg(target_os = "macos")]
        if let Some(bytes) = objc2_app_kit::NSPasteboard::generalPasteboard()
            .dataForType(&objc2_foundation::NSString::from_str("public.png"))
        {
            if bytes.len() > MAX_IMAGE_BYTES {
                return Ok(None);
            }
            return Ok(decode_image(&bytes.to_vec())
                .ok()
                .and_then(|image| encode_image(image).ok()));
        }
        match self.0.get_image() {
            Ok(image) => return Ok(encode_image(image).ok()),
            Err(arboard::Error::ContentNotAvailable) => {}
            Err(_) => return Err("clipboard is temporarily unavailable"),
        }
        match self.0.get_text() {
            Ok(text) if !text.is_empty() && text.len() <= MAX_TEXT_BYTES => {
                Ok(Some(ClipboardContent::Text(text)))
            }
            Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(_) => Err("clipboard is temporarily unavailable"),
        }
    }
    fn write(&mut self, content: &ClipboardContent) -> Result<(), &'static str> {
        match content {
            ClipboardContent::Text(text) => self
                .0
                .set_text(text.clone())
                .map_err(|_| "clipboard is busy"),
            ClipboardContent::Png(bytes) => {
                let revision = native_revision();
                let image = decode_image(bytes)?;
                if native_revision() != revision {
                    return Err("clipboard changed during image decoding");
                }
                self.0.set_image(image).map_err(|_| "clipboard is busy")
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn native_revision() -> u64 {
    objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as u64
}
#[cfg(target_os = "macos")]
fn excluded_content() -> bool {
    objc2_app_kit::NSPasteboard::generalPasteboard()
        .types()
        .is_some_and(|types| {
            types.iter().any(|kind| {
                matches!(
                    kind.to_string().as_str(),
                    "public.file-url"
                        | "NSFilenamesPboardType"
                        | "org.nspasteboard.ConcealedType"
                        | "org.nspasteboard.TransientType"
                )
            })
        })
}
#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetClipboardSequenceNumber() -> u32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
}
#[cfg(target_os = "windows")]
fn native_revision() -> u64 {
    // SAFETY: Query takes no arguments and retains no memory.
    u64::from(unsafe { GetClipboardSequenceNumber() })
}
#[cfg(target_os = "windows")]
fn excluded_content() -> bool {
    // SAFETY: CF_HDROP is the predefined file-list format, queried without data access.
    unsafe { IsClipboardFormatAvailable(15) != 0 }
}

struct SyncState {
    revisions: Revisions,
    os_revision: u64,
    last_digest: Option<Vec<u8>>,
    enabled: bool,
    pending: Option<ClipboardPacket>,
    retries: u8,
}
impl SyncState {
    fn new(os_revision: u64) -> Self {
        Self {
            revisions: Revisions::default(),
            os_revision,
            last_digest: None,
            enabled: false,
            pending: None,
            retries: 0,
        }
    }
    fn tick(
        &mut self,
        backend: &mut impl ClipboardBackend,
        enabled: bool,
        local: NodeId,
        peer: NodeId,
        incoming: Option<ClipboardPacket>,
    ) -> Option<ClipboardPacket> {
        let revision = backend.revision();
        if !enabled || !self.enabled {
            self.os_revision = revision;
            self.pending = None;
            self.last_digest = None;
            self.enabled = enabled;
            // Do not replay content copied before connecting or enabling sync.
            return None;
        }
        if let Some(packet) = incoming
            && self.revisions.receive(packet.revision, peer)
            && self
                .pending
                .as_ref()
                .is_none_or(|pending| packet.revision > pending.revision)
        {
            self.pending = Some(packet);
            self.retries = 0;
        }
        let mut outgoing = None;
        if revision != self.os_revision {
            match backend.read() {
                Ok(content) if revision == backend.revision() => {
                    self.os_revision = revision;
                    if let Some(content) = content {
                        let hash = digest(&SHA256, content.bytes()).as_ref().to_vec();
                        if self.last_digest.as_ref() != Some(&hash) {
                            self.last_digest = Some(hash);
                            outgoing = Some(ClipboardPacket {
                                revision: self.revisions.local(local),
                                content,
                            });
                            self.pending = None;
                        }
                    } else {
                        // Unsupported/cleared local content still supersedes a
                        // remote write that is waiting for the clipboard lock.
                        self.revisions.local(local);
                        self.pending = None;
                        self.last_digest = None;
                    }
                }
                // A busy clipboard or a copy still in progress must not be
                // overwritten by a late remote transfer. Retry next tick.
                _ => return None,
            }
        }
        if let Some(packet) = self.pending.take()
            && self.revisions.receive(packet.revision, peer)
        {
            if backend.revision() != self.os_revision {
                self.pending = Some(packet);
                return outgoing;
            }
            match backend.write(&packet.content) {
                Ok(()) => {
                    self.os_revision = backend.revision();
                    self.last_digest =
                        Some(digest(&SHA256, packet.content.bytes()).as_ref().to_vec());
                    self.revisions.applied(packet.revision, peer);
                }
                Err(_) if self.retries < 10 => {
                    self.retries += 1;
                    self.pending = Some(packet);
                }
                Err(_) => eprintln!("Clipboard sync could not apply content; copy again to retry"),
            }
        }
        outgoing
    }
}

pub struct ClipboardWorker {
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    sender: tokio::task::JoinHandle<()>,
    receiver: tokio::task::JoinHandle<()>,
}
impl ClipboardWorker {
    pub fn start(link: ClipboardLink, local: NodeId, peer: NodeId, preferences: PathBuf) -> Self {
        let outgoing: Slot = Arc::default();
        let incoming: Slot = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let send_slot = Arc::clone(&outgoing);
        let send_link = link.clone();
        let sender = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let packet = send_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(packet) = packet
                    && send_link.send(&packet).await.is_err()
                {
                    eprintln!("Clipboard transfer failed; input connection remains active");
                }
            }
        });
        let receive_slot = Arc::clone(&incoming);
        let receiver = tokio::spawn(async move {
            loop {
                match link.receive().await {
                    Ok(packet) => {
                        let mut slot = receive_slot.lock().unwrap_or_else(|e| e.into_inner());
                        if slot
                            .as_ref()
                            .is_none_or(|pending| packet.revision > pending.revision)
                        {
                            *slot = Some(packet);
                        }
                    }
                    Err(_) if link.is_closed() => break,
                    Err(_) => {}
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        let thread = std::thread::Builder::new()
            .name("edgemouse-clipboard".into())
            .spawn(move || {
                let mut backend = match arboard::Clipboard::new() {
                    Ok(clipboard) => NativeClipboard(clipboard),
                    Err(_) => {
                        eprintln!("Clipboard sync is unavailable on this desktop");
                        return;
                    }
                };
                let mut state = SyncState::new(backend.revision());
                while !stopping.load(Ordering::Acquire) {
                    let enabled = enabled(&preferences);
                    let packet = incoming.lock().unwrap_or_else(|e| e.into_inner()).take();
                    if let Some(packet) = state.tick(&mut backend, enabled, local, peer, packet) {
                        *outgoing.lock().unwrap_or_else(|e| e.into_inner()) = Some(packet);
                    }
                    if !enabled {
                        outgoing.lock().unwrap_or_else(|e| e.into_inner()).take();
                    }
                    std::thread::sleep(POLL);
                }
            })
            .ok();
        Self {
            stopping: stop,
            thread,
            sender,
            receiver,
        }
    }
}
impl Drop for ClipboardWorker {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.sender.abort();
        self.receiver.abort();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FakeClipboard {
        revision: u64,
        content: Option<ClipboardContent>,
        reads: usize,
        writes: usize,
        busy_read: bool,
        busy_write: bool,
    }
    impl FakeClipboard {
        fn copy(&mut self, text: &str) {
            self.revision += 1;
            self.content = Some(ClipboardContent::Text(text.into()));
        }
    }
    impl ClipboardBackend for FakeClipboard {
        fn revision(&self) -> u64 {
            self.revision
        }
        fn read(&mut self) -> Result<Option<ClipboardContent>, &'static str> {
            self.reads += 1;
            if self.busy_read {
                Err("busy")
            } else {
                Ok(self.content.clone())
            }
        }
        fn write(&mut self, content: &ClipboardContent) -> Result<(), &'static str> {
            if self.busy_write {
                return Err("busy");
            }
            self.writes += 1;
            self.revision += 1;
            self.content = Some(content.clone());
            Ok(())
        }
    }
    fn packet(revision: u64, text: &str) -> ClipboardPacket {
        ClipboardPacket {
            revision,
            content: ClipboardContent::Text(text.into()),
        }
    }
    fn tick(
        state: &mut SyncState,
        clipboard: &mut FakeClipboard,
        enabled: bool,
        incoming: Option<ClipboardPacket>,
    ) -> Option<ClipboardPacket> {
        state.tick(clipboard, enabled, NodeId(1), NodeId(2), incoming)
    }
    #[test]
    fn newly_copied_unicode_syncs_once_without_reading_history_or_echoing() {
        let mut clipboard = FakeClipboard::default();
        clipboard.copy("private old history");
        let mut state = SyncState::new(clipboard.revision());
        assert!(tick(&mut state, &mut clipboard, true, None).is_none());
        assert_eq!(clipboard.reads, 0);
        clipboard.copy("新文字 👋\n第二行");
        assert_eq!(
            tick(&mut state, &mut clipboard, true, None).unwrap(),
            packet(1, "新文字 👋\n第二行")
        );
        assert!(tick(&mut state, &mut clipboard, true, None).is_none());
        assert!(tick(&mut state, &mut clipboard, true, Some(packet(2, "remote"))).is_none());
        assert_eq!(clipboard.writes, 1);
        assert!(tick(&mut state, &mut clipboard, true, None).is_none());
        assert_eq!(clipboard.reads, 1);
        tick(
            &mut state,
            &mut clipboard,
            true,
            Some(packet(2, "duplicate")),
        );
        assert_eq!(
            clipboard.content,
            Some(ClipboardContent::Text("remote".into()))
        );
    }
    #[test]
    fn disabled_sync_does_not_read_write_or_replay_content_on_enable() {
        let mut clipboard = FakeClipboard::default();
        let mut state = SyncState::new(0);
        tick(&mut state, &mut clipboard, true, None);
        clipboard.copy("offline");
        assert!(
            tick(
                &mut state,
                &mut clipboard,
                false,
                Some(packet(20, "remote"))
            )
            .is_none()
        );
        tick(&mut state, &mut clipboard, true, None);
        tick(&mut state, &mut clipboard, true, None);
        assert_eq!((clipboard.reads, clipboard.writes), (0, 0));
        clipboard.copy("new copy");
        assert!(tick(&mut state, &mut clipboard, true, None).is_some());
    }
    #[test]
    fn clipboard_busy_retries_and_new_local_copy_supersedes_pending_remote() {
        let mut clipboard = FakeClipboard::default();
        let mut state = SyncState::new(0);
        tick(&mut state, &mut clipboard, true, None);
        clipboard.busy_write = true;
        tick(&mut state, &mut clipboard, true, Some(packet(8, "pending")));
        clipboard.copy("new local");
        clipboard.busy_write = false;
        assert_eq!(
            tick(&mut state, &mut clipboard, true, None)
                .unwrap()
                .revision,
            9
        );
        assert_eq!(clipboard.writes, 0);
        clipboard.busy_write = true;
        tick(&mut state, &mut clipboard, true, Some(packet(10, "retry")));
        clipboard.busy_write = false;
        tick(&mut state, &mut clipboard, true, None);
        assert_eq!(
            clipboard.content,
            Some(ClipboardContent::Text("retry".into()))
        );
    }
    #[test]
    fn simultaneous_copies_converge_in_both_directions() {
        let mut a = FakeClipboard::default();
        let mut b = FakeClipboard::default();
        let mut sa = SyncState::new(0);
        let mut sb = SyncState::new(0);
        sa.tick(&mut a, true, NodeId(1), NodeId(2), None);
        sb.tick(&mut b, true, NodeId(2), NodeId(1), None);
        a.copy("A");
        b.copy("B");
        let from_a = sa.tick(&mut a, true, NodeId(1), NodeId(2), None);
        let from_b = sb.tick(&mut b, true, NodeId(2), NodeId(1), None);
        sa.tick(&mut a, true, NodeId(1), NodeId(2), from_b);
        sb.tick(&mut b, true, NodeId(2), NodeId(1), from_a);
        assert_eq!(a.content, b.content);
        assert_eq!(a.content, Some(ClipboardContent::Text("B".into())));
    }
    #[test]
    fn png_roundtrip_preserves_transparency_and_rejects_invalid_or_large_images() {
        let rgba = vec![255, 0, 0, 255, 0, 128, 255, 0];
        let content = encode_image(arboard::ImageData {
            width: 2,
            height: 1,
            bytes: Cow::Borrowed(&rgba),
        })
        .unwrap();
        let image = decode_image(content.bytes()).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.bytes.as_ref(), rgba);
        assert!(decode_image(b"\x89PNG\r\n\x1a\ntruncated").is_err());
        assert!(image_dimensions(0, 5).is_err());
        assert!(image_dimensions(8192, 8192).is_err());
        assert!(image_dimensions(5120, 2880).is_ok());
        let packet = ClipboardPacket {
            revision: 1,
            content,
        };
        assert_eq!(
            ClipboardPacket::decode(&packet.encode().unwrap()).unwrap(),
            packet
        );
    }
    /// Run only on a disposable desktop (CI); never reads the existing payload.
    #[test]
    #[ignore = "uses the system clipboard; requires EDGEMOUSE_CLIPBOARD_NATIVE_TEST=1 on a disposable desktop"]
    fn native_clipboard_text_and_image_roundtrip() {
        assert_eq!(
            std::env::var("EDGEMOUSE_CLIPBOARD_NATIVE_TEST").as_deref(),
            Ok("1")
        );
        let mut native = NativeClipboard(arboard::Clipboard::new().unwrap());
        let text = ClipboardContent::Text("EdgeMouse 剪贴板 🖱\n第二行".into());
        native.write(&text).unwrap();
        assert_eq!(native.read().unwrap(), Some(text));
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 0, 0, 0, 0];
        let content = encode_image(arboard::ImageData {
            width: 2,
            height: 2,
            bytes: Cow::Borrowed(&pixels),
        })
        .unwrap();
        native.write(&content).unwrap();
        let copied = native.read().unwrap().unwrap();
        let roundtrip = decode_image(copied.bytes()).unwrap();
        assert_eq!((roundtrip.width, roundtrip.height), (2, 2));
        assert_eq!(roundtrip.bytes.as_ref(), pixels);
        native.0.clear().unwrap();
    }

    #[test]
    fn unreadable_clipboard_retains_remote_transfer_until_a_stable_read() {
        let mut clipboard = FakeClipboard::default();
        let mut state = SyncState::new(0);
        tick(&mut state, &mut clipboard, true, None);
        // A failed observation must not drop the incoming payload.
        clipboard.revision = 1;
        clipboard.busy_read = true;
        tick(&mut state, &mut clipboard, true, Some(packet(3, "pending")));
        assert!(state.pending.is_some());
        assert_eq!(clipboard.writes, 0);
        clipboard.busy_read = false;
        clipboard.revision = 0;
        tick(&mut state, &mut clipboard, true, None);
        assert_eq!(clipboard.writes, 1);
    }
}
