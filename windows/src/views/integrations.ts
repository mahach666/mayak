// Integration cards — DOM ports of IntegrationCardView and friends from IslandViewContent.swift.
// The home page shows them as tiles (views/usage.ts, `tile`): GitLab moves its pipeline counts into
// the header and keeps two rows, Stripe one payment, GitHub shows its pull requests the same way as
// GitLab; the rest is the card as it was.
//
// Cal.com is the one simplification: macOS shows a three-level calendar
// (month → day → booking); here it is the list of upcoming bookings.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { State, type AgentTask, type PillBadge } from "../core/state";
import { Bridge } from "../core/bridge";
import { t, type Key } from "../core/i18n";
import { AO_ID, aoCounts, orderSessions, readAo, statusTone, statusWord, type AoStatus, type AoTone } from "../core/ao";
import { githubHasPullRequests, githubRows } from "./home-model";

/** Same shape as the Swift `timeAgo` computed properties. */
export function timeAgo(value: unknown): string {
  const date = typeof value === "number" ? new Date(value) : new Date(String(value));
  const diff = (Date.now() - date.getTime()) / 1000;
  if (!Number.isFinite(diff)) return "";
  if (diff < 60) return t("time.justNow");
  if (diff < 3600) return t("unit.m", { n: Math.floor(diff / 60) });
  if (diff < 86400) return t("unit.h", { n: Math.floor(diff / 3600) });
  return t("unit.d", { n: Math.floor(diff / 86400) });
}

function header(color: string, name: string, kind: string, extra?: Node): HTMLElement {
  const row = h("div", { class: "int-head" }, dot(color, 7), h("b", { text: name }), h("span", { text: kind }));
  if (extra) row.append(extra);
  return row;
}

/** Highlighted first row + plain rows, the layout every list card shares. */
function listRow(accent: string, first: boolean, ...children: Node[]): HTMLElement {
  const row = h("div", { class: first ? "int-row first" : "int-row" }, dot(accent, 5), ...children);
  if (first) row.style.background = `${accent}14`;
  return row;
}

function get(id: string): Record<string, unknown> {
  return (State.integrations[id]?.data ?? {}) as Record<string, unknown>;
}

function arr(id: string, key: string): Record<string, unknown>[] {
  const v = get(id)[key];
  return Array.isArray(v) ? (v as Record<string, unknown>[]) : [];
}

// ── Not configured / idle ─────────────────────────────────────────────────────

const OPEN_URLS: Record<string, string> = {
  integration_resend: "https://resend.com/emails",
  integration_vercel: "https://vercel.com/dashboard",
  integration_github: "https://github.com",
  integration_stripe: "https://dashboard.stripe.com/payments",
  integration_notion: "https://notion.so",
  integration_calcom: "https://app.cal.com/bookings",
};

function idleCard(task: AgentTask, openSettings: () => void): HTMLElement {
  const info = State.integrations[task.id];
  const configured = info?.configured ?? false;
  const error = info?.error ?? null;
  const label = error ?? t(configured ? "int.connectedLoading" : "int.keyMissing");
  const statusColor = error || !configured ? "#F4505E" : "#22C55E";

  const actions = h("div", { class: "int-actions" });
  if (task.id === "integration_n8n") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: t("int.open", { name: "n8n" }),
        onclick: () => void Bridge.openN8n(),
      }),
    );
  } else if (task.id === "integration_gitlab") {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: t("int.open", { name: task.name }),
        onclick: () => void Bridge.openGitlab(State.settings.gitlabUrl),
      }),
    );
  } else if (OPEN_URLS[task.id]) {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: t("int.open", { name: task.name }),
        onclick: () => void Bridge.openUrl(OPEN_URLS[task.id]),
      }),
    );
  }
  if (configured) {
    actions.append(
      h("button", {
        class: "link-btn",
        style: `color:${task.color}d9`,
        text: t("int.refresh"),
        onclick: () => void Bridge.refreshIntegration(task.id),
      }),
    );
  } else {
    actions.append(
      h("button", { class: "link-btn", style: "color:#8e939c", text: t("common.settingsMore"), onclick: openSettings }),
    );
  }

  return h(
    "div",
    { class: "int-card" },
    header(task.color, task.name, t("int.integration")),
    h("div", { class: "int-status" }, dot(statusColor, 5), h("span", { text: label })),
    actions,
  );
}

