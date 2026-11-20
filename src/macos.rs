use crate::panel_view::{PANEL_TITLE, SharedWindowPrefs};
use crate::types::{BackendEvent, JobStatus, MonitorSnapshot, RunPhase};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
    kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowOwnerName,
};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngEncoder;
use image::{AnimationDecoder, ColorType, ImageEncoder, RgbaImage};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSCellImagePosition, NSImage, NSScreen, NSStatusBar, NSStatusItem, NSWindow,
};
use objc2_foundation::{NSData, NSObject, NSPoint, NSSize, NSString};
use smol::channel::Sender;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime};

const BUILTIN_IDLE_GIF: &[u8] = include_bytes!("../assets/icons/idle.gif");
const BUILTIN_RUN_GIF: &[u8] = include_bytes!("../assets/icons/run.gif");
const BUILTIN_WAIT_GIF: &[u8] = include_bytes!("../assets/icons/wait.gif");
const BUILTIN_TROUBLE_GIF: &[u8] = include_bytes!("../assets/icons/trouble.gif");
const BUILTIN_DONE_GIF: &[u8] = include_bytes!("../assets/icons/done.gif");

const TARGET_MENU_ICON_HEIGHT_PT: f64 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DockMode {
    TopRight = 0,
    BottomRight = 1,
    Manual = 2,
}

impl DockMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::TopRight,
            1 => Self::BottomRight,
            _ => Self::Manual,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TopRight => "追従:右上",
            Self::BottomRight => "追従:右下",
            Self::Manual => "配置:自由",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MenuBarMode {
    IconAndCounts = 0,
    IconOnly = 1,
    TextOnly = 2,
}

impl MenuBarMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::IconAndCounts,
            1 => Self::IconOnly,
            _ => Self::TextOnly,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::IconAndCounts => Self::IconOnly,
            Self::IconOnly => Self::TextOnly,
            Self::TextOnly => Self::IconAndCounts,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::IconAndCounts => "バー:狩人+数",
            Self::IconOnly => "バー:狩人のみ",
            Self::TextOnly => "バー:文字",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteKind {
    Idle,
    Run,
    Wait,
    Trouble,
    Done,
}

struct SpriteFrames {
    frames: Vec<Retained<NSImage>>,
}

struct SpriteBank {
    idle: SpriteFrames,
    run: SpriteFrames,
    wait: SpriteFrames,
    trouble: SpriteFrames,
    done: SpriteFrames,
    dir_stamp: u128,
}

pub fn icons_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".snowflake/cortex/job-monitor/icons")
}

pub fn open_icons_directory() {
    let dir = icons_directory();
    let _ = fs::create_dir_all(&dir);
    std::thread::spawn(move || {
        let _ = std::process::Command::new("/usr/bin/open").arg(&dir).status();
    });
}

fn compute_dir_stamp(dir: &Path) -> u128 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut acc: u128 = 14695981039346656037;
    for entry in entries.flatten() {
        if let Ok(meta) = entry.metadata() {
            let len = meta.len() as u128;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            acc = acc.wrapping_mul(1099511628211).wrapping_add(len ^ mtime);
        }
    }
    acc
}

fn is_monochrome_rgba(images: &[RgbaImage]) -> bool {
    for img in images {
        for p in img.pixels() {
            if p[3] > 32 {
                let max_c = p[0].max(p[1]).max(p[2]);
                let min_c = p[0].min(p[1]).min(p[2]);
                if max_c.saturating_sub(min_c) > 18 {
                    return false;
                }
            }
        }
    }
    true
}

fn rgba_to_nsimage(img: &RgbaImage, is_template: bool) -> Option<Retained<NSImage>> {
    let w = img.width();
    let h = img.height();
    if w == 0 || h == 0 {
        return None;
    }
    let mut png_buf = Vec::with_capacity((w * h) as usize);
    PngEncoder::new(&mut png_buf)
        .write_image(img.as_raw(), w, h, ColorType::Rgba8.into())
        .ok()?;

    let ns_data = NSData::with_bytes(&png_buf);
    let ns_img = NSImage::initWithData(NSImage::alloc(), &ns_data)?;
    let pt_h = TARGET_MENU_ICON_HEIGHT_PT;
    let pt_w = (pt_h * (w as f64 / h as f64)).clamp(14.0, 56.0);
    ns_img.setSize(NSSize::new(pt_w, pt_h));
    ns_img.setTemplate(is_template);
    Some(ns_img)
}

