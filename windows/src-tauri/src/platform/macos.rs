// macOS: AppKit for the island window, CoreGraphics for the cursor, ~/Library
// for files.
//
// The island works as on Windows: a borderless always-on-top window at the top
// of the screen, a 60 Hz cursor poll, and click-through flipped from it
// (`set_ignore_cursor_events`, which AppKit calls `ignoresMouseEvents`). The
// window sits above the menu bar, so on a MacBook it covers the notch.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::WebviewWindow;

use super::{home_dir, LocalTime};

/// File name of the hook relay.
pub const HOOK_EXE: &str = crate::ident::HOOK_EXE;

/// Environment variable holding the home directory.
pub const HOME_VAR: &str = "HOME";

// ── Files ─────────────────────────────────────────────────────────────────────

/// ~/Library/Application Support/Mayak — preferences and the sounds folder.
pub fn config_dir() -> PathBuf {
    home_dir().join("Library/Application Support").join(crate::ident::APP_DIR)
}

/// ~/.local/share/mayak — where mayak-hook, the inbox and the log live. Not
/// under Application Support: the relay's path goes into the Codex hook command
/// without quotes, so it must not contain a space.
pub fn local_dir() -> PathBuf {
    home_dir().join(".local/share").join(crate::ident::APP_SLUG)
}

/// A Finder-launched app gets launchd's PATH (`/usr/bin:/bin:/usr/sbin:/sbin`),
/// which has none of the places `claude`, `codex`, `code` or `cursor` are
/// installed to. Add them, after what is already there, before any thread or
/// process starts.
pub fn prepare_environment() {
    let home = home_dir();
    let extra = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".cargo/bin"),
        PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin"),
        PathBuf::from("/Applications/Cursor.app/Contents/Resources/app/bin"),
    ];
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for dir in extra {
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    if let Ok(joined) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", joined);
    }
}

pub fn local_time() -> LocalTime {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        libc::localtime_r(&now, &mut tm);
    }
    LocalTime {
        year: (tm.tm_year + 1900) as u32,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    }
}

/// Creates `dir` and closes it to other users: the log, the inbox of dropped
/// files and the relay binary live under these directories.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// The per-user temporary folder, `/var/folders/…/T/`, from the system rather
/// than `$TMPDIR`, so the app and a hook started from any environment agree.
fn user_temp_dir() -> Option<PathBuf> {
    let mut buf = vec![0u8; 1024];
    let len = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr().cast(), buf.len()) };
    if len == 0 || len > buf.len() {
        return None;
    }
    let text = std::ffi::CStr::from_bytes_until_nul(&buf).ok()?.to_str().ok()?;
    Some(PathBuf::from(text))
}

/// True when `dir` is a real directory (not a symlink), owned by us, with no
/// access for group or others: the socket in it decides who can answer a
/// permission request, so this is checked rather than assumed.
fn is_private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(dir)
        .map(|m| m.file_type().is_dir() && m.uid() == unsafe { libc::getuid() } && m.mode() & 0o077 == 0)
        .unwrap_or(false)
}

/// Where mayak-hook finds us: `<per-user temp folder>/mayak.sock`. A folder that
/// is not ours and private means no relay at all — never a fallback to a shared
/// place like /tmp. Must match `pipe_path()` in hook/src/mac.rs exactly.
pub fn relay_socket_path() -> Option<PathBuf> {
    user_temp_dir().filter(|d| is_private_dir(d)).map(|d| d.join("mayak.sock"))
}

// ── Processes ─────────────────────────────────────────────────────────────────

/// Nothing to hide: a spawned process only gets a terminal if it asks for one.
pub fn no_console(cmd: &mut Command) -> &mut Command {
    cmd
}

pub fn open_url(url: &str) {
    let _ = Command::new("/usr/bin/open").arg(url).spawn();
}

pub fn reveal_folder(path: &str) {
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
}

