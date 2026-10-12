// The Connect buttons of the GitHub and GitLab integrations (ideas §14): the OAuth 2.0 device
// authorization grant (RFC 8628). Mayak asks the service for a device code, Settings shows the user
// code and the verification page, and Mayak polls the token endpoint at the interval the service
// set until the user approves, refuses, or the code expires. No client secret is involved.
//
// GitHub: POST https://github.com/login/device/code, then POST
// https://github.com/login/oauth/access_token with grant_type
// urn:ietf:params:oauth:grant-type:device_code (Accept: application/json; errors come as 200).
// GitLab (17.2+): POST {host}/oauth/authorize_device, then POST {host}/oauth/token (errors as 400).
//
// Tokens live in the Credential Manager only: the access token under the integration's usual key
// (`github-token`, `gitlab-token`, the same one the token field writes, so the pollers read it as
// before), and what goes with it (host, client ID, refresh token, expiry, needs-reconnect) as JSON
// under `github-auth` / `gitlab-auth`, which the page can never read or write. Nothing here logs a
// token, a code, a response body or a URL with a query: log lines carry status codes and kinds.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::gitlab;
use crate::log;
use crate::oauth_clients;
use crate::secrets;

const TIMEOUT: Duration = Duration::from_secs(10);
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// A token is refreshed this long before it expires.
const REFRESH_MARGIN_MS: i64 = 120_000;
pub const GITHUB_HOST: &str = "https://github.com";

// ── Services ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    GitHub,
    GitLab,
}

impl Service {
    pub fn parse(name: &str) -> Option<Service> {
        match name {
            "github" => Some(Service::GitHub),
            "gitlab" => Some(Service::GitLab),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Service::GitHub => "github",
            Service::GitLab => "gitlab",
        }
    }

    /// The access token's Credential Manager entry, shared with the token field.
    pub fn token_key(self) -> &'static str {
        match self {
            Service::GitHub => "github-token",
            Service::GitLab => "gitlab-token",
        }
    }

    /// What goes with the token (`StoredAuth`), never readable from the page.
    pub fn auth_key(self) -> &'static str {
        match self {
            Service::GitHub => "github-auth",
            Service::GitLab => "gitlab-auth",
        }
    }

    /// The integration pill this service backs.
    pub fn integration_id(self) -> &'static str {
        match self {
            Service::GitHub => "integration_github",
            Service::GitLab => "integration_gitlab",
        }
    }

    /// Least privilege that keeps the integration's data readable. GitHub: `read:user` gives the
    /// private profile (`owned_private_repos`); public repositories and their stars need no scope.
    /// GitLab: `read_api` (merge requests, pipelines, issues, read only).
    pub fn scope(self) -> &'static str {
        match self {
            Service::GitHub => "read:user",
            Service::GitLab => "read_api",
        }
    }
}

/// The device-code and token endpoints of `service` on `host` (GitLab's checked address).
pub fn endpoints(service: Service, host: &str) -> (String, String) {
    match service {
        Service::GitHub => (
            format!("{GITHUB_HOST}/login/device/code"),
            format!("{GITHUB_HOST}/login/oauth/access_token"),
        ),
        Service::GitLab => (format!("{host}/oauth/authorize_device"), format!("{host}/oauth/token")),
    }
}

// ── Client IDs ───────────────────────────────────────────────────────────────

/// Why Connect cannot start; the page shows each as its own notice. `code()` is what it gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartProblem {
    /// Mayak's own application is not registered yet (empty constant) and no ID was entered.
    NotRegistered,
    /// A self-managed GitLab without an Application ID: it needs one registered on that server.
    SelfManagedNeedsId,
    /// The Application ID entered has characters no client ID has.
    InvalidId,
    /// The GitLab address is not a valid https:// address.
    InvalidHost,
    /// Mayak is paused from the tray: nothing goes to the network.
    Paused,
    /// The service refused or could not be reached.
    Failed,
}

impl StartProblem {
    pub fn code(self) -> &'static str {
        match self {
            StartProblem::NotRegistered => "notRegistered",
            StartProblem::SelfManagedNeedsId => "selfManaged",
            StartProblem::InvalidId => "invalidId",
            StartProblem::InvalidHost => "invalidHost",
            StartProblem::Paused => "paused",
            StartProblem::Failed => "failed",
        }
    }
}

/// A client ID as GitHub (`Iv1.…`, `Ov23…`) and GitLab (64 hex) issue them: 1–128 characters of
/// letters, digits, `.`, `_`, `-`.
pub fn valid_client_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The client ID Connect uses: the Application ID from Settings when one is entered, else Mayak's
/// own (GitHub, or GitLab when the address is gitlab.com).
pub fn client_id(service: Service, override_id: &str, gitlab_host: &str) -> Result<String, StartProblem> {
    let entered = override_id.trim();
    if !entered.is_empty() {
        return if valid_client_id(entered) { Ok(entered.to_string()) } else { Err(StartProblem::InvalidId) };
    }
    let own = match service {
        Service::GitHub => oauth_clients::GITHUB_CLIENT_ID,
        Service::GitLab if gitlab_host == gitlab::DEFAULT_BASE => oauth_clients::GITLAB_COM_CLIENT_ID,
        Service::GitLab => return Err(StartProblem::SelfManagedNeedsId),
    };
    if own.is_empty() {
        Err(StartProblem::NotRegistered)
    } else {
        Ok(own.to_string())
    }
}

