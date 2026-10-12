// The GitHub integration: my open pull requests, the pull requests waiting for my review and the
// CI of mine, plus the repository and star counts the card always had.
//
// Every 5 min while the pill is switched on and Mayak is not paused (integrations::spawn), with the
// token from Connect or the token field (oauth::usable_token). One poll is at most 3 requests, all
// to api.github.com, none following a redirect:
//   1. POST /graphql   viewer.pullRequests (mine, with the CI of each head commit) and a search for
//                      the open pull requests that request my review (QUERY)
//   2. GET  /user      the repository counts
//   3. GET  /user/repos?affiliation=owner   the stars
// Pull requests need a token that reaches them: the read-only GitHub App «Mayak» sees the
// repositories it is installed on; an OAuth token with `read:user` only public ones. A token that
// is refused a resource gets a «needs permissions» note on the card, not an error loop. A rate
// limit pauses every request until GitHub's reset time. Events fire only on a change against the
// previous read; the first read after start or after a new token fills the card silently. Nothing
// is logged but status codes and error kinds: no token, body, title or repository name.

use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::gitlab::{self, Change};
use crate::integrations::{emit, IntegrationUpdate};
use crate::log;
use crate::oauth::{self, Service, TokenProblem, GITHUB_HOST};
use crate::oauth_clients;

pub const ID: &str = "integration_github";
const API: &str = "https://api.github.com";

/// The one GraphQL read (docs.github.com/en/graphql/reference: User.pullRequests, Query.search,
/// Commit.statusCheckRollup). Review requests are a search, so archived repositories are left out.
pub const QUERY: &str = "query { viewer { login \
pullRequests(states: OPEN, first: 20, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount \
nodes { title url number isDraft reviewDecision repository { nameWithOwner } \
commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } } } } \
search(query: \"is:pr is:open review-requested:@me archived:false\", type: ISSUE, first: 20) { issueCount \
nodes { ... on PullRequest { title url number repository { nameWithOwner } author { login } } } } }";

/// A rate limit without a reset time waits this long (GitHub: «wait at least one minute»).
const DEFAULT_BACKOFF_MS: i64 = 60_000;
/// A reset time further than this is not believed.
const MAX_BACKOFF_MS: i64 = 3_600_000;

// ── Links ────────────────────────────────────────────────────────────────────

/// True for an https link on github.com, without user or password: the only links the card opens.
pub fn link_allowed(link: &str) -> bool {
    gitlab::link_allowed(GITHUB_HOST, link)
}

/// A GitHub App slug as GitHub makes them: 1–100 of lower-case letters, digits and `-`.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 100 && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The page that installs the GitHub App `slug` on the user's repositories; None for no slug.
pub fn install_url(slug: &str) -> Option<String> {
    valid_slug(slug).then(|| format!("{GITHUB_HOST}/apps/{slug}/installations/new"))
}

// ── Answers (pure) ───────────────────────────────────────────────────────────

/// The CI of a head commit (StatusState): FAILURE and ERROR failed, PENDING and EXPECTED pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ci {
    Failed,
    Pending,
    Success,
}

