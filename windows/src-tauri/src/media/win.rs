//! The Windows half of the player (night spec §B): the media thread and its WinRT subscriptions.
//!
//! One thread for the life of the app. It joins the multithreaded apartment, so waiting for a
//! WinRT async call (`wait`, at most `WAIT_LIMIT`) is fine there, and never runs on the Tauri main
//! thread or a tokio worker. The WinRT event handlers run on thread-pool threads and only send a
//! `Msg` over the channel; between messages the thread sleeps in `recv`: no timer, no poll. Its sink calls
//! `app.emit_to`, which does not block.

use std::sync::mpsc::{channel, Receiver, SendError, Sender};
use std::time::{Duration, Instant};

use ::windows::core::{Error, RuntimeType};
use ::windows::Foundation::TypedEventHandler;
use ::windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use ::windows::Win32::Foundation::ERROR_TIMEOUT;
use ::windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use super::{is_fresh, plan, Action, MediaState, Msg, Reading, WAIT_LIMIT};

// ── Bounded waits ─────────────────────────────────────────────────────────────
//
// `.get()` waits for ever: a player that never completes a call would wedge this thread. `wait`
// gives one call `WAIT_LIMIT`, on this thread and with no helper thread, then cancels it.
//
// It reads the call's `Status()` instead of waiting on `SetCompleted`: the completion handler
// type lives in the windows-future crate, which this crate does not name (the `windows` crate does
// not re-export it, and its `IntoFuture` needs that crate's "std" feature). The checks back off
// from 1 ms to 50 ms, and happen only while a call is in flight: an idle thread sleeps in `recv`.

/// The longest pause between two status checks of a call in flight.
const MAX_STEP: Duration = Duration::from_millis(50);

/// `AsyncStatus::Started`: the call is still running.
const STARTED: i32 = 0;

/// The result of a call whose `status` is its `AsyncStatus` value: `results()` once it is no longer
/// started, or ERROR_TIMEOUT after `cancel()` once `WAIT_LIMIT` has passed.
fn wait<T>(
    what: &str,
    status: impl Fn() -> ::windows::core::Result<i32>,
    results: impl FnOnce() -> ::windows::core::Result<T>,
    cancel: impl FnOnce() -> ::windows::core::Result<()>,
) -> ::windows::core::Result<T> {
    let deadline = Instant::now() + WAIT_LIMIT;
    let mut step = Duration::from_millis(1);
    while status()? == STARTED {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            crate::log::line(format!("media: {what} timed out"));
            if let Err(err) = cancel() {
                log_error(&format!("cancelling {what}"), &err);
            }
            return Err(Error::from(ERROR_TIMEOUT.to_hresult()));
        }
        std::thread::sleep(step.min(left));
        step = (step * 2).min(MAX_STEP);
    }
    results()
}

/// `wait` for a WinRT `IAsyncOperation`.
macro_rules! bounded {
    ($what:expr, $op:expr) => {{
        let op = $op;
        wait($what, || op.Status().map(|s| s.0), || op.GetResults(), || op.Cancel())
    }};
}

/// The running media thread.
pub struct Worker {
    tx: Sender<Msg>,
}

impl Worker {
    /// Starts the thread; every state it reads goes to `publish`, on that thread. It subscribes
    /// when `enabled`, as if the setting had just been turned on.
    pub fn start(enabled: bool, publish: impl FnMut(Option<MediaState>) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = channel::<Msg>();
        // Queued before the thread exists, so it is the first message the thread reads.
        tx.send(Msg::Enabled(enabled)).map_err(|_| std::io::Error::other("the media channel is closed"))?;
        let events = tx.clone();
        std::thread::Builder::new().name("media".into()).spawn(move || run(rx, events, publish))?;
        Ok(Self { tx })
    }

    pub fn send(&self, msg: Msg) -> Result<(), SendError<Msg>> {
        self.tx.send(msg)
    }
}