// ── Answers (pure) ───────────────────────────────────────────────────────────

/// A device authorization answer (RFC 8628 §3.2).
#[derive(Clone, PartialEq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("device_code", &"<redacted>")
            .field("user_code", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `{ device_code, user_code, verification_uri, expires_in, interval? }`; the interval defaults to
/// 5 s (RFC 8628 §3.2) and is at least 1 s. None for anything else.
pub fn parse_device_code(v: &Value) -> Option<DeviceCode> {
    Some(DeviceCode {
        device_code: text(v, "device_code")?,
        user_code: text(v, "user_code")?,
        verification_uri: text(v, "verification_uri")?,
        expires_in: v.get("expires_in")?.as_u64().filter(|n| *n > 0)?,
        interval: v.get("interval").and_then(Value::as_u64).unwrap_or(5).max(1),
    })
}

/// An access token and what may come with it.
#[derive(Clone, PartialEq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix ms when the access token expires; None when it does not (GitHub OAuth apps).
    pub expires_at_ms: Option<i64>,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// `{ access_token, refresh_token?, expires_in? }` (GitLab adds `created_at`, GitHub `scope`). The
/// expiry counts from `now_ms`, the clock the refresh check uses.
pub fn parse_token(v: &Value, now_ms: i64) -> Option<TokenSet> {
    Some(TokenSet {
        access_token: text(v, "access_token")?,
        refresh_token: text(v, "refresh_token"),
        expires_at_ms: v
            .get("expires_in")
            .and_then(Value::as_i64)
            .filter(|s| *s > 0)
            .map(|s| now_ms + s * 1000),
    })
}

/// One answer of the token endpoint while the user has not decided yet.
#[derive(Debug, PartialEq)]
pub enum PollReply {
    Token(TokenSet),
    Pending,
    /// Polled too fast; the service may name the new interval (GitHub does).
    SlowDown(Option<u64>),
    Expired,
    Denied,
    /// Not reached, or a server error: try again at the same interval.
    Retry,
    /// Anything else (a wrong client ID, device flow disabled…): stop. The kind is for the log.
    Failed(String),
}

/// The token endpoint's answer: GitHub says everything with 200, GitLab its errors with 400.
pub fn parse_poll(status: u16, body: Option<&Value>, now_ms: i64) -> PollReply {
    let Some(body) = body else {
        return if status >= 500 { PollReply::Retry } else { PollReply::Failed(format!("http {status}")) };
    };
    if (200..300).contains(&status) {
        if let Some(token) = parse_token(body, now_ms) {
            return PollReply::Token(token);
        }
    }
    match body.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => PollReply::Pending,
        Some("slow_down") => PollReply::SlowDown(body.get("interval").and_then(Value::as_u64)),
        Some("expired_token") => PollReply::Expired,
        Some("access_denied") => PollReply::Denied,
        Some(other) => PollReply::Failed(other.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect()),
        None if status >= 500 => PollReply::Retry,
        None => PollReply::Failed(format!("http {status}")),
    }
}

/// How a device flow ended without a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEnd {
    Expired,
    Denied,
    Failed,
}

impl FlowEnd {
    pub fn state(&self) -> &'static str {
        match self {
            FlowEnd::Expired => "expired",
            FlowEnd::Denied => "denied",
            FlowEnd::Failed => "error",
        }
    }
}

/// What to do after an answer.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Ask again after this many seconds.
    Wait(u64),
    Done(TokenSet),
    Stop(FlowEnd),
}

/// The polling schedule of one device flow: the service's interval (5 s more after each
/// `slow_down`, RFC 8628 §3.5), and the code's lifetime counted in the seconds waited.
#[derive(Debug, Clone, PartialEq)]
pub struct PollTimer {
    pub interval: u64,
    waited: u64,
    expires_in: u64,
}

impl PollTimer {
    pub fn new(code: &DeviceCode) -> PollTimer {
        PollTimer { interval: code.interval.max(1), waited: 0, expires_in: code.expires_in }
    }

    /// The first wait, before the first poll; Stop when the code would expire first.
    pub fn first(&mut self) -> Step {
        self.wait()
    }

    fn wait(&mut self) -> Step {
        if self.waited + self.interval > self.expires_in {
            return Step::Stop(FlowEnd::Expired);
        }
        self.waited += self.interval;
        Step::Wait(self.interval)
    }