fn decode_gif_bytes(bytes: &[u8]) -> Option<Vec<RgbaImage>> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    if frames.is_empty() {
        return None;
    }
    Some(frames.into_iter().map(|f| f.into_buffer()).collect())
}

fn load_seq_png_dir(dir: &Path) -> Option<Vec<RgbaImage>> {
    if !dir.is_dir() {
        return None;
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        if let Ok(bytes) = fs::read(&p)
            && let Ok(dyn_img) = image::load_from_memory(&bytes)
        {
            out.push(dyn_img.to_rgba8());
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn load_sprite_set(dir: &Path, stem: &str, fallback_gif: &[u8], prefer_template: bool) -> SpriteFrames {
    let gif_path = dir.join(format!("{stem}.gif"));
    let seq_dir = dir.join(stem);

    let rgba_frames = if let Ok(bytes) = fs::read(&gif_path)
        && let Some(decoded) = decode_gif_bytes(&bytes)
    {
        decoded
    } else if let Some(seq) = load_seq_png_dir(&seq_dir) {
        seq
    } else {
        decode_gif_bytes(fallback_gif).unwrap_or_default()
    };

    // Only apply macOS template auto-inversion if prefer_template AND the artwork is actually monochrome
    let use_template = prefer_template && is_monochrome_rgba(&rgba_frames);
    let frames = rgba_frames
        .iter()
        .filter_map(|img| rgba_to_nsimage(img, use_template))
        .collect();

    SpriteFrames { frames }
}

impl SpriteBank {
    fn load() -> Self {
        let dir = icons_directory();
        let dir_stamp = compute_dir_stamp(&dir);
        Self {
            idle: load_sprite_set(&dir, "idle", BUILTIN_IDLE_GIF, true),
            run: load_sprite_set(&dir, "run", BUILTIN_RUN_GIF, true),
            wait: load_sprite_set(&dir, "wait", BUILTIN_WAIT_GIF, false),
            trouble: load_sprite_set(&dir, "trouble", BUILTIN_TROUBLE_GIF, false),
            done: load_sprite_set(&dir, "done", BUILTIN_DONE_GIF, false),
            dir_stamp,
        }
    }

    fn frames_for(&self, kind: SpriteKind) -> &[Retained<NSImage>] {
        match kind {
            SpriteKind::Idle => &self.idle.frames,
            SpriteKind::Run => &self.run.frames,
            SpriteKind::Wait => &self.wait.frames,
            SpriteKind::Trouble => &self.trouble.frames,
            SpriteKind::Done => &self.done.frames,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CgRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub struct ClickTargetIvars {
    tx: Sender<BackendEvent>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CoCoJobMonitorStatusTarget"]
    #[ivars = ClickTargetIvars]
    pub struct StatusClickTarget;

    impl StatusClickTarget {
        #[unsafe(method(onStatusClick:))]
        fn on_status_click(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().tx.try_send(BackendEvent::ToggleMainPanel);
        }
    }
);

impl StatusClickTarget {
    fn new(mtm: MainThreadMarker, tx: Sender<BackendEvent>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClickTargetIvars { tx });
        unsafe { msg_send![super(this), init] }
    }
}

pub struct MacStatusController {
    item: Retained<NSStatusItem>,
    _target: Retained<StatusClickTarget>,
    sprites: SpriteBank,
    current_kind: SpriteKind,
    frame_interval_ms: u64,
    frame_idx: usize,
    last_frame_advance: Instant,
    compact_title: String,
    text_only_title: String,
    last_applied_mode: MenuBarMode,
    last_applied_title: String,
    auto_hidden_by_coco: bool,
    manually_hidden: bool,
}

impl MacStatusController {
    pub fn new(tx: Sender<BackendEvent>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let bar = NSStatusBar::systemStatusBar();
        let item = bar.statusItemWithLength(-1.0);
        let target = StatusClickTarget::new(mtm, tx);
        let sprites = SpriteBank::load();

        if let Some(btn) = item.button(mtm) {
            if let Some(first) = sprites.frames_for(SpriteKind::Idle).first() {
                btn.setImage(Some(first));
                btn.setImagePosition(NSCellImagePosition::ImageOnly);
            }
            btn.setTitle(&NSString::from_str(""));
            unsafe {
                btn.setTarget(Some(&target));
                btn.setAction(Some(sel!(onStatusClick:)));
            }
        }

        Some(Self {
            item,
            _target: target,
            sprites,
            current_kind: SpriteKind::Idle,
            frame_interval_ms: 600,
            frame_idx: 0,
            last_frame_advance: Instant::now(),
            compact_title: String::new(),
            text_only_title: "CoCo 待機".to_string(),
            last_applied_mode: MenuBarMode::IconAndCounts,
            last_applied_title: String::new(),
            auto_hidden_by_coco: false,
            manually_hidden: false,
        })
    }

    pub fn reload_icons_if_changed(&mut self) {
        let dir = icons_directory();
        let stamp = compute_dir_stamp(&dir);
        if stamp != self.sprites.dir_stamp {
            self.sprites = SpriteBank::load();
            self.frame_idx = 0;
        }
    }

    pub fn current_interval_ms(&self) -> u64 {
        self.frame_interval_ms
    }

    pub fn update_from_snapshot(&mut self, snap: &MonitorSnapshot, mode: MenuBarMode) {
        let waiting = snap.waiting_count;
        let running = snap.running_count;
        let stalled = snap.stalled_count;
        let unseen_done = snap.unseen_completed_count;
        let error_count = snap
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Error && j.unseen_done)
            .count();
        let has_subagents = snap.jobs.iter().any(|j| {
            j.status == JobStatus::Running
                && (matches!(j.phase, RunPhase::SubagentsRunning(_))
                    || j.subagents.iter().any(|s| s.active))
        });

        // Priority & dynamic FPS calculation matching specification
        let (kind, interval_ms, compact) = if waiting > 0 {
            let label = if running > 0 {
                format!("待ち{waiting}·稼働{running}")
            } else {
                format!("待ち{waiting}")
            };
            (SpriteKind::Wait, 220, label)
        } else if (error_count > 0 || stalled > 0) && running == 0 {
            let label = if error_count > 0 {
                format!("異常{error_count}")
            } else {
                format!("停滞{stalled}")
            };
            (SpriteKind::Trouble, 280, label)
        } else if running >= 3 || (running > 0 && has_subagents) {
            (SpriteKind::Run, 55, format!("稼働{running}"))
        } else if running == 2 {
            (SpriteKind::Run, 85, "稼働2".to_string())
        } else if running == 1 {
            (SpriteKind::Run, 130, "稼働1".to_string())
        } else if unseen_done > 0 {
            (SpriteKind::Done, 320, format!("完了{unseen_done}"))
        } else {
            (SpriteKind::Idle, 600, String::new())
        };

        let text_only = if waiting > 0 {
            format!("CoCo 待ち{waiting} · 稼働{running}")
        } else if running > 0 {
            format!("CoCo 稼働{running}")
        } else if error_count > 0 {
            format!("CoCo 異常{error_count}")
        } else if stalled > 0 {
            format!("CoCo 停滞{stalled}")
        } else if unseen_done > 0 {
            format!("CoCo 完了{unseen_done}")
        } else {
            "CoCo 待機".to_string()
        };

        let kind_changed = kind != self.current_kind;
        self.current_kind = kind;
        self.frame_interval_ms = interval_ms;
        self.compact_title = compact;
        self.text_only_title = text_only;

        if kind_changed {
            self.frame_idx = 0;
            self.last_frame_advance = Instant::now();
        }
        self.render_button(mode, kind_changed);
    }

    pub fn tick_animation(&mut self, mode: MenuBarMode) {
        if mode == MenuBarMode::TextOnly {
            if self.last_applied_mode != mode {
                self.render_button(mode, true);
            }
            return;
        }
        let elapsed_ms = self.last_frame_advance.elapsed().as_millis() as u64;
        let mode_changed = mode != self.last_applied_mode;
        if elapsed_ms >= self.frame_interval_ms || mode_changed {
            let count = self.sprites.frames_for(self.current_kind).len();
            if count > 1 && elapsed_ms >= self.frame_interval_ms {
                self.frame_idx = (self.frame_idx + 1) % count;
            }
            self.last_frame_advance = Instant::now();
            self.render_button(mode, true);
        }
    }

    fn render_button(&mut self, mode: MenuBarMode, force_image: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(btn) = self.item.button(mtm) else {
            return;
        };

        match mode {
            MenuBarMode::TextOnly => {
                if self.last_applied_mode != MenuBarMode::TextOnly {
                    btn.setImage(None);
                    btn.setImagePosition(NSCellImagePosition::NoImage);
                }
                if self.last_applied_title != self.text_only_title
                    || self.last_applied_mode != mode
                {
                    btn.setTitle(&NSString::from_str(&self.text_only_title));
                    self.last_applied_title = self.text_only_title.clone();
                }
            }
            MenuBarMode::IconOnly => {
                if force_image || self.last_applied_mode != mode {
                    let frames = self.sprites.frames_for(self.current_kind);
                    if let Some(img) = frames.get(self.frame_idx % frames.len().max(1)) {
                        btn.setImage(Some(img));
                    }
                    btn.setImagePosition(NSCellImagePosition::ImageOnly);
                }
                if !self.last_applied_title.is_empty() || self.last_applied_mode != mode {
                    btn.setTitle(&NSString::from_str(""));
                    self.last_applied_title.clear();
                }
            }
            MenuBarMode::IconAndCounts => {
                if force_image || self.last_applied_mode != mode {
                    let frames = self.sprites.frames_for(self.current_kind);
                    if let Some(img) = frames.get(self.frame_idx % frames.len().max(1)) {
                        btn.setImage(Some(img));
                    }
                }
                let desired_pos = if self.compact_title.is_empty() {
                    NSCellImagePosition::ImageOnly
                } else {
                    NSCellImagePosition::ImageLeft
                };
                btn.setImagePosition(desired_pos);
                if self.last_applied_title != self.compact_title || self.last_applied_mode != mode {
                    btn.setTitle(&NSString::from_str(&self.compact_title));
                    self.last_applied_title = self.compact_title.clone();
                }
            }
        }

        self.last_applied_mode = mode;
    }

    pub fn toggle_panel_visibility(&mut self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(win) = find_panel_nswindow(mtm) else {
            return;
        };
        if win.isVisible() && !self.manually_hidden {
            self.manually_hidden = true;
            win.orderOut(None);
        } else {
            self.manually_hidden = false;
            self.auto_hidden_by_coco = false;
            win.orderFrontRegardless();
        }
    }

    pub fn show_panel(&mut self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if let Some(win) = find_panel_nswindow(mtm) {
            self.manually_hidden = false;
            self.auto_hidden_by_coco = false;
            win.orderFrontRegardless();
        }
    }

    pub fn sync_with_coco_window(&mut self, prefs: &SharedWindowPrefs) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(win) = find_panel_nswindow(mtm) else {
            return;
        };

        let pinned = prefs.pinned_on_top.load(Ordering::Relaxed);
        let desired_level: isize = if pinned { 3 } else { 0 };
        win.setLevel(desired_level);

        if self.manually_hidden {
            return;
        }

        let dock = prefs.current_dock();
        if dock == DockMode::Manual {
            if self.auto_hidden_by_coco {
                self.auto_hidden_by_coco = false;
                win.orderFrontRegardless();
            }
            return;
        }

        let Some((screen_w, screen_h)) = primary_screen_size(mtm) else {
            return;
        };

        match query_coco_window_bounds() {
            Some(coco) => {
                if self.auto_hidden_by_coco {
                    self.auto_hidden_by_coco = false;
                    win.orderFrontRegardless();
                }

                let frame = win.frame();
                let pw = frame.size.width;
                let ph = frame.size.height;

                let cg_x = if coco.x + coco.width + pw + 10.0 <= screen_w {
                    coco.x + coco.width + 8.0
                } else {
                    (coco.x + coco.width - pw - 14.0).max(8.0)
                };

                let cg_y = match dock {
                    DockMode::TopRight => coco.y + 44.0,
                    DockMode::BottomRight => (coco.y + coco.height - ph - 18.0).max(coco.y + 44.0),
                    DockMode::Manual => unreachable!(),
                };

                let ns_x = cg_x;
                let ns_y = screen_h - (cg_y + ph);

                if (frame.origin.x - ns_x).abs() > 1.5 || (frame.origin.y - ns_y).abs() > 1.5 {
                    win.setFrameOrigin(NSPoint::new(ns_x, ns_y));
                }
            }
            None => {
                if win.isVisible() {
                    self.auto_hidden_by_coco = true;
                    win.orderOut(None);
                }
            }
        }
    }
}

pub fn apply_panel_level(pinned: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(win) = find_panel_nswindow(mtm) {
        win.setLevel(if pinned { 3 } else { 0 });
    }
}

fn find_panel_nswindow(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    for i in 0..windows.count() {
        let w = windows.objectAtIndex(i);
        if w.title().to_string() == PANEL_TITLE {
            return Some(w);
        }
    }
    None
}

fn primary_screen_size(mtm: MainThreadMarker) -> Option<(f64, f64)> {
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return None;
    }
    let s = screens.objectAtIndex(0);
    let f = s.frame();
    Some((f.size.width, f.size.height))
}

pub fn query_coco_window_bounds() -> Option<CgRect> {
    let opts = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let arr = copy_window_info(opts, kCGNullWindowID)?;

    let owner_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let x_key = CFString::from_static_string("X");
    let y_key = CFString::from_static_string("Y");
    let w_key = CFString::from_static_string("Width");
    let h_key = CFString::from_static_string("Height");

    let mut best: Option<CgRect> = None;

    for raw_dict in arr.iter() {
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule((*raw_dict) as _) };

        let Some(owner_val) = dict.find(&owner_key) else {
            continue;
        };
        let Some(owner_str) = owner_val.downcast::<CFString>() else {
            continue;
        };
        let owner = owner_str.to_string();
        if owner != "Cortex Code" {
            continue;
        }

        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(-1);
        if layer != 0 {
            continue;
        }

        let Some(bounds_val) = dict.find(&bounds_key) else {
            continue;
        };
        let bounds_dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(bounds_val.as_CFTypeRef() as _) };

        let x = cf_dict_f64(&bounds_dict, &x_key)?;
        let y = cf_dict_f64(&bounds_dict, &y_key)?;
        let width = cf_dict_f64(&bounds_dict, &w_key)?;
        let height = cf_dict_f64(&bounds_dict, &h_key)?;

        if width < 240.0 || height < 200.0 {
            continue;
        }

        if best
            .map(|b| width * height > b.width * b.height)
            .unwrap_or(true)
        {
            best = Some(CgRect {
                x,
                y,
                width,
                height,
            });
        }
    }

    best
}

