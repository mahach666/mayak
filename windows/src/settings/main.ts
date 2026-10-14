// Settings window — the place where anything that writes to disk is confirmed.
// Agent sessions are observed with no setup at all. The Codex hooks and answers
// from the island (Claude Code and Codex) are optional, and all go through the
// same status → preview (exact diff) → apply flow.

import "./settings.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Bridge, IS_TAURI, onEvent, type AoDetection, type AutostartStatus, type Health, type HookPreview, type HookStatus, type HookTarget,
  type HotkeyStatus, type McpCli, type McpStatus, type RustMessage, type SessionsSnapshot, type SilentReason, type CodexAnswers,
  type ConnState, type DeviceStart, type GithubInfo, type OAuthResult, type OAuthService,
} from "../core/bridge";
import {
  connColor, connText, connectLabel, flowNotice, githubModeKey, offersTokenPage, startFailure, startProblem, type Notice,
} from "./oauth-view";
import { CHAT_ENABLED, CLI_CHAT_ENABLED } from "../core/features";
import { AO_COLOR, AO_ID } from "../core/ao";
import { OPEN_ON_CLICK_VALUES } from "../core/open-view";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { showApp } from "../views/apps-model";
import { SOUND_NAMES } from "../core/sound-names";
import { LANGS, currentLanguage, setLanguage, t, type Key, type Lang } from "../core/i18n";
import { en } from "../core/i18n/en";
import { isRustMessage, messageText } from "../core/message";
import { hostLabel, silentText } from "../views/sessions";
import { keyNotice, type BalanceProviderId } from "../views/usage-format";
import { h, clear } from "../views/dom";
import { createSyncRegistry } from "./sync";
import { createRecordingLock } from "./recording";
import { WAIT_REMINDER_CHOICES, reminderMinutes } from "../island/wait-reminders";
import { FIRST_PAGE, PAGES, pageOrFirst, stepPage, type PageId, type SectionId } from "./pages";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";
let snapshot: SessionsSnapshot | null = null;
/** Rev of `snapshot`: an older answer never replaces a newer one (spec §9). */
let lastRev = -1;
/** Parts of the page drawn from the sessions snapshot, redrawn on every refresh. */
const snapshotViews: ((s: SessionsSnapshot | null) => void)[] = [];
/** One status refresher per integrationSection, and launch at login, run on every window focus. */
const statusLoaders: (() => Promise<void>)[] = [];
/** Every control of the page, redrawn from the new settings on a `settings-changed`, but the one being edited. */
const sync = createSyncRegistry<Settings>();
/** Views drawn from the Codex hooks status, re-read after the Codex integration is written. */
const codexHooksViews: (() => Promise<void>)[] = [];

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

/** Takes `s` only when it is newer than the snapshot already shown (spec §9). */
function takeSnapshot(s: SessionsSnapshot | null): boolean {
  if (!s || s.rev <= lastRev) return false;
  lastRev = s.rev;
  snapshot = s;
  return true;
}

/** The snapshot is fetched at boot and again every time the window gets focus (D37). */
async function refreshSnapshot() {
  if (!takeSnapshot(await Bridge.sessionsSnapshot())) return;
  for (const draw of snapshotViews) draw(snapshot);
}

