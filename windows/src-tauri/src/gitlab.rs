// The GitLab integration: gitlab.com or a self-managed server at the address set in Settings.
//
// Every 5 min while the pill is switched on and Mayak is not paused (integrations::spawn), with the
// token from Connect or the token field (oauth::usable_token: refreshed before it expires, never
// sent to another host than the one it was saved for). One poll is at most 14 GET requests, all
// to that host, none following a redirect:
//   1. GET /merge_requests?scope=assigned_to_me&state=opened  (my merge requests)
//   2. GET /merge_requests?scope=reviews_for_me&state=opened  (review requested)
//   3. GET /issues?scope=assigned_to_me&state=opened          (my issues)
//   4. GET /projects/:id/merge_requests/:iid for the MR_PIPELINES most recently updated of (1)
//      (the head pipeline is only in the single merge request)
//   5. GET /projects?membership=true&order_by=last_activity_at (the RECENT_PROJECTS latest)
//   6. GET /projects/:id/pipelines?ref=<default branch>&per_page=1 for each of them
// Events fire only on a change against the previous read; the first read after start, a new token
// or a new address fills the card silently. Nothing is logged but status codes.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::integrations::{emit, IntegrationEvent, IntegrationUpdate};
use crate::log;
use crate::oauth::{self, Service, TokenProblem};

pub const ID: &str = "integration_gitlab";
pub const DEFAULT_BASE: &str = "https://gitlab.com";
const PER_PAGE: &str = "20";
const ISSUES_PER_PAGE: &str = "5";
const MR_PIPELINES: usize = 5;
const RECENT_PROJECTS: usize = 5;

// ── The address ──────────────────────────────────────────────────────────────

/// Why an address is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadAddress;

/// GitLab's address as `https://host[:port]`, lower case, without a trailing slash. Only an https
/// address with a host and nothing else: no user or password, no path, query or fragment, no
/// whitespace or backslash anywhere.
pub fn normalize_base(input: &str) -> Result<String, BadAddress> {
    let raw = input.trim();
    if raw.is_empty()
        || raw.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        || !raw.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        return Err(BadAddress);
    }
    let url = reqwest::Url::parse(raw).map_err(|_| BadAddress)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || raw.contains('@')
    {
        return Err(BadAddress);
    }
    let host = url.host_str().filter(|h| !h.is_empty()).ok_or(BadAddress)?;
    Ok(match url.port() {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    })
}

/// True for an https link on exactly the host (and port) of `base`, with no user or password: the
/// only links the GitLab card and Connect open.
pub fn link_allowed(base: &str, link: &str) -> bool {
    let (Ok(base), Ok(link)) = (reqwest::Url::parse(base), reqwest::Url::parse(link)) else {
        return false;
    };
    link.scheme() == "https"
        && base.scheme() == "https"
        && link.username().is_empty()
        && link.password().is_none()
        && link.host_str().is_some()
        && link.host_str() == base.host_str()
        && link.port_or_known_default() == base.port_or_known_default()
}

// ── Answers (pure) ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergeRequest {
    pub project_id: u64,
    pub iid: u64,
    pub title: String,
    /// `group/project!12`.
    pub reference: String,
    pub web_url: Option<String>,
    /// Only for the MR_PIPELINES most recent of mine; None when not read or there is none.
    pub pipeline: Option<Pipeline>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub title: String,
    pub reference: String,
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: u64,
    pub path: String,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BranchPipeline {
    pub project_id: u64,
    pub project: String,
    pub branch: String,
    pub pipeline: Pipeline,
}