// ── Vercel ────────────────────────────────────────────────────────────────────

function vercelCard(onDetail: () => void): HTMLElement {
  const deployments = arr("integration_vercel", "deployments");
  const rows = h("div", { class: "int-rows" });
  deployments.slice(0, 3).forEach((d, i) => {
    const accent = d.state === "READY" ? "#22C55E" : "#F4505E";
    const name = h("span", { class: "int-name", text: String(d.projectName ?? "") });
    const ago = h("span", { class: "int-ago", text: timeAgo(d.createdAt) });
    if (i === 0) {
      const more = h(
        "button",
        { class: "int-more", title: t("int.details"), onclick: onDetail },
        svg(ICONS.ellipsis, 8),
      );
      rows.append(listRow(accent, true, name, ago, more));
    } else {
      rows.append(listRow(accent, false, name, ago));
    }
  });
  return h("div", { class: "int-card" }, header("#7C5CFF", "Vercel", t("int.deployments")), rows);
}

function vercelDetail(onBack: () => void): HTMLElement {
  const d = arr("integration_vercel", "deployments")[0] ?? {};
  const success = d.state === "READY";
  const accent = success ? "#22C55E" : "#F4505E";
  const status = t(success ? "int.ready" : d.state === "CANCELED" ? "int.canceled" : "int.error");
  const body = h("div", { class: "int-detail-body" });
  if (d.commitMessage) body.append(h("div", { class: "int-commit", text: String(d.commitMessage) }));
  const meta = h("div", { class: "int-meta" });
  if (d.branch) meta.append(h("span", { text: String(d.branch) }));
  meta.append(h("span", { text: t("time.ago", { time: timeAgo(d.createdAt) }) }));
  body.append(meta);
  if (d.url) {
    body.append(
      h("button", {
        class: "int-link",
        text: String(d.url),
        onclick: () => void Bridge.openUrl(`https://${d.url}`),
      }),
    );
  }
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: String(d.projectName ?? t("int.deployment")) }),
      h("span", { class: "int-badge", style: `color:${accent};background:${accent}24`, text: status }),
    ),
    body,
  );
}

// ── Resend ────────────────────────────────────────────────────────────────────

function resendCard(): HTMLElement {
  const emails = arr("integration_resend", "emails");
  const total = get("integration_resend").total;
  const extra =
    total != null
      ? h("span", { class: "int-total" }, h("i", { class: "pulse" }), h("span", { text: String(total) }))
      : undefined;
  const rows = h("div", { class: "int-rows" });
  emails.slice(0, 3).forEach((e, i) => {
    const delivered = e.lastEvent === "delivered";
    const accent = delivered ? "#22C55E" : "#F4505E";
    const to = Array.isArray(e.to) ? String(e.to[0] ?? "?") : "?";
    const short = to.split("@")[0];
    const cells: Node[] = [
      h("span", { class: "int-name", text: short }),
      h("span", { class: "int-ago", text: timeAgo(e.createdAt) }),
    ];
    if (i === 0 && e.subject) cells.push(h("span", { class: "int-sub", text: String(e.subject) }));
    rows.append(listRow(accent, i === 0, ...cells));
  });
  return h("div", { class: "int-card" }, header("#22C55E", "Resend", t("int.emails"), extra), rows);
}

// ── GitHub ────────────────────────────────────────────────────────────────────

function statRow(icon: string, color: string, label: string, value: string): HTMLElement {
  return h(
    "div",
    { class: "int-stat" },
    h("i", { class: "int-stat-icon", style: `color:${color}` }, svg(icon, 10)),
    h("span", { class: "int-stat-label", text: label }),
    h("span", { class: "int-stat-value", text: value }),
  );
}

/** The «needs permissions» line: the token may not read pull requests (github.rs card_data `note`). */
function githubNote(): HTMLElement {
  return h("div", { class: "int-status" }, dot("#F5A524", 5), h("span", { text: t("int.ghNeedsApp") }));
}

