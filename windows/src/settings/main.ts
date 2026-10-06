// Settings window — the place where anything that writes to disk is confirmed.
// Agent sessions are observed with no setup at all. The Codex hooks and the
// legacy Claude Code Allow/Deny hook are optional, and both go through the same
// status → preview (exact diff) → apply flow.

import "./settings.css";
import {
  Bridge, onEvent, type Health, type HookPreview, type HookStatus, type HookTarget,
  type HotkeyStatus, type SessionsSnapshot, type SilentReason,
} from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { HOST_LABEL, silentText } from "../views/sessions";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";
let snapshot: SessionsSnapshot | null = null;
/** Rev of `snapshot`: an older answer never replaces a newer one (spec §9). */
let lastRev = -1;
/** Parts of the page drawn from the sessions snapshot, redrawn on every refresh. */
const snapshotViews: ((s: SessionsSnapshot | null) => void)[] = [];
/** One status refresher per integrationSection, run on every window focus. */
const statusLoaders: (() => Promise<void>)[] = [];

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

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
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

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

function healthView(health: Health): { color: string; text: string } {
  if (health === "ok") return { color: "#22c55e", text: "OK" };
  if (health === "missing") return { color: "#f5a524", text: "Folder not found" };
  if ("error" in health) return { color: "#f4505e", text: `Error: ${health.error}` };
  return { color: "#f5a524", text: `Degraded — ${health.degraded.unreadable} unreadable` };
}

// ── Hook installer (shared by Codex and the legacy Claude block) ──────────────

const TARGET_FILE: Record<HookTarget, string> = {
  claude: "settings.json",
  codex: "hooks.json",
};

function integrationSection(target: HookTarget): HTMLElement {
  const file = TARGET_FILE[target];
  const body = h("div", { class: "integration" });
  let status: HookStatus | null = null;
  /** True while the status rows are on screen (not a preview, an error or a result). */
  let showingStatus = false;

  async function load() {
    showingStatus = false;
    clear(body);
    body.append(h("div", { class: "hint", text: "Checking…" }));
    status = await Bridge.hooksStatus(target);
    clear(body);
    draw();
  }

  /** Window focus: the file may have been changed outside Mayak since it was read. */
  async function refresh() {
    // ~/.claude/settings.json is read only while the Advanced block is open (spec §8).
    const advanced = body.closest("details");
    if (!showingStatus || (advanced !== null && !advanced.open)) return;
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
      body.append(h("div", { class: "notice err", text: `Could not read the status of ${file}.` }));
      return;
    }
    const state = st.installed ? (st.outdated ? "Installed — out of date" : "Installed") : "Not installed";
    body.append(
      h("div", { class: "row" }, h("label", { text: "Status" }), statusDot(st.installed && !st.outdated),
        h("span", { text: state })),
      h("div", { class: "row" }, h("label", { text: file }), h("span", { class: "path", text: st.settingsPath })),
      h("div", { class: "row" },
        h("label", { text: "Relay" }),
        h("span", { class: "path", text: st.hookPath }),
        statusDot(st.hookReady),
      ),
    );
    for (const note of st.notes) body.append(h("div", { class: "notice warn", text: note }));
    if (!st.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: "mayak-hook.exe is not in place yet. Restart Mayak; if it still fails, build it with `cargo build -p mayak-hook`.",
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: st.installed ? (st.outdated ? "Update…" : "Reinstall…") : "Install…",
      onclick: () => void showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every session a broken hook and nothing to show for it.
    if (!st.hookReady) {
      install.disabled = true;
      install.title = "The relay isn't installed yet.";
    }
    actions.append(install);
    if (st.installed) {
      actions.append(h("button", { class: "danger", text: "Remove…", onclick: () => void showPreview(false) }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    showingStatus = false;
    let preview: HookPreview;
    try {
      preview = await Bridge.hooksPreview(install, target);
    } catch (err) {
      // An unreadable or invalid file stops here rather than being treated as
      // empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", { text: "Back", onclick: () => { clear(body); draw(); } })),
      );
      return;
    }
    clear(body);
    body.append(h("div", {
      class: "hint",
      text: install
        ? `This is exactly what will change in ${file}. Entries that are not Mayak's stay where they are.`
        : `This removes Mayak's entries only. Everything else in ${file} stays as it is.`,
    }));
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? "Back up and write" : "Back up and remove",
    });
    if (preview.unchanged) {
      // The entry is already exactly there: nothing would be written (D53).
      body.append(h("div", { class: "notice ok", text: "No change." }));
      confirm.disabled = true;
    } else {
      body.append(
        renderDiff(preview.diff),
        // hooks_apply backs up only a file that exists, and the preview cannot tell.
        h("div", { class: "row" }, h("span", {
          class: "path",
          text: `Backup (made only if the file already exists) → ${preview.backup}`,
        })),
      );
    }
    for (const note of preview.notes) body.append(h("div", { class: "notice warn", text: note }));
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(install, preview.fingerprint, target);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: backup === ""
            ? "Done. There was no previous file, so there is no backup."
            : `Done. The previous file is saved as ${backup}.`,
        }));
        window.setTimeout(() => {
          void load();
          // The store has had time to apply CodexHooksInstalled by now.
          if (target === "codex") void refreshSnapshot();
        }, 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: `Could not write: ${String(err)}` }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: "Cancel",
      onclick: () => { clear(body); draw(); },
    })));
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
      rows.append(h("div", { class: "hint", text: "The observer has not published anything yet." }));
      return;
    }
    const line = (label: string, what: string, health: Health) => {
      const v = healthView(health);
      return h("div", { class: "row" },
        h("label", { text: label }), colorDot(v.color), h("span", { text: v.text }),
        h("span", { class: "hint", text: what }));
    };
    rows.append(
      line("Claude Code", "~/.claude/sessions and transcripts, read-only", s.sources.claude),
      line("Codex", "rollouts named by Codex hooks, read-only", s.sources.codex),
    );
  };
  draw(snap);
  snapshotViews.push(draw);
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Agent sessions" })),
    h("div", {
      class: "hint",
      text: "Claude Code sessions (Desktop, Cursor, VS Code) are read from the files Claude Code keeps for itself. Nothing is written to their configuration.",
    }),
    rows,
  );
}

