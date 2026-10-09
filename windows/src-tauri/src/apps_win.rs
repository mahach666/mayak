//! The Win32 side of the user's apps in the Apps tab (own-apps, ideas §30): the file dialog that
//! adds one, the target of a picked shortcut, an exe's file description and its own icon.
//! Every call starts COM on its own thread. Paths are user data: never logged.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use ::windows::core::{Interface, HSTRING, PCWSTR};
use ::windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use ::windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use ::windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use ::windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
use ::windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use ::windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellLinkW, SHDefExtractIconW, ShellLink, FOS_FILEMUSTEXIST,
    FOS_FORCEFILESYSTEM, FOS_NODEREFERENCELINKS, SIGDN_FILESYSPATH,
};
use ::windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

use crate::launcher::win::Com;

/// Shows the Open dialog for an `.exe` or a shortcut, owned by `owner`. `Ok(None)` when the user
/// cancels it; the picked file as it is (a shortcut is not followed: `resolve_shortcut` does it).
pub fn pick_file(owner: Option<HWND>, title: &str, filter_name: &str) -> Result<Option<PathBuf>, String> {
    let _com = Com::init();
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("dialog: {e}"))?;
        let options = dialog.GetOptions().map_err(|e| format!("dialog options: {e}"))?;
        dialog
            .SetOptions(options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM | FOS_NODEREFERENCELINKS)
            .map_err(|e| format!("dialog options: {e}"))?;
        let name = HSTRING::from(filter_name);
        let spec = HSTRING::from("*.exe;*.lnk");
        let filters = [COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) }];
        dialog.SetFileTypes(&filters).map_err(|e| format!("dialog filter: {e}"))?;
        // A title that does not take only costs the dialog its default one.
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Err(err) = dialog.Show(owner) {
            if err.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(None);
            }
            return Err(format!("dialog: {err}"));
        }
        let item = dialog.GetResult().map_err(|e| format!("dialog result: {e}"))?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| format!("dialog path: {e}"))?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const c_void));
        path.map(|p| Some(PathBuf::from(p))).map_err(|_| "dialog path: not UTF-16".to_string())
    }
}

