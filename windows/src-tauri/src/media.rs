//! What is playing in any player, from the Windows system media sessions (night spec §B): the
//! state the home page shows, how an app id reads as a name, and what a batch of messages asks
//! of the media thread. The WinRT side, and its thread, are in `win.rs`; macOS and Linux show no
//! player until they are ported.

#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Emitter;

#[cfg(windows)]
mod win;

/// The player row of the home page. `None` (null on the wire) when there is no session or its
/// title is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub title: String,
    pub artist: String,
    /// Display name of the playing app (`app_display_name`).
    pub app: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_prev: bool,
    /// The album art as a data URL (spec §B), `None` without one.
    pub cover: Option<String>,
}

/// What the session reports, before it becomes a `MediaState`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reading {
    pub title: String,
    pub artist: String,
    pub app_id: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub cover: Option<String>,
}

impl Reading {
    /// `None` for an empty (or blank) title: nothing worth a row.
    pub fn into_state(self) -> Option<MediaState> {
        let title = self.title.trim();
        if title.is_empty() {
            return None;
        }
        Some(MediaState {
            title: title.to_string(),
            artist: self.artist.trim().to_string(),
            app: app_display_name(&self.app_id),
            playing: self.playing,
            can_next: self.can_next,
            can_prev: self.can_prev,
            cover: self.cover,
        })
    }
}

/// Covers bigger than this are left out: they would cross the IPC on every track change.
pub const MAX_COVER_BYTES: usize = 262_144;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding (RFC 4648 §4); no crate for one small image.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The cover as the island shows it, or `None`: empty, too big, or not an image. The content type
/// goes into a URL, so it is lowercased, then must be `image/` and a non-empty subtype of only
/// lowercase letters, digits and `.+-`.
pub fn cover_data_url(content_type: &str, bytes: &[u8]) -> Option<String> {
    let content_type = content_type.to_ascii_lowercase();
    let subtype = content_type.strip_prefix("image/")?;
    let plain = !subtype.is_empty()
        && subtype
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-'));
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES || !plain {
        return None;
    }
    Some(format!("data:{content_type};base64,{}", base64(bytes)))
}

/// One track: a cover that was read is kept while this stays the same (spec §B), so play/pause
/// does not read it again.
pub fn cover_key(title: &str, artist: &str, app_id: &str) -> String {
    format!("{title}\u{1f}{artist}\u{1f}{app_id}")
}

/// The kept cover when it belongs to the track `key`. Only a cover that was read is kept (key and
/// data URL); a missing one is tried again on the next refresh, since a player may publish the
/// title before the thumbnail.
pub fn cached_cover<'a>(cache: &'a Option<(String, String)>, key: &str) -> Option<&'a str> {
    match cache {
        Some((cached, cover)) if cached == key => Some(cover),
        _ => None,
    }
}

/// The name shown for a `SourceAppUserModelId`. Known players by a part of their id, compared
/// without case (Chrome reports `Chrome`, Edge `MSEdge`); anything else is the part after the
/// last `!` or `\`, trimmed.
pub fn app_display_name(app_id: &str) -> String {
    let lower = app_id.to_ascii_lowercase();
    const KNOWN: [(&str, &str); 6] = [
        ("spotify", "Spotify"),
        ("applemusic", "Apple Music"),
        ("zunemusic", "Media Player"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
    ];
    if let Some((_, name)) = KNOWN.iter().find(|(part, _)| lower.contains(part)) {
        return (*name).to_string();
    }
    app_id.rsplit(['!', '\\']).next().unwrap_or("").trim().to_string()
}

/// `slot` becomes `new`; true when that changed it, the only time the island is told.
pub fn replace_if_changed(slot: &mut Option<MediaState>, new: Option<MediaState>) -> bool {
    if *slot == new {
        return false;
    }
    *slot = new;
    true
}

/// A player button of the home page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Next,
    Prev,
}

/// What reaches the media thread: from the WinRT event handlers, the commands and `save_settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// `CurrentSessionChanged`: subscribe to the new current session.
    SessionChanged,
    /// `MediaPropertiesChanged` or `PlaybackInfoChanged` of the current session.
    Changed,
    /// The "Show what's playing" setting.
    Enabled(bool),
    /// A button and when it was pressed: a press older than `WAIT_LIMIT` is dropped.
    Act(Action, Instant),
}