function githubCard(tile: boolean): HTMLElement {
  const d = get("integration_github");
  // The home tile shows the pull requests when the token could read them.
  if (tile && githubHasPullRequests(d)) return githubPullsCard(d);
  const stars = Number(d.totalStars ?? 0);
  const repos = Number(d.totalRepos ?? 0);
  const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  const card = h(
    "div",
    { class: "int-card" },
    header("#F4505E", "GitHub", t("int.overview")),
    h(
      "div",
      { class: "int-stats" },
      statRow(ICONS.star, "#F5A524", t("int.totalStars"), fmt(stars)),
      statRow(ICONS.stack, "#6B7079", t("int.repos"), String(repos)),
    ),
  );
  if (tile && d.note === "permissions") card.append(githubNote());
  return card;
}

/** The GitHub tile with pull requests: CI counts in the header, the counts line, two rows (as GitLab's tile). */
function githubPullsCard(d: Record<string, unknown>): HTMLElement {
  const ci = (d.ci ?? {}) as Record<string, unknown>;
  const count = (color: string, n: unknown, hot = false) =>
    h(
      "span",
      { class: hot && Number(n ?? 0) > 0 ? "int-ci hot" : "int-ci", style: "display:inline-flex;align-items:center;gap:3px" },
      dot(color, 5),
      h("span", { text: String(Number(n ?? 0)) }),
    );
  const counts = h("span", { class: "int-cis" }, count("#F4505E", ci.failed, true), count("#F5A524", ci.running), count("#22C55E", ci.success));
  const summary = h(
    "div",
    { class: "int-status" },
    h("span", { text: t("int.ghSummary", { mine: Number(d.mineTotal ?? 0), reviews: Number(d.reviewsTotal ?? 0) }) }),
  );
  const list = h("div", { class: "int-rows tight" });
  // The «needs permissions» line takes the second row's place: a tile has no room for more.
  const note = d.note === "permissions";
  const rows = githubRows(d, note ? 1 : 2);
  if (rows.length === 0) list.append(h("div", { class: "int-empty", text: t("int.glNothing") }));
  for (const r of rows) {
    const sub = r.kind === "review" ? `${t("int.glReview")} ${r.number}` : r.draft ? `${t("int.ghDraft")} ${r.number}` : r.number;
    list.append(
      h(
        "button",
        {
          class: "int-page",
          onclick: () => {
            if (r.url) void Bridge.openGithub(r.url);
          },
        },
        dot(r.kind === "review" ? "#F5A524" : "#F4505E", 5),
        h("span", { class: "int-name", text: r.title }),
        h("span", { class: "int-ago", text: sub }),
      ),
    );
  }
  const login = typeof d.login === "string" ? d.login : t("int.overview");
  const card = h("div", { class: "int-card" }, header("#F4505E", "GitHub", login, counts), summary, list);
  if (note) card.append(githubNote());
  return card;
}

// ── GitLab ────────────────────────────────────────────────────────────────────

const GITLAB = "#FC6D26";
/** A pipeline status's colour: failed red, in progress amber, passed green, anything else dim. */
function pipelineColor(status: unknown): string {
  if (status === "failed") return "#F4505E";
  if (status === "success") return "#22C55E";
  if (typeof status === "string" && status !== "canceled" && status !== "skipped" && status !== "manual") return "#F5A524";
  return "#6B7079";
}

/** A row that opens its merge request, issue or pipeline (Rust opens only links on the GitLab host). */
function gitlabRow(accent: string, title: string, sub: string, url: unknown): HTMLElement {
  return h(
    "button",
    {
      class: "int-page",
      onclick: () => {
        if (typeof url === "string") void Bridge.openGitlab(url);
      },
    },
    dot(accent, 5),
    h("span", { class: "int-name", text: title }),
    h("span", { class: "int-ago", text: sub }),
  );
}

/** The short form of `group/project!12`: `!12`. */
function shortRef(reference: unknown): string {
  const s = String(reference ?? "");
  const at = Math.max(s.lastIndexOf("!"), s.lastIndexOf("#"));
  return at >= 0 ? s.slice(at) : s;
}

