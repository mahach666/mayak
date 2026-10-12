//! macOS side of the Apps tab's own apps: the Open dialog of the "+" tile and an app's icon, the
//! counterparts of `apps_win.rs`.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSImage, NSWorkspace};
use objc2_foundation::NSString;

/// The AppleScript of the "+" tile: the Open dialog in /Applications, apps only, its prompt the
/// first argument (never part of the script's text). It answers the picked app's POSIX path.
const PICK_SCRIPT: [&str; 3] = [
    "on run argv",
    "POSIX path of (choose file with prompt (item 1 of argv) of type {\"com.apple.application-bundle\"} default location (path to applications folder))",
    "end run",
];

/// The Open dialog, shown by `osascript` in a process of its own: a modal panel run from inside
/// tauri's event loop would re-enter it (tao aborts), and would hold the main thread the island
/// needs to let the clicks through to the dialog. `Ok(None)` when cancelled.
pub fn pick_app(prompt: &str) -> std::io::Result<Option<PathBuf>> {
    let mut cmd = std::process::Command::new("/usr/bin/osascript");
    for line in PICK_SCRIPT {
        cmd.args(["-e", line]);
    }
    let out = cmd.arg(prompt).output()?;
    if !out.status.success() {
        // -128 is "User canceled"; anything else is a failure, said without the path.
        let err = String::from_utf8_lossy(&out.stderr);
        return if err.contains("-128") { Ok(None) } else { Err(std::io::Error::other("choose file failed")) };
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().trim_end_matches('/').to_string();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(ctx: *mut c_void, rect: objc2_foundation::NSRect, image: *mut c_void);
    fn CGContextRelease(ctx: *mut c_void);
}

/// kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big: R, G, B, A bytes in that order.
const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);

/// The app's icon as Finder shows it, `size`×`size`, straight RGBA rows top to bottom.
pub fn icon_rgba(app: &Path, size: u32) -> Option<Vec<u8>> {
    let image: Retained<NSImage> = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(app.to_str()?));
    let side = size as usize;
    let rect = objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(0.0, 0.0),
        objc2_foundation::NSSize::new(size as f64, size as f64),
    );
    let mut proposed = rect;
    // SAFETY: -[NSImage CGImageForProposedRect:context:hints:] with no context or hints; the
    // CGImage it returns is owned by the image, which outlives its use here.
    let cg: *mut c_void = unsafe {
        msg_send![&*image, CGImageForProposedRect: &mut proposed, context: std::ptr::null::<AnyObject>(), hints: std::ptr::null::<AnyObject>()]
    };
    if cg.is_null() {
        return None;
    }
    let mut pixels = vec![0u8; side * side * 4];
    unsafe {
        let space = CGColorSpaceCreateDeviceRGB();
        let ctx = CGBitmapContextCreate(pixels.as_mut_ptr().cast(), side, side, 8, side * 4, space, RGBA_PREMULTIPLIED);
        CGColorSpaceRelease(space);
        if ctx.is_null() {
            return None;
        }
        CGContextDrawImage(ctx, rect, cg);
        CGContextRelease(ctx);
    }
    unpremultiply(&mut pixels);
    Some(pixels)
}

/// CoreGraphics draws premultiplied: back to straight alpha, which the page takes.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplied_pixels_become_straight() {
        let mut px = [64, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
        unpremultiply(&mut px);
        assert_eq!(px, [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn an_app_has_its_finder_icon() {
        let icon = icon_rgba(Path::new("/System/Applications/Utilities/Terminal.app"), 48).expect("Terminal's icon");
        assert_eq!(icon.len(), 48 * 48 * 4);
        assert!(icon.chunks_exact(4).any(|px| px[3] == 255), "some opaque pixels");
    }
}
