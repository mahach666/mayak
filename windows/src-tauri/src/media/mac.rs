//! macOS: what plays, for the island's player.
//!
//! macOS keeps MediaRemote, the system's "Now Playing" (any app, browsers included), to Apple's
//! own processes since 15.4. `mayak-nowplaying.dylib` (windows/nowplaying/nowplaying.m) runs inside
//! `/usr/bin/perl`, one of them, and streams the player as JSON lines; the buttons go to it as
//! command lines. It runs only while the player setting is on, and waits for events in between.
//!
//! Should the helper not start or stop answering (a later macOS may close the door), the player
//! falls back to the two apps that say what they play themselves: Apple Music and Spotify post a
//! distributed notification on every change of track or state, with the track in it. Their buttons
//! are then AppleScript, which macOS lets through once the user allows it (Automation). That path
//! has no cover: the apps only name it by a web address, and Mayak fetches nothing it was not told to.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tauri::AppHandle;

use super::{Action, MediaState, Msg, Reading, Repeat, Timeline};

/// A player Mayak hears from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Player {
    Music,
    Spotify,
}

impl Player {
    const ALL: [Player; 2] = [Player::Music, Player::Spotify];

    /// The distributed notification the app posts on every change.
    fn notification(self) -> &'static str {
        match self {
            Player::Music => "com.apple.Music.playerInfo",
            Player::Spotify => "com.spotify.client.PlaybackStateChanged",
        }
    }

    /// What `app_display_name` reads as the app's name.
    fn app_id(self) -> &'static str {
        match self {
            Player::Music => "AppleMusic",
            Player::Spotify => "Spotify",
        }
    }

    /// The app as AppleScript names it.
    fn script_name(self) -> &'static str {
        match self {
            Player::Music => "Music",
            Player::Spotify => "Spotify",
        }
    }
}

/// A value of a notification's user info.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Text(String),
    Number(f64),
}

fn text<'a>(fields: &'a HashMap<String, Field>, key: &str) -> Option<&'a str> {
    match fields.get(key)? {
        Field::Text(t) => Some(t),
        Field::Number(_) => None,
    }
}

fn number(fields: &HashMap<String, Field>, key: &str) -> Option<f64> {
    match fields.get(key)? {
        Field::Number(n) => Some(*n),
        Field::Text(_) => None,
    }
}

/// The track a notification of `player` describes. Both apps send `Name`, `Artist` and
/// `Player State` (Playing, Paused, Stopped); Spotify also its place (`Playback Position`, in
/// seconds) and length (`Duration`, ms), Music only the length (`Total Time`, ms), so only a
/// Spotify track has a timeline. A stopped player, or one without a name, has nothing to show.
pub fn reading_from(player: Player, fields: &HashMap<String, Field>, now_unix_ms: u64) -> Option<Reading> {
    let state = text(fields, "Player State").unwrap_or("Stopped");
    if state == "Stopped" {
        return None;
    }
    let title = text(fields, "Name")?.to_string();
    let timeline = match player {
        Player::Spotify => {
            let duration = number(fields, "Duration").filter(|d| *d >= 1.0)? as u64;
            let position = (number(fields, "Playback Position").unwrap_or(0.0).max(0.0) * 1000.0) as u64;
            Some(Timeline { position_ms: position.min(duration), duration_ms: duration, updated_at: now_unix_ms, seekable: true })
        }
        Player::Music => None,
    };
    Some(Reading {
        title,
        artist: text(fields, "Artist").unwrap_or("").to_string(),
        app_id: player.app_id().to_string(),
        playing: state == "Playing",
        can_next: true,
        can_prev: true,
        cover: None,
        timeline,
        shuffle: None,
        repeat: None,
    })
}

/// The player the row shows: a playing one before a paused one, then the one heard from last.
pub fn current(readings: &[(Player, Reading, u64)]) -> Option<(Player, Reading)> {
    readings
        .iter()
        .max_by_key(|(_, reading, at)| (reading.playing, *at))
        .map(|(player, reading, _)| (*player, reading.clone()))
}