    pub fn after(&mut self, reply: PollReply) -> Step {
        match reply {
            PollReply::Token(token) => Step::Done(token),
            PollReply::Pending | PollReply::Retry => self.wait(),
            PollReply::SlowDown(named) => {
                self.interval = named.unwrap_or(0).max(self.interval + 5);
                self.wait()
            }
            PollReply::Expired => Step::Stop(FlowEnd::Expired),
            PollReply::Denied => Step::Stop(FlowEnd::Denied),
            PollReply::Failed(_) => Step::Stop(FlowEnd::Failed),
        }
    }
}

// ── What is stored with a token ──────────────────────────────────────────────

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAuth {
    /// The address the token belongs to: it is never sent anywhere else.
    pub host: String,
    /// The application that issued it (refreshes need it); "" for a pasted token.
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    /// The refresh token was refused: no request until the user connects again.
    #[serde(default)]
    pub needs_reconnect: bool,
}

impl std::fmt::Debug for StoredAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredAuth")
            .field("host", &self.host)
            .field("client_id", &self.client_id)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at_ms", &self.expires_at_ms)
            .field("needs_reconnect", &self.needs_reconnect)
            .finish()
    }
}

impl StoredAuth {
    pub fn from_tokens(host: &str, client_id: &str, tokens: &TokenSet) -> StoredAuth {
        StoredAuth {
            host: host.to_string(),
            client_id: client_id.to_string(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at_ms: tokens.expires_at_ms,
            needs_reconnect: false,
        }
    }

    /// True when the access token expires within REFRESH_MARGIN_MS and can be refreshed.
    pub fn needs_refresh(&self, now_ms: i64) -> bool {
        self.refresh_token.is_some()
            && !self.client_id.is_empty()
            && self.expires_at_ms.is_some_and(|at| now_ms >= at - REFRESH_MARGIN_MS)
    }

    /// The same after a refresh: a refresh token the answer left out is kept.
    pub fn refreshed(&self, tokens: &TokenSet) -> StoredAuth {
        StoredAuth {
            refresh_token: tokens.refresh_token.clone().or_else(|| self.refresh_token.clone()),
            expires_at_ms: tokens.expires_at_ms,
            needs_reconnect: false,
            ..self.clone()
        }
    }
}

/// A refresh answer.
#[derive(Debug, PartialEq)]
pub enum RefreshReply {
    Token(TokenSet),
    /// `invalid_grant` (refresh token revoked, used or expired) or `invalid_client` (application
    /// gone): only connecting again helps.
    Reconnect,
    Failed,
}

pub fn parse_refresh(status: u16, body: Option<&Value>, now_ms: i64) -> RefreshReply {
    if (200..300).contains(&status) {
        if let Some(token) = body.and_then(|b| parse_token(b, now_ms)) {
            return RefreshReply::Token(token);
        }
    }
    match body.and_then(|b| b.get("error")).and_then(Value::as_str) {
        Some("invalid_grant") | Some("invalid_client") => RefreshReply::Reconnect,
        _ => RefreshReply::Failed,
    }
}

/// What Settings shows for a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    None,
    Connected,
    Reconnect,
}

pub fn conn_state(token_present: bool, auth: Option<&StoredAuth>) -> ConnState {
    match (token_present, auth) {
        (_, Some(a)) if a.needs_reconnect => ConnState::Reconnect,
        (true, _) => ConnState::Connected,
        (false, _) => ConnState::None,
    }
}

// ── Storage ──────────────────────────────────────────────────────────────────

pub fn load_auth(service: Service) -> Option<StoredAuth> {
    let raw = secrets::get(service.auth_key())?;
    match serde_json::from_str(&raw) {
        Ok(auth) => Some(auth),
        Err(_) => {
            log::line(format!("oauth {}: stored auth unreadable", service.name()));
            None
        }
    }
}

fn save_auth(service: Service, auth: &StoredAuth) -> Result<(), String> {
    let raw = serde_json::to_string(auth).map_err(|e| e.to_string())?;
    secrets::set(service.auth_key(), &raw)
}

/// A token from Connect: what goes with it first (the refresh token, once rotated, is the only
/// one that works), then the token itself.
fn save_tokens(service: Service, auth: &StoredAuth, tokens: &TokenSet) -> Result<(), String> {
    save_auth(service, auth)?;
    secrets::set(service.token_key(), &tokens.access_token)
}

/// The token field saved (or emptied) a token by hand: it belongs to `host`, has no refresh token,
/// and clears a needs-reconnect.
pub fn manual_token(service: Service, host: &str, present: bool) -> Result<(), String> {
    forget(service);
    if present {
        save_auth(service, &StoredAuth { host: host.to_string(), ..StoredAuth::default() })
    } else {
        secrets::clear(service.auth_key())
    }
}

/// Disconnect: the token and everything stored with it go.
pub fn disconnect(service: Service) -> Result<(), String> {
    cancel(service);
    forget(service);
    secrets::clear(service.token_key())?;
    secrets::clear(service.auth_key())
}