impl Ci {
    pub fn of(state: &str) -> Option<Ci> {
        match state {
            "FAILURE" | "ERROR" => Some(Ci::Failed),
            "PENDING" | "EXPECTED" => Some(Ci::Pending),
            "SUCCESS" => Some(Ci::Success),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ci::Failed => "failed",
            Ci::Pending => "running",
            Ci::Success => "success",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequest {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    /// Only a link `link_allowed` lets through.
    pub url: Option<String>,
    pub draft: bool,
    /// None when there is no status or check on the head commit, or it could not be read.
    pub ci: Option<Ci>,
}

impl PullRequest {
    /// `owner/name#12`.
    pub fn reference(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    fn same(&self, other: &PullRequest) -> bool {
        self.repo == other.repo && self.number == other.number
    }
}

/// What a GraphQL `errors` array says, reduced to kinds (never a message: it may name a repository).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Problems {
    /// A resource the token may not read (`FORBIDDEN`, «Resource not accessible by …»).
    pub forbidden: bool,
    pub rate_limited: bool,
    /// The first other error type, reduced to letters, digits and `_`, at most 40.
    pub other: Option<String>,
}

/// One GraphQL answer: a list is None when GitHub did not give it (refused, or an error).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    pub login: Option<String>,
    pub mine: Option<Vec<PullRequest>>,
    pub mine_total: u64,
    pub reviews: Option<Vec<PullRequest>>,
    pub reviews_total: u64,
    pub problems: Problems,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A pull request node; None for a null node (one the token may not see) or a broken one.
fn pull_request(v: &Value) -> Option<PullRequest> {
    let ci = v
        .get("commits")
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .and_then(|n| n.last())
        .and_then(|n| n.get("commit"))
        .and_then(|c| c.get("statusCheckRollup"))
        .and_then(|r| r.get("state"))
        .and_then(Value::as_str)
        .and_then(Ci::of);
    Some(PullRequest {
        repo: v.get("repository")?.get("nameWithOwner")?.as_str()?.to_string(),
        number: v.get("number")?.as_u64()?,
        title: text(v, "title")?,
        url: text(v, "url").filter(|u| link_allowed(u)),
        draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        ci,
    })
}

fn connection(v: Option<&Value>, count_key: &str) -> Option<(Vec<PullRequest>, u64)> {
    let c = v.filter(|c| c.is_object())?;
    let list: Vec<PullRequest> =
        c.get("nodes").and_then(Value::as_array).map(|n| n.iter().filter_map(pull_request).collect()).unwrap_or_default();
    let total = c.get(count_key).and_then(Value::as_u64).unwrap_or(list.len() as u64);
    Some((list, total))
}

/// An error's `type` (or `extensions.code`) reduced to letters, digits and `_`, at most 40.
fn error_kind(e: &Value) -> String {
    let raw = e
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| e.get("extensions").and_then(|x| x.get("code")).and_then(Value::as_str))
        .unwrap_or("");
    raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect()
}

pub fn parse_problems(v: &Value) -> Problems {
    let mut p = Problems::default();
    for e in v.get("errors").and_then(Value::as_array).into_iter().flatten() {
        let kind = error_kind(e);
        let message = e.get("message").and_then(Value::as_str).unwrap_or("");
        if kind == "FORBIDDEN" || message.starts_with("Resource not accessible") {
            p.forbidden = true;
        } else if kind == "RATE_LIMITED" || (kind.is_empty() && message.to_ascii_lowercase().contains("rate limit")) {
            p.rate_limited = true;
        } else if p.other.is_none() {
            p.other = Some(if kind.is_empty() { "UNKNOWN".into() } else { kind });
        }
    }
    p
}

/// The answer of `QUERY`: whatever data came, and what the errors were.
pub fn parse_graphql(v: &Value) -> Answer {
    let data = v.get("data");
    let viewer = data.and_then(|d| d.get("viewer"));
    let mine = connection(viewer.and_then(|u| u.get("pullRequests")), "totalCount");
    let reviews = connection(data.and_then(|d| d.get("search")), "issueCount");
    Answer {
        login: viewer.and_then(|u| text(u, "login")),
        mine_total: mine.as_ref().map_or(0, |m| m.1),
        mine: mine.map(|m| m.0),
        reviews_total: reviews.as_ref().map_or(0, |r| r.1),
        reviews: reviews.map(|r| r.0),
        problems: parse_problems(v),
    }
}

/// What a GraphQL answer comes to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Pull requests, maybe partly (`note`: something was refused for lack of permissions).
    Data { note: bool },
    /// Nothing about pull requests may be read with this token: the card keeps its counts and
    /// shows the «needs permissions» note.
    NoAccess,
    RateLimited,
    /// An error GitHub named; the kind is for the card and the log.
    Failed(String),
}

pub fn outcome(answer: &Answer) -> Outcome {
    if answer.problems.rate_limited {
        return Outcome::RateLimited;
    }
    if answer.mine.is_none() && answer.reviews.is_none() {
        return if answer.problems.forbidden {
            Outcome::NoAccess
        } else {
            Outcome::Failed(answer.problems.other.clone().unwrap_or_else(|| "UNKNOWN".into()))
        };
    }
    Outcome::Data { note: answer.problems.forbidden }
}

/// Until when (Unix ms) every request waits after this answer, or None. A rate limit is a 429, a
/// 403 or 200 with `x-ratelimit-remaining: 0` or a `retry-after`, or a GraphQL RATE_LIMITED; it
/// lasts `retry-after` seconds, else until `x-ratelimit-reset` (epoch seconds), else one minute,
/// never more than an hour. A 403 without any of these is a permission problem, not a limit.
pub fn backoff_until(
    status: u16,
    remaining: Option<&str>,
    reset: Option<&str>,
    retry_after: Option<&str>,
    rate_limited_body: bool,
    now_ms: i64,
) -> Option<i64> {
    let num = |h: Option<&str>| h.and_then(|s| s.trim().parse::<i64>().ok());
    let exhausted = num(remaining) == Some(0);
    let retry = num(retry_after).filter(|s| *s >= 0).filter(|_| matches!(status, 200 | 403 | 429));
    if !(exhausted || retry.is_some() || rate_limited_body || status == 429) {
        return None;
    }
    let until = match (retry, num(reset).filter(|_| exhausted)) {
        (Some(secs), _) => now_ms + secs.saturating_mul(1000),
        (None, Some(reset)) => reset.saturating_mul(1000),
        (None, None) => now_ms + DEFAULT_BACKOFF_MS,
    };
    Some(until.clamp(now_ms + 1_000, now_ms + MAX_BACKOFF_MS))
}

// ── One read and what changed (pure) ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub mine: Option<Vec<PullRequest>>,
    pub mine_total: u64,
    pub reviews: Option<Vec<PullRequest>>,
    pub reviews_total: u64,
}