/** Every show of the window re-reads what it displays: the snapshot and the hook status. */
function refreshOnFocus() {
  void refreshSnapshot();
  for (const refresh of statusLoaders) void refresh();
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function setSwitch(el: HTMLElement, on: boolean) {
  el.classList.toggle("on", on);
  el.setAttribute("aria-pressed", String(on));
}

/** A switch kept in step with `read(settings)` on every `settings-changed`. */
function toggle(read: (s: Settings) => boolean, onChange: (v: boolean) => void): HTMLElement {
  const on = read(settings);
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  sync.add(null, (s) => setSwitch(el, read(s)));
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function colorDot(color: string): HTMLElement {
  return h("i", { class: "dot", style: `background:${color}` });
}

function statusDot(ok: boolean): HTMLElement {
  return colorDot(ok ? "#22c55e" : "#f4505e");
}

/** The answers rows: Off is a choice, not an error, so its dot is neutral; otherwise as statusDot. */
function answersDot(ok: boolean, off: boolean): HTMLElement {
  return off ? colorDot("var(--dim-2)") : statusDot(ok);
}

/** A text Rust built, in the chosen language; a code this build does not know shows Rust's English. */
function msgText(m: RustMessage): string {
  return messageText(m, (key, vars) => (Object.hasOwn(en, key) ? t(key as Key, vars) : null));
}

/** The text of a caught value, without the "Error:" that String() puts before an Error's message. */
function plainText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** What a rejected hook command says: its Rust message, or the text of anything else that was thrown. */
function errorText(err: unknown): string {
  return isRustMessage(err) ? msgText(err) : plainText(err);
}

/** The diff as read-only rows: a sign column (+ / −) and the line, which wraps. `labelledBy` is the id of its title. */
function renderDiff(text: string, labelledBy: string): HTMLElement {
  const box = h("div", { class: "diff", role: "group", "aria-labelledby": labelledBy });
  const lines = text.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  for (const line of lines) {
    const kind = line.startsWith("+ ") ? "add" : line.startsWith("- ") ? "del" : "ctx";
    // The Rust diff prefixes every line with two characters: "+ ", "- " or two spaces.
    const content = line.startsWith("+ ") || line.startsWith("- ") || line.startsWith("  ") ? line.slice(2) : line;
    box.append(h("div", { class: kind },
      h("span", { class: "sign", "aria-hidden": "true", text: kind === "add" ? "+" : kind === "del" ? "−" : "" }),
      h("span", { class: "line", text: content }),
    ));
  }
  return box;
}

/** The last part of a path, whichever separator it uses. */
function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function healthView(health: Health): { color: string; text: string } {
  if (health === "ok") return { color: "#22c55e", text: t("set.healthOk") };
  if (health === "missing") return { color: "#f5a524", text: t("set.healthMissing") };
  if ("error" in health) return { color: "#f4505e", text: t("set.healthError", { error: msgText(health.error) }) };
  return { color: "#f5a524", text: t("set.healthDegraded", { n: health.degraded.unreadable }) };
}

// ── Hook installer (shared by Codex and answers from the island) ─────────────

const TARGET_FILE: Record<HookTarget, string> = {
  claude: "settings.json",
  codex: "hooks.json",
  antigravity: "hooks.json",
};

/** One preview → diff → Apply round on `body`; nothing is written before the click on Apply. */
interface PreviewFlow {
  body: HTMLElement;
  /** Takes Mayak's entries out: the removal wording and a red button. */
  removing: boolean;
  preview: () => Promise<HookPreview>;
  apply: (fingerprint: string) => Promise<string>;
  /** Cancel, or Back from an error: the status rows again. */
  back: () => void;
  /** The write went through and its notice is on screen. */
  written: () => void;
}

/** Gives each preview title its own id, for the `aria-labelledby` of its diff. */
let previewCount = 0;

async function previewFlow(f: PreviewFlow) {
  const { body, removing } = f;
  let preview: HookPreview;
  try {
    preview = await f.preview();
  } catch (err) {
    // An unreadable or invalid file stops here rather than being treated as
    // empty and written over.
    clear(body);
    body.append(
      h("div", { class: "notice err", text: errorText(err) }),
      h("div", { class: "row" }, h("button", { text: t("set.back"), onclick: f.back })),
    );
    return;
  }
  clear(body);
  const confirm = h("button", {
    class: removing ? "danger" : "primary",
    text: t("set.apply"),
  });
  const titleId = `preview-title-${++previewCount}`;
  const block = h("div", { class: "preview" },
    h("div", { id: titleId, class: "preview-title", text: t("set.previewTitle", { path: preview.settingsPath }) }),
    h("div", { class: "hint", text: t(removing ? "set.previewRemove" : "set.previewInstall") }),
  );
  if (preview.unchanged) {
    // The entry is already exactly there: nothing would be written (D53).
    block.append(h("div", { class: "notice ok", text: t("set.noChange") }));
    confirm.disabled = true;
  } else {
    block.append(
      renderDiff(preview.diff, titleId),
      // The apply backs up only a file that exists, and the preview cannot tell.
      h("div", { class: "hint", text: t("set.backupNote", { name: baseName(preview.backup) }) }),
    );
  }
  for (const note of preview.notes) block.append(h("div", { class: "notice warn", text: msgText(note) }));
  body.append(block);
  confirm.addEventListener("click", async () => {
    confirm.disabled = true;
    try {
      const backup = await f.apply(preview.fingerprint);
      clear(body);
      body.append(h("div", {
        class: "notice ok",
        text: backup === "" ? t("set.doneNoBackup") : t("set.doneBackup", { path: backup }),
      }));
      f.written();
    } catch (err) {
      confirm.disabled = false;
      body.append(h("div", { class: "notice err", text: t("set.couldNotWrite", { error: errorText(err) }) }));
    }
  });
  body.append(h("div", { class: "row" }, confirm, h("button", {
    text: t("common.cancel"),
    onclick: f.back,
  })));
}

/** What an integrationSection calls its states and buttons. */
interface HookWords {
  installed: Key; outdated: Key; notInstalled: Key;
  install: Key; update: Key; remove: Key;
  /** Offered while installed and up to date; null offers only `remove` then. */
  reinstall: Key | null;
}

const INSTALL_WORDS: HookWords = {
  installed: "set.hookInstalled", outdated: "set.hookOutdated", notInstalled: "set.hookNotInstalled",
  install: "set.install", update: "set.update", remove: "set.removeDots", reinstall: "set.reinstall",
};

/** Answers from the island are switched on and off, not installed. */
const SWITCH_WORDS: HookWords = {
  installed: "set.answersOn", outdated: "set.answersOutdated", notInstalled: "set.answersOff",
  install: "set.enable", update: "set.update", remove: "set.disable", reinstall: null,
};

function integrationSection(target: HookTarget, words: HookWords = INSTALL_WORDS): HTMLElement {
  const file = TARGET_FILE[target];
  const body = h("div", { class: "integration" });
  let status: HookStatus | null = null;
  /** True while the status rows are on screen (not a preview, an error or a result). */
  let showingStatus = false;

  async function load() {
    showingStatus = false;
    clear(body);
    body.append(h("div", { class: "hint", text: t("set.checking") }));
    status = await Bridge.hooksStatus(target);
    clear(body);
    draw();
  }

  /** Window focus: the file may have been changed outside Mayak since it was read. */
  async function refresh() {
    if (!showingStatus) return;
    const next = await Bridge.hooksStatus(target);
    // A preview opened while the status was being read keeps the screen.
    if (!showingStatus) return;
    status = next;
    clear(body);
    draw();
  }

  function draw() {
    showingStatus = true;
    const st = status;
    if (!st) {
      body.append(h("div", { class: "notice err", text: t("set.statusUnreadable", { file }) }));
      return;
    }
    const state = t(st.installed ? (st.outdated ? words.outdated : words.installed) : words.notInstalled);
    body.append(
      h("div", { class: "row" }, h("label", { text: t("set.status") }),
        words === SWITCH_WORDS ? answersDot(st.installed && !st.outdated, !st.installed) : statusDot(st.installed && !st.outdated),
        h("span", { text: state })),
      h("div", { class: "row" }, h("label", { text: file }), h("span", { class: "path", text: st.settingsPath })),
      h("div", { class: "row" },
        h("label", { text: t("set.relay") }),
        h("span", { class: "path", text: st.hookPath }),
        statusDot(st.hookReady),
      ),
    );
    for (const note of st.notes) body.append(h("div", { class: "notice warn", text: msgText(note) }));
    if (!st.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: t("set.relayMissing"),
      }));
    }

    const actions = h("div", { class: "row" });
    const label = st.installed ? (st.outdated ? words.update : words.reinstall) : words.install;
    if (label !== null) {
      const install = h("button", { class: "primary", text: t(label), onclick: () => void showPreview(true) });
      // Writing hook commands that point at a relay which isn't there would give
      // every session a broken hook and nothing to show for it.
      if (!st.hookReady) {
        install.disabled = true;
        install.title = t("set.relayNotInstalled");
      }
      actions.append(install);
    }
    if (st.installed) {
      actions.append(h("button", { class: "danger", text: t(words.remove), onclick: () => void showPreview(false) }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    showingStatus = false;
    await previewFlow({
      body, removing: !install,
      preview: () => Bridge.hooksPreview(install, target),
      apply: (fingerprint) => Bridge.hooksApply(install, fingerprint, target),
      back: () => { clear(body); draw(); },
      written: () => window.setTimeout(() => {
        void load();
        if (target === "codex") {
          // The store has had time to apply CodexHooksInstalled by now.
          void refreshSnapshot();
          for (const view of codexHooksViews) void view();
        }
      }, 2600),
    });
  }

  statusLoaders.push(refresh);
  void load();
  return body;
}

// ── Agent sessions: sources ───────────────────────────────────────────────────

function sourcesSection(snap: SessionsSnapshot | null): HTMLElement {
  const rows = h("div", { style: "display:flex;flex-direction:column;gap:8px" });
  const draw = (s: SessionsSnapshot | null) => {
    clear(rows);
    if (!s) {
      rows.append(h("div", { class: "hint", text: t("set.observerNone") }));
      return;
    }
    const line = (label: string, what: string, health: Health) => {
      const v = healthView(health);
      return h("div", { class: "row" },
        h("label", { text: label }), colorDot(v.color), h("span", { text: v.text }),
        h("span", { class: "hint", text: what }));
    };
    rows.append(
      line("Claude Code", t("set.sourceClaude"), s.sources.claude),
      line("Codex", t("set.sourceCodex"), s.sources.codex),
    );
  };
  draw(snap);
  snapshotViews.push(draw);
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.agentSessions") })),
    h("div", { class: "hint", text: t("set.agentSessionsHint") }),
    rows,
  );
}

// ── Codex ─────────────────────────────────────────────────────────────────────

function receivingText(s: SessionsSnapshot | null): string {
  if (!s) return t("set.recvUnknown");
  const hooks = s.sources.codexHooks;
  if (hooks === "not_installed") return t("set.recvNotInstalled");
  if (hooks === "installed_unseen") return t("set.recvUnseen");
  const hosts = hooks.receiving.hosts.map((x) => hostLabel(x)).join(", ") || "Codex";
  // The clock in the chosen language, like the words around it.
  const since = new Date(hooks.receiving.since).toLocaleTimeString(currentLanguage(), { hour: "2-digit", minute: "2-digit" });
  return t("set.recvFrom", { hosts, time: since });
}

function trustSteps(): HTMLElement {
  return h(
    "div",
    { class: "steps" },
    h("div", { class: "hint", text: t("set.trustIntro") }),
    h("ol", {},
      h("li", { text: t("set.trust1") }),
      h("li", { text: t("set.trust2") }),
      h("li", { text: t("set.trust3") }),
    ),
    h("div", { class: "hint", text: t("set.trustOr") }),
    h("div", { class: "hint", text: t("set.trustNever") }),
  );
}

function codexSection(snap: SessionsSnapshot | null): HTMLElement {
  const receiving = h("span", {});
  const draw = (s: SessionsSnapshot | null) => {
    receiving.textContent = receivingText(s);
  };
  draw(snap);
  snapshotViews.push(draw);
  const silentRow = h("div", { class: "hint" });
  const drawSilent = (s: SessionsSnapshot | null) => {
    const reasons = new Set<SilentReason>();
    // The snapshot is the copy from the last store batch: drop rows older than the 2 min life.
    const now = Date.now();
    for (const p of s?.projects ?? [])
      for (const x of p.sessions) if (x.silent && now - x.since <= SILENT_LIVE_MS) reasons.add(x.silent);
    silentRow.textContent = [...reasons].map((r) => silentText(r).help).join(" ");
    silentRow.style.display = reasons.size ? "" : "none";
  };
  drawSilent(snap);
  snapshotViews.push(drawSilent);
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Codex" })),
    h("div", { class: "hint", text: t("set.codexHint") }),
    integrationSection("codex"),
    trustSteps(),
    h("div", { class: "row" }, h("label", { text: t("set.events") }), receiving),
    silentRow,
    h("div", { class: "agent" },
      h("div", { class: "agent-head", text: t("set.answers") }),
      h("div", { class: "hint", text: t("set.answersCodexHint") }),
      codexAnswersRow(),
    ),
  );
}

// ── Antigravity ───────────────────────────────────────────────────────────────

/** Antigravity's card: Mayak's hook group in ~/.gemini/config/hooks.json, observe only. */
function antigravitySection(): HTMLElement {
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Antigravity" })),
    h("div", { class: "hint", text: t("set.antigravityHint") }),
    integrationSection("antigravity"),
    h("div", { class: "hint", text: t("set.antigravityRestart") }),
  );
}