function gitlabCard(tile: boolean): HTMLElement {
  const d = get("integration_gitlab");
  const ci = (d.ci ?? {}) as Record<string, unknown>;
  const count = (color: string, n: unknown, hot = false) =>
    h(
      "span",
      { class: hot && Number(n ?? 0) > 0 ? "int-ci hot" : "int-ci", style: "display:inline-flex;align-items:center;gap:3px" },
      dot(color, 5),
      h("span", { text: String(Number(n ?? 0)) }),
    );
  const counts = [count("#F4505E", ci.failed, tile), count("#F5A524", ci.running), count("#22C55E", ci.success)];
  const summary = h(
    "div",
    { class: "int-status" },
    h("span", {
      text: t("int.glSummary", {
        mine: Number(d.mineTotal ?? 0),
        reviews: Number(d.reviewsTotal ?? 0),
        issues: Number(d.issuesTotal ?? 0),
      }),
    }),
  );
  // A tile is narrow: the pipeline counts go to the header's right, the summary keeps its line.
  if (!tile) summary.append(...counts);

  // Review requests first, then default branches that fail, my merge requests, my issues.
  const rows: HTMLElement[] = [];
  for (const r of arr("integration_gitlab", "reviews")) {
    rows.push(gitlabRow("#F5A524", String(r.title ?? ""), `${t("int.glReview")} ${shortRef(r.reference)}`, r.webUrl));
  }
  for (const b of arr("integration_gitlab", "branches")) {
    if (b.status !== "failed") continue;
    rows.push(gitlabRow("#F4505E", String(b.project ?? ""), String(b.branch ?? ""), b.webUrl));
  }
  for (const m of arr("integration_gitlab", "mine")) {
    rows.push(gitlabRow(pipelineColor(m.pipeline), String(m.title ?? ""), shortRef(m.reference), m.webUrl));
  }
  for (const i of arr("integration_gitlab", "issues")) {
    rows.push(gitlabRow("#6B7079", String(i.title ?? ""), shortRef(i.reference), i.webUrl));
  }
  const list = h("div", { class: "int-rows tight" });
  if (rows.length === 0) list.append(h("div", { class: "int-empty", text: t("int.glNothing") }));
  list.append(...rows.slice(0, tile ? 2 : 3));

  const head = tile
    ? header(GITLAB, "GitLab", String(d.host ?? ""), h("span", { class: "int-cis" }, ...counts))
    : header(GITLAB, "GitLab", String(d.host ?? ""));
  return h("div", { class: "int-card" }, head, summary, list);
}

// ── Agent Orchestrator ────────────────────────────────────────────────────────

/** The sessions list's tone classes as dot colours. */
export const AO_TONE_COLOR: Record<AoTone, string> = {
  amber: "#F5A524", red: "#F4505E", green: "#22C55E", blue: "#3B9EFF", dim: "#6B7079",
};

/** Why AO has no sessions to show, and the colour of the dot before it. */
const AO_NOTES: Record<Exclude<AoStatus, "ok">, { key: Key; color: string }> = {
  notInstalled: { key: "int.aoNotInstalled", color: "#6B7079" },
  notRunning: { key: "int.aoNotRunning", color: "#F5A524" },
  unreachable: { key: "int.aoUnreachable", color: "#F4505E" },
  unsupported: { key: "int.aoUnsupported", color: "#F4505E" },
};

/**
 * Agent Orchestrator: the sessions that ask for someone first, then the ones working. Nothing is
 * clickable (Mayak only reads AO); the list with every session is the fold on the sessions page.
 * Without sessions to show, the card says why not.
 */
function aoCard(task: AgentTask, tile: boolean): HTMLElement {
  const data = readAo(State.integrations[task.id]?.data);
  if (data === null || data.status !== "ok") {
    const note = data === null ? { key: "int.aoLooking" as Key, color: "#6B7079" } : AO_NOTES[data.status as Exclude<AoStatus, "ok">];
    return h(
      "div",
      { class: "int-card" },
      header(task.color, task.name, t("int.integration")),
      h("div", { class: "int-status" }, dot(note.color, 5), h("span", { text: t(note.key) })),
      h(
        "div",
        { class: "int-actions" },
        h("button", {
          class: "link-btn",
          style: `color:${task.color}d9`,
          text: t("int.refresh"),
          onclick: () => void Bridge.refreshIntegration(task.id),
        }),
      ),
    );
  }
  const counts = aoCounts(data.sessions);
  const count = (color: string, n: number, hot = false) =>
    h(
      "span",
      { class: hot && n > 0 ? "int-ci hot" : "int-ci", style: "display:inline-flex;align-items:center;gap:3px" },
      dot(color, 5),
      h("span", { text: String(n) }),
    );
  const rows = h("div", { class: "int-rows tight" });
  if (data.sessions.length === 0) rows.append(h("div", { class: "int-empty", text: t("int.aoNone") }));
  for (const s of orderSessions(data.sessions).slice(0, tile ? 2 : 3)) {
    const word = statusWord(s.status);
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot(AO_TONE_COLOR[statusTone(s.status)], 5),
        h("span", { class: "int-name", text: s.name ?? s.id }),
        h("span", { class: "int-ago", text: word ? t(word) : s.status }),
      ),
    );
  }
  return h(
    "div",
    { class: "int-card" },
    header(task.color, task.name, t("int.aoKind"), h("span", { class: "int-cis" }, count("#F4505E", counts.attention, true), count("#3B9EFF", counts.working))),
    h("div", { class: "int-status" }, h("span", { text: t("int.aoSummary", { working: counts.working, attention: counts.attention, prs: counts.prsOpen }) })),
    rows,
  );
}

