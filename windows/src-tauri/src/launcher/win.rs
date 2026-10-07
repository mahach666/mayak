//! The Win32 of the launcher: the registry lookup of URL protocols, links, the clipboard, keys,
//! the foreground window and AppX activation. Tests never open a link, never send a key and
//! never touch the clipboard: those calls are checked by hand, with real agent windows.

use std::process::Command;

use ::windows::core::{HSTRING, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::{CloseHandle, GlobalFree, ERROR_SUCCESS, HANDLE};
use ::windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use ::windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use ::windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use ::windows::Win32::System::Ole::CF_UNICODETEXT;
use ::windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};
use ::windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_V,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SW_SHOWNORMAL};

/// True when `scheme` is a registered URL protocol: `HKEY_CLASSES_ROOT\<scheme>` (HKCU and HKLM
/// classes merged) has a `URL Protocol` value. Only the presence of that value is read.
pub fn protocol_registered(scheme: &str) -> bool {
    if scheme.is_empty() || scheme.contains(['\\', '/']) {
        return false;
    }
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CLASSES_ROOT, &HSTRING::from(scheme), Some(0), KEY_READ, &mut key) != ERROR_SUCCESS {
            return false;
        }
        let found = RegQueryValueExW(key, &HSTRING::from("URL Protocol"), None, None, None, None) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        found
    }
}

/// COM on the launch thread for its lifetime: `ShellExecuteW` may hand a link to a shell
/// extension, which needs it. Uninitialised on drop when the init succeeded.
pub struct Com(bool);

impl Com {
    pub fn init() -> Com {
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Opens `uri` with the handler Windows has for its protocol (spec §4, `ShellExecuteW` "open").
pub fn open_uri(uri: &str) -> Result<(), String> {
    let code = unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    }
    .0 as isize;
    // ShellExecuteW reports success as a value above 32.
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Windows could not open the link (error {code})."))
    }
}

/// `text` as the NUL-terminated UTF-16 that CF_UNICODETEXT holds.
pub fn utf16_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn clipboard_busy() -> String {
    "The clipboard is busy. Try again.".to_string()
}

/// Puts `text` on the clipboard as Unicode text, replacing what is there. One attempt.
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    let wide = utf16_nul(text);
    unsafe {
        OpenClipboard(None).map_err(|_| clipboard_busy())?;
        let result = fill_clipboard(&wide);
        let _ = CloseClipboard();
        result
    }
}

/// The clipboard is open: empty it and hand it a moveable copy of `wide`, which the system owns
/// once `SetClipboardData` succeeds.
unsafe fn fill_clipboard(wide: &[u16]) -> Result<(), String> {
    unsafe {
        EmptyClipboard().map_err(|_| clipboard_busy())?;
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|_| clipboard_busy())?;
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(clipboard_busy());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        // FALSE with no error once the lock count is zero: nothing to check.
        let _ = GlobalUnlock(mem);
        if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
            return Err(clipboard_busy());
        }
        Ok(())
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Ctrl down, V down, V up, Ctrl up: the only keys Mayak ever sends (spec §1.4: never Enter).
pub fn ctrl_v_inputs() -> [INPUT; 4] {
    [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)]
}

/// Sends Ctrl+V to the foreground window. The caller has checked which window that is.
pub fn send_ctrl_v() -> Result<(), String> {
    let inputs = ctrl_v_inputs();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows refused the key presses.".to_string())
    }
}

/// True while Enter, Shift, Alt or a Windows key is held: Ctrl+V then would be another shortcut,
/// or an Enter still going down would reach the agent.
pub fn keys_held() -> bool {
    [VK_RETURN, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|vk| (unsafe { GetAsyncKeyState(i32::from(vk.0)) } as u16 & 0x8000) != 0)
}

/// The foreground window and the pid of its process; None when no window is in front.
pub fn foreground() -> Option<(isize, u32)> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    Some((hwnd.0 as isize, pid))
}

/// Full path of the image of `pid`; None when the process cannot be opened.
pub fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        let text = buf.get(..len as usize)?;
        ok.then(|| String::from_utf16_lossy(text))
    }
}

/// Starts the AppX application `<family>!App` (spec §4: `explorer shell:AppsFolder\…`).
pub fn launch_appx(family: &str) -> Result<(), String> {
    Command::new("explorer")
        .arg(format!(r"shell:AppsFolder\{family}!App"))
        .spawn()
        .map(|_| ())
        .map_err(|_| "Explorer did not start.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_protocol_is_found_and_an_unknown_one_is_not() {
        // `http` is registered on every Windows; the second name exists nowhere.
        assert!(protocol_registered("http"));
        assert!(!protocol_registered("mayak-no-such-protocol"));
        assert!(!protocol_registered(""), "the root of the classes is no protocol");
        assert!(!protocol_registered(r"http\shell"), "a path is no scheme");
    }

    #[test]
    fn ctrl_v_is_four_key_events_and_never_enter() {
        let keys: Vec<(VIRTUAL_KEY, bool)> = ctrl_v_inputs()
            .iter()
            .map(|input| {
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let ki = unsafe { input.Anonymous.ki };
                (ki.wVk, ki.dwFlags == KEYEVENTF_KEYUP)
            })
            .collect();
        assert_eq!(keys, [(VK_CONTROL, false), (VK_V, false), (VK_V, true), (VK_CONTROL, true)]);
        assert!(keys.iter().all(|(vk, _)| *vk != VK_RETURN));
    }

    #[test]
    fn clipboard_text_is_utf16_with_a_terminating_nul() {
        assert_eq!(utf16_nul("aя\n"), [0x61, 0x44F, 0x0A, 0]);
        assert_eq!(utf16_nul(""), [0]);
    }

    #[test]
    fn the_foreground_and_image_lookups_run() {
        // Read-only lookups; nothing here sends a key or touches the clipboard.
        let me = image_path(std::process::id()).expect("the test binary has an image path");
        assert!(me.to_ascii_lowercase().ends_with(".exe"), "{me}");
        assert_eq!(image_path(0), None, "the idle process has no image");
        let _ = foreground();
        let _ = keys_held();
    }
}