fn run(rx: Receiver<Msg>, events: Sender<Msg>, mut publish: impl FnMut(Option<MediaState>)) {
    if let Err(err) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        log_error("CoInitializeEx", &err);
        return;
    }
    let mut subs = Subscriptions { events, manager: None, manager_token: None, session: None };
    let mut enabled = false;
    while let Ok(first) = rx.recv() {
        // Everything waiting at once: a burst of events is read once.
        let batch: Vec<Msg> = std::iter::once(first).chain(rx.try_iter()).collect();
        let (todo, now) = plan(enabled, &batch);
        enabled = now;
        if todo.unsubscribe {
            subs.drop_all();
            publish(None);
        }
        if todo.subscribe {
            subs.subscribe();
        }
        if todo.resubscribe {
            subs.follow_current();
        }
        for (action, pressed) in todo.actions {
            // Checked here, not in the plan: an earlier call of this batch may have taken the
            // whole wait limit.
            if is_fresh(pressed, Instant::now()) {
                subs.act(action);
            } else {
                crate::log::line(format!("media: {action:?} dropped, pressed too long ago"));
            }
        }
        if todo.refresh {
            publish(subs.read());
        }
    }
}

/// The session manager, its `CurrentSessionChanged` token, and the current session with its tokens.
struct Subscriptions {
    events: Sender<Msg>,
    manager: Option<Manager>,
    manager_token: Option<i64>,
    session: Option<Followed>,
}

struct Followed {
    session: Session,
    properties_token: Option<i64>,
    playback_token: Option<i64>,
}

/// A handler that only tells the media thread `msg`.
fn notify<S: RuntimeType + 'static, A: RuntimeType + 'static>(events: &Sender<Msg>, msg: Msg) -> TypedEventHandler<S, A> {
    let events = events.clone();
    TypedEventHandler::new(move |_, _| {
        // The receiver lives as long as the media thread, which lives as long as the app: a
        // failed send can only happen while the process exits, with nobody left to tell.
        let _ = events.send(msg);
        Ok(())
    })
}

impl Subscriptions {
    /// The setting turned on: the manager (requested once), its event, and the current session.
    fn subscribe(&mut self) {
        if self.manager.is_none() {
            // Retried on the next turn-on when it fails: `manager` stays None until it succeeds.
            match Manager::RequestAsync().and_then(|op| bounded!("RequestAsync", op)) {
                Ok(manager) => self.manager = Some(manager),
                Err(err) => {
                    log_error("RequestAsync", &err);
                    return;
                }
            }
        }
        let Some(manager) = self.manager.as_ref() else { return };
        if self.manager_token.is_none() {
            match manager.CurrentSessionChanged(&notify(&self.events, Msg::SessionChanged)) {
                Ok(token) => self.manager_token = Some(token),
                Err(err) => log_error("CurrentSessionChanged", &err),
            }
        }
        self.follow_current();
    }

    /// Moves the session subscriptions to the current session; none when there is no session.
    fn follow_current(&mut self) {
        self.drop_session();
        let Some(manager) = self.manager.as_ref() else { return };
        let session = match manager.GetCurrentSession() {
            Ok(session) => session,
            // A null session (no player) comes back as an error with no code.
            Err(err) if err.code().is_ok() => return,
            Err(err) => {
                log_error("GetCurrentSession", &err);
                return;
            }
        };
        let properties_token = session
            .MediaPropertiesChanged(&notify(&self.events, Msg::Changed))
            .map_err(|err| log_error("MediaPropertiesChanged", &err))
            .ok();
        let playback_token = session
            .PlaybackInfoChanged(&notify(&self.events, Msg::Changed))
            .map_err(|err| log_error("PlaybackInfoChanged", &err))
            .ok();
        self.session = Some(Followed { session, properties_token, playback_token });
    }

    fn drop_session(&mut self) {
        let Some(old) = self.session.take() else { return };
        if let Some(token) = old.properties_token {
            if let Err(err) = old.session.RemoveMediaPropertiesChanged(token) {
                log_error("RemoveMediaPropertiesChanged", &err);
            }
        }
        if let Some(token) = old.playback_token {
            if let Err(err) = old.session.RemovePlaybackInfoChanged(token) {
                log_error("RemovePlaybackInfoChanged", &err);
            }
        }
    }

    /// The setting turned off: no subscription is left. The manager object is kept for the next on.
    fn drop_all(&mut self) {
        self.drop_session();
        if let (Some(manager), Some(token)) = (self.manager.as_ref(), self.manager_token.take()) {
            if let Err(err) = manager.RemoveCurrentSessionChanged(token) {
                log_error("RemoveCurrentSessionChanged", &err);
            }
        }
    }