/// The AppleScript that runs `action` in `player`; it does nothing when the app is not running
/// (a `tell` would start it).
pub fn script(player: Player, action: Action) -> String {
    let name = player.script_name();
    let command = match (player, action) {
        (_, Action::Toggle) => "playpause".to_string(),
        (_, Action::Next) => "next track".to_string(),
        (_, Action::Prev) => "previous track".to_string(),
        (_, Action::Seek(ms)) => format!("set player position to {}", ms as f64 / 1000.0),
        (Player::Music, Action::Shuffle(on)) => format!("set shuffle enabled to {on}"),
        (Player::Spotify, Action::Shuffle(on)) => format!("set shuffling to {on}"),
        (Player::Music, Action::Repeat(mode)) => format!(
            "set song repeat to {}",
            match mode {
                Repeat::Off => "off",
                Repeat::All => "all",
                Repeat::One => "one",
            }
        ),
        (Player::Spotify, Action::Repeat(mode)) => format!("set repeating to {}", mode != Repeat::Off),
    };
    format!("if application \"{name}\" is running then tell application \"{name}\" to {command}")
}

// ── The helper's lines ────────────────────────────────────────────────────────

/// File name of the helper library.
pub const HELPER: &str = "mayak-nowplaying.dylib";

/// Loads the helper into Perl and runs it: `mayak_nowplaying_run` as a sub, after the load has
/// returned (see nowplaying.m on why not from the load itself).
const HELPER_PERL: &str = "use DynaLoader; \
    my $lib = DynaLoader::dl_load_file($ARGV[0], 0) or exit 4; \
    my $run = DynaLoader::dl_find_symbol($lib, 'mayak_nowplaying_run') or exit 5; \
    DynaLoader::dl_install_xsub('main::run', $run); run();";

/// What a helper line says about the cover: unchanged (no `artwork` key), gone, or a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverChange {
    Same,
    Gone,
    New(Option<String>),
}

/// The cover a line carries, as a data URL. The type goes into a URL, so it must look like
/// `image/<subtype>` (the rule of `cover_data_url`), and the data must be base64 within
/// `MAX_COVER_BYTES`; anything else is no cover.
pub fn cover_change(line: &Value) -> CoverChange {
    let Some(artwork) = line.get("artwork") else { return CoverChange::Same };
    let (Some(mime), Some(data)) = (artwork.get("mime").and_then(Value::as_str), artwork.get("data").and_then(Value::as_str))
    else {
        return CoverChange::Gone;
    };
    let mime = mime.to_ascii_lowercase();
    let subtype_ok = mime.strip_prefix("image/").is_some_and(|sub| {
        !sub.is_empty() && sub.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-'))
    });
    let data_ok = !data.is_empty()
        && data.len() <= super::MAX_COVER_BYTES.div_ceil(3) * 4
        && data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    CoverChange::New((subtype_ok && data_ok).then(|| format!("data:{mime};base64,{data}")))
}

/// What `app_display_name` reads: the two players by their own ids, any other app by its name
/// (`Google Chrome` reads as Chrome).
fn helper_app_id(line: &Value) -> String {
    match line.get("bundle").and_then(Value::as_str) {
        Some("com.apple.Music") => "AppleMusic".to_string(),
        Some("com.spotify.client") => "Spotify".to_string(),
        _ => line.get("app").and_then(Value::as_str).unwrap_or("").to_string(),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(n) => n.as_f64().is_some_and(|n| n != 0.0),
        None => false,
    }
}

