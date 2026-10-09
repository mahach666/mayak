//! What is playing in any player, from the Windows system media sessions (night spec §B): the
//! state the player shows, how an app id reads as a name, and what a batch of messages asks
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

/// The player row (the footer of the main tabs). `None` (null on the wire) when there is no session or its
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

/// When the cover of a new track is read again, counted from the read that saw the new track: a
/// player may publish the title before the thumbnail, or with the old track's thumbnail still in
/// place, and it does not always send an event when the thumbnail catches up.
pub const COVER_REREADS: [Duration; 3] =
    [Duration::from_millis(400), Duration::from_millis(1200), Duration::from_millis(2500)];

/// What a session reports about its track, for the cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackInfo<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub app_id: &'a str,
    /// The album title; empty when the player does not say.
    pub album: &'a str,
}

/// The cover of the current track and its settling window (`COVER_REREADS`). While the window
/// runs, every refresh reads the thumbnail again and a different read replaces the kept one. A read
/// equal to the previous cover shown may be the old track's cover still in place: it is shown
/// at once when both tracks are from one album of one app (shared art), never when their albums
/// differ (the fallback stays and the thumbnail is read again on every refresh, so a late one still
/// arrives), and from the last re-read on when an album is unknown. Once settled, a cover that was
/// read is kept for the track; a missing one is still tried again on each refresh.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Covers {
    track: Option<TrackCover>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackCover {
    key: String,
    album: String,
    app_id: String,
    /// The read that saw this track: the re-reads count from here.
    since: Instant,
    /// The re-reads of `COVER_REREADS` already due.
    due: usize,
    /// The cover shown for this track.
    cover: Option<String>,
    /// The last cover shown before this track, with the album and app of the track it was shown
    /// for (a skipped track that showed none hands on the one before it).
    previous: Option<String>,
    previous_album: String,
    previous_app_id: String,
}

impl TrackCover {
    /// Whether a read equal to the previous cover shown is this track's own cover.
    fn old_thumbnail_is_ours(&self, settled: bool) -> bool {
        let known = !self.album.is_empty() && !self.previous_album.is_empty();
        if known && self.album == self.previous_album && self.app_id == self.previous_app_id {
            true // one album: the art is shared
        } else if known && self.album != self.previous_album {
            false // another album: the player has not caught up yet
        } else {
            settled // unknown: taken at the last re-read
        }
    }
}

impl Covers {
    /// The cover to show for `track`, calling `read` for the thumbnail when it is needed. A blank
    /// title (a player between two tracks) is no track: nothing is read, nothing is forgotten.
    pub fn cover(&mut self, track: TrackInfo, now: Instant, read: impl FnOnce() -> Option<String>) -> Option<String> {
        if track.title.trim().is_empty() {
            return None;
        }
        let key = cover_key(track.title, track.artist, track.app_id);
        if !matches!(&self.track, Some(t) if t.key == key) {
            // The previous cover is the last one shown, with its own track's album and app: a stale
            // read a skipped track never showed must not pass for that track's art.
            let (previous, previous_album, previous_app_id) = match self.track.take() {
                Some(old) if old.cover.is_some() => (old.cover, old.album, old.app_id),
                Some(old) => (old.previous, old.previous_album, old.previous_app_id),
                None => (None, String::new(), String::new()),
            };
            self.track = Some(TrackCover {
                key,
                album: track.album.to_string(),
                app_id: track.app_id.to_string(),
                since: now,
                due: 0,
                cover: None,
                previous,
                previous_album,
                previous_app_id,
            });
        }
        let current = self.track.as_mut()?;
        let settled = current.due == COVER_REREADS.len();
        if !settled || current.cover.is_none() {
            if let Some(got) = read() {
                if current.previous.as_ref() != Some(&got) || current.old_thumbnail_is_ours(settled) {
                    current.cover = Some(got);
                }
            }
        }
        current.cover.clone()
    }