impl Snapshot {
    pub fn of(answer: &Answer) -> Snapshot {
        Snapshot {
            mine: answer.mine.clone(),
            mine_total: answer.mine_total,
            reviews: answer.reviews.clone(),
            reviews_total: answer.reviews_total,
        }
    }
}

/// What changed from `prev` to `next` (gitlab::Change, so one event rule serves both); nothing on
/// the first read. A list compares only when both reads have it: one refused this time is no
/// change. A review request is a pull request new on the review list; one of mine fires when its
/// CI, known on both reads, turns failed or success.
pub fn changes(prev: Option<&Snapshot>, next: &Snapshot) -> Vec<Change> {
    let Some(prev) = prev else { return Vec::new() };
    let mut out = Vec::new();
    if let (Some(before), Some(now)) = (&prev.reviews, &next.reviews) {
        for pr in now.iter().filter(|pr| !before.iter().any(|p| p.same(pr))) {
            out.push(Change::ReviewRequested { title: pr.title.clone(), reference: pr.reference() });
        }
    }
    if let (Some(before), Some(now)) = (&prev.mine, &next.mine) {
        for pr in now {
            let Some(ci) = pr.ci else { continue };
            let Some(was) = before.iter().find(|p| p.same(pr)).and_then(|p| p.ci) else { continue };
            if was == ci {
                continue;
            }
            let (title, reference) = (pr.title.clone(), pr.reference());
            match ci {
                Ci::Failed => out.push(Change::MineFailed { title, reference }),
                Ci::Success => out.push(Change::MinePassed { title, reference }),
                Ci::Pending => {}
            }
        }
    }
    out
}

/// The repository and star counts of the REST reads (the card's «overview»).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Counts {
    pub repos: i64,
    pub stars: i64,
}

/// What the island's card gets. `snapshot` None: the pull requests could not be read (`note`).
pub fn card_data(login: Option<&str>, snapshot: Option<&Snapshot>, note: bool, counts: Counts) -> Value {
    let pr = |p: &PullRequest| {
        json!({
            "title": p.title,
            "reference": p.reference(),
            "number": p.number,
            "url": p.url,
            "draft": p.draft,
            "ci": p.ci.map(Ci::name),
        })
    };
    let mut data = json!({
        "totalRepos": counts.repos,
        "totalStars": counts.stars,
        "login": login,
        "note": if note { Some("permissions") } else { None },
    });
    if let Some(s) = snapshot {
        let mine = s.mine.as_deref().unwrap_or_default();
        let (mut failed, mut running, mut success) = (0u32, 0u32, 0u32);
        for ci in mine.iter().filter_map(|p| p.ci) {
            match ci {
                Ci::Failed => failed += 1,
                Ci::Pending => running += 1,
                Ci::Success => success += 1,
            }
        }
        let obj = data.as_object_mut().expect("an object");
        obj.insert("mine".into(), json!(mine.iter().take(20).map(pr).collect::<Vec<_>>()));
        obj.insert("mineTotal".into(), json!(s.mine_total));
        obj.insert("reviews".into(), json!(s.reviews.as_deref().unwrap_or_default().iter().take(5).map(pr).collect::<Vec<_>>()));
        obj.insert("reviewsTotal".into(), json!(s.reviews_total));
        obj.insert("ci".into(), json!({ "failed": failed, "running": running, "success": success }));
    }
    data
}

/// The card's error line for a refused request.
pub fn status_error(code: u16) -> String {
    match code {
        401 => "Token rejected (401): connect GitHub again in Settings".into(),
        403 => "Token lacks access (403)".into(),
        300..=399 => format!("GitHub answered with a redirect ({code})"),
        _ => format!("API error {code}"),
    }
}

