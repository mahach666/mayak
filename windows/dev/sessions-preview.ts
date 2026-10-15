// Dev harness: the island's sessions list built with the real buildSessions() and styles, on a fake
// snapshot with one session per host (Claude Desktop, Cursor, VS Code, the Codex app, Antigravity,
// agy, an unknown host) and a silent Codex row. Agent Orchestrator is on, with the real daemon's list
// (anonymised, src-tauri/src/ao/fixtures): two projects lie in AO session folders and carry its tag, the
// fold lists every AO session (&ao=open opens it, &ao=0 switches AO off). Not part of the app bundle.

import "../src/style.css";
import { buildSessions } from "../src/views/sessions";
import { Sessions } from "../src/core/sessions";
import { State } from "../src/core/state";
import { AO_ID } from "../src/core/ao";
import aoPayload from "../src-tauri/src/ao/fixtures/payload.json";
import { setLanguage } from "../src/core/i18n";
import type { ViewActions } from "../src/views/views";
import type { Host, Phase, SessionAgent, SessionView, SessionsSnapshot } from "../src/core/bridge";

const params = new URLSearchParams(location.search);
setLanguage(params.get("lang") ?? "en");

const now = Date.now();
const M = 60_000;

function session(key: string, agent: SessionAgent, host: Host, phase: Phase, ago: number, activity: string | null): SessionView {
  return {
    key, agent, host, title: null, phase, since: now - ago * M, activity, detail: null,
    outcome: null, attention: phase.kind === "waiting" ? "permission" : null, acked: false,
  };
}

const working: Phase = { kind: "working" };
const idle: Phase = { kind: "idle" };
const waiting: Phase = { kind: "waiting", wait: "permission", label: "" };

const snapshot: SessionsSnapshot = {
  rev: 1, now, attention: [], counts: { waiting: 1, error: 0, finished: 0, working: 4 },
  hold: false, aggregate: "working", sources: { claude: "ok", codex: "ok", codexHooks: "not_installed" },
  usage: { claude: null, codex: null, openrouter: null },
  projects: [
    // The folders of AO's mayak-6 and of its mayak orchestrator (the payload's workspace keys).
    {
      key: "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\mayak-6", name: "mayak-6",
      root: "C:\\Users\\dev\\.ao\\data\\worktrees\\mayak\\mayak-6", sessions: [
        session("claude:5", "claude", "vsCode", working, 1, "Edit ao.ts"),
        session("codex:2", "codex", "unknown", idle, 9, null),
      ],
    },
    {
      key: "c:\\users\\dev\\.ao\\data\\worktrees\\mayak\\orchestrator\\mayak-orchestrator", name: "mayak-orchestrator",
      root: "C:\\Users\\dev\\.ao\\data\\worktrees\\mayak\\orchestrator\\mayak-orchestrator", sessions: [
        session("claude:6", "claude", "cursor", idle, 4, null),
      ],
    },
    {
      key: "mayak", name: "mayak", root: "C:\src\mayak", sessions: [
        session("claude:1", "claude", "cursor", working, 3, "Edit sessions.ts"),
        session("claude:2", "claude", "claudeDesktop", waiting, 1, "Bash npm run check"),
        session("claude:3", "claude", "vsCode", working, 12, "Read style.css"),
        session("claude:4", "claude", "unknown", idle, 40, null),
      ],
    },
    {
      key: "site", name: "site", root: "C:\src\site", sessions: [
        // Outside every AO folder, but its hooks named AO's orbit-2 (the relay's AO_SESSION_ID): the header says so.
        { ...session("codex:1", "codex", "codexDesktop", working, 7, "Run tests"), ao: { session: "orbit-2", project: "orbit" } },
        session("antigravity:1", "antigravity", "antigravityIde", working, 2, "write_to_file"),
        session("antigravity:2", "antigravity", "antigravityCli", idle, 25, null),
        { ...session("silent:1", "codex", "cursor", idle, 0, null), silent: "hooksNotInstalled" },
      ],
    },
  ],
};

const actions = new Proxy({}, { get: () => () => {} }) as ViewActions;

const root = document.getElementById("panels")!;
const wrap = document.createElement("div");
wrap.className = "panel";
const h2 = document.createElement("h2");
h2.textContent = "Sessions list: one row per host";
const stage = document.createElement("div");
stage.className = "stage";
// &h=<px> makes the stage taller (the list scrolls inside it) to see a long list at once.
if (params.get("h")) stage.style.height = `${Number(params.get("h"))}px`;
wrap.append(h2, stage);
root.append(wrap);

State.settings.activeIntegrations = params.get("ao") === "0" ? [] : [AO_ID];
State.integrations[AO_ID] = { data: aoPayload, error: null, loaded: true, configured: true };
Sessions.aoOpen = params.get("ao") === "open";
State.mode = "expanded";
State.view = "sessions";
Sessions.accept(snapshot);
const host = buildSessions(actions);
host.el.classList.add("on");
stage.append(host.el);
host.sync();
document.body.dataset.ready = "1";