pub fn status(service: Service) -> ConnState {
    conn_state(secrets::present(service.token_key()), load_auth(service).as_ref())
}

/// The data seen so far goes: the next read after a new token fills the card silently.
fn forget(service: Service) {
    if service == Service::GitLab {
        gitlab::forget();
    }
}

// ── Network ──────────────────────────────────────────────────────────────────

/// No redirects: a token, a code or a refresh token never follows one anywhere.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

async fn post_form(http: &reqwest::Client, url: &str, form: &[(&str, &str)]) -> Result<(u16, Option<Value>), ()> {
    let response = http
        .post(url)
        .header("Accept", "application/json")
        .header("User-Agent", "Mayak")
        .form(form)
        .send()
        .await
        .map_err(|_| ())?;
    let status = response.status().as_u16();
    Ok((status, response.json::<Value>().await.ok()))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Why a usable token could not be had.
#[derive(Debug, PartialEq)]
pub enum TokenProblem {
    /// No token saved.
    Missing,
    /// The refresh token was refused earlier or now: Settings → Connect again.
    Reconnect,
    /// The token belongs to another address than the one configured now.
    OtherHost,
    /// The refresh did not go through (no connection, server error): next poll tries again.
    RefreshFailed,
}

/// One refresh at a time per service: two at once would spend the same refresh token twice, and
/// the second would come back invalid_grant.
static REFRESHING: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// The access token to send to `host`, refreshed first when it is about to expire. A token saved
/// for another host is never returned. Without stored auth (a token pasted before Connect existed,
/// GitHub only) the token is used as it is.
pub async fn usable_token(service: Service, host: &str) -> Result<String, TokenProblem> {
    let _guard = REFRESHING.lock().await;
    let token = secrets::get(service.token_key()).ok_or(TokenProblem::Missing)?;
    let Some(auth) = load_auth(service) else {
        return match service {
            Service::GitHub => Ok(token),
            // Every GitLab token is saved with its host; one without is not sent anywhere.
            Service::GitLab => Err(TokenProblem::OtherHost),
        };
    };
    if auth.needs_reconnect {
        return Err(TokenProblem::Reconnect);
    }
    if auth.host != host {
        return Err(TokenProblem::OtherHost);
    }
    if !auth.needs_refresh(now_ms()) {
        return Ok(token);
    }
    let (_, token_url) = endpoints(service, &auth.host);
    let refresh = auth.refresh_token.clone().unwrap_or_default();
    let answer = post_form(
        &http(),
        &token_url,
        &[("client_id", &auth.client_id), ("grant_type", "refresh_token"), ("refresh_token", &refresh)],
    )
    .await;
    let reply = match answer {
        Ok((status, body)) => {
            log::line(format!("oauth {}: refresh HTTP {status}", service.name()));
            parse_refresh(status, body.as_ref(), now_ms())
        }
        Err(()) => {
            log::line(format!("oauth {}: refresh not reached", service.name()));
            RefreshReply::Failed
        }
    };
    match reply {
        RefreshReply::Token(tokens) => {
            let next = auth.refreshed(&tokens);
            if save_tokens(service, &next, &tokens).is_err() {
                log::line(format!("oauth {}: refreshed token not saved", service.name()));
                return Err(TokenProblem::RefreshFailed);
            }
            Ok(tokens.access_token)
        }
        RefreshReply::Reconnect => {
            let marked = StoredAuth { needs_reconnect: true, refresh_token: None, ..auth };
            if save_auth(service, &marked).is_err() {
                log::line(format!("oauth {}: needs-reconnect not saved", service.name()));
            }
            Err(TokenProblem::Reconnect)
        }
        RefreshReply::Failed => Err(TokenProblem::RefreshFailed),
    }
}

// ── The device flow ──────────────────────────────────────────────────────────

/// What Settings shows while the user approves: meant for display.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStart {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

/// What Settings receives when a flow ends: `state` is connected, expired, denied or error.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FlowResult {
    service: &'static str,
    state: &'static str,
}

/// The running flow per service, by number: a newer Connect, Cancel or Disconnect ends the older.
static FLOWS: std::sync::LazyLock<Mutex<(u64, HashMap<Service, u64>)>> =
    std::sync::LazyLock::new(|| Mutex::new((0, HashMap::new())));

fn begin(service: Service) -> u64 {
    let mut flows = FLOWS.lock().unwrap();
    flows.0 += 1;
    let n = flows.0;
    flows.1.insert(service, n);
    n
}

fn current(service: Service, n: u64) -> bool {
    FLOWS.lock().unwrap().1.get(&service) == Some(&n)
}

pub fn cancel(service: Service) {
    FLOWS.lock().unwrap().1.remove(&service);
}

/// The verification page must be https on the service's own host.
pub fn verification_allowed(service: Service, host: &str, uri: &str) -> bool {
    match service {
        Service::GitHub => gitlab::link_allowed(GITHUB_HOST, uri),
        Service::GitLab => gitlab::link_allowed(host, uri),
    }
}