fn cf_dict_f64(dict: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<f64> {
    dict.find(key)
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_f64())
}

pub fn is_coco_frontmost() -> bool {
    let opts = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(arr) = copy_window_info(opts, kCGNullWindowID) else {
        return false;
    };
    let owner_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };

    for raw_dict in arr.iter() {
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule((*raw_dict) as _) };
        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(-1);
        if layer != 0 {
            continue;
        }
        let Some(owner_val) = dict.find(&owner_key) else {
            continue;
        };
        let Some(owner_str) = owner_val.downcast::<CFString>() else {
            continue;
        };
        return owner_str.to_string() == "Cortex Code";
    }
    false
}

#[derive(Debug, Clone, Copy)]
pub enum AlertSound {
    Done,
    Attention,
    Error,
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioServicesCreateSystemSoundID(
        in_file_url: core_foundation::url::CFURLRef,
        out_system_sound_id: *mut u32,
    ) -> i32;
    fn AudioServicesPlayAlertSound(in_system_sound_id: u32);
}

struct CachedAlertSounds {
    done_id: u32,
    attention_id: u32,
    error_id: u32,
}

static ALERT_SOUNDS: std::sync::OnceLock<CachedAlertSounds> = std::sync::OnceLock::new();

fn load_system_sound(path: &str) -> u32 {
    use core_foundation::base::TCFType;
    use core_foundation::url::CFURL;
    let Some(url) = CFURL::from_path(std::path::Path::new(path), false) else {
        return 0x00001000;
    };
    let mut sid: u32 = 0;
    let status = unsafe { AudioServicesCreateSystemSoundID(url.as_concrete_TypeRef(), &mut sid) };
    if status == 0 && sid != 0 {
        sid
    } else {
        0x00001000
    }
}

pub fn play_alert_sound(kind: AlertSound) {
    let sounds = ALERT_SOUNDS.get_or_init(|| CachedAlertSounds {
        done_id: load_system_sound("/System/Library/Sounds/Glass.aiff"),
        attention_id: load_system_sound("/System/Library/Sounds/Ping.aiff"),
        error_id: load_system_sound("/System/Library/Sounds/Sosumi.aiff"),
    });
    let id = match kind {
        AlertSound::Done => sounds.done_id,
        AlertSound::Attention => sounds.attention_id,
        AlertSound::Error => sounds.error_id,
    };
    unsafe {
        AudioServicesPlayAlertSound(id);
    }
}