/// The target a shortcut (`.lnk`) points at, `None` when it cannot be read or has no file target
/// (a shortcut to a Store app, a URL…). Not resolved further: no search, no UI.
pub fn resolve_shortcut(link: &Path) -> Option<PathBuf> {
    let _com = Com::init();
    unsafe {
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = shell.cast().ok()?;
        file.Load(&HSTRING::from(link.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        shell.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let target = String::from_utf16(&buf[..len]).ok()?;
        (!target.is_empty()).then(|| PathBuf::from(target))
    }
}

/// The exe's file description from its version resource ("Google Chrome"), `None` without one.
pub fn file_description(exe: &Path) -> Option<String> {
    let wide = HSTRING::from(exe.as_os_str());
    unsafe {
        let size = GetFileVersionInfoSizeW(&wide, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(&wide, None, size, block.as_mut_ptr().cast()).ok()?;
        let query = |sub: &str| -> Option<(*const c_void, u32)> {
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let mut len = 0u32;
            let ok = VerQueryValueW(block.as_ptr().cast(), &HSTRING::from(sub), &mut ptr, &mut len).as_bool();
            (ok && !ptr.is_null() && len > 0).then_some((ptr as *const c_void, len))
        };
        // The resource's own languages first, then US English in Unicode and in Windows-1252.
        let mut langs: Vec<(u16, u16)> = Vec::new();
        if let Some((ptr, len)) = query(r"\VarFileInfo\Translation") {
            let pairs = std::slice::from_raw_parts(ptr as *const u16, (len as usize / 2) & !1);
            langs.extend(pairs.chunks_exact(2).map(|p| (p[0], p[1])));
        }
        langs.extend([(0x0409, 0x04B0), (0x0409, 0x04E4)]);
        langs.into_iter().find_map(|(lang, cp)| {
            let (ptr, len) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\FileDescription"))?;
            let chars = std::slice::from_raw_parts(ptr as *const u16, len as usize);
            let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
            let text = String::from_utf16_lossy(&chars[..end]).trim().to_string();
            (!text.is_empty()).then_some(text)
        })
    }
}

/// The exe's own icon at `size`×`size`, as straight RGBA rows top to bottom; `None` when it has none.
pub fn icon_rgba(exe: &Path, size: u32) -> Option<Vec<u8>> {
    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut icon = HICON::default();
    // `niconsize`: the large icon's size in the low word; no small one is asked for.
    let hr = unsafe { SHDefExtractIconW(PCWSTR(wide.as_ptr()), 0, 0, Some(&mut icon), None, size & 0xFFFF) };
    if hr.is_err() || icon.is_invalid() {
        return None;
    }
    let pixels = icon_pixels(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    pixels.filter(|(w, h, _)| *w == size && *h == size).map(|(_, _, rgba)| rgba)
}

/// The pixels of `icon`: width, height and RGBA. An icon without an alpha channel takes it from its mask.
fn icon_pixels(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let result = (|| {
        let (w, h, mut bgra) = bitmap_bgra(info.hbmColor)?;
        if bgra.chunks_exact(4).all(|p| p[3] == 0) {
            let (mw, mh, mask) = bitmap_bgra(info.hbmMask)?;
            if (mw, mh) != (w, h) {
                return None;
            }
            // The AND mask: black where the icon is drawn.
            for (px, m) in bgra.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                px[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Some((w, h, bgra))
    })();
    unsafe {
        for bitmap in [info.hbmColor, info.hbmMask] {
            if !bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
        }
    }
    result
}

/// A bitmap as 32-bit BGRA rows, top to bottom.
fn bitmap_bgra(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if bitmap.is_invalid() {
        return None;
    }
    unsafe {
        let mut bm = BITMAP::default();
        let got = GetObjectW(HGDIOBJ(bitmap.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
        if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
        let mut header = BITMAPINFO::default();
        header.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut out = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let lines = GetDIBits(dc, bitmap, 0, h, Some(out.as_mut_ptr().cast()), &mut header, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        (lines == h as i32).then_some((w, h, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn system32(exe: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        root.join("System32").join(exe)
    }

    /// Writes a shortcut to `target` at `link`, as Explorer would.
    fn make_shortcut(link: &Path, target: &Path) {
        let _com = Com::init();
        unsafe {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            shell.SetPath(&HSTRING::from(target.as_os_str())).unwrap();
            let file: IPersistFile = shell.cast().unwrap();
            file.Save(&HSTRING::from(link.as_os_str()), true).unwrap();
        }
    }

    #[test]
    fn a_shortcut_resolves_to_its_target_and_a_plain_file_to_nothing() {
        let dir = TempDir::new("apps-win-lnk");
        let exe = dir.path.join("tool.exe");
        std::fs::write(&exe, b"").unwrap();
        let link = dir.path.join("Tool.lnk");
        make_shortcut(&link, &exe);
        assert_eq!(resolve_shortcut(&link), Some(exe.clone()));
        assert_eq!(crate::apps::picked_exe(&link, resolve_shortcut), Some(exe.clone()));
        assert_eq!(resolve_shortcut(&exe), None, "an empty file is no shortcut");
        assert_eq!(resolve_shortcut(&dir.path.join("gone.lnk")), None);
    }

    #[test]
    fn a_windows_exe_has_a_description_and_an_icon() {
        let cmd = system32("cmd.exe");
        assert!(file_description(&cmd).is_some_and(|d| !d.is_empty()));
        let rgba = icon_rgba(&cmd, 48).expect("cmd.exe has an icon");
        assert_eq!(rgba.len(), 48 * 48 * 4);
        assert!(rgba.chunks_exact(4).any(|p| p[3] > 0), "some pixels are drawn");
    }

    #[test]
    fn a_file_without_resources_has_no_description_and_no_icon_of_its_own() {
        let dir = TempDir::new("apps-win-res");
        let exe = dir.path.join("empty.exe");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(file_description(&exe), None);
        assert_eq!(file_description(&dir.path.join("gone.exe")), None);
    }
}