/// The CLI an app ships for itself, for a user who never installed one: `claude` inside Claude
/// Desktop (the newest version it keeps), `codex` inside ChatGPT or Codex. None for any other name.
pub fn app_cli(stem: &str) -> Option<PathBuf> {
    match stem {
        "claude" => {
            let root = home_dir().join("Library/Application Support/Claude/claude-code");
            let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
            for version in std::fs::read_dir(root).ok()?.flatten() {
                let Ok(builds) = std::fs::read_dir(version.path()) else { continue };
                for build in builds.flatten() {
                    let exe = build.path().join("claude.app/Contents/MacOS/claude");
                    if let Ok(modified) = std::fs::metadata(&exe).and_then(|m| m.modified()) {
                        found.push((modified, exe));
                    }
                }
            }
            found.into_iter().max_by_key(|(at, _)| *at).map(|(_, exe)| exe)
        }
        "codex" => {
            let home = home_dir();
            ["/Applications".into(), home.join("Applications")]
                .iter()
                .flat_map(|dir: &PathBuf| ["ChatGPT.app", "Codex.app"].map(|app| dir.join(app).join("Contents/Resources/codex-cli/bin/codex")))
                .find(|exe| exe.is_file())
        }
        _ => None,
    }
}

/// Our own `which`: the first executable file named `stem` on $PATH.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    let dirs = std::env::var_os("PATH")?;
    std::env::split_paths(&dirs)
        .map(|dir| dir.join(stem))
        .find(|p| {
            std::fs::metadata(p)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

/// Puts `text` on the general pasteboard through `pbcopy`.
pub fn set_clipboard_text(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = Command::new("/usr/bin/pbcopy").stdin(std::process::Stdio::piped()).spawn()?;
    child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
    let status = child.wait()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("pbcopy: {status}"))) }
}

// ── Process tree ──────────────────────────────────────────────────────────────

/// Parent and start time (µs since the epoch) of `pid`, from libproc.
fn proc_parent(pid: u32) -> Option<(u32, u64)> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size)
    };
    (got == size).then(|| (info.pbi_ppid, info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec))
}

/// Full path of the image of `pid`.
pub fn image_path(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return None;
    }
    buf.truncate(len as usize);
    String::from_utf8(buf).ok()
}

/// The ancestors of `pid`, nearest first, with their image paths: at most `max`, never past
/// launchd, and never to a "parent" younger than its child (a reused pid ends the walk).
pub fn ancestors(pid: u32, max: usize) -> Vec<(u32, String)> {
    let mut chain = Vec::new();
    let Some((mut parent, mut child_start)) = proc_parent(pid) else { return chain };
    while chain.len() < max && parent > 1 {
        let Some((grand, start)) = proc_parent(parent) else { break };
        if start > child_start {
            break;
        }
        let Some(image) = image_path(parent) else { break };
        chain.push((parent, image));
        child_start = start;
        parent = grand;
    }
    chain
}

/// The terminal device of `pid` (`/dev/ttys003`): what Terminal and iTerm2 name a tab by. None
/// for a process without one.
pub fn tty_of(pid: u32) -> Option<String> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size)
    };
    // NODEV (all ones) when the process has no controlling terminal.
    if got != size || info.e_tdev == u32::MAX {
        return None;
    }
    let name = unsafe { libc::devname(info.e_tdev as libc::dev_t, libc::S_IFCHR) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().ok()?;
    (name != "??").then(|| format!("/dev/{name}"))
}

/// Every pid we can see.
pub fn all_pids() -> Vec<u32> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(got.max(0) as usize);
    pids.into_iter().filter(|p| *p > 0).map(|p| p as u32).collect()
}

/// The outermost app bundle an image lives in, and its name: `/Applications/Cursor.app/Contents/
/// Frameworks/Cursor Helper (Plugin).app/…` → (`/Applications/Cursor.app`, `Cursor`). None for an
/// image outside any bundle (a shell, a CLI).
pub fn app_bundle(image: &str) -> Option<(PathBuf, String)> {
    let end = image.find(".app/")? + ".app".len();
    let bundle = &image[..end];
    let name = Path::new(bundle).file_stem()?.to_str()?.to_string();
    Some((PathBuf::from(bundle), name))
}

