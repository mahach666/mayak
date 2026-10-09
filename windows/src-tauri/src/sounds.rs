// Your own sounds: a file named like one of Луч's sounds in the sounds folder
// replaces it (finish.wav, approval.mp3, greet.m4a…), as on the Mac
// (SoundEngine.customFolder). The folder is platform::config_dir()/sounds:
// %APPDATA%\Mayak\sounds on Windows.
//
// The page decodes the bytes itself; a file it can't decode falls back to the
// built-in sound there (src/core/sound.ts). Log lines name the sound, never a path.

use crate::{log, platform};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

/// Formats a webview can usually decode, looked for in this order.
const EXTENSIONS: [&str; 10] = ["wav", "mp3", "ogg", "oga", "m4a", "aac", "flac", "aiff", "aif", "caf"];
/// A sound effect, not an album: anything bigger is left alone.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub fn folder() -> PathBuf {
    platform::config_dir().join("sounds")
}

/// A sound name is a bare lowercase word ("finish"): nothing that could leave the folder.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase())
}

/// Whether `path` is a usable sound file. A missing file is the usual case and stays quiet;
/// any other failure to read its metadata is logged.
fn usable(path: &Path, name: &str, ext: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && m.len() > 0 && m.len() <= MAX_BYTES,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            log::line(format!("sounds: {name}.{ext} unreadable: {e}"));
            false
        }
    }
}

/// The user's file for `name` in `dir`, if there is a usable one.
fn custom_file(dir: &Path, name: &str) -> Option<PathBuf> {
    if !valid_name(name) {
        return None;
    }
    EXTENSIONS.iter().find_map(|ext| {
        let path = dir.join(format!("{name}.{ext}"));
        usable(&path, name, ext).then_some(path)
    })
}

/// Which of `names` have a file of the user's own.
#[tauri::command]
pub fn custom_sounds(names: Vec<String>) -> Vec<String> {
    let dir = folder();
    names.into_iter().filter(|n| custom_file(&dir, n).is_some()).collect()
}

/// The bytes of the user's file for `name`, sent as they are.
#[tauri::command]
pub fn custom_sound(name: String) -> Result<tauri::ipc::Response, String> {
    if !valid_name(&name) {
        return Err("invalid sound name".into());
    }
    let path = custom_file(&folder(), &name).ok_or_else(|| format!("no custom sound for {name}"))?;
    std::fs::read(&path).map(tauri::ipc::Response::new).map_err(|e| {
        log::line(format!("sounds: {name} could not be read: {e}"));
        e.to_string()
    })
}

/// Settings → Open sounds folder: creates it if needed and shows it.
#[tauri::command]
pub fn reveal_sounds_folder() {
    let dir = folder();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::line(format!("sounds: the sounds folder could not be created: {e}"));
        return;
    }
    platform::reveal_folder(&dir.to_string_lossy());
}

/// Settings → Reload sounds: the island reads the folder again.
#[tauri::command]
pub fn reload_sounds(app: AppHandle) {
    if let Err(e) = app.emit("sounds-changed", ()) {
        log::line(format!("sounds: sounds-changed not sent: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mayak-sounds-{tag}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_named_like_a_sound_replaces_it_in_the_first_format_found() {
        let dir = scratch("pick");
        std::fs::write(dir.join("finish.mp3"), b"mp3").unwrap();
        std::fs::write(dir.join("finish.wav"), b"wav").unwrap();
        assert_eq!(custom_file(&dir, "finish"), Some(dir.join("finish.wav")));
        std::fs::write(dir.join("greet.m4a"), b"m4a").unwrap();
        assert_eq!(custom_file(&dir, "greet"), Some(dir.join("greet.m4a")));
        assert_eq!(custom_file(&dir, "approval"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_outside_the_folder_or_unusable_is_read() {
        let dir = scratch("guard");
        std::fs::write(dir.join("empty.wav"), b"").unwrap();
        std::fs::create_dir_all(dir.join("folder.wav")).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let big = std::fs::File::create(dir.join("big.wav")).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        drop(big);
        assert_eq!(custom_file(&dir, "empty"), None);
        assert_eq!(custom_file(&dir, "folder"), None);
        assert_eq!(custom_file(&dir, "notes"), None);
        assert_eq!(custom_file(&dir, "big"), None);
        for bad in ["../finish", "Finish", "fin ish", "", "a/b", "finish.wav", &"a".repeat(33)] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(valid_name(&"a".repeat(32)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