/// The longest the media thread waits for one WinRT call. A press that waited longer than this
/// in the queue (behind a stuck call) is dropped rather than fired late.
pub const WAIT_LIMIT: Duration = Duration::from_secs(3);

/// Whether a press made `at` is still worth running `now`.
pub fn is_fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= WAIT_LIMIT
}

/// What the media thread does for one batch of messages (everything waiting at once), in this
/// order: subscribe or drop the subscriptions, run the buttons, read and publish the state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Subscribe to the manager and the current session (the setting turned on).
    pub subscribe: bool,
    /// Drop every subscription and publish null (the setting turned off).
    pub unsubscribe: bool,
    /// Move the session subscriptions to the new current session.
    pub resubscribe: bool,
    /// The buttons pressed, in order, with when they were pressed.
    pub actions: Vec<(Action, Instant)>,
    /// Read the state again and publish it if it changed.
    pub refresh: bool,
}

/// The plan for `batch` on a thread whose setting is `enabled`, and the setting after it. The last
/// `Enabled` wins; while off, events and buttons do nothing.
pub fn plan(enabled: bool, batch: &[Msg]) -> (Plan, bool) {
    let now = batch
        .iter()
        .rev()
        .find_map(|m| if let Msg::Enabled(on) = m { Some(*on) } else { None })
        .unwrap_or(enabled);
    let mut plan = Plan::default();
    if !now {
        plan.unsubscribe = enabled;
        return (plan, now);
    }
    plan.subscribe = !enabled;
    plan.resubscribe = enabled && batch.contains(&Msg::SessionChanged);
    plan.actions = batch.iter().filter_map(|m| if let Msg::Act(a, at) = m { Some((*a, *at)) } else { None }).collect();
    plan.refresh = plan.subscribe || plan.resubscribe || batch.contains(&Msg::Changed);
    (plan, now)
}

// ── The app's player ──────────────────────────────────────────────────────────

/// The last state published: what `media_state` answers.
static CURRENT: Mutex<Option<MediaState>> = Mutex::new(None);
/// The media thread, started at launch and kept for the life of the app.
#[cfg(windows)]
static WORKER: Mutex<Option<win::Worker>> = Mutex::new(None);

/// Starts the media thread at launch; it subscribes when `enabled`.
#[cfg(windows)]
pub fn start(app: &AppHandle, enabled: bool) {
    let events = app.clone();
    match win::Worker::start(enabled, move |state| publish(&events, state)) {
        Ok(worker) => *WORKER.lock().unwrap() = Some(worker),
        Err(err) => crate::log::line(format!("media: cannot start the media thread: {:?}", err.kind())),
    }
}

/// macOS and Linux: no player yet.
#[cfg(not(windows))]
pub fn start(_app: &AppHandle, _enabled: bool) {}

/// The setting changed (`save_settings`).
pub fn set_enabled(on: bool) {
    send(Msg::Enabled(on));
}

#[cfg(windows)]
fn send(msg: Msg) {
    if let Some(worker) = WORKER.lock().unwrap().as_ref() {
        if worker.send(msg).is_err() {
            crate::log::line("media: the media thread is gone");
        }
    }
}

#[cfg(not(windows))]
fn send(_msg: Msg) {}

/// On the media thread: keeps the state `media_state` answers and tells the island, when it changed.
#[cfg(windows)]
fn publish(app: &AppHandle, state: Option<MediaState>) {
    if !replace_if_changed(&mut CURRENT.lock().unwrap(), state.clone()) {
        return;
    }
    // A fixed text: the error could carry the payload, and with it the title.
    if app.emit_to(crate::island::WINDOW_LABEL, "media", state).is_err() {
        crate::log::line("media: emit failed");
    }
}

/// The home page asks when it is built; later changes arrive as `media`.
#[tauri::command]
pub fn media_state() -> Option<MediaState> {
    CURRENT.lock().unwrap().clone()
}

/// The player buttons: handed to the media thread, never waited for.
#[tauri::command]
pub fn media_toggle() {
    send(Msg::Act(Action::Toggle, Instant::now()));
}

#[tauri::command]
pub fn media_next() {
    send(Msg::Act(Action::Next, Instant::now()));
}