/// The reading of a helper line (`cover` is the cover kept for the track); `None` for
/// `{"none":true}` or a line without a title. MediaRemote's shuffle mode is 1 off, 2–3 on; its
/// repeat mode 1 off, 2 one, 3 all; 0 or nothing means the player does not say.
pub fn reading_from_helper(line: &Value, cover: Option<String>) -> Option<Reading> {
    let title = line.get("title").and_then(Value::as_str)?.to_string();
    let seconds = |key: &str| line.get(key).and_then(Value::as_f64).filter(|s| s.is_finite() && *s >= 0.0);
    let timeline = seconds("duration").filter(|d| *d >= 0.001).map(|duration| {
        let duration_ms = (duration * 1000.0) as u64;
        Timeline {
            position_ms: ((seconds("elapsed").unwrap_or(0.0) * 1000.0) as u64).min(duration_ms),
            duration_ms,
            updated_at: line.get("at").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64,
            seekable: true,
        }
    });
    let mode = |key: &str| line.get(key).and_then(Value::as_i64);
    Some(Reading {
        title,
        artist: line.get("artist").and_then(Value::as_str).unwrap_or("").to_string(),
        app_id: helper_app_id(line),
        playing: truthy(line.get("playing")),
        can_next: true,
        can_prev: true,
        cover,
        timeline,
        shuffle: match mode("shuffle") {
            Some(1) => Some(false),
            Some(2 | 3) => Some(true),
            _ => None,
        },
        repeat: match mode("repeat") {
            Some(1) => Some(Repeat::Off),
            Some(2) => Some(Repeat::One),
            Some(3) => Some(Repeat::All),
            _ => None,
        },
    })
}

/// The command line the helper takes for a button.
pub fn helper_command(action: Action) -> String {
    match action {
        Action::Toggle => "toggle".into(),
        Action::Next => "next".into(),
        Action::Prev => "prev".into(),
        Action::Seek(ms) => format!("seek {ms}"),
        Action::Shuffle(on) => format!("shuffle {}", u8::from(on)),
        Action::Repeat(mode) => format!(
            "repeat {}",
            match mode {
                Repeat::Off => "off",
                Repeat::All => "all",
                Repeat::One => "one",
            }
        ),
    }
}

// ── The app's player ──────────────────────────────────────────────────────────

static APP: OnceLock<AppHandle> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The last reading of each player the notifications told of, with when it came (Unix ms).
static READINGS: Mutex<Vec<(Player, Reading, u64)>> = Mutex::new(Vec::new());

/// The running helper, its stdin (the buttons) and what it said last.
struct Helper {
    child: Child,
    stdin: ChildStdin,
}
static HELPER_PROC: Mutex<Option<Helper>> = Mutex::new(None);
/// The helper has answered: its readings win over the notifications'.
static HELPER_OK: AtomicBool = AtomicBool::new(false);
/// What the helper said last (`None`: nothing plays) and the cover kept for it.
static HELPER_READING: Mutex<(Option<Reading>, Option<String>)> = Mutex::new((None, None));

/// Where the helper library is: next to the app in the bundle (`Contents/Resources`), or where
/// the build leaves it (`target/release`, next to `target/debug` in development).
fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("../Resources").join(HELPER), dir.join(HELPER), dir.join("../release").join(HELPER)]
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| std::fs::canonicalize(p).ok())
}

/// Starts the helper unless it runs; its lines are read on a thread of their own.
fn start_helper() {
    let mut slot = HELPER_PROC.lock().unwrap();
    if slot.is_some() {
        return;
    }
    let Some(path) = helper_path() else {
        crate::log::line(format!("media: {HELPER} not found, Music and Spotify only"));
        return;
    };
    let spawned = Command::new("/usr/bin/perl")
        .args(["-e", HELPER_PERL])
        .arg(&path)
        .env("MAYAK_NOWPLAYING", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            crate::log::line(format!("media: the helper did not start: {:?}", err.kind()));
            return;
        }
    };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return };
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if !HELPER_OK.swap(true, Ordering::Relaxed) {
                crate::log::line("media: the system's Now Playing answers");
            }
            {
                let mut kept = HELPER_READING.lock().unwrap();
                let cover = match cover_change(&value) {
                    CoverChange::Same => kept.1.clone(),
                    CoverChange::Gone => None,
                    CoverChange::New(cover) => cover,
                };
                let reading = reading_from_helper(&value, cover.clone());
                *kept = (reading, cover);
            }
            publish_current();
        }
        // The helper ended (quit by the setting, or it failed): the notifications take over.
        HELPER_OK.store(false, Ordering::Relaxed);
        *HELPER_READING.lock().unwrap() = (None, None);
        publish_current();
    });
    *slot = Some(Helper { child, stdin });
}