// ── Mayak MCP server ──────────────────────────────────────────────────────────

/**
 * Mayak's own MCP server for agents (src-tauri mcp_server), off until switched on here. Nothing is
 * written to any agent's settings: «Copy command» only puts the line on the clipboard, and the user
 * pastes it. The token is in that line, so Rust puts it on the clipboard itself; this page never
 * has it.
 */
function mcpSection(): HTMLElement {
  const statusRow = h("div", { class: "row" });
  const actions = h("div", { class: "row" });
  const feedback = h("div", {});

  function say(kind: "ok" | "err", text: string) {
    clear(feedback);
    feedback.append(h("div", { class: `notice ${kind}`, text }));
  }

  /** The status and, while the server listens, the buttons that need it. */
  function draw(s: McpStatus | null) {
    clear(statusRow);
    clear(actions);
    const on = s?.enabled === true;
    const listening = on && s.running;
    const text = !on ? t("set.mcpOff") : listening ? t("set.mcpListening", { url: s.url }) : s.error ? msgText(s.error) : t("set.mcpNotRunning");
    statusRow.append(
      h("label", { text: t("set.status") }),
      !on ? colorDot("var(--dim-2)") : statusDot(listening),
      h("span", { text }),
    );
    actions.style.display = listening ? "" : "none";
    if (!listening) return;
    actions.append(
      h("button", { text: t("set.mcpCopyClaude"), onclick: () => void copy("claude") }),
      h("button", { text: t("set.mcpCopyCodex"), onclick: () => void copy("codex") }),
      h("button", { text: t("set.mcpNewToken"), onclick: () => void newToken() }),
    );
  }

  async function load() {
    draw(await Bridge.mcpStatus());
  }

  async function copy(cli: McpCli) {
    const done = await Bridge.mcpCopyCommand(cli);
    if (!done || done.kind === "failed") return say("err", t("set.mcpCopyFailed"));
    if (done.kind === "done") return say("ok", t("set.mcpCopied"));
    // Linux and macOS: the page puts it on the clipboard.
    navigator.clipboard.writeText(done.text).then(
      () => say("ok", t("set.mcpCopied")),
      () => say("err", t("set.mcpCopyFailed")),
    );
  }

  async function newToken() {
    try {
      draw(await Bridge.mcpNewToken());
      say("ok", t("set.mcpNewTokenDone"));
    } catch (err) {
      say("err", t("set.mcpNewTokenFailed", { error: errorText(err) }));
    }
  }

  const enable = h("div", { class: "row" },
    h("label", { text: t("set.mcpEnable") }),
    // Rust saves the switch and starts or stops the server: a failure to start is in the status.
    toggle((s) => s.mcpEnabled, (v) => {
      settings.mcpEnabled = v;
      clear(feedback);
      void Bridge.mcpSetEnabled(v).then(draw);
    }),
  );
  statusLoaders.push(load);
  void load();
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.mcpTitle") })),
    h("div", { class: "hint", text: t("set.mcpHint") }),
    enable,
    statusRow,
    actions,
    feedback,
    h("div", { class: "hint", text: t("set.mcpTokenNote") }),
    h("div", { class: "hint", text: t("set.mcpTimeoutNote") }),
  );
}

// ── The Agents chat's saved chats ─────────────────────────────────────────────

/** How many chats of the Agents tab are kept on disk, and the button that clears them. */
function chatsSection(): HTMLElement | null {
  if (!CLI_CHAT_ENABLED) return null;
  const count = h("span", { text: "…" });
  const feedback = h("div", {});

  function say(kind: "ok" | "err", text: string) {
    clear(feedback);
    feedback.append(h("div", { class: `notice ${kind}`, text }));
  }

  async function load() {
    const n = await Bridge.agentChatSavedCount();
    count.textContent = n == null ? "…" : n === 0 ? t("set.chatsNone") : t("set.chatsCount", { n });
  }

  async function clearAll() {
    try {
      const n = await Bridge.agentChatClearSaved();
      say("ok", t("set.chatsCleared", { n }));
    } catch (err) {
      say("err", t("set.chatsClearFailed", { error: errorText(err) }));
    }
    await load();
  }

  statusLoaders.push(load);
  void load();
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.chatsTitle") })),
    h("div", { class: "hint", text: t("set.chatsHint") }),
    h("div", { class: "row" },
      h("label", { text: t("set.chatsSaved") }),
      count,
      h("button", { text: t("set.chatsClear"), onclick: () => void clearAll() })),
    feedback,
  );
}

// ── Alerts ────────────────────────────────────────────────────────────────────

function alertsSection(): HTMLElement {
  const item = (label: string, what: string, read: (s: Settings) => boolean, set: (v: boolean) => void) =>
    h("div", { class: "row" },
      h("label", { text: label }),
      toggle(read, (v) => { set(v); void save(); }),
      h("span", { class: "hint", text: what }),
    );
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.alerts") })),
    h("div", { class: "hint", text: t("set.alertsHint") }),
    item(t("set.alertWaiting"), t("set.alertWaitingHint"), (s) => s.alertWaiting, (v) => { settings.alertWaiting = v; }),
    waitReminderRow(),
    item(t("set.alertFinished"), t("set.alertFinishedHint"), (s) => s.alertFinished, (v) => { settings.alertFinished = v; }),
    item(t("set.alertError"), t("set.alertErrorHint"), (s) => s.alertError, (v) => { settings.alertError = v; }),
    item(t("set.usageAlerts"), t("set.usageAlertsHint"), (s) => s.usageAlerts, (v) => { settings.usageAlerts = v; }),
    item(t("set.contextAlerts"), t("set.contextAlertsHint"), (s) => s.contextAlerts, (v) => { settings.contextAlerts = v; }),
  );
}

/** How soon a wait nobody answered is reminded of (island/wait-reminders.ts); a value it does not offer shows as the default. */
function waitReminderRow(): HTMLElement {
  const select = h("select", {}) as HTMLSelectElement;
  for (const n of WAIT_REMINDER_CHOICES) {
    select.append(h("option", { value: String(n), text: n === 0 ? t("set.waitReminderOff") : t("set.waitReminderAfter", { n }) }));
  }
  select.value = String(reminderMinutes(settings.waitReminderMinutes));
  sync.add(select, (s) => { select.value = String(reminderMinutes(s.waitReminderMinutes)); });
  select.addEventListener("change", () => {
    settings.waitReminderMinutes = reminderMinutes(Number(select.value));
    void save();
  });
  return h("div", { class: "row" },
    h("label", { text: t("set.waitReminder") }),
    select,
    h("span", { class: "hint", text: t("set.waitReminderHint") }),
  );
}

// ── Answers from the island (spec §3) ─────────────────────────────────────────

/**
 * Codex's answers switch: its own preview and apply on Mayak's installed PermissionRequest group.
 * The state comes from hooks.json (`HookStatus.answers`), never from Mayak's settings.
 */
function codexAnswersRow(): HTMLElement {
  const file = TARGET_FILE.codex;
  const body = h("div", { class: "integration" });
  let status: HookStatus | null = null;
  /** True while the status rows are on screen (not a preview, an error or a result). */
  let showingStatus = false;
  /**
   * Codex trusts a hook by its hash, timeout and statusMessage included: after either switch it
   * skips the hook until it is trusted again. Shown until the window is reloaded.
   */
  let trustAgain = false;

  async function load() {
    showingStatus = false;
    clear(body);
    body.append(h("div", { class: "hint", text: t("set.checking") }));
    status = await Bridge.hooksStatus("codex");
    clear(body);
    draw();
  }

  /** Window focus, or a write of the Codex integration: hooks.json may have changed. */
  async function refresh() {
    if (!showingStatus) return;
    const next = await Bridge.hooksStatus("codex");
    // A preview opened while the status was being read keeps the screen.
    if (!showingStatus) return;
    status = next;
    clear(body);
    draw();
  }

  function draw() {
    showingStatus = true;
    const answers: CodexAnswers | undefined = status?.answers;
    if (!status || answers === undefined) {
      body.append(h("div", { class: "notice err", text: t("set.statusUnreadable", { file }) }));
      return;
    }
    const state = answers === "on" ? "set.answersOn" : answers === "off" ? "set.answersOff" : "set.answersNeedsCodex";
    body.append(h("div", { class: "row" },
      h("label", { text: t("set.status") }), answersDot(answers === "on", answers === "off"), h("span", { text: t(state) })));
    // What the status read found (a bad shape of hooks.json…), as the other rows show it.
    for (const note of status.notes) body.append(h("div", { class: "notice warn", text: msgText(note) }));
    // Not installed: the Codex section above installs the integration first.
    if (answers === "notInstalled") {
      body.append(h("div", { class: "hint", text: t("set.answersNeedsCodexHint") }));
      return;
    }
    // The one button switches to the other state.
    const enable = answers === "off";
    const button = h("button", {
      class: enable ? "primary" : "danger",
      text: t(enable ? "set.enable" : "set.disable"),
      onclick: () => void showPreview(enable),
    });
    if (!status.hookReady) {
      button.disabled = true;
      button.title = t("set.relayNotInstalled");
      body.append(h("div", { class: "notice warn", text: t("set.relayMissing") }));
    }
    body.append(h("div", { class: "row" }, button));
    if (trustAgain) body.append(h("div", { class: "notice warn", text: t("set.answersCodexTrust") }));
  }

  async function showPreview(on: boolean) {
    showingStatus = false;
    await previewFlow({
      body, removing: false,
      preview: () => Bridge.codexAnswersPreview(on),
      apply: (fingerprint) => Bridge.codexAnswersApply(on, fingerprint),
      back: () => { clear(body); draw(); },
      written: () => {
        trustAgain = true;
        body.append(h("div", { class: "notice warn", text: t("set.answersCodexTrust") }));
        window.setTimeout(() => void load(), 2600);
      },
    });
  }

  statusLoaders.push(refresh);
  codexHooksViews.push(refresh);
  void load();
  return body;
}