/// The address Connect and the poller use for `service`: github.com, or GitLab's address from
/// Settings once it passes `gitlab::normalize_base`.
pub fn host_of(app: &AppHandle, service: Service) -> Option<String> {
    match service {
        Service::GitHub => Some(GITHUB_HOST.to_string()),
        Service::GitLab => {
            let raw = app.try_state::<crate::Shared>()?.settings.lock().unwrap().gitlab_url.clone();
            gitlab::normalize_base(&raw).ok()
        }
    }
}

fn override_id(app: &AppHandle, service: Service) -> String {
    app.try_state::<crate::Shared>()
        .map(|shared| {
            let settings = shared.settings.lock().unwrap();
            match service {
                Service::GitHub => settings.github_client_id.clone(),
                Service::GitLab => settings.gitlab_client_id.clone(),
            }
        })
        .unwrap_or_default()
}

/// Connect: asks for a device code and starts polling for the token in the background. The user
/// code and the verification page go to Settings; the device code never leaves Rust.
pub async fn start(app: AppHandle, service: Service) -> Result<DeviceStart, StartProblem> {
    if crate::integrations::PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(StartProblem::Paused);
    }
    let host = host_of(&app, service).ok_or(StartProblem::InvalidHost)?;
    let client = client_id(service, &override_id(&app, service), &host)?;
    let (device_url, token_url) = endpoints(service, &host);
    let http = http();

    let answer = post_form(&http, &device_url, &[("client_id", &client), ("scope", service.scope())]).await;
    let code = match answer {
        Ok((status, body)) if (200..300).contains(&status) => body.as_ref().and_then(parse_device_code),
        Ok((status, _)) => {
            log::line(format!("oauth {}: device code HTTP {status}", service.name()));
            None
        }
        Err(()) => {
            log::line(format!("oauth {}: device code not reached", service.name()));
            None
        }
    };
    let Some(code) = code else { return Err(StartProblem::Failed) };
    if !verification_allowed(service, &host, &code.verification_uri) {
        log::line(format!("oauth {}: verification page on another host, refused", service.name()));
        return Err(StartProblem::Failed);
    }

    let n = begin(service);
    let shown = DeviceStart {
        user_code: code.user_code.clone(),
        verification_uri: code.verification_uri.clone(),
        expires_in: code.expires_in,
    };
    tauri::async_runtime::spawn(poll_flow(app, service, n, host, client, token_url, code, http));
    Ok(shown)
}