/// Brings the app of `bundle` to the front through LaunchServices (`open`), the way the Dock
/// does. Unlike `-[NSRunningApplication activate…]`, which macOS 14 ignores from an app that is
/// not active itself, this always reaches the app.
pub fn bring_app_forward(bundle: &Path) -> bool {
    Command::new("/usr/bin/open").arg(bundle).status().is_ok_and(|s| s.success())
}

// ── Cursor ────────────────────────────────────────────────────────────────────

/// The 60 Hz poll reads the cursor and flips click-through from it, as on Windows.
pub const CURSOR_POLL: bool = true;

/// The few CoreGraphics calls the poll needs. All of them are safe off the main
/// thread and need no permission.
mod cg {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }

    pub const COMBINED_SESSION_STATE: i32 = 0;
    pub const MOUSE_BUTTON_LEFT: u32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventCreate(source: *const c_void) -> *mut c_void;
        pub fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        pub fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
        pub fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
        pub fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        pub fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
        pub fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
        pub fn CGDisplayModeRelease(mode: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(cf: *const c_void);
    }
}

/// Cursor position in the global top-left coordinate space, in points.
fn cursor_points() -> Option<cg::CGPoint> {
    unsafe {
        let event = cg::CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let p = cg::CGEventGetLocation(event);
        cg::CFRelease(event);
        Some(p)
    }
}

/// Backing scale of the display under `p` (2 on a Retina screen).
fn scale_at(p: cg::CGPoint) -> f64 {
    unsafe {
        let mut display = 0u32;
        let mut count = 0u32;
        if cg::CGGetDisplaysWithPoint(p, 1, &mut display, &mut count) != 0 || count == 0 {
            return 1.0;
        }
        // The mode knows both: its width in points and in backing pixels.
        let mode = cg::CGDisplayCopyDisplayMode(display);
        if mode.is_null() {
            return 1.0;
        }
        let points = cg::CGDisplayModeGetWidth(mode) as f64;
        let pixels = cg::CGDisplayModeGetPixelWidth(mode) as f64;
        cg::CGDisplayModeRelease(mode);
        if points > 0.0 && pixels > 0.0 { pixels / points } else { 1.0 }
    }
}

/// Cursor position in physical pixels, the unit tao reports window and monitor
/// positions in on macOS (points × the display's scale).
pub fn cursor_physical() -> Option<(f64, f64)> {
    let p = cursor_points()?;
    let scale = scale_at(p);
    Some((p.x * scale, p.y * scale))
}

/// True while the left mouse button is held — the only signal we get that a
/// drag might be in flight before it reaches the window.
pub fn left_button_down() -> bool {
    unsafe { cg::CGEventSourceButtonState(cg::COMBINED_SESSION_STATE, cg::MOUSE_BUTTON_LEFT) }
}

/// The island window as the cursor poll reads it every tick: here through
/// tauri's own getters.
pub struct WindowProbe(WebviewWindow);

impl WindowProbe {
    pub fn new(win: &WebviewWindow) -> Option<Self> {
        Some(Self(win.clone()))
    }

    /// Outer position (physical px), inner size (physical px) and scale factor.
    /// None once the window is gone, so the poll asks tauri for it again.
    pub fn geometry(&self) -> Option<((i32, i32), (u32, u32), f64)> {
        let pos = self.0.outer_position().ok()?;
        let size = self.0.inner_size().ok()?;
        let scale = self.0.scale_factor().ok()?;
        Some(((pos.x, pos.y), (size.width, size.height), scale))
    }
}

// ── Island window ─────────────────────────────────────────────────────────────

/// `NSMainMenuWindowLevel + 3`, as the Mac app's panel: above the menu bar and its status
/// items (level 25, which would take the clicks in the menu bar band), so the island in the
/// notch gets them.
const ISLAND_LEVEL: isize = 24 + 3;

/// The Objective-C runtime calls that turn the island's window into a panel.
mod objc_rt {
    use std::ffi::{c_char, c_void};