/** Claude Code's card: its answers from the island, through their own diff and Apply. Codex's are in its card. */
function claudeCodeSection(): HTMLElement {
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Claude Code" })),
    h("div", { class: "agent lead" },
      h("div", { class: "agent-head", text: t("set.answers") }),
      h("div", { class: "hint", text: t("set.answersHint") }),
      h("div", { class: "hint", text: t("set.answersClaudeHint") }),
      h("div", { class: "hint", text: t("set.answersClaudeVersion") }),
      h("div", { class: "notice warn", text: t("set.answersClaudeDotfiles") }),
      integrationSection("claude", SWITCH_WORDS),
    ),
  );
}

// ── Limits ────────────────────────────────────────────────────────────────────

function limitsSection(): HTMLElement {
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.limits") })),
    h("div", { class: "row" },
      h("label", { text: t("set.codexFreshLimits") }),
      toggle((s) => s.codexFreshLimits, (v) => { settings.codexFreshLimits = v; void save(); }),
      h("span", { class: "hint", text: t("set.codexFreshLimitsHint") }),
    ),
  );
}

// ── Claude API section ────────────────────────────────────────────────────────

/** Used without a key or when the API cannot be reached (ideas §15: the list comes from the API). */
const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
];

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: t(hasKey ? "set.keySaved" : "set.keyNone") });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? `••••••••••••  ${t("set.stored")}` : "sk-ant-...",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: t("set.saveKey") });
  const clearBtn = h("button", { class: "danger", text: t("set.remove") });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = t(present ? "set.keySaved" : "set.keyNone");
    field.placeholder = present ? `••••••••••••  ${t("set.stored")}` : "sk-ant-...";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("anthropic-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: t("set.savedNoDisk") }));
      await refresh();
      void loadModels();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.couldNotSave", { error: plainText(err) }) }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("anthropic-api-key");
      feedback.append(h("div", { class: "notice ok", text: t("set.keyRemoved") }));
      await refresh();
      void loadModels();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.couldNotRemove", { error: plainText(err) }) }));
    }
  });

  const model = h("select", {}) as HTMLSelectElement;
  const modelHint = h("span", { class: "hint" });

  /** Fills the picker; the saved model always stays selectable, marked when the list lacks it. */
  function fillModels(list: [string, string][], missingNote: string) {
    clear(model);
    for (const [id, label] of list) model.append(h("option", { value: id, text: label }));
    if (!list.some(([id]) => id === settings.model)) {
      model.append(h("option", { value: settings.model, text: `${settings.model}${missingNote}` }));
    }
    model.value = settings.model;
  }

  /** Bumped by each load: an answer that comes back after a newer load started is dropped. */
  let loadSeq = 0;

  /** The key's own models from the API; the built-in list without a key or when the call fails. */
  async function loadModels() {
    const my = ++loadSeq;
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    if (my !== loadSeq) return;
    if (!present) {
      fillModels(MODELS, "");
      modelHint.textContent = t("set.addKeyModels");
      return;
    }
    modelHint.textContent = t("set.loadingModels");
    try {
      const models = await Bridge.chatModels();
      if (my !== loadSeq) return;
      if (models.length === 0) throw new Error(t("set.noModelsListed"));
      fillModels(models.map((m) => [m.id, m.displayName]), ` ${t("set.notInAccount")}`);
      modelHint.textContent = "";
    } catch (err) {
      if (my !== loadSeq) return;
      fillModels(MODELS, "");
      modelHint.textContent = t("set.couldntLoadModels", { error: plainText(err) });
    }
  }

  model.addEventListener("change", () => {
    settings.model = model.value;
    void save();
  });
  sync.add(model, (s) => {
    if ([...model.options].some((o) => o.value === s.model)) model.value = s.model;
  });
  fillModels(MODELS, "");
  void loadModels();

  clearBtn.style.display = hasKey ? "" : "none";

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: "Claude" })),
    state,
    h("div", { class: "row" }, h("label", { text: t("set.apiKey") }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: t("set.model") }), model, modelHint),
    feedback,
  );
}

// ── Balance providers (home page credits and balances) ───────────────────────

interface BalanceKeyDef {
  provider: BalanceProviderId;
  name: string;
  /** The Credential Manager entry (src-tauri secrets.rs KNOWN_KEYS). */
  key: string;
  hint: Key;
  placeholder: string;
}

const BALANCE_KEYS: BalanceKeyDef[] = [
  { provider: "openrouter", name: "OpenRouter", key: "openrouter-api-key", hint: "set.openrouterHint", placeholder: "sk-or-v1-…" },
  { provider: "deepseek", name: "DeepSeek", key: "deepseek-api-key", hint: "set.deepseekHint", placeholder: "sk-…" },
  { provider: "moonshot", name: "Kimi (Moonshot)", key: "moonshot-api-key", hint: "set.moonshotHint", placeholder: "sk-…" },
];

/** What the last read of a provider's key said (the snapshot): refused, or not reached. */
const PROBLEM_FIELD = { openrouter: "openrouterProblem", deepseek: "deepseekProblem", moonshot: "moonshotProblem" } as const;

/**
 * A provider's API key: saved to the Credential Manager, never shown again (the page only learns
 * whether one is there). With it the home page shows the key's credits or balance (src-tauri
 * openrouter.rs, deepseek.rs, moonshot.rs).
 */