    /// A player button, on the current session. Its effect comes back as `PlaybackInfoChanged`.
    fn act(&self, action: Action) {
        let Some(followed) = self.session.as_ref() else { return };
        let session = &followed.session;
        let op = match action {
            Action::Toggle => session.TryTogglePlayPauseAsync(),
            Action::Next => session.TrySkipNextAsync(),
            Action::Prev => session.TrySkipPreviousAsync(),
        };
        match op.and_then(|op| bounded!(&format!("{action:?}"), op)) {
            Ok(true) => {}
            Ok(false) => crate::log::line(format!("media: {action:?} refused by the player")),
            Err(err) => log_error(&format!("{action:?}"), &err),
        }
    }

    /// The state of the current session; `None` without one, or when it cannot be read.
    fn read(&self) -> Option<MediaState> {
        let followed = self.session.as_ref()?;
        match reading(&followed.session) {
            Ok(reading) => reading.into_state(),
            Err(err) => {
                log_error("reading the session", &err);
                None
            }
        }
    }
}

fn reading(session: &Session) -> ::windows::core::Result<Reading> {
    let properties = bounded!("TryGetMediaPropertiesAsync", session.TryGetMediaPropertiesAsync()?)?;
    let info = session.GetPlaybackInfo()?;
    let controls = info.Controls()?;
    Ok(Reading {
        title: properties.Title()?.to_string_lossy(),
        artist: properties.Artist()?.to_string_lossy(),
        app_id: session.SourceAppUserModelId()?.to_string_lossy(),
        playing: info.PlaybackStatus()? == PlaybackStatus::Playing,
        can_next: controls.IsNextEnabled()?,
        can_prev: controls.IsPreviousEnabled()?,
    })
}

/// Only what failed and its HRESULT: never a title, an artist or an app id.
fn log_error(what: &str, err: &Error) {
    crate::log::line(format!("media: {what} failed: 0x{:08X}", err.code().0 as u32));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the Windows media service of a desktop session; run with --ignored"]
    fn the_session_manager_can_be_requested_on_this_machine() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().expect("the test thread joins the MTA");
        Manager::RequestAsync()
            .and_then(|op| bounded!("RequestAsync", op))
            .expect("RequestAsync succeeds");
    }

    #[test]
    fn a_call_that_is_already_done_is_returned_at_once() {
        let cancelled = std::cell::Cell::new(false);
        let got = wait("test", || Ok(1), || Ok(5), || {
            cancelled.set(true);
            Ok(())
        });
        assert_eq!(got, Ok(5));
        assert!(!cancelled.get());
    }

    #[test]
    fn a_call_completed_meanwhile_gives_its_results_well_within_the_limit() {
        let checks = std::cell::Cell::new(0);
        let started = Instant::now();
        // Started for three checks, then completed: 1 + 2 + 4 ms of backing off.
        let status = || {
            checks.set(checks.get() + 1);
            Ok(if checks.get() <= 3 { STARTED } else { 1 })
        };
        assert_eq!(wait("test", status, || Ok(7), || Ok(())), Ok(7));
        assert_eq!(checks.get(), 4);
        assert!(started.elapsed() < WAIT_LIMIT);
    }

    #[test]
    fn a_failed_status_read_is_the_error_and_nothing_is_cancelled() {
        let cancelled = std::cell::Cell::new(false);
        let failed = Error::from(::windows::Win32::Foundation::E_FAIL);
        let got = wait("test", || Err(failed.clone()), || Ok(()), || {
            cancelled.set(true);
            Ok(())
        });
        assert_eq!(got, Err(failed));
        assert!(!cancelled.get());
    }

    #[test]
    fn a_call_that_never_completes_is_cancelled_after_the_wait_limit() {
        let cancelled = std::cell::Cell::new(false);
        let started = Instant::now();
        let got = wait("test", || Ok(STARTED), || Ok(()), || {
            cancelled.set(true);
            Ok(())
        });
        assert_eq!(got.map_err(|e| e.code()), Err(ERROR_TIMEOUT.to_hresult()));
        assert!(cancelled.get());
        let took = started.elapsed();
        assert!(took >= WAIT_LIMIT && took < WAIT_LIMIT + Duration::from_secs(1), "{took:?}");
    }
}