/// A link kept only when it may be opened (`link_allowed`).
fn link(base: &str, v: &Value) -> Option<String> {
    v.get("web_url").and_then(Value::as_str).filter(|u| link_allowed(base, u)).map(str::to_string)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn full_reference(v: &Value) -> Option<String> {
    v.get("references").and_then(|r| r.get("full")).and_then(Value::as_str).map(str::to_string)
}

pub fn parse_merge_requests(base: &str, v: &Value) -> Vec<MergeRequest> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    Some(MergeRequest {
                        project_id: m.get("project_id")?.as_u64()?,
                        iid: m.get("iid")?.as_u64()?,
                        title: str_of(m, "title")?,
                        reference: full_reference(m).unwrap_or_default(),
                        web_url: link(base, m),
                        pipeline: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_pipeline(base: &str, v: &Value) -> Option<Pipeline> {
    Some(Pipeline { id: v.get("id")?.as_u64()?, status: str_of(v, "status")?, web_url: link(base, v) })
}

/// The `head_pipeline` of a single merge request; None when there is none (or it is hidden).
pub fn parse_head_pipeline(base: &str, v: &Value) -> Option<Pipeline> {
    parse_pipeline(base, v.get("head_pipeline")?)
}

/// The newest pipeline of a `GET /projects/:id/pipelines` page.
pub fn parse_latest_pipeline(base: &str, v: &Value) -> Option<Pipeline> {
    parse_pipeline(base, v.as_array()?.first()?)
}

pub fn parse_issues(base: &str, v: &Value) -> Vec<Issue> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|i| {
                    Some(Issue {
                        title: str_of(i, "title")?,
                        reference: full_reference(i).unwrap_or_default(),
                        web_url: link(base, i),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_projects(v: &Value) -> Vec<Project> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    Some(Project {
                        id: p.get("id")?.as_u64()?,
                        path: str_of(p, "path_with_namespace").or_else(|| str_of(p, "name"))?,
                        default_branch: str_of(p, "default_branch").filter(|b| !b.is_empty()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `x-total` header when GitLab sends it (it leaves it out past 10 000), else what was listed.
pub fn total(header: Option<&str>, listed: usize) -> u64 {
    header.and_then(|h| h.trim().parse().ok()).unwrap_or(listed as u64)
}

// ── One read and what changed (pure) ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub mine: Vec<MergeRequest>,
    pub mine_total: u64,
    pub reviews: Vec<MergeRequest>,
    pub reviews_total: u64,
    pub issues: Vec<Issue>,
    pub issues_total: u64,
    pub branches: Vec<BranchPipeline>,
}

/// A pipeline status in one of the card's three counts.
pub fn bucket(status: &str) -> Option<&'static str> {
    match status {
        "failed" => Some("failed"),
        "success" => Some("success"),
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" | "waiting_for_callback"
        | "scheduled" | "canceling" => Some("running"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    ReviewRequested { title: String, reference: String },
    MineFailed { title: String, reference: String },
    MinePassed { title: String, reference: String },
    BranchFailed { project: String, branch: String },
}

/// What changed from `prev` to `next`; nothing on the first read (`prev` None). A review request
/// is a merge request new on the review list; a pipeline of mine fires when its status turns
/// failed or success (the merge request and a pipeline status known on both reads); a default
/// branch fires when its latest pipeline turns failed.
pub fn changes(prev: Option<&Snapshot>, next: &Snapshot) -> Vec<Change> {
    let Some(prev) = prev else { return Vec::new() };
    let mut out = Vec::new();
    for mr in &next.reviews {
        if !prev.reviews.iter().any(|p| p.project_id == mr.project_id && p.iid == mr.iid) {
            out.push(Change::ReviewRequested { title: mr.title.clone(), reference: mr.reference.clone() });
        }
    }
    for mr in &next.mine {
        let Some(now) = &mr.pipeline else { continue };
        let before = prev
            .mine
            .iter()
            .find(|p| p.project_id == mr.project_id && p.iid == mr.iid)
            .and_then(|p| p.pipeline.as_ref());
        let Some(before) = before else { continue };
        if before.status == now.status {
            continue;
        }
        let (title, reference) = (mr.title.clone(), mr.reference.clone());
        match now.status.as_str() {
            "failed" => out.push(Change::MineFailed { title, reference }),
            "success" => out.push(Change::MinePassed { title, reference }),
            _ => {}
        }
    }
    for b in &next.branches {
        let before = prev.branches.iter().find(|p| p.project_id == b.project_id && p.branch == b.branch);
        if let Some(before) = before {
            if b.pipeline.status == "failed" && before.pipeline.status != "failed" {
                out.push(Change::BranchFailed { project: b.project.clone(), branch: b.branch.clone() });
            }
        }
    }
    out
}

/// The pill's one event for a read: a failure first, then a review request, then a pipeline that
/// passed. The label is the merge request's title or the project's path, the detail its reference
/// or branch (data, so nothing to translate).
pub fn event_of(changes: &[Change]) -> Option<IntegrationEvent> {
    let rank = |c: &Change| match c {
        Change::BranchFailed { .. } | Change::MineFailed { .. } => 0,
        Change::ReviewRequested { .. } => 1,
        Change::MinePassed { .. } => 2,
    };
    let first = changes.iter().min_by_key(|c| rank(c))?;
    let more = changes.len() - 1;
    let with_more = |s: &str| if more > 0 { format!("{s} (+{more})") } else { s.to_string() };
    Some(match first {
        Change::BranchFailed { project, branch } => {
            IntegrationEvent { success: false, label: project.clone(), detail: Some(with_more(branch)) }
        }
        Change::MineFailed { title, reference } => {
            IntegrationEvent { success: false, label: title.clone(), detail: Some(with_more(reference)) }
        }
        Change::ReviewRequested { title, reference } | Change::MinePassed { title, reference } => {
            IntegrationEvent { success: true, label: title.clone(), detail: Some(with_more(reference)) }
        }
    })
}

/// What the island's card gets. Links are only ones `link_allowed` let through.
pub fn card_data(base: &str, s: &Snapshot) -> Value {
    let mr = |m: &MergeRequest| {
        json!({
            "title": m.title,
            "reference": m.reference,
            "webUrl": m.web_url,
            "pipeline": m.pipeline.as_ref().map(|p| p.status.clone()),
        })
    };
    let mut ci = HashMap::from([("failed", 0u32), ("running", 0), ("success", 0)]);
    for p in s.mine.iter().filter_map(|m| m.pipeline.as_ref()) {
        if let Some(b) = bucket(&p.status) {
            *ci.entry(b).or_default() += 1;
        }
    }
    json!({
        "host": base.trim_start_matches("https://"),
        "mine": s.mine.iter().take(5).map(mr).collect::<Vec<_>>(),
        "mineTotal": s.mine_total,
        "reviews": s.reviews.iter().take(5).map(mr).collect::<Vec<_>>(),
        "reviewsTotal": s.reviews_total,
        "ci": { "failed": ci["failed"], "running": ci["running"], "success": ci["success"] },
        "branches": s.branches.iter().map(|b| json!({
            "project": b.project,
            "branch": b.branch,
            "status": b.pipeline.status,
            "webUrl": b.pipeline.web_url,
        })).collect::<Vec<_>>(),
        "issues": s.issues.iter().take(3).map(|i| json!({
            "title": i.title,
            "reference": i.reference,
            "webUrl": i.web_url,
        })).collect::<Vec<_>>(),
        "issuesTotal": s.issues_total,
    })
}

/// The card's error line for a refused request (the integrations' status_error pattern).
pub fn status_error(code: u16) -> String {
    match code {
        401 => "Token rejected (401): connect GitLab again in Settings".into(),
        403 => "Token lacks the read_api scope (403)".into(),
        300..=399 => format!("GitLab answered with a redirect ({code}): check the address in Settings"),
        _ => format!("API error {code}"),
    }
}

pub fn token_error(problem: &TokenProblem) -> Option<String> {
    match problem {
        TokenProblem::Missing => None,
        TokenProblem::Reconnect => Some("Needs reconnecting: Settings → Integrations → Connect".into()),
        TokenProblem::OtherHost => Some("The token belongs to another GitLab address: connect again".into()),
        TokenProblem::RefreshFailed => Some("Could not refresh the token: no connection".into()),
    }
}

// ── The poller ───────────────────────────────────────────────────────────────

/// The previous read: (address, snapshot). A read for another address starts over silently.
static PREVIOUS: Mutex<Option<(String, Snapshot)>> = Mutex::new(None);

/// A new token, a disconnect or a new address: the next read fills the card without events.
pub fn forget() {
    *PREVIOUS.lock().unwrap() = None;
}

fn error(app: &AppHandle, text: String) {
    emit(app, IntegrationUpdate { id: ID, data: json!({}), error: Some(text), event: None });
}

struct Api<'a> {
    http: reqwest::Client,
    base: &'a str,
    token: String,
}

impl Api<'_> {
    /// One GET under /api/v4; Err(status) for a refused one, Err(0) when not reached.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<(Value, Option<String>), u16> {
        let response = self
            .http
            .get(format!("{}/api/v4{path}", self.base))
            .query(query)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| 0u16)?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(status);
        }
        let total = response.headers().get("x-total").and_then(|h| h.to_str().ok()).map(str::to_string);
        let body = response.json::<Value>().await.map_err(|_| 0u16)?;
        Ok((body, total))
    }
}

pub async fn poll(app: AppHandle) {
    let Some(base) = oauth::host_of(&app, Service::GitLab) else {
        error(&app, "Invalid GitLab address: check it in Settings".into());
        return;
    };
    let token = match oauth::usable_token(Service::GitLab, &base).await {
        Ok(token) => token,
        Err(problem) => {
            if let Some(text) = token_error(&problem) {
                error(&app, text);
            }
            return;
        }
    };
    let api = Api { http: oauth::http(), base: &base, token };

    let list = |scope: &'static str| {
        [("scope", scope), ("state", "opened"), ("order_by", "updated_at"), ("sort", "desc"), ("per_page", PER_PAGE)]
    };
    // The three lists must all be read: a missing one would look like everything on it changed.
    let lists = async {
        let mine = api.get("/merge_requests", &list("assigned_to_me")).await?;
        let reviews = api.get("/merge_requests", &list("reviews_for_me")).await?;
        let mut issues_query = list("assigned_to_me");
        issues_query[4] = ("per_page", ISSUES_PER_PAGE);
        let issues = api.get("/issues", &issues_query).await?;
        Ok::<_, u16>((mine, reviews, issues))
    };
    let ((mine_v, mine_total), (reviews_v, reviews_total), (issues_v, issues_total)) = match lists.await {
        Ok(lists) => lists,
        Err(0) => {
            error(&app, "No connection".into());
            return;
        }
        Err(code) => {
            log::line(format!("gitlab HTTP {code}"));
            error(&app, status_error(code));
            return;
        }
    };

    let mut snapshot = Snapshot::default();
    snapshot.mine = parse_merge_requests(&base, &mine_v);
    snapshot.mine_total = total(mine_total.as_deref(), snapshot.mine.len());
    snapshot.reviews = parse_merge_requests(&base, &reviews_v);
    snapshot.reviews_total = total(reviews_total.as_deref(), snapshot.reviews.len());
    snapshot.issues = parse_issues(&base, &issues_v);
    snapshot.issues_total = total(issues_total.as_deref(), snapshot.issues.len());

    // Head pipelines of my most recently updated merge requests (the list is sorted that way).
    for mr in snapshot.mine.iter_mut().take(MR_PIPELINES) {
        let path = format!("/projects/{}/merge_requests/{}", mr.project_id, mr.iid);
        if let Ok((v, _)) = api.get(&path, &[]).await {
            mr.pipeline = parse_head_pipeline(&base, &v);
        }
    }

    // The latest default-branch pipeline of my most recently active projects.
    let per_projects = RECENT_PROJECTS.to_string();
    let projects = api
        .get(
            "/projects",
            &[
                ("membership", "true"),
                ("archived", "false"),
                ("simple", "true"),
                ("order_by", "last_activity_at"),
                ("sort", "desc"),
                ("per_page", per_projects.as_str()),
            ],
        )
        .await
        .map(|(v, _)| parse_projects(&v))
        .unwrap_or_default();
    for project in projects.into_iter().take(RECENT_PROJECTS) {
        let Some(branch) = project.default_branch.clone() else { continue };
        let path = format!("/projects/{}/pipelines", project.id);
        if let Ok((v, _)) = api.get(&path, &[("ref", branch.as_str()), ("per_page", "1")]).await {
            if let Some(pipeline) = parse_latest_pipeline(&base, &v) {
                snapshot.branches.push(BranchPipeline { project_id: project.id, project: project.path, branch, pipeline });
            }
        }
    }

    let event = {
        let mut previous = PREVIOUS.lock().unwrap();
        let prev = previous.as_ref().filter(|(host, _)| *host == base).map(|(_, s)| s);
        let event = event_of(&changes(prev, &snapshot));
        *previous = Some((base.clone(), snapshot.clone()));
        event
    };
    emit(&app, IntegrationUpdate { id: ID, data: card_data(&base, &snapshot), error: None, event });
}

/// GitLab's «new personal access token» page on `base` (a `normalize_base` address), prefilled
/// through its documented URL parameters: the name Mayak and the scope read_api. Connect's way in
/// when GitLab does not let Mayak's application sign in by code.
pub fn token_page(base: &str) -> String {
    format!("{base}/-/user_settings/personal_access_tokens?name=Mayak&scopes=read_api")
}

/// Opens `token_page` on the configured GitLab address; false when the address is not valid.
pub fn open_token_page(app: &AppHandle) -> bool {
    let Some(base) = oauth::host_of(app, Service::GitLab) else { return false };
    open_link(app, &token_page(&base))
}

/// The GitLab card's links: opened only when they are https on the configured host.
pub fn open_link(app: &AppHandle, url: &str) -> bool {
    let Some(base) = oauth::host_of(app, Service::GitLab) else { return false };
    if !link_allowed(&base, url) {
        return false;
    }
    crate::platform::open_url(url);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://gitlab.example.com";

    #[test]
    fn addresses_are_https_hosts_only() {
        assert_eq!(normalize_base("https://gitlab.com"), Ok("https://gitlab.com".into()));
        assert_eq!(normalize_base("  https://git.example.com/ "), Ok("https://git.example.com".into()));
        assert_eq!(normalize_base("HTTPS://Git.Example.COM/"), Ok("https://git.example.com".into()));
        assert_eq!(normalize_base("https://git.example.com:8443"), Ok("https://git.example.com:8443".into()));
        assert_eq!(normalize_base("https://git.example.com:443/"), Ok("https://git.example.com".into()));
        for bad in [
            "",
            "   ",
            "http://gitlab.com",
            "gitlab.com",
            "https://user:pass@gitlab.com",
            "https://user@gitlab.com",
            "https://evil.com\\@gitlab.com",
            "javascript:alert(1)",
            "https://gitlab.com/?x=1",
            "https://gitlab.com?x=1",
            "https://gitlab.com/#top",
            "https://gitlab.com/group/project",
            "https://gitlab .com",
            "https://",
            "ftp://gitlab.com",
            "file:///C:/x",
        ] {
            assert_eq!(normalize_base(bad), Err(BadAddress), "{bad:?}");
        }
    }

    #[test]
    fn only_https_links_on_the_configured_host_may_open() {
        assert!(link_allowed(BASE, "https://gitlab.example.com/group/p/-/merge_requests/1"));
        assert!(link_allowed(BASE, "https://GITLAB.example.com/x"));
        assert!(!link_allowed(BASE, "http://gitlab.example.com/group/p/-/merge_requests/1"));
        assert!(!link_allowed(BASE, "https://gitlab.example.com.evil.io/x"));
        assert!(!link_allowed(BASE, "https://evil.io/gitlab.example.com"));
        assert!(!link_allowed(BASE, "https://user:pw@gitlab.example.com/x"));
        assert!(!link_allowed(BASE, "https://gitlab.example.com:8443/x"));
        assert!(link_allowed("https://gitlab.example.com:8443", "https://gitlab.example.com:8443/x"));
        assert!(!link_allowed(BASE, "javascript:alert(1)"));
        assert!(!link_allowed(BASE, "file:///C:/Windows"));
        assert!(!link_allowed(BASE, ""));
        assert!(!link_allowed("http://gitlab.example.com", "http://gitlab.example.com/x"));
    }

    // Fixtures shaped as the examples of docs.gitlab.com/api (merge_requests, pipelines, issues,
    // projects), on the https host the tests configure.
    fn mr_list() -> Value {
        serde_json::from_str(
            r#"[
              { "id": 1, "iid": 1, "project_id": 3, "title": "test1", "state": "opened",
                "web_url": "https://gitlab.example.com/my-group/my-project/merge_requests/1",
                "references": { "short": "!1", "relative": "my-group/my-project!1", "full": "my-group/my-project!1" } },
              { "id": 2, "iid": 7, "project_id": 4, "title": "Fix the build", "state": "opened",
                "web_url": "http://elsewhere.example.com/x/merge_requests/7",
                "references": { "short": "!7", "relative": "x!7", "full": "x!7" } },
              { "id": 3, "project_id": 4, "title": "no iid" }
            ]"#,
        )
        .unwrap()
    }

    fn single_mr(status: &str) -> Value {
        json!({
            "id": 1, "iid": 1, "project_id": 3, "title": "test1",
            "head_pipeline": {
                "id": 77, "iid": 3, "project_id": 3, "sha": "2be7ddb7", "ref": "test1", "status": status,
                "source": "push", "web_url": "https://gitlab.example.com/my-group/my-project/-/pipelines/77"
            }
        })
    }

    #[test]
    fn merge_requests_parse_and_keep_only_allowed_links() {
        let list = parse_merge_requests(BASE, &mr_list());
        assert_eq!(list.len(), 2, "the entry without an iid is skipped");
        assert_eq!((list[0].project_id, list[0].iid), (3, 1));
        assert_eq!(list[0].reference, "my-group/my-project!1");
        assert_eq!(list[0].web_url.as_deref(), Some("https://gitlab.example.com/my-group/my-project/merge_requests/1"));
        assert_eq!(list[1].web_url, None, "an http link on another host is dropped");
        assert!(parse_merge_requests(BASE, &json!({ "message": "401 Unauthorized" })).is_empty());
    }

    #[test]
    fn pipelines_parse() {
        let head = parse_head_pipeline(BASE, &single_mr("failed")).unwrap();
        assert_eq!((head.id, head.status.as_str()), (77, "failed"));
        assert!(head.web_url.is_some());
        assert_eq!(parse_head_pipeline(BASE, &json!({ "id": 1, "head_pipeline": null })), None);
        let list = json!([
            { "id": 47, "iid": 12, "project_id": 1, "status": "pending", "source": "push", "ref": "new-pipeline",
              "sha": "a91957a858320c0e17f3a0eca7cfacbff50ea29a", "web_url": "https://example.com/foo/bar/pipelines/47",
              "created_at": "2016-08-11T11:28:34.085Z", "updated_at": "2016-08-11T11:32:35.169Z" },
            { "id": 48, "status": "success" }
        ]);
        let latest = parse_latest_pipeline(BASE, &list).unwrap();
        assert_eq!((latest.id, latest.status.as_str()), (47, "pending"));
        assert_eq!(latest.web_url, None, "example.com is not the configured host");
        assert_eq!(parse_latest_pipeline(BASE, &json!([])), None);
        assert_eq!(bucket("pending"), Some("running"));
        assert_eq!(bucket("manual"), None);
    }

    #[test]
    fn issues_and_projects_parse() {
        let issues = parse_issues(
            BASE,
            &json!([{ "state": "opened", "id": 76, "title": "Consequatur vero maxime deserunt laboriosam est voluptas dolorem.",
                      "iid": 6, "project_id": 1, "web_url": "https://gitlab.example.com/my-group/my-project/issues/6",
                      "references": { "short": "#6", "relative": "my-group/my-project#6", "full": "my-group/my-project#6" } }]),
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].reference, "my-group/my-project#6");
        assert!(issues[0].web_url.is_some());
        let projects = parse_projects(&json!([
            { "id": 4, "name": "Diaspora Client", "path_with_namespace": "diaspora/diaspora-client", "default_branch": "main",
              "web_url": "https://gitlab.example.com/diaspora/diaspora-client", "last_activity_at": "2013-09-30T13:46:02Z" },
            { "id": 5, "name": "Empty", "path_with_namespace": "me/empty", "default_branch": null }
        ]));
        assert_eq!(projects[0], Project { id: 4, path: "diaspora/diaspora-client".into(), default_branch: Some("main".into()) });
        assert_eq!(projects[1].default_branch, None);
        assert_eq!(total(Some("42"), 5), 42);
        assert_eq!(total(None, 5), 5);
        assert_eq!(total(Some("x"), 2), 2);
    }

    fn mr(project_id: u64, iid: u64, title: &str, status: Option<&str>) -> MergeRequest {
        MergeRequest {
            project_id,
            iid,
            title: title.into(),
            reference: format!("p{project_id}!{iid}"),
            web_url: None,
            pipeline: status.map(|s| Pipeline { id: iid * 10, status: s.into(), web_url: None }),
        }
    }

    fn branch(project_id: u64, id: u64, status: &str) -> BranchPipeline {
        BranchPipeline {
            project_id,
            project: format!("group/p{project_id}"),
            branch: "main".into(),
            pipeline: Pipeline { id, status: status.into(), web_url: None },
        }
    }

    #[test]
    fn the_first_read_is_silent_then_changes_fire_once() {
        let first = Snapshot {
            mine: vec![mr(1, 1, "Mine", Some("running")), mr(1, 2, "Other", Some("success"))],
            reviews: vec![mr(2, 5, "Old review", None)],
            branches: vec![branch(1, 100, "success"), branch(2, 200, "failed")],
            ..Snapshot::default()
        };
        assert!(changes(None, &first).is_empty());
        assert!(changes(Some(&first), &first).is_empty(), "nothing changed, nothing fires");

        let second = Snapshot {
            mine: vec![
                mr(1, 1, "Mine", Some("failed")),
                mr(1, 2, "Other", Some("success")),
                mr(1, 3, "New, already failed", Some("failed")),
            ],
            reviews: vec![mr(2, 5, "Old review", None), mr(2, 6, "Please review", None)],
            branches: vec![branch(1, 101, "failed"), branch(2, 201, "failed"), branch(3, 300, "failed")],
            ..Snapshot::default()
        };
        let got = changes(Some(&first), &second);
        assert_eq!(
            got,
            vec![
                Change::ReviewRequested { title: "Please review".into(), reference: "p2!6".into() },
                Change::MineFailed { title: "Mine".into(), reference: "p1!1".into() },
                Change::BranchFailed { project: "group/p1".into(), branch: "main".into() },
            ],
            "a new MR, a branch still failing and a project not seen before stay quiet"
        );
        assert!(changes(Some(&second), &second).is_empty(), "the same read again fires nothing");

        let third = Snapshot {
            mine: vec![mr(1, 1, "Mine", Some("success")), mr(1, 2, "Other", None)],
            ..second.clone()
        };
        assert_eq!(
            changes(Some(&second), &third),
            vec![Change::MinePassed { title: "Mine".into(), reference: "p1!1".into() }],
            "a pipeline not read this time is no change"
        );
    }

    #[test]
    fn one_event_per_read_failures_first() {
        assert!(event_of(&[]).is_none());
        let passed = Change::MinePassed { title: "A".into(), reference: "p!1".into() };
        let review = Change::ReviewRequested { title: "B".into(), reference: "p!2".into() };
        let failed = Change::BranchFailed { project: "group/p".into(), branch: "main".into() };
        let e = event_of(&[passed.clone()]).unwrap();
        assert!(e.success);
        assert_eq!((e.label.as_str(), e.detail.as_deref()), ("A", Some("p!1")));
        let e = event_of(&[passed.clone(), review.clone()]).unwrap();
        assert_eq!((e.label.as_str(), e.detail.as_deref()), ("B", Some("p!2 (+1)")));
        let e = event_of(&[passed, review, failed]).unwrap();
        assert!(!e.success);
        assert_eq!((e.label.as_str(), e.detail.as_deref()), ("group/p", Some("main (+2)")));
    }

    #[test]
    fn the_card_data_counts_pipelines_and_carries_only_allowed_links() {
        let mut mine = parse_merge_requests(BASE, &mr_list());
        mine[0].pipeline = parse_head_pipeline(BASE, &single_mr("failed"));
        mine[1].pipeline = Some(Pipeline { id: 1, status: "running".into(), web_url: None });
        let s = Snapshot { mine, mine_total: 9, issues_total: 3, ..Snapshot::default() };
        let v = card_data(BASE, &s);
        assert_eq!(v["host"], "gitlab.example.com");
        assert_eq!(v["mineTotal"], 9);
        assert_eq!(v["ci"], json!({ "failed": 1, "running": 1, "success": 0 }));
        assert_eq!(v["mine"][0]["pipeline"], "failed");
        assert_eq!(v["mine"][1]["webUrl"], Value::Null);
        assert_eq!(v["issuesTotal"], 3);
    }

    #[test]
    fn the_token_page_is_https_on_the_configured_host_with_name_and_scopes_only() {
        for (input, expected) in [
            ("https://gitlab.com", "https://gitlab.com/-/user_settings/personal_access_tokens?name=Mayak&scopes=read_api"),
            (
                "https://Git.Example.com:8443/",
                "https://git.example.com:8443/-/user_settings/personal_access_tokens?name=Mayak&scopes=read_api",
            ),
        ] {
            let base = normalize_base(input).unwrap();
            let page = token_page(&base);
            assert_eq!(page, expected);
            assert!(link_allowed(&base, &page), "{page} may be opened");
            let url = reqwest::Url::parse(&page).unwrap();
            assert_eq!(url.scheme(), "https");
            assert_eq!(url.path(), "/-/user_settings/personal_access_tokens");
            let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
            assert_eq!(query, vec![("name".into(), "Mayak".into()), ("scopes".into(), "read_api".into())]);
        }
        // Never opened for another host than the configured one.
        assert!(!link_allowed("https://git.example.com", &token_page("https://gitlab.com")));
    }

    #[test]
    fn refused_requests_say_what_to_do() {
        assert!(status_error(401).contains("connect GitLab again"));
        assert!(status_error(403).contains("read_api"));
        assert!(status_error(302).contains("redirect"));
        assert_eq!(status_error(500), "API error 500");
        assert_eq!(token_error(&TokenProblem::Missing), None);
        assert!(token_error(&TokenProblem::Reconnect).unwrap().contains("reconnecting"));
    }
}