function balanceKeySection(def: BalanceKeyDef, hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const placeholder = (present: boolean) => (present ? `••••••••  ${t("set.stored")}` : def.placeholder);
  const input = h("input", {
    type: "password",
    placeholder: placeholder(hasKey),
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const saveBtn = h("button", { text: t("set.save") });
  const removeBtn = h("button", { class: "danger", text: t("set.remove") });
  const feedback = h("div", {});
  // What the last read of the key said (the snapshot): refused, or not reached.
  const problem = h("div", {});
  const drawProblem = (s: SessionsSnapshot | null) => {
    clear(problem);
    const notice = keyNotice(def.provider, s?.usage[PROBLEM_FIELD[def.provider]]);
    if (notice) problem.append(h("div", { class: `notice ${notice.tone}`, text: t(notice.key) }));
  };
  drawProblem(snapshot);
  snapshotViews.push(drawProblem);

  function show(present: boolean) {
    dot.style.background = present ? "#22c55e" : "#f4505e";
    input.placeholder = placeholder(present);
    removeBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = input.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet(def.key, value);
      input.value = "";
      show(true);
      // Rust dropped the old key's problem; the next snapshot says so too.
      clear(problem);
      feedback.append(h("div", { class: "notice ok", text: t("set.savedNoDisk") }));
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.couldNotSave", { error: plainText(err) }) }));
    }
  });
  removeBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear(def.key);
      show(false);
      clear(problem);
      feedback.append(h("div", { class: "notice ok", text: t("set.keyRemoved") }));
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.couldNotRemove", { error: plainText(err) }) }));
    }
  });
  show(hasKey);

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: def.name })),
    h("div", { class: "hint", text: t(def.hint) }),
    h("div", { class: "row" }, h("label", { text: t("set.apiKey") }), input, saveBtn, removeBtn),
    problem,
    feedback,
  );
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. `label` is translated when drawn. */
  fields: { key: string; label: Key; placeholder: string; secret: boolean }[];
  /** A Connect button (OAuth device flow, src-tauri oauth.rs) above the token field. */
  oauth?: OAuthService;
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "set.fieldSecretKey", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E", oauth: "github",
    fields: [{ key: "github-token", label: "set.orPasteToken", placeholder: "ghp_…", secret: true }] },
  { id: "integration_gitlab", name: "GitLab", color: "#FC6D26", oauth: "gitlab",
    fields: [{ key: "gitlab-token", label: "set.orPasteToken", placeholder: "glpat-…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "set.fieldToken", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "set.fieldInstanceUrl", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "set.apiKey", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "set.apiKey", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "set.fieldIntegrationToken", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "set.apiKey", placeholder: "cal_…", secret: true }] },
  // No key and no account: Agent Orchestrator is read from its local daemon (aoStatusRows).
  { id: AO_ID, name: "Agent Orchestrator", color: AO_COLOR, fields: [] },
];

const MAX_ACTIVE = 4;
/** How long a silent Codex row stays live after its last rollout write (silent-codex spec §3). */
const SILENT_LIVE_MS = 120_000;

const FIELD_LABEL = "min-width:104px";

/** The settings field holding a service's own Application ID. */
const CLIENT_ID_FIELD = { github: "githubClientId", gitlab: "gitlabClientId" } as const;

/** One inline notice in `box`, replacing what was there. */
function showNotice(box: HTMLElement, n: Notice, vars?: Record<string, string | number>) {
  clear(box);
  box.append(h("div", { class: `notice ${n.tone}`, text: t(n.key, vars) }));
}

/**
 * GitLab's address: checked by Rust (https, a host and nothing else) before it is saved; the token
 * is only ever sent to it.
 */
function gitlabAddressRow(feedback: HTMLElement): HTMLElement {
  const input = h("input", {
    type: "text",
    value: settings.gitlabUrl,
    placeholder: "https://gitlab.com",
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  sync.add(input, (s) => { input.value = s.gitlabUrl; });
  const saveBtn = h("button", { text: t("set.save") });
  saveBtn.addEventListener("click", async () => {
    const normal = await Bridge.gitlabCheckUrl(input.value);
    if (!normal) {
      showNotice(feedback, { tone: "err", key: "set.gitlabAddressBad" });
      return;
    }
    input.value = normal;
    settings.gitlabUrl = normal;
    await save();
    showNotice(feedback, { tone: "ok", key: "set.gitlabAddressSaved" });
  });
  return h("div", { class: "row" }, h("label", { style: FIELD_LABEL, text: t("set.gitlabAddress") }), input, saveBtn);
}

/** An Application ID of the user's own (a public value, kept in settings.json). */
function clientIdRow(service: OAuthService): HTMLElement {
  const field = CLIENT_ID_FIELD[service];
  const input = h("input", {
    type: "text",
    value: settings[field],
    placeholder: t(service === "gitlab" ? "set.appIdPlaceholderGitlab" : "set.appIdPlaceholder"),
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  sync.add(input, (s) => { input.value = s[field]; });
  const saveBtn = h("button", { text: t("set.save") });
  saveBtn.addEventListener("click", () => {
    input.value = input.value.trim();
    settings[field] = input.value;
    void save();
  });
  return h("div", { class: "row" }, h("label", { style: FIELD_LABEL, text: t("set.appId") }), input, saveBtn);
}

/**
 * Connect (OAuth device flow): the account line with Connect / Disconnect and, while the user
 * approves, the code to enter and the page to enter it on. `tokenChanged` keeps the token field's
 * dot in step. When GitLab does not let Mayak sign in by code (or there is no Application ID), it
 * offers GitLab's «create a token» page and `focusToken` takes the user to the token field.
 * Returns the rows and the refresh of the account line.
 */
function oauthRows(
  service: OAuthService,
  name: string,
  tokenChanged: (present: boolean) => void,
  focusToken: () => void,
): { rows: HTMLElement[]; refresh: () => Promise<void> } {
  const stateDot = statusDot(false);
  const stateText = h("span", { class: "hint" });
  const connectBtn = h("button", { class: "primary" }) as HTMLButtonElement;
  const disconnectBtn = h("button", { class: "danger", text: t("set.oauthDisconnect") });
  const panel = h("div", { style: "display:none;flex-direction:column;gap:6px" });
  const feedback = h("div", {});
  // GitHub: Mayak's GitHub App sees only the repositories it is installed on.
  const installRow = h("div", { class: "row", style: "display:none" });
  const installHint = h("div", { class: "hint", style: "display:none", text: t("set.ghInstallHint") });
  if (service === "github") {
    const installBtn = h("button", { text: t("set.ghInstall") });
    // Rust opens github.com/apps/<slug>/installations/new only.
    installBtn.addEventListener("click", () => void Bridge.openGithubInstall());
    installRow.append(h("label", { style: FIELD_LABEL }), installBtn);
  }

  function show(state: ConnState, info: GithubInfo | null = null) {
    stateDot.style.background = connColor(state);
    const mode = githubModeKey(state, info?.mode);
    stateText.textContent = mode ? `${t(connText(state))} · ${t(mode)}` : t(connText(state));
    connectBtn.textContent = t(connectLabel(state));
    disconnectBtn.style.display = state === "none" ? "none" : "";
    const installable = info?.installable === true;
    installRow.style.display = installable ? "" : "none";
    installHint.style.display = installable ? "" : "none";
  }
  async function refresh() {
    const state = (await Bridge.oauthStatus(service)) ?? "none";
    show(state, service === "github" ? await Bridge.githubInfo() : null);
  }
  function hidePanel() {
    panel.style.display = "none";
    clear(panel);
  }
  function showPanel(start: DeviceStart) {
    clear(panel);
    const copyBtn = h("button", { text: t("set.oauthCopyCode") });
    copyBtn.addEventListener("click", () => {
      navigator.clipboard.writeText(start.userCode).then(
        () => { copyBtn.textContent = t("set.oauthCopied"); },
        () => { copyBtn.textContent = t("set.oauthCopyCode"); },
      );
    });
    const cancelBtn = h("button", { text: t("common.cancel") });
    cancelBtn.addEventListener("click", () => {
      void Bridge.oauthCancel(service);
      hidePanel();
    });
    panel.append(
      h("div", { class: "hint", text: t("set.oauthEnterCode", { name }) }),
      h("div", { class: "row" },
        h("label", { style: FIELD_LABEL, text: t("set.oauthCode") }),
        h("b", { style: "font:600 15px var(--mono, monospace);letter-spacing:1px;user-select:all", text: start.userCode }),
        copyBtn,
        // Rust checked the page is https on the service's own host.
        h("button", { class: "primary", text: t("set.oauthOpenPage"), onclick: () => void Bridge.openUrl(start.verificationUri) }),
      ),
      h("div", { class: "row" }, h("label", { style: FIELD_LABEL }), h("span", { class: "path", text: start.verificationUri })),
      h("div", { class: "row" }, h("label", { style: FIELD_LABEL }), h("span", { class: "hint", text: t("set.oauthWaiting") }), cancelBtn),
    );
    panel.style.display = "flex";
  }
  /** GitLab's token page, prefilled, opened by Rust on the configured host; then the token field. */
  function offerTokenPage() {
    const hint = h("div", {});
    const createBtn = h("button", { class: "primary", text: t("set.gitlabCreateToken") });
    createBtn.addEventListener("click", async () => {
      await Bridge.openGitlabTokenPage();
      showNotice(hint, { tone: "ok", key: "set.gitlabTokenHint" });
      focusToken();
    });
    feedback.append(h("div", { class: "row" }, createBtn), hint);
  }

  connectBtn.addEventListener("click", async () => {
    clear(feedback);
    hidePanel();
    connectBtn.disabled = true;
    try {
      showPanel(await Bridge.oauthStart(service));
    } catch (err) {
      const failure = startFailure(err);
      showNotice(feedback, startProblem(failure.code), { name });
      if (offersTokenPage(service, failure.fallback)) offerTokenPage();
    } finally {
      connectBtn.disabled = false;
    }
  });
  disconnectBtn.addEventListener("click", async () => {
    try {
      await Bridge.oauthDisconnect(service);
    } catch (err) {
      clear(feedback);
      feedback.append(h("div", { class: "notice err", text: t("set.couldNotRemove", { error: plainText(err) }) }));
    }
  });
  // How the flow ended, or a Disconnect: the result comes as an event.
  void onEvent<OAuthResult>("oauth", (result) => {
    if (result.service !== service) return;
    hidePanel();
    showNotice(feedback, flowNotice(result.state), { name });
    if (offersTokenPage(service, result.fallback)) offerTokenPage();
    if (result.state === "connected") tokenChanged(true);
    if (result.state === "disconnected") tokenChanged(false);
    void refresh();
  });

  show("none");
  statusLoaders.push(refresh);
  void refresh();

  return {
    rows: [
      h("div", { class: "row" }, h("label", { style: FIELD_LABEL, text: t("set.oauthAccount") }), stateDot, stateText, connectBtn, disconnectBtn),
      installRow,
      installHint,
      panel,
      feedback,
    ],
    refresh,
  };
}

/** The words and the dot of what this PC has of Agent Orchestrator. */
function aoLine(found: AoDetection | null): { color: string; text: string } {
  switch (found?.state) {
    case "running":
      return { color: "#22c55e", text: t("set.aoRunning", { port: found.port ?? "" }) };
    case "notRunning":
      return { color: "#f5a524", text: t("set.aoNotRunning") };
    case "unsupported":
      return { color: "#f4505e", text: t("set.aoUnsupported") };
    case "notInstalled":
      return { color: "var(--dim-2)", text: t("set.aoNotInstalled") };
    default:
      return { color: "var(--dim-2)", text: "…" };
  }
}

/**
 * Agent Orchestrator's rows: what Mayak found (run file and PATH, read again on every focus of this
 * window; the daemon is not asked) and what Mayak does with it. There is nothing to enter.
 */
function aoStatusRows(): HTMLElement[] {
  const dotHolder = h("span", { style: "display:inline-flex" });
  const text = h("span", { style: "font-size:12px" });
  const cli = h("div", { class: "hint" });
  async function refresh() {
    const found = await Bridge.aoDetect();
    const line = aoLine(found);
    clear(dotHolder);
    dotHolder.append(colorDot(line.color));
    text.textContent = line.text;
    cli.textContent = found?.cli ? t("set.aoCli", { path: found.cli }) : "";
    cli.style.display = found?.cli ? "" : "none";
  }
  statusLoaders.push(refresh);
  void refresh();
  return [
    h("div", { class: "row" }, h("label", { style: FIELD_LABEL, text: t("set.aoStatus") }), dotHolder, text),
    cli,
    h("div", { class: "hint", text: t("set.aoHint") }),
  ];
}

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = t("set.integrationsNote", { max: MAX_ACTIVE, used });
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sync.add(null, (st) => {
      sw.classList.toggle("on", st.activeIntegrations.includes(def.id));
      updateNote();
    });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    /** The token field's dot and placeholder, redrawn when Connect or Disconnect changes the token. */
    const tokenViews: Record<string, (present: boolean) => void> = {};
    const tokenFocus: Record<string, () => void> = {};
    let afterTokenSave: (() => Promise<void>) | null = null;
    if (def.oauth) {
      const service = def.oauth;
      const addressFeedback = h("div", {});
      if (service === "gitlab") rows.append(gitlabAddressRow(addressFeedback), addressFeedback);
      const tokenKey = def.fields[0].key;
      const oauth = oauthRows(service, def.name, (on) => tokenViews[tokenKey]?.(on), () => tokenFocus[tokenKey]?.());
      rows.append(...oauth.rows, clientIdRow(service));
      afterTokenSave = oauth.refresh;
    }
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? `••••••••  ${t("set.stored")}` : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: t("set.save") });
      const dotEl = statusDot(present[field.key] ?? false);
      const shown = (on: boolean) => {
        present[field.key] = on;
        input.placeholder = on ? `••••••••  ${t("set.stored")}` : field.placeholder;
        dotEl.style.background = on ? "#22c55e" : "#f4505e";
      };
      tokenViews[field.key] = shown;
      tokenFocus[field.key] = () => input.focus();
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          input.value = "";
          shown(value.length > 0);
          if (afterTokenSave) void afterTokenSave();
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: FIELD_LABEL, text: t(field.label) }),
          input, saveBtn, dotEl,
        ),
      );
    }

    if (def.id === AO_ID) rows.append(...aoStatusRows());

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: t("set.integrations") })), note, list);
}