    #[link(name = "objc")]
    extern "C" {
        pub fn objc_getClass(name: *const c_char) -> *const c_void;
        pub fn objc_allocateClassPair(superclass: *const c_void, name: *const c_char, extra: usize) -> *mut c_void;
        pub fn objc_registerClassPair(cls: *mut c_void);
        pub fn object_setClass(obj: *const c_void, cls: *const c_void) -> *const c_void;
        pub fn sel_registerName(name: *const c_char) -> *const c_void;
        pub fn class_getMethodImplementation(cls: *const c_void, name: *const c_void) -> *const c_void;
        pub fn class_createInstance(cls: *const c_void, extra: usize) -> *mut c_void;
        pub fn class_addMethod(cls: *mut c_void, name: *const c_void, imp: *const c_void, types: *const c_char) -> bool;
    }
}

/// Whether the island may take the keyboard now (`set_activating`): a text field in it is in use.
static TAKES_KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `-[NSWindow constrainFrameRect:toScreen:]` that changes nothing. AppKit calls it whenever a
/// window moves or the screen changes and pushes every window below the menu bar; the island
/// has to stay over it, in the notch.
extern "C" fn keep_frame(
    _this: *const std::ffi::c_void,
    _sel: *const std::ffi::c_void,
    rect: objc2_foundation::NSRect,
    _screen: *const std::ffi::c_void,
) -> objc2_foundation::NSRect {
    rect
}

extern "C" fn can_become_key(_this: *const std::ffi::c_void, _sel: *const std::ffi::c_void) -> bool {
    TAKES_KEYBOARD.load(std::sync::atomic::Ordering::Relaxed)
}

/// NSPanel's own `setLevel:`, called by `keep_level`.
static SUPER_SET_LEVEL: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// `setLevel:` that never goes under the island's level: tauri's `set_always_on_top` sets the
/// floating level on every placement, and under the menu bar the menu bar takes the clicks.
extern "C" fn keep_level(this: *const std::ffi::c_void, sel: *const std::ffi::c_void, level: isize) {
    let Some(&imp) = SUPER_SET_LEVEL.get() else { return };
    // SAFETY: the IMP of -[NSPanel setLevel:], which has exactly this signature.
    let set: extern "C" fn(*const std::ffi::c_void, *const std::ffi::c_void, isize) = unsafe { std::mem::transmute(imp) };
    set(this, sel, level.max(ISLAND_LEVEL));
}

extern "C" fn never(_this: *const std::ffi::c_void, _sel: *const std::ffi::c_void) -> bool {
    false
}

/// `MayakIslandPanel`, an NSPanel subclass made once at run time, as the Mac app's `IslandPanel`:
/// keeps its frame over the menu bar, becomes key only while a text field needs the keyboard,
/// never becomes main.
fn panel_class() -> *const std::ffi::c_void {
    static CLASS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let superclass = objc_rt::objc_getClass(c"NSPanel".as_ptr());
        let cls = objc_rt::objc_allocateClassPair(superclass, c"MayakIslandPanel".as_ptr(), 0);
        let add = |sel: &std::ffi::CStr, imp: *const std::ffi::c_void, types: &std::ffi::CStr| {
            objc_rt::class_addMethod(cls, objc_rt::sel_registerName(sel.as_ptr()), imp, types.as_ptr());
        };
        add(
            c"constrainFrameRect:toScreen:",
            keep_frame as *const std::ffi::c_void,
            c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@",
        );
        add(c"canBecomeKeyWindow", can_become_key as *const std::ffi::c_void, c"B@:");
        add(c"canBecomeMainWindow", never as *const std::ffi::c_void, c"B@:");
        let set_level = objc_rt::sel_registerName(c"setLevel:".as_ptr());
        let _ = SUPER_SET_LEVEL.set(objc_rt::class_getMethodImplementation(superclass, set_level) as usize);
        add(c"setLevel:", keep_level as *const std::ffi::c_void, c"v@:q");
        objc_rt::objc_registerClassPair(cls);
        cls as usize
    }) as *const std::ffi::c_void
}

/// Makes the island's window a non-activating panel, the way the Mac app's island is one. A
/// click on an ordinary window of an inactive app activates the app first, and in the menu bar
/// band that click never reaches the page; a non-activating panel takes it without activating
/// anything, so the notch and the island's header can be clicked.
fn become_panel(ns: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::NSWindowStyleMask;
    unsafe { objc_rt::object_setClass((ns as *const objc2_app_kit::NSWindow).cast(), panel_class()) };
    ns.setStyleMask(ns.styleMask() | NSWindowStyleMask::NonactivatingPanel);
}