pub fn token_error(problem: &TokenProblem) -> Option<String> {
    match problem {
        TokenProblem::Missing => None,
        TokenProblem::RefreshFailed => Some("Could not refresh the token: no connection".into()),
        TokenProblem::Reconnect | TokenProblem::OtherHost => Some("Needs reconnecting: Settings → Integrations → Connect".into()),
    }
}

/// The card's line while a rate limit lasts.
pub fn rate_limit_error(until_ms: i64, now_ms: i64) -> String {
    let minutes = ((until_ms - now_ms).max(0) + 59_999) / 60_000;
    format!("GitHub rate limit reached: next read in {minutes} min")
}

// ── The poller ───────────────────────────────────────────────────────────────

/// The previous read: (login, snapshot). Another account starts over silently.
static PREVIOUS: Mutex<Option<(String, Snapshot)>> = Mutex::new(None);
/// No request before this Unix ms (a rate limit).
static BACKOFF_UNTIL: Mutex<i64> = Mutex::new(0);

/// A new token or a disconnect: the next read fills the card without events.
pub fn forget() {
    *PREVIOUS.lock().unwrap() = None;
}

fn error(app: &AppHandle, text: String) {
    emit(app, IntegrationUpdate { id: ID, data: json!({}), error: Some(text), event: None });
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|h| h.to_str().ok()).map(str::to_string)
}

/// Sets the back-off from an answer's headers; true when there is one now.
fn note_limit(response: &reqwest::Response, rate_limited_body: bool) -> bool {
    let until = backoff_until(
        response.status().as_u16(),
        header(response, "x-ratelimit-remaining").as_deref(),
        header(response, "x-ratelimit-reset").as_deref(),
        header(response, "retry-after").as_deref(),
        rate_limited_body,
        oauth::now_ms(),
    );
    if let Some(until) = until {
        *BACKOFF_UNTIL.lock().unwrap() = until;
        log::line("github: rate limited, waiting for the reset");
        return true;
    }
    false
}

/// What a request came back with: the body, or a reason to stop this poll.
enum Got {
    Body(reqwest::StatusCode, Value),
    Stop,
}

/// Stops the poll on no answer, a rate limit or a refusal (a 401 also marks the token for
/// reconnecting, so no further request goes out with it). `soft_403`: a 403 that is not a rate
/// limit is handed back instead (the GraphQL read turns it into the permissions note).
async fn send(app: &AppHandle, request: reqwest::RequestBuilder, what: &str, soft_403: bool) -> Got {
    let Ok(response) = request.send().await else {
        log::line(format!("github {what}: not reached"));
        error(app, "No connection".into());
        return Got::Stop;
    };
    let status = response.status();
    let code = status.as_u16();
    let limited = !status.is_success() && note_limit(&response, false);
    if limited {
        error(app, rate_limit_error(*BACKOFF_UNTIL.lock().unwrap(), oauth::now_ms()));
        return Got::Stop;
    }
    if code == 401 {
        log::line(format!("github {what} HTTP 401"));
        oauth::mark_reconnect(Service::GitHub);
        error(app, status_error(401));
        return Got::Stop;
    }
    if !status.is_success() && !(soft_403 && code == 403) {
        log::line(format!("github {what} HTTP {code}"));
        error(app, status_error(code));
        return Got::Stop;
    }
    // A 200 that spent the last request: the answer counts, the next poll waits.
    if status.is_success() {
        note_limit(&response, false);
    }
    match response.json::<Value>().await {
        Ok(body) => Got::Body(status, body),
        Err(_) if code == 403 => Got::Body(status, Value::Null),
        Err(_) => {
            log::line(format!("github {what}: unreadable answer"));
            error(app, "GitHub answered with something unreadable".into());
            Got::Stop
        }
    }
}