// ── Global shortcut (hotkey spec §5) ──────────────────────────────────────────

/**
 * A keydown while recording → the shortcut in Rust's normal form (src-tauri/src/hotkey.rs: Ctrl,
 * Alt, Shift, Win, then the key), a hint for a press that cannot be one, or null for a lone
 * modifier (keep waiting). The key is read from `keyCode`, which WebView2 on Windows fills with the
 * layout's virtual key, the very code RegisterHotKey takes (`code` would be the US-QWERTY position
 * and disagree on AZERTY, QWERTZ, Dvorak): A–Z 0x41–0x5A, 0–9 0x30–0x39, F1–F24 0x70–0x87, Space 0x20.
 */
function shortcutOf(e: KeyboardEvent): { combo: string } | { hint: string } | null {
  const vk = e.keyCode;
  // Shift, Ctrl, Alt, left and right Win (the Menu key, 93, is no modifier and falls to the hint).
  if (vk === 16 || vk === 17 || vk === 18 || vk === 91 || vk === 92) return null;
  const isF = vk >= 0x70 && vk <= 0x87;
  const key =
    vk >= 0x41 && vk <= 0x5a ? String.fromCharCode(vk)
    : vk >= 0x30 && vk <= 0x39 ? String.fromCharCode(vk)
    : isF ? `F${vk - 0x6f}`
    : vk === 0x20 ? "Space"
    : null;
  if (key == null) return { hint: t("set.hotkeyUseKey") };
  const mods = [
    e.ctrlKey ? "Ctrl" : null,
    e.altKey ? "Alt" : null,
    e.shiftKey ? "Shift" : null,
    e.metaKey ? "Win" : null,
  ].filter((m): m is string => m != null);
  // A letter, digit or Space needs Ctrl, Alt or Win (Shift alone is just a capital: it would take
  // over ordinary typing); F1–F24 may stand alone.
  if (!isF && !(e.ctrlKey || e.altKey || e.metaKey)) {
    return { hint: t("set.hotkeyAddMod") };
  }
  return { combo: [...mods, key].join("+") };
}

/** The line under the shortcut for a status from hotkey.rs: empty for active and off. */
function hotkeyStatusText(s: HotkeyStatus): string {
  switch (s.state) {
    case "taken":
      return s.error
        ? t("set.hotkeyTakenError", { combo: s.combo, error: msgText(s.error) })
        : t("set.hotkeyTaken", { combo: s.combo });
    case "invalid":
    case "unsupported":
      return s.error ? msgText(s.error) : "";
    default:
      return "";
  }
}

/** One of the two shortcuts: its label, its settings fields, its status command and event. */
interface ShortcutRow {
  label: "set.hotkey" | "set.nextHotkey";
  combo: "hotkey" | "nextHotkey";
  enabled: "hotkeyEnabled" | "nextHotkeyEnabled";
  status: () => Promise<HotkeyStatus | null>;
  event: "hotkey-status" | "hotkey-next-status";
}

/** «Show / hide island» (hotkey spec §5). */
const ISLAND_SHORTCUT: ShortcutRow = {
  label: "set.hotkey", combo: "hotkey", enabled: "hotkeyEnabled",
  status: () => Bridge.hotkeyStatus(), event: "hotkey-status",
};

/** «Next waiting session» (ideas §2): raises the window of the session waiting longest. */
const NEXT_SHORTCUT: ShortcutRow = {
  label: "set.nextHotkey", combo: "nextHotkey", enabled: "nextHotkeyEnabled",
  status: () => Bridge.hotkeyNextStatus(), event: "hotkey-next-status",
};

/**
 * A shortcut's rows: an on/off switch, the shortcut as a button that records a new one, and
 * the status line. Off keeps the shortcut, so switching on brings the same one back. While recording both shortcuts are released (hotkey_pause), or pressing one would act
 * instead of reaching this page.
 */
/** The two shortcut rows record one at a time: clicking one ends the other's recording unchanged. */
const recordingLock = createRecordingLock();