// ── Codex ─────────────────────────────────────────────────────────────────────

function receivingText(s: SessionsSnapshot | null): string {
  if (!s) return "Unknown.";
  const hooks = s.sources.codexHooks;
  if (hooks === "not_installed") return "Not installed.";
  if (hooks === "installed_unseen") {
    return "Installed. No Codex event since Mayak started — trust the hooks in Codex, then reload them or start a new thread.";
  }
  const hosts = hooks.receiving.hosts.map((x) => HOST_LABEL[x]).join(", ") || "Codex";
  const since = new Date(hooks.receiving.since).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `Receiving from ${hosts} since ${since}.`;
}

function trustSteps(): HTMLElement {
  return h(
    "div",
    { class: "steps" },
    h("div", { class: "hint", text: "Codex runs new hooks only after you trust them:" }),
    h("ol", {},
      h("li", { text: "In Codex, open the “hooks need review” notice." }),
      h("li", { text: "Choose Trust." }),
      h("li", { text: "Choose “Reload hooks”, or start a new thread: threads that are already open keep the hooks they loaded." }),
    ),
    h("div", { class: "hint", text: "Or run /hooks in Codex (Codex app: Settings → Hooks)." }),
    h("div", {
      class: "hint",
      text: "Mayak never marks hooks as trusted for you. Reordering hooks.json by hand asks for Trust again.",
    }),
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
    h("div", {
      class: "hint",
      text: "Optional. Mayak appends one hook group per event to ~/.codex/hooks.json, so Codex turns — working, waiting for permission, finished — show up in the island. The island only watches: it never answers Codex.",
    }),
    integrationSection("codex"),
    trustSteps(),
    h("div", { class: "row" }, h("label", { text: "Events" }), receiving),
    silentRow,
  );
}