// ── Stripe ────────────────────────────────────────────────────────────────────

function stripeCard(tile: boolean): HTMLElement {
  const d = get("integration_stripe");
  const balance = (Number(d.balance ?? 0) / 100).toFixed(2);
  const currency = String(d.currency ?? "eur").toUpperCase();
  const rows = h("div", { class: "int-rows tight" });
  // A tile has room for the balance and one payment.
  for (const p of arr("integration_stripe", "payments").slice(0, tile ? 1 : undefined)) {
    const success = p.status === "succeeded";
    const accent = success ? "#22C55E" : "#F4505E";
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot(accent, 5),
        h("span", { class: "int-name", text: String(p.description ?? t("int.payment")) }),
        h("span", {
          class: "int-amount",
          style: "color:#22c55e",
          text: `+${(Number(p.amount ?? 0) / 100).toFixed(2)}`,
        }),
        h("span", { class: "int-ago", text: timeAgo(p.createdAt) }),
      ),
    );
  }
  return h(
    "div",
    { class: "int-card" },
    header("#0570DE", "Stripe", t("int.payments")),
    h("div", { class: "int-balance" }, h("span", { text: balance }), h("i", { text: currency })),
    rows,
  );
}

// ── Notion ────────────────────────────────────────────────────────────────────

function notionCard(): HTMLElement {
  const rows = h("div", { class: "int-rows tight" });
  for (const p of arr("integration_notion", "pages").slice(0, 3)) {
    rows.append(
      h(
        "button",
        {
          class: "int-page",
          onclick: () => {
            if (typeof p.url === "string") void Bridge.openUrl(p.url);
          },
        },
        p.emoji
          ? h("span", { class: "int-emoji", text: String(p.emoji) })
          : h("i", { class: "int-emoji" }, svg(ICONS.doc, 9)),
        h("span", { class: "int-name", text: String(p.title ?? t("int.untitled")) }),
        h("span", { class: "int-ago", text: timeAgo(p.lastEditedAt) }),
      ),
    );
  }
  return h("div", { class: "int-card" }, header("#E8E8E8", "Notion", t("int.recent")), rows);
}

// ── Cal.com ───────────────────────────────────────────────────────────────────

function calcomCard(): HTMLElement {
  const bookings = arr("integration_calcom", "bookings")
    .slice()
    .sort((a, b) => new Date(String(a.start)).getTime() - new Date(String(b.start)).getTime());
  const rows = h("div", { class: "int-rows tight" });
  if (bookings.length === 0) {
    rows.append(h("div", { class: "int-empty", text: t("int.noCalls") }));
  }
  for (const b of bookings.slice(0, 3)) {
    const when = new Date(String(b.start));
    const day = when.toLocaleDateString(undefined, { day: "2-digit", month: "2-digit" });
    const time = when.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    rows.append(
      h(
        "div",
        { class: "int-row" },
        dot("#C9956A", 4),
        h("span", { class: "int-time", text: `${day} ${time}` }),
        h("span", { class: "int-name", text: String(b.title ?? t("int.meeting")) }),
      ),
    );
  }
  return h("div", { class: "int-card" }, header("#C9956A", "Cal.com", t("int.schedule")), rows);
}

// ── n8n ───────────────────────────────────────────────────────────────────────