function hotkeyRows(row: ShortcutRow): HTMLElement[] {
  const field = h("button", {}) as HTMLButtonElement;
  const line = h("div", { class: "hint" });
  let recording = false;
  // Counts clicks: a recording ended and started again while the other row's ended is not armed twice.
  let attempt = 0;
  // The pause invoke in flight: the resume waits for it, so the two reach Rust in order.
  let pausing: Promise<unknown> = Promise.resolve();
  let status: HotkeyStatus | null = null;

  function draw() {
    field.textContent = recording ? t("set.pressKeys") : settings[row.combo];
    field.disabled = !settings[row.enabled];
    // macOS and Linux: the platform's message instead of the switch and the field (spec §3.3).
    const unsupported = status?.state === "unsupported";
    field.style.display = unsupported ? "none" : "";
    sw.style.display = unsupported ? "none" : "";
    if (!recording) line.textContent = status ? hotkeyStatusText(status) : "";
    line.style.display = line.textContent ? "" : "none";
  }

  function hint(text: string) {
    line.textContent = text;
    line.style.display = "";
  }

  /** Ends a recording. `chosen` is saved before the pause ends, so the new shortcut is registered, not the old one. */
  async function finish(chosen: string | null) {
    if (!recording) return;
    recording = false;
    recordingLock.end(finish);
    window.removeEventListener("keydown", onKey, true);
    const next = chosen ?? settings[row.combo];
    const changed = next !== settings[row.combo];
    settings[row.combo] = next;
    draw();
    try {
      if (changed) await save();
    } finally {
      await pausing;
      await Bridge.hotkeyPause(false);
    }
  }

  function onKey(e: KeyboardEvent) {
    // Nothing reaches the page while recording: no reload on F5, no click on Enter.
    // Immediate: no other keydown listener on the window hears the key either.
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.repeat) return;
    if (e.code === "Escape") {
      void finish(null);
      return;
    }
    const got = shortcutOf(e);
    if (got == null) return;
    if ("hint" in got) hint(got.hint);
    else void finish(got.combo);
  }

  field.addEventListener("click", () => void (async () => {
    if (recording) return;
    recording = true;
    const mine = ++attempt;
    // A focused button would be clicked again by the Space or Enter of the recorded shortcut.
    field.blur();
    line.textContent = "";
    draw();
    // The other row recording ends unchanged first, its resume before this pause: one key press
    // never sets both shortcuts (scripts/checks/recording-lock.mjs).
    await recordingLock.start(finish);
    // Ended meanwhile (window left, switched off), or clicked again after that.
    if (!recording || mine !== attempt) return;
    pausing = Bridge.hotkeyPause(true);
    window.addEventListener("keydown", onKey, true);
  })());

  const sw = toggle((s) => s[row.enabled], (on) => {
    void (async () => {
      // A recording in progress ends unchanged before the switch takes effect.
      await finish(null);
      settings[row.enabled] = on;
      draw();
      await save();
    })();
  });

  // Leaving the window (Alt+Tab, or closing it: it only hides) ends a recording unchanged, so
  // the shortcut never stays released.
  window.addEventListener("blur", () => void finish(null));
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) void finish(null);
  });

  let heard = false;
  void onEvent<HotkeyStatus>(row.event, (s) => {
    heard = true;
    status = s;
    draw();
  });
  void row.status().then((s) => {
    // An event that arrived meanwhile is newer than this answer.
    if (heard) return;
    status = s;
    draw();
  });
  sync.add(null, draw);
  // A page reloaded in the middle of a recording left the shortcut released: give it back.
  // A resume without a pause does nothing (hotkey.rs Slot::resume).
  void Bridge.hotkeyPause(false);
  draw();

  return [
    h("div", { class: "row" }, h("label", { text: t(row.label) }), sw, field),
    line,
  ];
}

// ── Language (batch spec §C) ──────────────────────────────────────────────────

/** Each language in its own words: never translated. */
const LANGUAGE_NAMES: Record<Lang, string> = {
  en: "English",
  ru: "Русский",
  de: "Deutsch",
  fr: "Français",
  es: "Español",
  zh: "中文",
  ja: "日本語",
};

/**
 * «Language»: System, then the languages. Every view took its texts when it was built, so a
 * change saves and reloads the page; the island reloads on the settings-changed it gets.
 */
function languageRows(): HTMLElement[] {
  const select = h("select", {}) as HTMLSelectElement;
  select.append(h("option", { value: "system", text: t("set.languageSystem") }));
  for (const lang of LANGS) select.append(h("option", { value: lang, text: LANGUAGE_NAMES[lang] }));
  // A value from a newer build that this one does not know shows as System, which it falls back to.
  select.value = settings.language === "system" || (LANGS as readonly string[]).includes(settings.language)
    ? settings.language
    : "system";
  select.addEventListener("change", () => {
    void (async () => {
      settings.language = select.value;
      await save();
      location.reload();
    })();
  });
  return [
    h("div", { class: "row" }, h("label", { text: t("set.language") }), select),
    h("div", { class: "hint", text: t("set.languageHint") }),
  ];
}

// ── Island, tabs, sound, system ───────────────────────────────────────────────

function islandSection(): HTMLElement {
  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });
  sync.add(autoClose, (s) => { autoClose.value = String(Math.round(s.autoCloseInterval)); });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: t("set.screenPrimary") }),
    h("option", { value: "cursor", text: t("set.screenCursor") }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });
  sync.add(screen, (s) => { screen.value = s.screen; });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.pageIsland") })),
    h("div", { class: "row" },
      h("label", { text: t("set.screen") }),
      screen,
    ),
    ...hotkeyRows(ISLAND_SHORTCUT),
    ...hotkeyRows(NEXT_SHORTCUT),
    h("div", { class: "hint", text: t("set.nextHotkeyHint") }),
    h("div", { class: "row" },
      h("label", { text: t("set.autoClose") }),
      autoClose,
      h("span", { class: "hint", text: t("set.autoCloseHint") }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("set.openOnHover") }),
      toggle((s) => s.openOnHover, (v) => { settings.openOnHover = v; void save(); }),
    ),
    h("div", { class: "hint", text: t("set.openOnHoverHint") }),
  );
}

function tabsSection(): HTMLElement {
  // «Open on click»: the page a click, the shortcut or the tray opens the island on. A value this
  // build does not know shows (and acts) as Sessions. Last opened is kept by the island, in memory.
  const openOn = h("select", {}) as HTMLSelectElement;
  for (const v of OPEN_ON_CLICK_VALUES) {
    openOn.append(h("option", { value: v, text: t(v === "last" ? "set.openLast" : `tab.${v}`) }));
  }
  const openOnValue = (s: Settings) =>
    (OPEN_ON_CLICK_VALUES as readonly string[]).includes(s.openOnClick) ? s.openOnClick : "sessions";
  openOn.value = openOnValue(settings);
  sync.add(openOn, (s) => { openOn.value = openOnValue(s); });
  openOn.addEventListener("change", () => {
    settings.openOnClick = openOn.value;
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.tabs") })),
    h("div", { class: "row" },
      h("label", { text: t("set.openOnClick") }),
      openOn,
    ),
    h("div", { class: "hint", text: t("set.openOnClickHint") }),
    h("div", { class: "row" },
      h("label", { text: t("set.showMedia") }),
      toggle((s) => s.showMedia, (v) => { settings.showMedia = v; void save(); }),
    ),
    hiddenAppsRow(),
  );
}

function soundSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });
  sync.add(volume, (s) => { volume.value = String(s.soundVolume); });

  // Your own sounds: the folder, a reload, and how many are replaced.
  const customCount = h("span", { class: "hint" });
  const countCustom = async () => {
    const n = (await Bridge.customSounds(SOUND_NAMES))?.length ?? 0;
    customCount.textContent = n > 0 ? t("set.soundsCustom", { n }) : "";
  };
  void countCustom();
  const soundsFolder = h("button", { text: t("set.soundsFolder"), onclick: () => void Bridge.revealSoundsFolder() });
  const reloadSounds = h("button", {
    text: t("set.soundsReload"),
    onclick: () => void Bridge.reloadSounds().then(countCustom),
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("settings.sound") })),
    h("div", { class: "row" },
      h("label", { text: t("settings.sound") }),
      toggle((s) => s.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" }, soundsFolder, reloadSounds, customCount),
    h("div", { class: "hint", text: t("set.soundsHint") }),
  );
}

function systemSection(): HTMLElement {
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.pageSystem") })),
    ...languageRows(),
    ...autostartRows(),
  );
}

/**
 * The built-in apps hidden from the island's Apps tab (own-apps, ideas §30), each with Show. The
 * row is there only while one is hidden; it follows a hide made in the island.
 */