// ── Alerts ────────────────────────────────────────────────────────────────────

function alertsSection(): HTMLElement {
  const item = (label: string, what: string, on: boolean, set: (v: boolean) => void) =>
    h("div", { class: "row" },
      h("label", { text: label }),
      toggle(on, (v) => { set(v); void save(); }),
      h("span", { class: "hint", text: what }),
    );
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Alerts" })),
    h("div", {
      class: "hint",
      text: "A card and a sound when an agent needs you. A type that is off neither sounds nor opens a card; its row and the counters still update.",
    }),
    item("Waiting", "permission requests and questions", settings.alertWaiting, (v) => { settings.alertWaiting = v; }),
    item("Finished", "a turn ended", settings.alertFinished, (v) => { settings.alertFinished = v; }),
    item("Error", "a turn ended on an error", settings.alertError, (v) => { settings.alertError = v; }),
  );
}

// ── Advanced: legacy Claude Code Allow/Deny ───────────────────────────────────

function advancedSection(): HTMLElement {
  const body = h("div", { class: "advanced-body" });
  const details = h(
    "details",
    { class: "advanced" },
    h("summary", { text: "Advanced — Claude Code Allow/Deny in the island (legacy)" }),
    body,
  );
  let built = false;
  // ~/.claude/settings.json is read only once this block is opened (spec §8).
  details.addEventListener("toggle", () => {
    if (!details.open || built) return;
    built = true;
    body.append(
      h("div", {
        class: "hint",
        text: "Not needed to see Claude Code sessions. This optional hook adds Allow / Deny buttons to the island for Claude Code permission requests by writing a PermissionRequest hook into ~/.claude/settings.json.",
      }),
      h("div", {
        class: "notice warn",
        text: "If ~/.claude/settings.json comes from your dotfiles: sync.ps1 pull will erase this entry, and sync.ps1 push will commit this machine's path into the dotfiles repository.",
      }),
      integrationSection("claude"),
    );
  });
  return h("section", {}, details);
}