#[allow(clippy::too_many_arguments)]
async fn poll_flow(
    app: AppHandle,
    service: Service,
    n: u64,
    host: String,
    client: String,
    token_url: String,
    code: DeviceCode,
    http: reqwest::Client,
) {
    let mut timer = PollTimer::new(&code);
    let mut step = timer.first();
    let end = loop {
        match step {
            Step::Wait(secs) => {
                tokio::time::sleep(Duration::from_secs(secs)).await;
                if !current(service, n) {
                    return; // cancelled, or a newer Connect took over
                }
                // Paused from the tray meanwhile: no request; the code keeps expiring.
                if crate::integrations::PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
                    step = timer.after(PollReply::Retry);
                    continue;
                }
                let reply = match post_form(
                    &http,
                    &token_url,
                    &[("client_id", &client), ("device_code", &code.device_code), ("grant_type", DEVICE_GRANT)],
                )
                .await
                {
                    Ok((status, body)) => parse_poll(status, body.as_ref(), now_ms()),
                    Err(()) => PollReply::Retry,
                };
                if let PollReply::Failed(kind) = &reply {
                    log::line(format!("oauth {}: token poll failed ({kind})", service.name()));
                }
                step = timer.after(reply);
            }
            Step::Done(tokens) => {
                if !current(service, n) {
                    return;
                }
                let auth = StoredAuth::from_tokens(&host, &client, &tokens);
                forget(service);
                match save_tokens(service, &auth, &tokens) {
                    Ok(()) => break None,
                    Err(_) => {
                        log::line(format!("oauth {}: token not saved", service.name()));
                        break Some(FlowEnd::Failed);
                    }
                }
            }
            Step::Stop(end) => break Some(end),
        }
    };
    if !current(service, n) {
        return;
    }
    cancel(service);
    let state = end.as_ref().map_or("connected", FlowEnd::state);
    log::line(format!("oauth {}: {state}", service.name()));
    let _ = app.emit("oauth", FlowResult { service: service.name(), state });
    if end.is_none() {
        // The pill fills at once instead of at the next tick (only when it is switched on).
        crate::integrations::poll_now(app, service.integration_id()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Fixtures: the examples of docs.github.com "Authorizing OAuth apps" (device flow) and
    // docs.gitlab.com/api/oauth2 (device authorization grant).
    fn github_device() -> Value {
        json!({
            "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5",
            "user_code": "WDJB-MJHT",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        })
    }

    fn gitlab_device() -> Value {
        json!({
            "device_code": "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS",
            "user_code": "0A44L90H",
            "verification_uri": "https://gitlab.example.com/oauth/device",
            "verification_uri_complete": "https://gitlab.example.com/oauth/device?user_code=0A44L90H",
            "expires_in": 300,
            "interval": 5
        })
    }

    #[test]
    fn device_code_answers_of_both_services_parse() {
        let gh = parse_device_code(&github_device()).unwrap();
        assert_eq!(gh.user_code, "WDJB-MJHT");
        assert_eq!(gh.verification_uri, "https://github.com/login/device");
        assert_eq!((gh.expires_in, gh.interval), (900, 5));
        let gl = parse_device_code(&gitlab_device()).unwrap();
        assert_eq!(gl.device_code, "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS");
        assert_eq!((gl.expires_in, gl.interval), (300, 5));
    }

    #[test]
    fn a_device_code_answer_without_interval_waits_five_seconds_and_a_broken_one_is_refused() {
        let mut v = github_device();
        v.as_object_mut().unwrap().remove("interval");
        assert_eq!(parse_device_code(&v).unwrap().interval, 5);
        for key in ["device_code", "user_code", "verification_uri", "expires_in"] {
            let mut v = github_device();
            v.as_object_mut().unwrap().remove(key);
            assert!(parse_device_code(&v).is_none(), "{key} missing");
        }
        assert!(parse_device_code(&json!({"error": "device_flow_disabled"})).is_none());
        assert!(parse_device_code(&json!({ "device_code": "", "user_code": "A", "verification_uri": "https://github.com/login/device", "expires_in": 900 })).is_none());
    }

    #[test]
    fn token_answers_parse_with_and_without_expiry() {
        // GitHub (OAuth app, no expiry).
        let gh = parse_token(
            &json!({ "access_token": "gho_16C7e42F292c6912E7710c838347Ae178B4a", "token_type": "bearer", "scope": "repo,gist" }),
            1_000,
        )
        .unwrap();
        assert_eq!(gh.access_token, "gho_16C7e42F292c6912E7710c838347Ae178B4a");
        assert_eq!((gh.refresh_token.as_deref(), gh.expires_at_ms), (None, None));
        // GitLab: two hours, with a refresh token.
        let gl = parse_token(
            &json!({ "access_token": "TOKEN", "token_type": "Bearer", "expires_in": 7200, "refresh_token": "REFRESH", "scope": "read_api", "created_at": 1593096829 }),
            1_000,
        )
        .unwrap();
        assert_eq!(gl.refresh_token.as_deref(), Some("REFRESH"));
        assert_eq!(gl.expires_at_ms, Some(1_000 + 7_200_000));
        assert!(parse_token(&json!({ "token_type": "bearer" }), 0).is_none());
        assert!(parse_token(&json!({ "access_token": "" }), 0).is_none());
    }

    #[test]
    fn poll_answers_of_both_services_parse() {
        let pending = json!({ "error": "authorization_pending", "error_description": "…" });
        assert_eq!(parse_poll(200, Some(&pending), 0), PollReply::Pending); // GitHub
        assert_eq!(parse_poll(400, Some(&pending), 0), PollReply::Pending); // GitLab
        assert_eq!(parse_poll(200, Some(&json!({ "error": "slow_down", "interval": 10 })), 0), PollReply::SlowDown(Some(10)));
        assert_eq!(parse_poll(400, Some(&json!({ "error": "slow_down" })), 0), PollReply::SlowDown(None));
        assert_eq!(parse_poll(400, Some(&json!({ "error": "expired_token" })), 0), PollReply::Expired);
        assert_eq!(parse_poll(200, Some(&json!({ "error": "access_denied" })), 0), PollReply::Denied);
        assert_eq!(
            parse_poll(200, Some(&json!({ "error": "incorrect_client_credentials" })), 0),
            PollReply::Failed("incorrect_client_credentials".into())
        );
        assert_eq!(parse_poll(502, None, 0), PollReply::Retry);
        assert_eq!(parse_poll(404, None, 0), PollReply::Failed("http 404".into()));
        // A token in an error status is not a token.
        assert!(matches!(parse_poll(400, Some(&json!({ "access_token": "x" })), 0), PollReply::Failed(_)));
        assert!(matches!(parse_poll(200, Some(&json!({ "access_token": "x" })), 0), PollReply::Token(_)));
    }

    #[test]
    fn polling_waits_the_interval_slows_down_by_five_seconds_and_ends_with_the_token() {
        let code = parse_device_code(&github_device()).unwrap();
        let mut timer = PollTimer::new(&code);
        assert_eq!(timer.first(), Step::Wait(5));
        assert_eq!(timer.after(PollReply::Pending), Step::Wait(5));
        assert_eq!(timer.after(PollReply::SlowDown(None)), Step::Wait(10));
        assert_eq!(timer.after(PollReply::Pending), Step::Wait(10), "the slower pace stays");
        // A named interval smaller than ours + 5 does not speed us up; a larger one is taken.
        assert_eq!(timer.after(PollReply::SlowDown(Some(3))), Step::Wait(15));
        assert_eq!(timer.after(PollReply::SlowDown(Some(30))), Step::Wait(30));
        assert_eq!(timer.after(PollReply::Retry), Step::Wait(30));
        let token = TokenSet { access_token: "t".into(), refresh_token: None, expires_at_ms: None };
        assert_eq!(timer.after(PollReply::Token(token.clone())), Step::Done(token));
    }

    #[test]
    fn polling_stops_when_the_code_expires_or_the_user_refuses() {
        let code = DeviceCode { expires_in: 12, interval: 5, ..parse_device_code(&gitlab_device()).unwrap() };
        let mut timer = PollTimer::new(&code);
        assert_eq!(timer.first(), Step::Wait(5));
        assert_eq!(timer.after(PollReply::Pending), Step::Wait(5));
        assert_eq!(timer.after(PollReply::Pending), Step::Stop(FlowEnd::Expired), "15 s would pass 12 s");

        let mut timer = PollTimer::new(&parse_device_code(&gitlab_device()).unwrap());
        timer.first();
        assert_eq!(timer.after(PollReply::Expired), Step::Stop(FlowEnd::Expired));
        let mut timer = PollTimer::new(&parse_device_code(&gitlab_device()).unwrap());
        timer.first();
        assert_eq!(timer.after(PollReply::Denied), Step::Stop(FlowEnd::Denied));
        let mut timer = PollTimer::new(&parse_device_code(&gitlab_device()).unwrap());
        timer.first();
        assert_eq!(timer.after(PollReply::Failed("x".into())), Step::Stop(FlowEnd::Failed));
    }

    #[test]
    fn refresh_answers_parse_and_rotate_the_refresh_token() {
        let rotated = json!({ "access_token": "NEW", "token_type": "Bearer", "expires_in": 7200, "refresh_token": "ROTATED", "created_at": 1 });
        let RefreshReply::Token(tokens) = parse_refresh(200, Some(&rotated), 10) else { panic!() };
        let auth = StoredAuth {
            host: "https://gitlab.com".into(),
            client_id: "abc".into(),
            refresh_token: Some("OLD".into()),
            expires_at_ms: Some(5),
            needs_reconnect: false,
        };
        let next = auth.refreshed(&tokens);
        assert_eq!(next.refresh_token.as_deref(), Some("ROTATED"));
        assert_eq!(next.expires_at_ms, Some(10 + 7_200_000));
        assert_eq!((next.host.as_str(), next.client_id.as_str()), ("https://gitlab.com", "abc"));
        // No refresh token in the answer: the old one stays.
        let RefreshReply::Token(bare) = parse_refresh(200, Some(&json!({ "access_token": "N", "expires_in": 60 })), 0) else { panic!() };
        assert_eq!(auth.refreshed(&bare).refresh_token.as_deref(), Some("OLD"));

        let invalid = json!({ "error": "invalid_grant", "error_description": "The provided authorization grant is invalid" });
        assert_eq!(parse_refresh(400, Some(&invalid), 0), RefreshReply::Reconnect);
        assert_eq!(parse_refresh(401, Some(&json!({ "error": "invalid_client" })), 0), RefreshReply::Reconnect);
        assert_eq!(parse_refresh(500, None, 0), RefreshReply::Failed);
        assert_eq!(parse_refresh(400, Some(&json!({ "error": "invalid_request" })), 0), RefreshReply::Failed);
    }

    #[test]
    fn a_token_is_refreshed_shortly_before_it_expires_and_only_when_it_can_be() {
        let auth = StoredAuth {
            host: "https://gitlab.com".into(),
            client_id: "abc".into(),
            refresh_token: Some("R".into()),
            expires_at_ms: Some(1_000_000),
            needs_reconnect: false,
        };
        assert!(!auth.needs_refresh(1_000_000 - REFRESH_MARGIN_MS - 1));
        assert!(auth.needs_refresh(1_000_000 - REFRESH_MARGIN_MS));
        assert!(auth.needs_refresh(2_000_000));
        assert!(!StoredAuth { refresh_token: None, ..auth.clone() }.needs_refresh(2_000_000));
        assert!(!StoredAuth { expires_at_ms: None, ..auth.clone() }.needs_refresh(2_000_000));
        assert!(!StoredAuth { client_id: String::new(), ..auth }.needs_refresh(2_000_000));
    }

    #[test]
    fn stored_auth_round_trips_and_older_shapes_load() {
        let auth = StoredAuth {
            host: "https://git.example.com".into(),
            client_id: "id".into(),
            refresh_token: Some("R".into()),
            expires_at_ms: Some(7),
            needs_reconnect: true,
        };
        let raw = serde_json::to_string(&auth).unwrap();
        assert_eq!(serde_json::from_str::<StoredAuth>(&raw).unwrap(), auth);
        let pasted: StoredAuth = serde_json::from_str(r#"{"host":"https://gitlab.com"}"#).unwrap();
        assert_eq!(pasted, StoredAuth { host: "https://gitlab.com".into(), ..StoredAuth::default() });
    }

    #[test]
    fn the_connection_state_settings_shows() {
        let auth = StoredAuth { host: "h".into(), ..StoredAuth::default() };
        assert_eq!(conn_state(false, None), ConnState::None);
        assert_eq!(conn_state(true, None), ConnState::Connected, "a token pasted before Connect existed");
        assert_eq!(conn_state(true, Some(&auth)), ConnState::Connected);
        let marked = StoredAuth { needs_reconnect: true, ..auth };
        assert_eq!(conn_state(true, Some(&marked)), ConnState::Reconnect);
        assert_eq!(conn_state(false, Some(&marked)), ConnState::Reconnect);
    }

    #[test]
    fn client_ids_come_from_settings_or_mayaks_own_application() {
        let gl = gitlab::DEFAULT_BASE;
        assert_eq!(client_id(Service::GitHub, " Ov23liAbC.1_x-9 ", gl), Ok("Ov23liAbC.1_x-9".into()));
        assert_eq!(client_id(Service::GitLab, "abc123", "https://git.example.com"), Ok("abc123".into()));
        assert_eq!(client_id(Service::GitHub, "a b", gl), Err(StartProblem::InvalidId));
        assert_eq!(client_id(Service::GitHub, "a&scope=repo", gl), Err(StartProblem::InvalidId));
        assert_eq!(client_id(Service::GitHub, &"a".repeat(129), gl), Err(StartProblem::InvalidId));
        // A self-managed server always needs its own application.
        assert_eq!(client_id(Service::GitLab, "", "https://git.example.com"), Err(StartProblem::SelfManagedNeedsId));
        // Mayak's own applications, while their IDs are still empty, are not registered yet.
        let gh = client_id(Service::GitHub, "", gl);
        if oauth_clients::GITHUB_CLIENT_ID.is_empty() {
            assert_eq!(gh, Err(StartProblem::NotRegistered));
        } else {
            assert_eq!(gh, Ok(oauth_clients::GITHUB_CLIENT_ID.to_string()));
        }
        let glc = client_id(Service::GitLab, "  ", gl);
        if oauth_clients::GITLAB_COM_CLIENT_ID.is_empty() {
            assert_eq!(glc, Err(StartProblem::NotRegistered));
        } else {
            assert_eq!(glc, Ok(oauth_clients::GITLAB_COM_CLIENT_ID.to_string()));
        }
    }

    #[test]
    fn endpoints_and_scopes_are_the_documented_ones() {
        assert_eq!(
            endpoints(Service::GitHub, "ignored"),
            ("https://github.com/login/device/code".into(), "https://github.com/login/oauth/access_token".into())
        );
        assert_eq!(
            endpoints(Service::GitLab, "https://git.example.com"),
            ("https://git.example.com/oauth/authorize_device".into(), "https://git.example.com/oauth/token".into())
        );
        assert_eq!((Service::GitHub.scope(), Service::GitLab.scope()), ("read:user", "read_api"));
        assert_eq!(Service::parse("gitlab"), Some(Service::GitLab));
        assert_eq!(Service::parse("stripe"), None);
    }

    #[test]
    fn the_verification_page_must_be_on_the_services_host() {
        assert!(verification_allowed(Service::GitHub, "", "https://github.com/login/device"));
        assert!(!verification_allowed(Service::GitHub, "", "https://github.com.evil.io/login/device"));
        assert!(!verification_allowed(Service::GitHub, "", "http://github.com/login/device"));
        assert!(verification_allowed(Service::GitLab, "https://gitlab.example.com", "https://gitlab.example.com/oauth/device"));
        assert!(!verification_allowed(Service::GitLab, "https://gitlab.example.com", "https://gitlab.com/oauth/device"));
    }

    #[test]
    fn no_secret_reaches_debug_output() {
        let code = parse_device_code(&github_device()).unwrap();
        let tokens = TokenSet { access_token: "ACCESS-SECRET".into(), refresh_token: Some("REFRESH-SECRET".into()), expires_at_ms: Some(1) };
        let auth = StoredAuth {
            host: "https://gitlab.com".into(),
            client_id: "id".into(),
            refresh_token: Some("REFRESH-SECRET".into()),
            expires_at_ms: None,
            needs_reconnect: false,
        };
        let printed = format!(
            "{code:?} {tokens:?} {auth:?} {:?} {:?}",
            PollReply::Token(tokens.clone()),
            Step::Done(tokens.clone())
        );
        for secret in ["3584d83530557fdd1f46af8289938c8ef79f9dc5", "WDJB-MJHT", "ACCESS-SECRET", "REFRESH-SECRET"] {
            assert!(!printed.contains(secret), "{secret} in {printed}");
        }
        assert!(printed.contains("<redacted>"));
    }
}