#[tauri::command]
pub fn media_prev() {
    send(Msg::Act(Action::Prev, Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn known_players_read_as_their_names() {
        for (id, name) in [
            ("Spotify.exe", "Spotify"),
            ("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Spotify"),
            ("Chrome", "Chrome"),
            ("chrome.exe", "Chrome"),
            ("MSEdge", "Edge"),
            ("msedge.exe", "Edge"),
            ("firefox.exe", "Firefox"),
            ("AppleInc.AppleMusicWin_nzyj5cx40ttqa!App", "Apple Music"),
            ("AppleMusic.exe", "Apple Music"),
            ("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic", "Media Player"),
        ] {
            assert_eq!(app_display_name(id), name, "{id}");
        }
    }

    #[test]
    fn any_other_app_is_the_part_after_the_last_bang_or_backslash() {
        for (id, name) in [
            ("VideoLAN.VLC_paz6r1rewnh0a!App", "App"),
            (r"C:\Program Files\foobar2000\foobar2000.exe", "foobar2000.exe"),
            (r"Some.Pkg!Sub\ Player ", "Player"),
            ("  aimp.exe  ", "aimp.exe"),
            ("", ""),
        ] {
            assert_eq!(app_display_name(id), name, "{id:?}");
        }
    }

    fn reading(title: &str) -> Reading {
        Reading {
            title: title.into(),
            artist: "Artist".into(),
            app_id: "Spotify.exe".into(),
            playing: true,
            can_next: true,
            can_prev: false,
            cover: None,
        }
    }

    #[test]
    fn a_session_with_no_title_is_no_state() {
        assert_eq!(reading("").into_state(), None);
        assert_eq!(reading("   ").into_state(), None);
        assert_eq!(
            reading(" Song ").into_state(),
            Some(MediaState {
                title: "Song".into(),
                artist: "Artist".into(),
                app: "Spotify".into(),
                playing: true,
                can_next: true,
                can_prev: false,
                cover: None,
            })
        );
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn a_cover_is_a_data_url_only_for_a_small_image() {
        assert_eq!(cover_data_url("image/png", b"abc").as_deref(), Some("data:image/png;base64,YWJj"));
        assert_eq!(cover_data_url("image/png", b""), None);
        assert_eq!(cover_data_url("text/plain", b"abc"), None);
        assert_eq!(cover_data_url("image/jpeg", &vec![0u8; MAX_COVER_BYTES + 1]), None);
        assert!(cover_data_url("image/jpeg", &vec![0u8; MAX_COVER_BYTES]).is_some());
    }

    #[test]
    fn a_content_type_with_anything_but_a_plain_image_type_is_no_cover() {
        assert_eq!(cover_data_url("image/png,<x>", b"abc"), None);
        assert_eq!(cover_data_url("image/png;base64", b"abc"), None);
        assert_eq!(cover_data_url("image/PNG", b"abc").as_deref(), Some("data:image/png;base64,YWJj"));
        assert_eq!(cover_data_url("image/JPEG", b"abc").as_deref(), Some("data:image/jpeg;base64,YWJj"));
        assert_eq!(cover_data_url("image/", b"abc"), None);
        assert_eq!(cover_data_url("image/a/b", b"abc"), None);
        assert_eq!(cover_data_url("image/png\n", b"abc"), None);
        assert!(cover_data_url("image/svg+xml", b"abc").is_some());
        assert!(cover_data_url("image/vnd.microsoft.icon", b"abc").is_some());
        assert!(cover_data_url("image/x-icon", b"abc").is_some());
    }

    #[test]
    fn a_cached_cover_is_reused_only_for_the_same_track() {
        let cache = Some(("Song\u{1f}Band".to_string(), "data:image/png;base64,AA==".to_string()));
        assert_eq!(cached_cover(&cache, "Song\u{1f}Band"), Some("data:image/png;base64,AA=="));
        assert_eq!(cached_cover(&cache, "Song 2\u{1f}Band"), None);
        assert_eq!(cached_cover(&None, "Song\u{1f}Band"), None);
    }

    #[test]
    fn the_cover_key_changes_with_the_track_not_with_play_state() {
        assert_eq!(cover_key("Song", "Band", "Spotify"), cover_key("Song", "Band", "Spotify"));
        assert_ne!(cover_key("Song", "Band", "Spotify"), cover_key("Song 2", "Band", "Spotify"));
        assert_ne!(cover_key("a\u{1f}b", "", "x"), cover_key("a", "b", "x"));
    }

    #[test]
    fn the_cover_goes_on_the_wire_as_cover() {
        let r = Reading { title: "T".into(), cover: Some("data:image/png;base64,AA==".into()), ..Default::default() };
        let v = serde_json::to_value(r.into_state().unwrap()).unwrap();
        assert_eq!(v["cover"].as_str(), Some("data:image/png;base64,AA=="));
        let none = Reading { title: "T".into(), ..Default::default() }.into_state().unwrap();
        assert!(serde_json::to_value(none).unwrap()["cover"].is_null());
    }

    #[test]
    fn only_a_different_state_counts_as_a_change() {
        let mut slot = None;
        assert!(!replace_if_changed(&mut slot, None), "null to null");
        let song = reading("Song").into_state();
        assert!(replace_if_changed(&mut slot, song.clone()));
        assert!(!replace_if_changed(&mut slot, song.clone()), "the same state again");
        let mut paused = song.clone().unwrap();
        paused.playing = false;
        assert!(replace_if_changed(&mut slot, Some(paused.clone())));
        assert_eq!(slot, Some(paused));
        assert!(replace_if_changed(&mut slot, None));
        assert_eq!(slot, None);
    }

    #[test]
    fn the_state_reaches_the_page_in_camel_case_and_none_as_null() {
        assert_eq!(
            serde_json::to_value(reading("Song").into_state()).unwrap(),
            json!({ "title": "Song", "artist": "Artist", "app": "Spotify", "playing": true, "canNext": true, "canPrev": false, "cover": null })
        );
        assert_eq!(serde_json::to_value(None::<MediaState>).unwrap(), json!(null));
    }

    #[test]
    fn events_refresh_and_a_new_session_is_subscribed_to_once_per_batch() {
        use Msg::*;
        let (p, on) = plan(true, &[Changed, Changed, SessionChanged, Changed]);
        assert!(on);
        assert_eq!(p, Plan { resubscribe: true, refresh: true, ..Plan::default() });
        let (p, _) = plan(true, &[Changed]);
        assert_eq!(p, Plan { refresh: true, ..Plan::default() });
    }

    #[test]
    fn buttons_run_in_order_and_do_not_read_the_state_by_themselves() {
        use Msg::*;
        let at = Instant::now();
        let (p, _) = plan(true, &[Act(Action::Next, at), Act(Action::Toggle, at), Act(Action::Prev, at)]);
        assert_eq!(
            p,
            Plan { actions: vec![(Action::Next, at), (Action::Toggle, at), (Action::Prev, at)], ..Plan::default() }
        );
    }

    #[test]
    fn a_press_older_than_the_wait_limit_is_stale() {
        let pressed = Instant::now();
        assert!(is_fresh(pressed, pressed));
        assert!(is_fresh(pressed, pressed + WAIT_LIMIT));
        // Queued behind a stuck call: firing it at recovery would surprise the user.
        assert!(!is_fresh(pressed, pressed + WAIT_LIMIT + Duration::from_millis(1)));
        // A clock read before the press (another thread) is not a reason to drop it.
        assert!(is_fresh(pressed + Duration::from_secs(1), pressed));
    }

    #[test]
    fn turning_the_setting_off_drops_the_subscriptions_and_ignores_the_rest() {
        use Msg::*;
        let at = Instant::now();
        let (p, on) = plan(true, &[Changed, Enabled(false), Act(Action::Toggle, at), SessionChanged]);
        assert!(!on);
        assert_eq!(p, Plan { unsubscribe: true, ..Plan::default() });
        // Already off: events still in flight and buttons do nothing.
        let (p, on) = plan(false, &[Changed, SessionChanged, Act(Action::Next, at), Enabled(false)]);
        assert!(!on);
        assert_eq!(p, Plan::default());
    }

    #[test]
    fn turning_the_setting_on_subscribes_and_reads_the_state() {
        use Msg::*;
        let (p, on) = plan(false, &[Enabled(true)]);
        assert!(on);
        assert_eq!(p, Plan { subscribe: true, refresh: true, ..Plan::default() });
        // On again while on: nothing to do. Off then on in one batch: the last one wins.
        assert_eq!(plan(true, &[Enabled(true)]), (Plan::default(), true));
        assert_eq!(plan(true, &[Enabled(false), Enabled(true)]), (Plan::default(), true));
    }
}