// ── Claude API section ────────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
];

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: hasKey ? "Key saved in the Windows Credential Manager." : "No key yet — the chat needs one." });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? "••••••••••••  (stored)" : "sk-ant-...",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: "Save key" });
  const clearBtn = h("button", { class: "danger", text: "Remove" });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present
      ? "Key saved in the Windows Credential Manager."
      : "No key yet — the chat needs one.";
    field.placeholder = present ? "••••••••••••  (stored)" : "sk-ant-...";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("anthropic-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Saved. It never touches disk." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not save: ${String(err)}` }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("anthropic-api-key");
      feedback.append(h("div", { class: "notice ok", text: "Key removed." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not remove: ${String(err)}` }));
    }
  });

  const model = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of MODELS) model.append(h("option", { value: id, text: label }));
  if (!MODELS.some(([id]) => id === settings.model)) {
    model.append(h("option", { value: settings.model, text: settings.model }));
  }
  model.value = settings.model;
  model.addEventListener("change", () => {
    settings.model = model.value;
    void save();
  });

  clearBtn.style.display = hasKey ? "" : "none";

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: "Claude" })),
    state,
    h("div", { class: "row" }, h("label", { text: "API key" }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: "Model" }), model),
    feedback,
  );
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;
/** How long a silent Codex row stays live after its last rollout write (silent-codex spec §3). */
const SILENT_LIVE_MS = 120_000;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Pick up to ${MAX_ACTIVE} pills to show next to Mochi — ${used}/${MAX_ACTIVE} in use. Keys are stored in the Windows Credential Manager, never on disk.`;
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
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
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: "Save" });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: field.label }),
          input, saveBtn, dotEl,
        ),
      );
    }

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
  return h("section", {}, h("h2", {}, h("span", { text: "Integrations" })), note, list);
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
  if (key == null) return { hint: "Use a letter, a digit, F1–F24 or Space." };
  const mods = [
    e.ctrlKey ? "Ctrl" : null,
    e.altKey ? "Alt" : null,
    e.shiftKey ? "Shift" : null,
    e.metaKey ? "Win" : null,
  ].filter((m): m is string => m != null);
  // A letter, digit or Space needs Ctrl, Alt or Win (Shift alone is just a capital: it would take
  // over ordinary typing); F1–F24 may stand alone.
  if (!isF && !(e.ctrlKey || e.altKey || e.metaKey)) {
    return { hint: "Add Ctrl, Alt or Win: Shift alone is not enough, only F1–F24 work without." };
  }
  return { combo: [...mods, key].join("+") };
}

/** The line under the shortcut for a status from hotkey.rs: empty for active and off. */
function hotkeyStatusText(s: HotkeyStatus): string {
  switch (s.state) {
    case "taken":
      return `${s.combo} is taken by another app — choose another${s.error ? ` (${s.error})` : ""}`;
    case "invalid":
    case "unsupported":
      return s.error ?? "";
    default:
      return "";
  }
}

/**
 * «Show / hide island»: an on/off switch, the shortcut as a button that records a new one, and
 * the status line. Off keeps the shortcut, so switching on brings the same one back. While recording the shortcut is released (hotkey_pause), or pressing it would toggle
 * the island instead of reaching this page.
 */
function hotkeyRows(): HTMLElement[] {
  const field = h("button", {}) as HTMLButtonElement;
  const line = h("div", { class: "hint" });
  let recording = false;
  // The pause invoke in flight: the resume waits for it, so the two reach Rust in order.
  let pausing: Promise<unknown> = Promise.resolve();
  let status: HotkeyStatus | null = null;

  function draw() {
    field.textContent = recording ? "Press keys…" : settings.hotkey;
    field.disabled = !settings.hotkeyEnabled;
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
    window.removeEventListener("keydown", onKey, true);
    const next = chosen ?? settings.hotkey;
    const changed = next !== settings.hotkey;
    settings.hotkey = next;
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
    e.preventDefault();
    e.stopPropagation();
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

  field.addEventListener("click", () => {
    if (recording) return;
    recording = true;
    // A focused button would be clicked again by the Space or Enter of the recorded shortcut.
    field.blur();
    line.textContent = "";
    draw();
    pausing = Bridge.hotkeyPause(true);
    window.addEventListener("keydown", onKey, true);
  });

  const sw = toggle(settings.hotkeyEnabled, (on) => {
    void (async () => {
      // A recording in progress ends unchanged before the switch takes effect.
      await finish(null);
      settings.hotkeyEnabled = on;
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
  void onEvent<HotkeyStatus>("hotkey-status", (s) => {
    heard = true;
    status = s;
    draw();
  });
  void Bridge.hotkeyStatus().then((s) => {
    // An event that arrived meanwhile is newer than this answer.
    if (heard) return;
    status = s;
    draw();
  });
  // A page reloaded in the middle of a recording left the shortcut released: give it back.
  // A resume without a pause does nothing (hotkey.rs Slot::resume).
  void Bridge.hotkeyPause(false);
  draw();

  return [
    h("div", { class: "row" }, h("label", { text: "Show / hide island" }), sw, field),
    line,
  ];
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

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

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Main display" }),
    h("option", { value: "cursor", text: "Display under the cursor" }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" },
      h("label", { text: "Sound" }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: "Auto-close" }),
      autoClose,
      h("span", { class: "hint", text: "seconds after you leave the island" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Island lives on" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
    ...hotkeyRows(),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  // The window is created hidden at launch and shown later; registered before
  // anything is awaited, so a show during the awaits below is never missed.
  window.addEventListener("focus", refreshOnFocus);

  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  takeSnapshot(await Bridge.sessionsSnapshot());

  const hasKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Mayak" }), h("span", { class: "version", text: version })),
    sourcesSection(snapshot),
    codexSection(snapshot),
    alertsSection(),
    apiSection(hasKey),
    integrationsSection(present),
    generalSection(),
    advancedSection(),
    h("div", {
      class: "hint",
      text: "No telemetry. Network requests only go to the services you configure yourself.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();