    /// Marks the re-reads whose time has come as done; true when there was one, so the caller
    /// reads the state again.
    pub fn take_due(&mut self, now: Instant) -> bool {
        let Some(track) = self.track.as_mut() else { return false };
        let before = track.due;
        while track.due < COVER_REREADS.len() && now >= track.since + COVER_REREADS[track.due] {
            track.due += 1;
        }
        track.due > before
    }

    /// How long until the next re-read; `None` once the track has settled, or without one.
    pub fn next_reread(&self, now: Instant) -> Option<Duration> {
        let track = self.track.as_ref()?;
        let at = track.since + *COVER_REREADS.get(track.due)?;
        Some(at.saturating_duration_since(now))
    }
}

/// How long "nothing playing" must last before the island is told: a player between two tracks,
/// or a browser moving to a new session, often reports nothing for a moment.
pub const NONE_GRACE: Duration = Duration::from_millis(1500);

/// Holds a `None` read back for `NONE_GRACE` while a state is shown; a state read within that time
/// cancels it. Turning the setting off does not go through here: it clears at once (`reset`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoneGrace {
    /// A state was published last (the island shows the player).
    showing: bool,
    /// The first `None` read since the state was shown.
    since: Option<Instant>,
}

impl NoneGrace {
    /// A read: the state to publish now; `None` while a `None` read is held back (or there is
    /// nothing shown to clear).
    pub fn read(&mut self, state: Option<MediaState>, now: Instant) -> Option<MediaState> {
        match state {
            Some(state) => {
                self.showing = true;
                self.since = None;
                Some(state)
            }
            None => {
                if self.showing && self.since.is_none() {
                    self.since = Some(now);
                }
                None
            }
        }
    }

    /// True when the held `None` has lasted `NONE_GRACE`: publish `None` now.
    pub fn take_expired(&mut self, now: Instant) -> bool {
        match self.since {
            Some(since) if now >= since + NONE_GRACE => {
                self.since = None;
                self.showing = false;
                true
            }
            _ => false,
        }
    }

    /// How long until the held `None` is published; `None` when none is held.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        self.since.map(|since| (since + NONE_GRACE).saturating_duration_since(now))
    }

    /// The setting turned off: `None` is published at once by the caller, nothing stays held.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The earlier of two optional waits.
pub fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
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

/// A player button.
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