function n8nCard(task: AgentTask, onDetail: () => void, openSettings: () => void): HTMLElement {
  const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
  if (!hasActivity) return idleCard(task, openSettings);
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  return h(
    "div",
    { class: "int-card" },
    header("#F29B38", "n8n", t("int.workflow")),
    h(
      "div",
      { class: "int-actions" },
      h(
        "button",
        {
          class: "int-pill",
          style: `background:${accent}1a;border-color:${accent}38`,
          onclick: onDetail,
        },
        dot(accent, 5),
        h("span", { class: "int-name", text: task.steps[0] ?? t("int.workflow") }),
        svg(ICONS.ellipsis, 8),
      ),
    ),
  );
}

function n8nDetail(task: AgentTask, onBack: () => void): HTMLElement {
  const success = task.state === "finished";
  const accent = success ? "#22C55E" : "#F4505E";
  const detail = task.steps[1];
  return h(
    "div",
    { class: "int-card detail" },
    h(
      "div",
      { class: "int-detail-head" },
      h("button", { class: "int-back", onclick: onBack }, svg(ICONS.chevronLeft, 10, { stroke: 2.4 })),
      dot(accent, 6),
      h("b", { text: task.steps[0] ?? t("int.workflow") }),
      h("span", {
        class: "int-badge",
        style: `color:${accent};background:${accent}24`,
        text: t(success ? "int.success" : "int.failed"),
      }),
    ),
    detail
      ? h("pre", { class: "int-detail-text", text: detail })
      : h("div", {
          class: "int-status",
          text: t(success ? "int.completed" : "int.noErrorDetails"),
        }),
  );
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

export interface IntegrationCardHooks {
  detailOpen: boolean;
  openDetail(): void;
  closeDetail(): void;
  openSettings(): void;
}

/** True when this integration has data worth showing instead of the idle card. */
export function hasIntegrationData(id: string): boolean {
  const info = State.integrations[id];
  if (!info || info.error) return false;
  switch (id) {
    case "integration_vercel":
      return arr(id, "deployments").length > 0;
    case "integration_resend":
      return arr(id, "emails").length > 0;
    case "integration_github":
      return get(id).totalRepos != null;
    case "integration_gitlab":
      return Array.isArray(get(id).mine);
    case "integration_stripe":
      return info.loaded;
    case "integration_notion":
      return arr(id, "pages").length > 0;
    case "integration_calcom":
      return info.loaded;
    case AO_ID:
      return readAo(info.data)?.status === "ok";
    default:
      return false;
  }
}

/** `tile`: the card as a home page tile (views/usage.ts), shorter where it would not fit. */
export function renderIntegrationCard(task: AgentTask, hooks: IntegrationCardHooks, opts: { tile?: boolean } = {}): HTMLElement {
  const tile = opts.tile === true;
  if (task.id === "integration_n8n") {
    const hasActivity = task.steps.length > 0 && (task.state === "finished" || task.state === "error");
    return hooks.detailOpen && hasActivity
      ? n8nDetail(task, hooks.closeDetail)
      : n8nCard(task, hooks.openDetail, hooks.openSettings);
  }
  if (task.id === "integration_vercel" && hasIntegrationData(task.id)) {
    return hooks.detailOpen ? vercelDetail(hooks.closeDetail) : vercelCard(hooks.openDetail);
  }
  // AO says why it has nothing to show (not installed, not running…) instead of a missing key.
  if (task.id === AO_ID) return aoCard(task, tile);
  if (!hasIntegrationData(task.id)) return idleCard(task, hooks.openSettings);

  switch (task.id) {
    case "integration_resend":
      return resendCard();
    case "integration_github":
      return githubCard(tile);
    case "integration_gitlab":
      return gitlabCard(tile);
    case "integration_stripe":
      return stripeCard(tile);
    case "integration_notion":
      return notionCard();
    case "integration_calcom":
      return calcomCard();
    default:
      return idleCard(task, hooks.openSettings);
  }
}

/** The pills' corner badge (views.ts buildPill): approval amber "!", finished green ✓, error red ✕. */
export function pillBadgeEl(kind: PillBadge): HTMLElement {
  const colors = { approval: "#F5A524", finished: "#22C55E", error: "#F4505E" } as const;
  const icons = { approval: ICONS.bang, finished: ICONS.check, error: ICONS.xmark } as const;
  const inner = h("i", { style: `background:${colors[kind]}` }, svg(icons[kind], 6, { stroke: kind === "finished" ? 3 : 0 }));
  const badge = h("div", { class: "pill-badge" }, inner);
  badge.style.boxShadow = `0 0 4px ${colors[kind]}99`;
  return badge;
}

export { clear };