pub async fn poll(app: AppHandle) {
    let now = oauth::now_ms();
    let until = *BACKOFF_UNTIL.lock().unwrap();
    if now < until {
        error(&app, rate_limit_error(until, now));
        return;
    }
    let token = match oauth::usable_token(Service::GitHub, GITHUB_HOST).await {
        Ok(token) => token,
        Err(problem) => {
            if let Some(text) = token_error(&problem) {
                error(&app, text);
            }
            return;
        }
    };
    let http = oauth::http();
    let authed = |r: reqwest::RequestBuilder| {
        r.header("Authorization", format!("Bearer {token}")).header("User-Agent", "Mayak")
    };

    // 1. Pull requests (one GraphQL read).
    let request = authed(http.post(format!("{API}/graphql"))).json(&json!({ "query": QUERY }));
    let Got::Body(status, body) = send(&app, request, "graphql", true).await else { return };
    let answer = if status.is_success() { parse_graphql(&body) } else { Answer::default() };
    let read = if status.is_success() { outcome(&answer) } else { Outcome::NoAccess };
    match &read {
        Outcome::RateLimited => {
            let now = oauth::now_ms();
            let mut until = BACKOFF_UNTIL.lock().unwrap();
            *until = (*until).max(now + DEFAULT_BACKOFF_MS);
            log::line("github graphql: RATE_LIMITED");
            let text = rate_limit_error(*until, now);
            drop(until);
            error(&app, text);
            return;
        }
        Outcome::Failed(kind) => {
            log::line(format!("github graphql: {kind}"));
            error(&app, format!("GitHub refused the pull request query ({kind})"));
            return;
        }
        Outcome::NoAccess => log::line(format!("github graphql HTTP {}: no access to pull requests", status.as_u16())),
        Outcome::Data { note: true } => log::line("github graphql: FORBIDDEN in part"),
        Outcome::Data { note: false } => {}
    }

    // 2–3. The repository and star counts, as before.
    let accept = |r: reqwest::RequestBuilder| authed(r).header("Accept", "application/vnd.github+json");
    let Got::Body(_, user) = send(&app, accept(http.get(format!("{API}/user"))), "user", false).await else { return };
    let public = user.get("public_repos").and_then(Value::as_i64).unwrap_or(0);
    let private = user
        .get("owned_private_repos")
        .or_else(|| user.get("total_private_repos"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let repos = accept(http.get(format!("{API}/user/repos?per_page=100&affiliation=owner&sort=pushed")));
    let stars = match send(&app, repos, "repos", true).await {
        Got::Body(status, v) if status.is_success() => v
            .as_array()
            .map(|list| list.iter().filter_map(|r| r.get("stargazers_count").and_then(Value::as_i64)).sum())
            .unwrap_or(0),
        Got::Body(..) => 0,
        Got::Stop => return,
    };
    let counts = Counts { repos: public + private, stars };

    let (snapshot, note) = match read {
        Outcome::Data { note } => (Some(Snapshot::of(&answer)), note),
        _ => (None, true),
    };
    let login = answer.login.clone();
    let event = {
        let mut previous = PREVIOUS.lock().unwrap();
        match (&snapshot, &login) {
            (Some(s), Some(login)) => {
                let prev = previous.as_ref().filter(|(who, _)| who == login).map(|(_, s)| s);
                let event = gitlab::event_of(&changes(prev, s));
                *previous = Some((login.clone(), s.clone()));
                event
            }
            // Nothing to compare with: the next read that has pull requests starts silently.
            _ => {
                *previous = None;
                None
            }
        }
    };
    emit(&app, IntegrationUpdate { id: ID, data: card_data(login.as_deref(), snapshot.as_ref(), note, counts), error: None, event });
}

/// A link of the GitHub card: opened only when it is https on github.com.
pub fn open_link(url: &str) -> bool {
    if !link_allowed(url) {
        return false;
    }
    crate::platform::open_url(url);
    true
}

/// Opens the installation page of Mayak's GitHub App; false while its slug is not set.
pub fn open_install_page() -> bool {
    install_url(oauth_clients::GITHUB_APP_SLUG).is_some_and(|url| open_link(&url))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures shaped as docs.github.com/en/graphql (PullRequest, StatusCheckRollup, search
    // results, the errors array of a refused field).
    fn answer() -> Value {
        json!({
            "data": {
                "viewer": {
                    "login": "octocat",
                    "pullRequests": {
                        "totalCount": 3,
                        "nodes": [
                            { "title": "Fix the build", "url": "https://github.com/octo-org/hello-world/pull/12", "number": 12,
                              "isDraft": false, "reviewDecision": "REVIEW_REQUIRED",
                              "repository": { "nameWithOwner": "octo-org/hello-world" },
                              "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "FAILURE" } } } ] } },
                            { "title": "WIP: new parser", "url": "https://github.com/octocat/spoon-knife/pull/3", "number": 3,
                              "isDraft": true, "reviewDecision": null,
                              "repository": { "nameWithOwner": "octocat/spoon-knife" },
                              "commits": { "nodes": [ { "commit": { "statusCheckRollup": null } } ] } },
                            { "title": "Docs", "url": "http://evil.example.com/pull/9", "number": 9,
                              "isDraft": false, "reviewDecision": "APPROVED",
                              "repository": { "nameWithOwner": "octocat/docs" },
                              "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "PENDING" } } } ] } }
                        ]
                    }
                },
                "search": {
                    "issueCount": 7,
                    "nodes": [
                        { "title": "Add a cache", "url": "https://github.com/octo-org/api/pull/41", "number": 41,
                          "repository": { "nameWithOwner": "octo-org/api" }, "author": { "login": "monalisa" } },
                        {}
                    ]
                }
            }
        })
    }

    #[test]
    fn the_query_asks_for_what_the_card_reads() {
        for part in [
            "viewer { login",
            "pullRequests(states: OPEN, first: 20, orderBy: {field: UPDATED_AT, direction: DESC})",
            "statusCheckRollup { state }",
            "review-requested:@me",
            "archived:false",
            "type: ISSUE",
            "issueCount",
            "... on PullRequest",
        ] {
            assert!(QUERY.contains(part), "{part}");
        }
        assert_eq!(QUERY.matches('{').count(), QUERY.matches('}').count());
    }

    #[test]
    fn pull_requests_parse_with_drafts_and_without_ci() {
        let a = parse_graphql(&answer());
        assert_eq!(a.login.as_deref(), Some("octocat"));
        assert_eq!(a.problems, Problems::default());
        let mine = a.mine.unwrap();
        assert_eq!((mine.len(), a.mine_total), (3, 3));
        assert_eq!(mine[0].reference(), "octo-org/hello-world#12");
        assert_eq!(mine[0].ci, Some(Ci::Failed));
        assert_eq!(mine[0].url.as_deref(), Some("https://github.com/octo-org/hello-world/pull/12"));
        assert!(mine[1].draft);
        assert_eq!(mine[1].ci, None, "a null statusCheckRollup is no CI");
        assert_eq!(mine[2].ci, Some(Ci::Pending));
        assert_eq!(mine[2].url, None, "a link off github.com is dropped");
        let reviews = a.reviews.unwrap();
        assert_eq!((reviews.len(), a.reviews_total), (1, 7), "an empty search node (not a pull request) is skipped");
        assert_eq!(reviews[0].reference(), "octo-org/api#41");
        assert_eq!(outcome(&parse_graphql(&answer())), Outcome::Data { note: false });
    }

    #[test]
    fn an_empty_answer_is_data_with_nothing_in_it() {
        let v = json!({ "data": {
            "viewer": { "login": "octocat", "pullRequests": { "totalCount": 0, "nodes": [] } },
            "search": { "issueCount": 0, "nodes": [] }
        } });
        let a = parse_graphql(&v);
        assert_eq!((a.mine.as_deref(), a.reviews.as_deref()), (Some(&[][..]), Some(&[][..])));
        assert_eq!(outcome(&a), Outcome::Data { note: false });
    }

    #[test]
    fn states_map_to_the_three_ci_buckets() {
        assert_eq!(Ci::of("FAILURE"), Some(Ci::Failed));
        assert_eq!(Ci::of("ERROR"), Some(Ci::Failed));
        assert_eq!(Ci::of("PENDING"), Some(Ci::Pending));
        assert_eq!(Ci::of("EXPECTED"), Some(Ci::Pending));
        assert_eq!(Ci::of("SUCCESS"), Some(Ci::Success));
        assert_eq!(Ci::of("success"), None);
    }

    #[test]
    fn a_refused_field_keeps_the_rest_and_sets_the_note() {
        // The CI of a repository the token may not read (Checks / Commit statuses not granted).
        let mut v = answer();
        v["data"]["viewer"]["pullRequests"]["nodes"][0]["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = Value::Null;
        v["errors"] = json!([{
            "type": "FORBIDDEN",
            "path": ["viewer", "pullRequests", "nodes", 0, "commits", "nodes", 0, "commit", "statusCheckRollup"],
            "locations": [{ "line": 1, "column": 200 }],
            "message": "Resource not accessible by integration"
        }]);
        let a = parse_graphql(&v);
        assert!(a.problems.forbidden);
        assert_eq!(a.mine.as_ref().unwrap()[0].ci, None);
        assert_eq!(outcome(&a), Outcome::Data { note: true });

        // The whole viewer list refused (a fine-grained token without Pull requests).
        let refused = json!({
            "data": { "viewer": { "login": "octocat", "pullRequests": null }, "search": { "issueCount": 0, "nodes": [] } },
            "errors": [{ "path": ["viewer", "pullRequests"], "message": "Resource not accessible by personal access token" }]
        });
        let a = parse_graphql(&refused);
        assert_eq!(a.mine, None);
        assert_eq!(outcome(&a), Outcome::Data { note: true });

        // Nothing at all.
        let none = json!({ "data": null, "errors": [{ "type": "FORBIDDEN", "message": "Resource not accessible by integration" }] });
        assert_eq!(outcome(&parse_graphql(&none)), Outcome::NoAccess);
    }

    #[test]
    fn graphql_errors_become_kinds_without_their_messages() {
        let limited = json!({ "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded for user ID 1." }] });
        assert_eq!(outcome(&parse_graphql(&limited)), Outcome::RateLimited);
        let broken = json!({ "errors": [{ "extensions": { "code": "undefinedField <b>" }, "message": "Field 'x' doesn't exist on type 'secret/repo'" }] });
        let a = parse_graphql(&broken);
        assert_eq!(outcome(&a), Outcome::Failed("undefinedFieldb".into()), "only letters, digits and _ of the code are kept");
        let untyped = json!({ "errors": [{ "message": "Something went wrong in octo-org/private" }] });
        assert_eq!(outcome(&parse_graphql(&untyped)), Outcome::Failed("UNKNOWN".into()));
        let not_found = json!({ "data": { "viewer": null, "search": null }, "errors": [{ "type": "NOT_FOUND", "message": "x" }] });
        let Outcome::Failed(kind) = outcome(&parse_graphql(&not_found)) else { panic!() };
        assert_eq!(kind, "NOT_FOUND");
        assert!(!format!("{:?}", parse_graphql(&untyped)).contains("octo-org/private"));
    }

    #[test]
    fn rate_limits_wait_for_the_reset() {
        let now = 1_700_000_000_000;
        // Primary limit: 403 (REST) or 200 (GraphQL) with remaining 0, until the reset.
        let reset = "1700000600";
        assert_eq!(backoff_until(403, Some("0"), Some(reset), None, false, now), Some(1_700_000_600_000));
        assert_eq!(backoff_until(200, Some("0"), Some(reset), None, false, now), Some(1_700_000_600_000));
        // Secondary limit: retry-after wins.
        assert_eq!(backoff_until(403, Some("12"), Some(reset), Some("30"), false, now), Some(now + 30_000));
        assert_eq!(backoff_until(429, None, None, Some("5"), false, now), Some(now + 5_000));
        // No headers: one minute.
        assert_eq!(backoff_until(429, None, None, None, false, now), Some(now + 60_000));
        assert_eq!(backoff_until(200, None, None, None, true, now), Some(now + 60_000));
        // A 403 that is not a limit is a permission problem.
        assert_eq!(backoff_until(403, Some("4999"), Some(reset), None, false, now), None);
        assert_eq!(backoff_until(200, Some("4999"), Some(reset), None, false, now), None);
        assert_eq!(backoff_until(401, Some("x"), None, None, false, now), None);
        // A reset in the past or far away is clamped.
        assert_eq!(backoff_until(403, Some("0"), Some("1"), None, false, now), Some(now + 1_000));
        assert_eq!(backoff_until(403, Some("0"), Some("9999999999"), None, false, now), Some(now + 3_600_000));
        assert!(rate_limit_error(now + 90_000, now).contains("2 min"));
    }

    fn pr(repo: &str, number: u64, title: &str, ci: Option<Ci>) -> PullRequest {
        PullRequest { repo: repo.into(), number, title: title.into(), url: None, draft: false, ci }
    }

    fn snap(mine: Vec<PullRequest>, reviews: Vec<PullRequest>) -> Snapshot {
        Snapshot { mine_total: mine.len() as u64, reviews_total: reviews.len() as u64, mine: Some(mine), reviews: Some(reviews) }
    }

    #[test]
    fn the_first_read_is_silent_then_review_requests_and_ci_turns_fire_once() {
        let first = snap(
            vec![pr("o/a", 1, "Mine", Some(Ci::Pending)), pr("o/a", 2, "Green", Some(Ci::Success))],
            vec![pr("o/b", 5, "Old review", None)],
        );
        assert!(changes(None, &first).is_empty());
        assert!(changes(Some(&first), &first).is_empty());

        let second = snap(
            vec![
                pr("o/a", 1, "Mine", Some(Ci::Failed)),
                pr("o/a", 2, "Green", Some(Ci::Success)),
                pr("o/a", 3, "New, already red", Some(Ci::Failed)),
            ],
            vec![pr("o/b", 5, "Old review", None), pr("o/b", 6, "Please review", None)],
        );
        assert_eq!(
            changes(Some(&first), &second),
            vec![
                Change::ReviewRequested { title: "Please review".into(), reference: "o/b#6".into() },
                Change::MineFailed { title: "Mine".into(), reference: "o/a#1".into() },
            ]
        );
        assert!(changes(Some(&second), &second).is_empty(), "the same read again fires nothing");

        let third = snap(
            vec![pr("o/a", 1, "Mine", Some(Ci::Success)), pr("o/a", 2, "Green", None), pr("o/a", 3, "x", Some(Ci::Pending))],
            vec![],
        );
        assert_eq!(
            changes(Some(&second), &third),
            vec![Change::MinePassed { title: "Mine".into(), reference: "o/a#1".into() }],
            "a CI not read this time, or turning pending, is no event"
        );

        // A list refused on one read compares with nothing.
        let refused = Snapshot { reviews: None, ..second.clone() };
        assert!(changes(Some(&refused), &second).is_empty());
        assert!(changes(Some(&second), &refused).is_empty());

        let e = gitlab::event_of(&changes(Some(&first), &second)).unwrap();
        assert!(!e.success, "a red CI comes first");
        assert_eq!((e.label.as_str(), e.detail.as_deref()), ("Mine", Some("o/a#1 (+1)")));
    }

    #[test]
    fn the_card_data_counts_ci_and_carries_only_github_links() {
        let a = parse_graphql(&answer());
        let s = Snapshot::of(&a);
        let v = card_data(a.login.as_deref(), Some(&s), false, Counts { repos: 4, stars: 10 });
        assert_eq!(v["login"], "octocat");
        assert_eq!((v["totalRepos"].clone(), v["totalStars"].clone()), (json!(4), json!(10)));
        assert_eq!(v["ci"], json!({ "failed": 1, "running": 1, "success": 0 }));
        assert_eq!((v["mineTotal"].clone(), v["reviewsTotal"].clone()), (json!(3), json!(7)));
        assert_eq!(v["mine"][0]["ci"], "failed");
        assert_eq!(v["mine"][1]["draft"], true);
        assert_eq!(v["mine"][2]["url"], Value::Null);
        assert_eq!(v["reviews"][0]["reference"], "octo-org/api#41");
        assert_eq!(v["note"], Value::Null);

        let without = card_data(Some("octocat"), None, true, Counts { repos: 1, stars: 0 });
        assert_eq!(without["note"], "permissions");
        assert!(without.get("mine").is_none() && without.get("ci").is_none(), "no pull request data, no counts");
        assert_eq!(without["totalRepos"], 1);
    }

    #[test]
    fn the_install_page_is_github_com_apps_slug_installations_new() {
        assert_eq!(install_url("mayak").as_deref(), Some("https://github.com/apps/mayak/installations/new"));
        assert_eq!(install_url("mayak-app-2").as_deref(), Some("https://github.com/apps/mayak-app-2/installations/new"));
        for bad in ["", "Mayak", "mayak/../x", "mayak?x=1", "may ak", "майак", "a.b", "a_b", &"a".repeat(101)] {
            assert_eq!(install_url(bad), None, "{bad:?}");
        }
        assert!(link_allowed(&install_url("mayak").unwrap()));
        if oauth_clients::GITHUB_APP_SLUG.is_empty() {
            assert_eq!(install_url(oauth_clients::GITHUB_APP_SLUG), None, "no slug, no button");
        } else {
            assert!(valid_slug(oauth_clients::GITHUB_APP_SLUG), "the registered slug is valid");
        }
    }

    #[test]
    fn only_https_github_links_open() {
        assert!(link_allowed("https://github.com/octo-org/api/pull/41"));
        assert!(!link_allowed("http://github.com/octo-org/api/pull/41"));
        assert!(!link_allowed("https://github.com.evil.io/x"));
        assert!(!link_allowed("https://gist.github.com/x"));
        assert!(!link_allowed("https://user:pw@github.com/x"));
        assert!(!link_allowed("javascript:alert(1)"));
    }

    #[test]
    fn refused_requests_say_what_to_do() {
        assert!(status_error(401).contains("connect GitHub again"));
        assert_eq!(status_error(500), "API error 500");
        assert_eq!(token_error(&TokenProblem::Missing), None);
        assert!(token_error(&TokenProblem::Reconnect).unwrap().contains("reconnecting"));
    }
}