/// Raises the island above the menu bar, on every Space and over full-screen
/// apps, and keeps it out of Cmd-Tab and Mission Control. The island never
/// becomes the key window until `set_activating(true)`.
pub fn make_non_activating(win: &WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let Ok(ptr) = win.ns_window() else { return };
    let ptr = ptr as usize;
    on_main(win, move || {
        // SAFETY: tauri hands out the NSWindow it owns; we are on the main thread.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        become_panel(ns);
        ns.setLevel(ISLAND_LEVEL);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        ns.setHidesOnDeactivate(false);
        ns.setHasShadow(false);
        measure_notch(ns);
    });
}

/// Runs `f` right away when already on the main thread (tauri's `setup`), else queues it there.
fn on_main(win: &WebviewWindow, f: impl FnOnce() + Send + 'static) {
    if objc2::MainThreadMarker::new().is_some() {
        f();
    } else {
        let _ = win.run_on_main_thread(f);
    }
}

/// Size of the camera housing of the island's screen, in points: (width, height). None on a
/// screen without one (an external display, a Mac without a notch).
static NOTCH: std::sync::Mutex<Option<(f64, f64)>> = std::sync::Mutex::new(None);

/// The notch as the island has to cover it, measured on the island window's screen.
pub fn notch() -> Option<(f64, f64)> {
    *NOTCH.lock().unwrap()
}

/// macOS 12 describes the notch by the safe area at the top and the two strips of menu bar
/// left and right of it; the notch is what the two strips leave of the screen's width.
fn measure_notch(ns: &objc2_app_kit::NSWindow) {
    let found = ns.screen().and_then(|screen| {
        let top = screen.safeAreaInsets().top;
        let left = screen.auxiliaryTopLeftArea().size.width;
        let right = screen.auxiliaryTopRightArea().size.width;
        let width = screen.frame().size.width - left - right;
        (top > 0.0 && left > 0.0 && right > 0.0 && width > 0.0).then_some((width, top))
    });
    let mut notch = NOTCH.lock().unwrap();
    if *notch != found {
        crate::log::line(format!("notch: {found:?}"));
    }
    *notch = found;
}

/// What `watch_pointer_entry` calls, on the main thread.
static POINTER_ENTERED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

extern "C" fn pointer_entered(_this: *const std::ffi::c_void, _sel: *const std::ffi::c_void, _event: *const std::ffi::c_void) {
    if let Some(callback) = POINTER_ENTERED.get() {
        callback();
    }
}

extern "C" fn pointer_left(_this: *const std::ffi::c_void, _sel: *const std::ffi::c_void, _event: *const std::ffi::c_void) {}

/// Calls `on_enter` whenever the pointer enters the island's window, from AppKit's own tracking:
/// an `NSTrackingArea` over the whole content view (it follows its size), active even while Mayak
/// is not the active app. No polling: nothing runs until the pointer crosses the edge. Installed
/// once.
pub fn watch_pointer_entry(win: &WebviewWindow, on_enter: impl Fn() + Send + Sync + 'static) {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions, NSWindow};

    if POINTER_ENTERED.set(Box::new(on_enter)).is_err() {
        return;
    }
    let Ok(ptr) = win.ns_window() else { return };
    let ptr = ptr as usize;
    on_main(win, move || unsafe {
        let superclass = objc_rt::objc_getClass(c"NSObject".as_ptr());
        let cls = objc_rt::objc_allocateClassPair(superclass, c"MayakPointerWatch".as_ptr(), 0);
        if cls.is_null() {
            return;
        }
        objc_rt::class_addMethod(
            cls,
            objc_rt::sel_registerName(c"mouseEntered:".as_ptr()),
            pointer_entered as *const std::ffi::c_void,
            c"v@:@".as_ptr(),
        );
        // The area reports the exit too, and an owner without the method would raise
        // "unrecognized selector" out of AppKit's event dispatch.
        objc_rt::class_addMethod(
            cls,
            objc_rt::sel_registerName(c"mouseExited:".as_ptr()),
            pointer_left as *const std::ffi::c_void,
            c"v@:@".as_ptr(),
        );
        objc_rt::objc_registerClassPair(cls);
        // Kept for the life of the app: a tracking area does not retain its owner.
        let owner = objc_rt::class_createInstance(cls, 0) as *const objc2::runtime::AnyObject;
        // SAFETY: tauri hands out the NSWindow it owns; we are on the main thread.
        let ns: &NSWindow = &*(ptr as *const NSWindow);
        let Some(view) = ns.contentView() else { return };
        let area = NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            view.bounds(),
            NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect,
            Some(&*owner),
            None,
        );
        view.addTrackingArea(&area);
    });
}