function hiddenAppsRow(): HTMLElement {
  const list = h("span", { class: "row" });
  const row = h("div", { class: "row" }, h("label", { text: t("set.hiddenApps") }), list);
  let names: Record<string, string> = {};
  function draw() {
    clear(list);
    const hidden = settings.hiddenApps ?? [];
    row.hidden = hidden.length === 0;
    for (const id of hidden) {
      list.append(
        h("span", { text: names[id] ?? id }),
        h("button", {
          text: t("set.showApp"),
          onclick: () => {
            settings = { ...settings, ...showApp({ appOrder: settings.appOrder, hiddenApps: hidden, customApps: settings.customApps }, id) };
            draw();
            void save();
          },
        }),
      );
    }
  }
  sync.add(null, draw);
  draw();
  void Bridge.appsBuiltin().then((apps) => {
    names = Object.fromEntries((apps ?? []).map((a) => [a.id, a.name]));
    draw();
  });
  return row;
}

/**
 * Launch at login, kept in step with Windows (Task Manager → Startup): every show of the window
 * reads the real state, and a change that fails shows what Windows kept, with a line saying so.
 */
function autostartRows(): HTMLElement[] {
  const note = h("div", { class: "notice err" });
  note.hidden = true;
  const sw = toggle((s) => s.autostart, (v) => {
    settings.autostart = v;
    void save().then(refresh);
  });
  const draw = (on: boolean) => setSwitch(sw, on);
  function showStatus(status: AutostartStatus) {
    settings.autostart = status.enabled;
    draw(status.enabled);
    note.hidden = status.error == null;
    note.textContent = status.error === "read" ? t("set.autostartReadError")
      : status.error === "write" ? t("set.autostartWriteError") : "";
  }
  async function refresh() {
    const status = await Bridge.autostartStatus();
    if (status) showStatus(status);
  }
  statusLoaders.push(refresh);
  void refresh();
  return [
    h("div", { class: "row" }, h("label", { text: t("set.autostart") }), sw),
    note,
  ];
}

// ── Pages (ideas §37) ─────────────────────────────────────────────────────────

/** The page on screen, kept for this session only (a language change reloads the page). */
const PAGE_STORE = "mayak.settings.page";

function storedPage(): PageId {
  try {
    return pageOrFirst(sessionStorage.getItem(PAGE_STORE));
  } catch (err) {
    console.warn("[mayak] could not read the last Settings page", err);
    return FIRST_PAGE;
  }
}

function storePage(id: PageId) {
  try {
    sessionStorage.setItem(PAGE_STORE, id);
  } catch (err) {
    console.warn("[mayak] could not keep the Settings page", err);
  }
}

/**
 * The colour of the dot by «Agents»: red when a source cannot be read, amber when one reads with
 * gaps, green when one reads fine, none before the first snapshot. A missing folder is an agent
 * that is not installed, not a problem: it does not colour the dot.
 */
function agentsDotColor(s: SessionsSnapshot | null): string | null {
  if (!s) return null;
  const all = [s.sources.claude, s.sources.codex];
  if (all.some((x) => typeof x === "object" && "error" in x)) return "#f4505e";
  if (all.some((x) => typeof x === "object" && "degraded" in x)) return "#f5a524";
  if (all.some((x) => x === "ok")) return "#22c55e";
  return null;
}

/** The sidebar (one button per page) and the pane, with every page built and all but one hidden. */
function pagesView(builders: Record<SectionId, () => HTMLElement | null>): HTMLElement[] {
  const pane = h("main", { class: "pane" });
  const nav = h("nav", { class: "nav", "aria-label": t("set.pages") },
    h("h1", {}, h("span", { text: "Mayak" }), h("span", { class: "version", text: version })),
  );
  const items = new Map<PageId, HTMLButtonElement>();
  const bodies = new Map<PageId, HTMLElement>();
  let current = storedPage();

  function show(id: PageId) {
    current = id;
    storePage(id);
    for (const [page, item] of items) {
      item.classList.toggle("on", page === id);
      if (page === id) item.setAttribute("aria-current", "page");
      else item.removeAttribute("aria-current");
    }
    for (const [page, body] of bodies) body.hidden = page !== id;
    pane.scrollTop = 0;
  }

  for (const page of PAGES) {
    const item = h("button", { class: "nav-item", text: t(page.title), onclick: () => show(page.id) }) as HTMLButtonElement;
    if (page.id === "agents") {
      const dot = colorDot("");
      const draw = (s: SessionsSnapshot | null) => {
        const color = agentsDotColor(s);
        dot.style.background = color ?? "";
        // .dot sets its own display, which the hidden attribute would not override.
        dot.style.display = color == null ? "none" : "";
      };
      draw(snapshot);
      snapshotViews.push(draw);
      item.append(dot);
    }
    items.set(page.id, item);
    nav.append(item);

    const body = h("div", { class: "page" }, h("div", { class: "page-title", text: t(page.title) }));
    for (const id of page.sections) {
      const section = builders[id]();
      if (section) body.append(section);
    }
    bodies.set(page.id, body);
    pane.append(body);
  }
  nav.append(h("div", { class: "nav-foot hint", text: t("set.noTelemetryShort") }));

  // ↑ / ↓ on the sidebar: the page above or below, and its button takes the focus.
  nav.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    if (!(e.target instanceof HTMLElement) || !e.target.classList.contains("nav-item")) return;
    e.preventDefault();
    const next = stepPage(current, e.key === "ArrowUp" ? -1 : 1);
    show(next);
    items.get(next)?.focus();
  });

  show(current);
  return [nav, pane];
}

// ── Boot ──────────────────────────────────────────────────────────────────────

/**
 * The window title in the chosen language (batch spec §F), the page's and the window's. Only this
 * window may set its title (capabilities/settings-window.json); if that is refused, as with an
 * older build, the window keeps the English title Rust gave it and works as before.
 */
async function applyWindowTitle() {
  const title = t("set.windowTitle");
  document.title = title;
  if (!IS_TAURI) return;
  try {
    await getCurrentWindow().setTitle(title);
  } catch (err) {
    console.error("[mayak] could not set the Settings window title", err);
  }
}

async function main() {
  // The window is created hidden at launch and shown later; registered before
  // anything is awaited, so a show during the awaits below is never missed.
  window.addEventListener("focus", refreshOnFocus);

  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  // Before anything is drawn: every view takes its texts when it is made.
  setLanguage(settings.language);
  void applyWindowTitle();
  takeSnapshot(await Bridge.sessionsSnapshot());

  const hasKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
  const balanceKeys: boolean[] = [];
  for (const def of BALANCE_KEYS) balanceKeys.push((await Bridge.secretPresent(def.key)) ?? false);

  const keys = [
    "stripe-api-key", "github-token", "gitlab-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  // Every page is built at boot, hidden or not: each control registers its sync and its
  // focus refresh as before, whichever page is on screen.
  const balance = (id: BalanceProviderId) => {
    const i = BALANCE_KEYS.findIndex((d) => d.provider === id);
    return balanceKeySection(BALANCE_KEYS[i], balanceKeys[i]);
  };
  const builders: Record<SectionId, () => HTMLElement | null> = {
    island: islandSection,
    tabs: tabsSection,
    sound: soundSection,
    alerts: alertsSection,
    sources: () => sourcesSection(snapshot),
    claudeCode: claudeCodeSection,
    codex: () => codexSection(snapshot),
    antigravity: antigravitySection,
    mcp: mcpSection,
    chats: chatsSection,
    limits: limitsSection,
    claudeApi: () => (CHAT_ENABLED ? apiSection(hasKey) : null),
    openrouter: () => balance("openrouter"),
    deepseek: () => balance("deepseek"),
    moonshot: () => balance("moonshot"),
    integrations: () => integrationsSection(present),
    system: systemSection,
    about: () => h("div", { class: "hint", text: `Mayak ${version} · ${t("set.noTelemetry")}` }),
  };

  clear(root);
  root.append(...pagesView(builders));
  // A field skipped by a settings-changed while it was focused catches up when it is left.
  document.addEventListener("focusout", (e) => sync.settle(e.target, settings));

  void onEvent<Settings>("settings-changed", (s) => {
    // The island or another save chose a new language: the page took its texts at boot (spec §C).
    if (s.language !== undefined && s.language !== settings.language) {
      location.reload();
      return;
    }
    settings = { ...settings, ...s };
    sync.run(settings, document.activeElement);
  });
}

void main();