/// Ends the helper: its stdin closes and it exits on its own; the kill is for one that hangs.
fn stop_helper() {
    if let Some(mut helper) = HELPER_PROC.lock().unwrap().take() {
        drop(helper.stdin);
        let _ = helper.child.kill();
        let _ = helper.child.wait();
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn publish_current() {
    let Some(app) = APP.get() else { return };
    let state: Option<MediaState> = if !ENABLED.load(Ordering::Relaxed) {
        None
    } else if HELPER_OK.load(Ordering::Relaxed) {
        HELPER_READING.lock().unwrap().0.clone().and_then(Reading::into_state)
    } else {
        current(&READINGS.lock().unwrap()).and_then(|(_, reading)| reading.into_state())
    };
    super::publish(app, state);
}

/// Starts listening at launch; the row shows only while `enabled`.
pub fn start(app: &AppHandle, enabled: bool) {
    let _ = APP.set(app.clone());
    ENABLED.store(enabled, Ordering::Relaxed);
    let install = || observe::install();
    if objc2::MainThreadMarker::new().is_some() {
        install();
    } else {
        let _ = app.run_on_main_thread(install);
    }
    if enabled {
        start_helper();
    }
}

/// The setting changed, or a button was pressed. Buttons run on their own thread: AppleScript
/// may wait for the app, and the first press waits for the user to answer macOS.
pub fn send(msg: Msg) {
    match msg {
        Msg::Enabled(on) => {
            ENABLED.store(on, Ordering::Relaxed);
            if on { start_helper() } else { stop_helper() }
            publish_current();
        }
        Msg::Act(action, _) if ENABLED.load(Ordering::Relaxed) && HELPER_OK.load(Ordering::Relaxed) => {
            if let Some(helper) = HELPER_PROC.lock().unwrap().as_mut() {
                if writeln!(helper.stdin, "{}", helper_command(action)).is_err() {
                    crate::log::line("media: the helper took no command");
                }
            }
        }
        Msg::Act(action, _) if ENABLED.load(Ordering::Relaxed) => {
            let Some((player, _)) = current(&READINGS.lock().unwrap()) else { return };
            let text = script(player, action);
            std::thread::spawn(move || {
                match std::process::Command::new("/usr/bin/osascript").args(["-e", &text]).output() {
                    Ok(out) if out.status.success() => {}
                    // A fixed text: the script's error could name the track.
                    Ok(_) => crate::log::line(format!("media: {} did not take the command", player.script_name())),
                    Err(err) => crate::log::line(format!("media: osascript did not start: {:?}", err.kind())),
                }
            });
        }
        _ => {}
    }
}

/// On the main thread: a player said what it plays.
fn heard(player: Player, fields: HashMap<String, Field>) {
    let at = now_ms();
    {
        let mut readings = READINGS.lock().unwrap();
        readings.retain(|(p, _, _)| *p != player);
        if let Some(reading) = reading_from(player, &fields, at) {
            readings.push((player, reading, at));
        }
    }
    publish_current();
}

/// The distributed notification observer: an NSObject subclass made once at run time, whose
/// `mayakHeard:` reads the notification's user info.
mod observe {
    use std::collections::HashMap;
    use std::ffi::c_void;

    use objc2::runtime::{AnyObject, Sel};
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSNumber, NSString};

    use super::{heard, Field, Player};

    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *const c_void;
        fn objc_allocateClassPair(superclass: *const c_void, name: *const std::ffi::c_char, extra: usize) -> *mut c_void;
        fn objc_registerClassPair(cls: *mut c_void);
        fn sel_registerName(name: *const std::ffi::c_char) -> *const c_void;
        fn class_addMethod(cls: *mut c_void, name: *const c_void, imp: *const c_void, types: *const std::ffi::c_char) -> bool;
        fn class_createInstance(cls: *const c_void, extra: usize) -> *mut c_void;
    }

    extern "C" fn on_notification(_this: *mut c_void, _sel: *const c_void, notification: *mut c_void) {
        // SAFETY: AppKit calls the selector with the NSNotification it delivers.
        let note: &NSNotification = unsafe { &*(notification as *const NSNotification) };
        let name = note.name().to_string();
        let Some(player) = Player::ALL.into_iter().find(|p| p.notification() == name) else { return };
        let mut fields = HashMap::new();
        if let Some(info) = note.userInfo() {
            for key in info.allKeys().iter() {
                let Some(key_text) = key.downcast_ref::<NSString>().map(|k| k.to_string()) else { continue };
                let Some(value) = info.objectForKey(&key) else { continue };
                let field = if let Some(s) = value.downcast_ref::<NSString>() {
                    Field::Text(s.to_string())
                } else if let Some(n) = value.downcast_ref::<NSNumber>() {
                    Field::Number(n.doubleValue())
                } else {
                    continue;
                };
                fields.insert(key_text, field);
            }
        }
        heard(player, fields);
    }

    /// Registers the observer for both players. Main thread only.
    pub fn install() {
        unsafe {
            let superclass = objc_getClass(c"NSObject".as_ptr());
            let cls = objc_allocateClassPair(superclass, c"MayakMediaObserver".as_ptr(), 0);
            if cls.is_null() {
                return; // already installed
            }
            class_addMethod(cls, sel_registerName(c"mayakHeard:".as_ptr()), on_notification as *const c_void, c"v@:@".as_ptr());
            objc_registerClassPair(cls);
            // Kept for the life of the app: the center does not retain its observers.
            let observer = class_createInstance(cls, 0) as *const AnyObject;
            let center = NSDistributedNotificationCenter::defaultCenter();
            for player in Player::ALL {
                let name = NSString::from_str(player.notification());
                center.addObserver_selector_name_object(&*observer, Sel::register(c"mayakHeard:"), Some(&name), None);
            }
        }
        crate::log::line("media: listening to Music and Spotify");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, Field)]) -> HashMap<String, Field> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn t(s: &str) -> Field {
        Field::Text(s.into())
    }

    #[test]
    fn a_spotify_track_has_its_timeline() {
        let f = fields(&[
            ("Name", t("Song")),
            ("Artist", t("Band")),
            ("Player State", t("Playing")),
            ("Playback Position", Field::Number(12.5)),
            ("Duration", Field::Number(200_000.0)),
        ]);
        let r = reading_from(Player::Spotify, &f, 1_000).unwrap();
        assert_eq!((r.title.as_str(), r.artist.as_str(), r.playing), ("Song", "Band", true));
        assert_eq!(r.timeline, Some(Timeline { position_ms: 12_500, duration_ms: 200_000, updated_at: 1_000, seekable: true }));
        assert_eq!(r.into_state().unwrap().app, "Spotify");
    }

    #[test]
    fn a_music_track_is_paused_without_a_timeline_and_stopped_is_nothing() {
        let paused = fields(&[("Name", t("Song")), ("Player State", t("Paused")), ("Total Time", Field::Number(1.0))]);
        let r = reading_from(Player::Music, &paused, 0).unwrap();
        assert!(!r.playing && r.timeline.is_none());
        assert_eq!(r.into_state().unwrap().app, "Apple Music");
        assert_eq!(reading_from(Player::Music, &fields(&[("Name", t("x")), ("Player State", t("Stopped"))]), 0), None);
        assert_eq!(reading_from(Player::Music, &fields(&[("Player State", t("Playing"))]), 0), None, "no name");
    }

    #[test]
    fn the_playing_player_wins_then_the_latest() {
        let r = |playing| Reading { title: "x".into(), playing, ..Default::default() };
        let readings = vec![(Player::Music, r(true), 1), (Player::Spotify, r(false), 5)];
        assert_eq!(current(&readings).unwrap().0, Player::Music);
        let readings = vec![(Player::Music, r(false), 1), (Player::Spotify, r(false), 5)];
        assert_eq!(current(&readings).unwrap().0, Player::Spotify);
        assert_eq!(current(&[]), None);
    }

    #[test]
    fn a_helper_line_reads_as_the_player_row() {
        let line = serde_json::json!({
            "title": "Heroin", "artist": "Lana Del Rey", "album": "", "app": "Google Chrome",
            "bundle": "com.google.Chrome", "playing": 1, "duration": 355.2, "elapsed": 230.47,
            "at": 1_791_748_934_442.19, "shuffle": null, "repeat": 3
        });
        let r = reading_from_helper(&line, None).unwrap();
        assert_eq!((r.title.as_str(), r.artist.as_str(), r.playing), ("Heroin", "Lana Del Rey", true));
        assert_eq!(r.timeline, Some(Timeline { position_ms: 230_470, duration_ms: 355_200, updated_at: 1_791_748_934_442, seekable: true }));
        assert_eq!((r.shuffle, r.repeat), (None, Some(Repeat::All)));
        assert_eq!(r.into_state().unwrap().app, "Chrome");
        let music = serde_json::json!({ "title": "x", "bundle": "com.apple.Music", "app": "Music", "playing": false });
        let r = reading_from_helper(&music, None).unwrap();
        assert!(!r.playing && r.timeline.is_none());
        assert_eq!(r.into_state().unwrap().app, "Apple Music");
        assert_eq!(reading_from_helper(&serde_json::json!({ "none": true }), None), None);
    }

    #[test]
    fn covers_come_only_as_plain_base64_images() {
        assert_eq!(cover_change(&serde_json::json!({ "title": "x" })), CoverChange::Same);
        assert_eq!(cover_change(&serde_json::json!({ "artwork": null })), CoverChange::Gone);
        assert_eq!(
            cover_change(&serde_json::json!({ "artwork": { "mime": "image/JPEG", "data": "AAEC" } })),
            CoverChange::New(Some("data:image/jpeg;base64,AAEC".into()))
        );
        for bad in [
            serde_json::json!({ "artwork": { "mime": "text/html", "data": "AAEC" } }),
            serde_json::json!({ "artwork": { "mime": "image/png;x", "data": "AAEC" } }),
            serde_json::json!({ "artwork": { "mime": "image/png", "data": "AA\"EC" } }),
        ] {
            assert_eq!(cover_change(&bad), CoverChange::New(None), "{bad}");
        }
    }

    #[test]
    fn buttons_are_one_line_commands_for_the_helper() {
        assert_eq!(helper_command(Action::Toggle), "toggle");
        assert_eq!(helper_command(Action::Seek(61_500)), "seek 61500");
        assert_eq!(helper_command(Action::Shuffle(true)), "shuffle 1");
        assert_eq!(helper_command(Action::Repeat(Repeat::One)), "repeat one");
    }

    #[test]
    fn buttons_never_start_a_player() {
        assert_eq!(
            script(Player::Music, Action::Toggle),
            "if application \"Music\" is running then tell application \"Music\" to playpause"
        );
        assert!(script(Player::Spotify, Action::Seek(61_500)).ends_with("set player position to 61.5"));
        assert!(script(Player::Spotify, Action::Repeat(Repeat::One)).ends_with("set repeating to true"));
        assert!(script(Player::Music, Action::Repeat(Repeat::All)).ends_with("set song repeat to all"));
    }
}