/// Puts the top edge of the island on the top edge of its screen, over the menu bar and the
/// notch. AppKit keeps ordinary windows below the menu bar and moves ours there after tauri
/// placed it; setting the frame ourselves, after tauri, keeps the x and size and only lifts it.
pub fn pin_to_top(win: &WebviewWindow) {
    use objc2_app_kit::NSWindow;

    let Ok(ptr) = win.ns_window() else { return };
    let ptr = ptr as usize;
    let _ = win.run_on_main_thread(move || {
        // SAFETY: tauri hands out the NSWindow it owns; we are on the main thread.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        let Some(screen) = ns.screen() else { return };
        let top = screen.frame().origin.y + screen.frame().size.height;
        let mut frame = ns.frame();
        frame.origin.y = top - frame.size.height;
        ns.setFrame_display(frame, true);
        measure_notch(ns);
    });
}

/// Temporarily allow keyboard focus so a text field inside the island can be
/// typed in.
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    TAKES_KEYBOARD.store(activating, std::sync::atomic::Ordering::Relaxed);
    if activating {
        let _ = win.set_focus();
    }
}

/// The pid of the app in front, as AppKit says.
fn frontmost_pid() -> Option<i32> {
    objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier())
}

/// The app that has the keyboard now, unless it is Mayak itself: where the hotkey was pressed,
/// captured before the island takes the keyboard. Its pid stands for the "window" of Windows.
pub fn foreground_window(_island: &WebviewWindow) -> Option<isize> {
    let pid = frontmost_pid()?;
    (pid > 0 && pid as u32 != std::process::id()).then_some(pid as isize)
}

/// Gives the keyboard back to the app `target` as the island folds, when should_restore_focus
/// allows it: Mayak still in front, that app still running, nothing opened from the island.
pub fn restore_foreground(_island: &WebviewWindow, target: isize, opened_something: bool) {
    let held = frontmost_pid().is_some_and(|pid| pid as u32 == std::process::id());
    let bundle = image_path(target as u32).and_then(|image| app_bundle(&image));
    if super::should_restore_focus(held, bundle.is_some(), opened_something) {
        if let Some((bundle, _)) = bundle {
            bring_app_forward(&bundle);
        }
    }
}

/// Click-through here is the poll's `ignoresMouseEvents` toggle, not a region.
pub fn set_input_region(_win: &WebviewWindow, _rect: Option<(f64, f64, f64, f64)>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_dirs_are_closed_to_everyone_else() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("mayak-priv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundles_are_the_outermost_app_of_an_image() {
        assert_eq!(
            app_bundle("/Applications/Cursor.app/Contents/Frameworks/Cursor Helper (Plugin).app/Contents/MacOS/Cursor Helper (Plugin)"),
            Some((PathBuf::from("/Applications/Cursor.app"), "Cursor".to_string()))
        );
        assert_eq!(app_bundle("/bin/zsh"), None);
    }

    #[test]
    fn the_ancestors_of_this_test_lead_to_its_parent() {
        let chain = ancestors(std::process::id(), 8);
        assert!(!chain.is_empty());
        assert!(chain.iter().all(|(pid, image)| *pid > 1 && image.starts_with('/')));
        assert!(all_pids().contains(&std::process::id()));
    }

    #[test]
    fn the_relay_folder_has_no_space_in_it() {
        assert!(!local_dir().to_string_lossy().contains(' '));
    }
}