/// The island asks once when it builds the player; later changes arrive as `media`.
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

    const OLD: &str = "data:image/png;base64,T0xE";
    const NEW: &str = "data:image/png;base64,TkVX";

    /// Reads the cover of track `title` of `album` (Spotify) at `at`, the thumbnail being `thumb`;
    /// also says whether it was read.
    fn cover_in(covers: &mut Covers, title: &str, album: &str, at: Instant, thumb: Option<&str>) -> (Option<String>, bool) {
        let read = std::cell::Cell::new(false);
        let track = TrackInfo { title, artist: "Band", app_id: "Spotify.exe", album };
        let got = covers.cover(track, at, || {
            read.set(true);
            thumb.map(str::to_string)
        });
        (got, read.get())
    }

    /// The same, for a player that does not say the album.
    fn cover_at(covers: &mut Covers, title: &str, at: Instant, thumb: Option<&str>) -> (Option<String>, bool) {
        cover_in(covers, title, "", at, thumb)
    }

    /// Track "A" of `album`, whose cover `OLD` has settled, seen at `t0`.
    fn settled_on_old_in(album: &str, t0: Instant) -> Covers {
        let mut covers = Covers::default();
        assert_eq!(cover_in(&mut covers, "A", album, t0, Some(OLD)).0.as_deref(), Some(OLD));
        assert!(covers.take_due(t0 + Duration::from_secs(10)));
        covers
    }

    fn settled_on_old(t0: Instant) -> Covers {
        settled_on_old_in("", t0)
    }

    #[test]
    fn the_next_track_of_one_album_shows_the_shared_art_at_once() {
        let t0 = Instant::now();
        let mut covers = settled_on_old_in("Album", t0);
        let t1 = t0 + Duration::from_secs(20);
        assert_eq!(cover_in(&mut covers, "B", "Album", t1, Some(OLD)).0.as_deref(), Some(OLD), "no ♪ flash");
        // Still replaced if the player later publishes a different one inside the window.
        assert_eq!(cover_in(&mut covers, "B", "Album", t1 + Duration::from_millis(400), Some(NEW)).0.as_deref(), Some(NEW));
        // Another app with an album of the same name is not taken for shared art.
        let mut other = settled_on_old_in("Album", t0);
        let track = TrackInfo { title: "B", artist: "Band", app_id: "Chrome", album: "Album" };
        assert_eq!(other.cover(track, t1, || Some(OLD.to_string())), None);
    }

    #[test]
    fn a_blank_title_between_two_tracks_keeps_the_previous_thumbnail_in_mind() {
        let t0 = Instant::now();
        let mut covers = settled_on_old(t0);
        let t1 = t0 + Duration::from_secs(20);
        assert_eq!(cover_at(&mut covers, "", t1, Some(OLD)), (None, false));
        assert_eq!(cover_at(&mut covers, "  ", t1, Some(OLD)), (None, false));
        assert_eq!(covers.next_reread(t1), None, "no track, no re-read");
        // The next track still knows OLD as the old one's thumbnail.
        assert_eq!(cover_at(&mut covers, "B", t1 + Duration::from_millis(100), Some(OLD)), (None, true));
        // And the track before the gap keeps its pinned cover if it comes back.
        let mut back = settled_on_old(t0);
        cover_at(&mut back, "", t1, None);
        assert_eq!(cover_at(&mut back, "A", t1, Some(NEW)), (Some(OLD.to_string()), false));
    }

    #[test]
    fn a_skipped_track_does_not_pass_the_stale_art_on_as_its_own() {
        let t0 = Instant::now();
        // A (album X) settles on OLD.
        let mut covers = settled_on_old_in("X", t0);
        // B (album Y) is skipped while the player still reports OLD: never shown.
        let t1 = t0 + Duration::from_secs(20);
        assert_eq!(cover_in(&mut covers, "B", "Y", t1, Some(OLD)).0, None);
        // C (album Y, same app) still gets OLD: it is A's art, not B's shared album art.
        let t2 = t1 + Duration::from_millis(300);
        assert_eq!(cover_in(&mut covers, "C", "Y", t2, Some(OLD)).0, None);
        for ms in [400, 1200, 2500] {
            assert!(covers.take_due(t2 + Duration::from_millis(ms)));
            assert_eq!(cover_in(&mut covers, "C", "Y", t2 + Duration::from_millis(ms), Some(OLD)).0, None, "{ms} ms");
        }
        assert_eq!(cover_in(&mut covers, "C", "Y", t2 + Duration::from_secs(4), Some(NEW)).0.as_deref(), Some(NEW));
        // Once C showed NEW, D of album Y shares it at once.
        assert_eq!(cover_in(&mut covers, "D", "Y", t2 + Duration::from_secs(9), Some(NEW)).0.as_deref(), Some(NEW));
    }

    #[test]
    fn the_old_albums_art_is_never_taken_for_another_albums_track() {
        let t0 = Instant::now();
        let mut covers = settled_on_old_in("First", t0);
        let t1 = t0 + Duration::from_secs(20);
        assert_eq!(cover_in(&mut covers, "B", "Second", t1, Some(OLD)).0, None);
        for ms in [400, 1200, 2500] {
            assert!(covers.take_due(t1 + Duration::from_millis(ms)));
            assert_eq!(cover_in(&mut covers, "B", "Second", t1 + Duration::from_millis(ms), Some(OLD)).0, None, "{ms} ms");
        }
        assert_eq!(covers.next_reread(t1 + Duration::from_secs(3)), None, "no timer after the window");
        // Each later refresh (an event) still reads it, and the late thumbnail is taken.
        assert_eq!(cover_in(&mut covers, "B", "Second", t1 + Duration::from_secs(4), Some(OLD)), (None, true));
        assert_eq!(cover_in(&mut covers, "B", "Second", t1 + Duration::from_secs(5), Some(NEW)), (Some(NEW.to_string()), true));
        assert_eq!(cover_in(&mut covers, "B", "Second", t1 + Duration::from_secs(6), Some(OLD)), (Some(NEW.to_string()), false));
    }

    #[test]
    fn a_new_track_is_read_again_at_the_rereads_and_then_no_more() {
        let t0 = Instant::now();
        let mut covers = Covers::default();
        assert_eq!(covers.next_reread(t0), None, "nothing scheduled without a track");
        cover_at(&mut covers, "A", t0, Some(NEW));
        assert_eq!(covers.next_reread(t0), Some(Duration::from_millis(400)));
        assert_eq!(covers.next_reread(t0 + Duration::from_millis(100)), Some(Duration::from_millis(300)));
        assert!(!covers.take_due(t0 + Duration::from_millis(399)));
        assert!(covers.take_due(t0 + Duration::from_millis(400)));
        assert!(!covers.take_due(t0 + Duration::from_millis(400)), "each re-read once");
        assert_eq!(covers.next_reread(t0 + Duration::from_millis(400)), Some(Duration::from_millis(800)));
        assert!(covers.take_due(t0 + Duration::from_millis(1200)));
        assert_eq!(covers.next_reread(t0 + Duration::from_millis(1200)), Some(Duration::from_millis(1300)));
        assert!(covers.take_due(t0 + Duration::from_millis(2600)));
        assert_eq!(covers.next_reread(t0 + Duration::from_millis(2600)), None, "settled: nothing scheduled");
        assert!(!covers.take_due(t0 + Duration::from_secs(60)));
        // A late wake (a stuck call) takes every re-read that passed at once.
        let mut late = Covers::default();
        cover_at(&mut late, "A", t0, Some(NEW));
        assert!(late.take_due(t0 + Duration::from_secs(5)));
        assert_eq!(late.next_reread(t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn a_settled_cover_is_kept_for_its_track_without_reading_it_again() {
        let t0 = Instant::now();
        let mut covers = settled_on_old(t0);
        // Play/pause: the same track, the thumbnail is not read.
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_secs(11), Some(NEW)), (Some(OLD.to_string()), false));
    }

    #[test]
    fn the_old_tracks_thumbnail_is_not_shown_for_the_new_track() {
        let t0 = Instant::now();
        let mut covers = settled_on_old(t0);
        let t1 = t0 + Duration::from_secs(20);
        // The title changed first, the thumbnail is still the old one: the fallback, not OLD.
        assert_eq!(cover_at(&mut covers, "B", t1, Some(OLD)), (None, true));
        assert!(covers.next_reread(t1).is_some());
        // An event inside the window reads it again; the new thumbnail has arrived.
        assert_eq!(cover_at(&mut covers, "B", t1 + Duration::from_millis(200), Some(NEW)).0.as_deref(), Some(NEW));
    }

    #[test]
    fn a_cover_read_in_the_window_is_replaced_by_a_later_different_read() {
        let t0 = Instant::now();
        let mut covers = Covers::default();
        assert_eq!(cover_at(&mut covers, "A", t0, Some(OLD)).0.as_deref(), Some(OLD));
        assert!(covers.take_due(t0 + Duration::from_millis(400)));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_millis(400), Some(NEW)).0.as_deref(), Some(NEW));
        // A failed read keeps what was read before it.
        assert!(covers.take_due(t0 + Duration::from_millis(1200)));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_millis(1200), None).0.as_deref(), Some(NEW));
        // After the last re-read the cover is pinned.
        assert!(covers.take_due(t0 + Duration::from_millis(2500)));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_millis(2500), Some(NEW)).0.as_deref(), Some(NEW));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_secs(9), Some(OLD)), (Some(NEW.to_string()), false));
    }

    #[test]
    fn the_last_reread_accepts_the_previous_tracks_thumbnail_for_one_album() {
        let t0 = Instant::now();
        let mut covers = settled_on_old(t0);
        let t1 = t0 + Duration::from_secs(20);
        assert_eq!(cover_at(&mut covers, "B", t1, Some(OLD)).0, None);
        assert!(covers.take_due(t1 + Duration::from_millis(400)));
        assert_eq!(cover_at(&mut covers, "B", t1 + Duration::from_millis(400), Some(OLD)).0, None);
        assert!(covers.take_due(t1 + Duration::from_millis(1200)));
        assert_eq!(cover_at(&mut covers, "B", t1 + Duration::from_millis(1200), Some(OLD)).0, None);
        assert!(covers.take_due(t1 + Duration::from_millis(2500)));
        assert_eq!(cover_at(&mut covers, "B", t1 + Duration::from_millis(2500), Some(OLD)).0.as_deref(), Some(OLD));
    }

    #[test]
    fn a_missing_cover_is_still_tried_again_after_the_window() {
        let t0 = Instant::now();
        let mut covers = Covers::default();
        assert_eq!(cover_at(&mut covers, "A", t0, None), (None, true));
        assert!(covers.take_due(t0 + Duration::from_secs(5)));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_secs(5), None), (None, true));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_secs(6), Some(NEW)), (Some(NEW.to_string()), true));
        assert_eq!(cover_at(&mut covers, "A", t0 + Duration::from_secs(7), Some(OLD)), (Some(NEW.to_string()), false));
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
    fn nothing_shown_is_held_back_for_the_grace_and_a_state_cancels_it() {
        let t0 = Instant::now();
        let song = reading("Song").into_state();
        let mut grace = NoneGrace::default();
        // Nothing shown yet: a None read has nothing to clear, no timer.
        assert_eq!(grace.read(None, t0), None);
        assert_eq!(grace.due_in(t0), None);
        assert!(!grace.take_expired(t0 + NONE_GRACE));
        assert_eq!(grace.read(song.clone(), t0), song);
        // Between two tracks: held, a state within the grace cancels it.
        assert_eq!(grace.read(None, t0), None);
        assert_eq!(grace.due_in(t0), Some(NONE_GRACE));
        assert!(!grace.take_expired(t0 + NONE_GRACE - Duration::from_millis(1)));
        let next = reading("Next").into_state();
        assert_eq!(grace.read(next.clone(), t0 + Duration::from_millis(900)), next);
        assert_eq!(grace.due_in(t0 + Duration::from_millis(900)), None);
        assert!(!grace.take_expired(t0 + Duration::from_secs(10)));
        // A second None read does not push the deadline back.
        let t1 = t0 + Duration::from_secs(20);
        grace.read(None, t1);
        grace.read(None, t1 + Duration::from_millis(1000));
        assert_eq!(grace.due_in(t1 + Duration::from_millis(1000)), Some(Duration::from_millis(500)));
        assert!(grace.take_expired(t1 + NONE_GRACE));
        assert!(!grace.take_expired(t1 + NONE_GRACE), "published once");
        assert_eq!(grace.due_in(t1 + NONE_GRACE), None);
    }

    #[test]
    fn turning_the_setting_off_leaves_nothing_held() {
        let t0 = Instant::now();
        let mut grace = NoneGrace::default();
        grace.read(reading("Song").into_state(), t0);
        grace.read(None, t0);
        grace.reset();
        assert_eq!(grace.due_in(t0), None);
        assert!(!grace.take_expired(t0 + NONE_GRACE));
        assert_eq!(grace, NoneGrace::default());
    }

    #[test]
    fn the_earliest_wait_wins() {
        let s = Duration::from_secs;
        assert_eq!(earliest(None, None), None);
        assert_eq!(earliest(Some(s(2)), None), Some(s(2)));
        assert_eq!(earliest(None, Some(s(1))), Some(s(1)));
        assert_eq!(earliest(Some(s(2)), Some(s(1))), Some(s(1)));
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
