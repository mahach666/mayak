# Mayak Island Answers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From the island, answer Claude Code permission requests (Allow / Always / Deny), Claude Code `AskUserQuestion` questions, and Codex permission requests. While the card is open the agent waits; once the island folds, the agent asks in its own window.

**Architecture:** The relay (`windows/hook`) holds the hook open, forwards the payload over the named pipe, and prints the agent's reply JSON. The server (`pipe.rs`) routes each request to an island card and waits for a click, a release or a timeout. The island shows the card and sends the decision. The installers write the hook entries through the existing backup → diff → confirm flow.

**Tech Stack:** Rust (relay crate `windows/hook`, app `windows/src-tauri`), TypeScript (`windows/src`).

**Spec:** `docs/superpowers/specs/2026-10-08-mayak-island-answers-design.md`. Research with sources: `docs/superpowers/specs/2026-10-08-mayak-island-answers-research.md`.

## Global Constraints

- **Exact reply JSON** (spec §2.2–2.4):
  - allow: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`;
  - always: the same with `"updatedPermissions":<permission_suggestions verbatim>` (Claude only);
  - deny: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Mayak"}}}`;
  - answer: `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"questions":<original array, untruncated>,"answers":{"<question text>":"<label>" or ["<label>",…]}}}}`;
  - release or timeout: empty stdout, exit 0.
- **Timeouts:**
  - hook entries `timeout: 600`;
  - relay budget 595 s;
  - pipe decision 590 s;
  - `ACK_TIMEOUT` 0.8 s, unchanged;
  - the island card lasts `autoCloseInterval` while the cursor is off the island, and is released when the island folds.
- **Pipe reply line** (server → relay): exactly one of `allow`, `always`, `deny`, `release`, `answer <compact JSON object of answers>`, then `\n`. The relay treats anything else as `release`.
- **Codex** is routed to the island only while Mayak's Codex `PermissionRequest` entry has `timeout: 600`, read from `hooks.json` at launch and after each install or removal. Otherwise it stays `Observe`, as today.
- **Safety rules:**
  - writes to `~/.claude/settings.json` and `~/.codex/hooks.json` only through the existing preview/apply flow (backup, diff, confirm);
  - never write `[hooks.state]` or `trusted_hash`;
  - never answer without an explicit click;
  - never log payload text, commands, questions or paths;
  - the pipe keeps its current-user-only DACL;
  - tests use temp files only, never the real config files.
- **The relay must never slow agents beyond the user's choice.** It still exits at once when Mayak is not running (`CONNECT_TIMEOUT`) and for every event that is not answered.
- **Codex `statusMessage`:** `"Waiting for your answer in Mayak"`.
- **UI texts** go into all seven dictionaries `windows/src/core/i18n/{en,ru,de,fr,es,zh,ja}.ts` with identical `{var}` sets (`C:/tmp/mayak-spike/i18n-keys-check.mjs`). User-visible Rust messages are `crate::message::Message` with codes `msg.*`, plus `message-check.mjs`.
- **Commits:** one line, no trailers, no `Co-Authored-By`, never mention Claude or AI. Stage by explicit path.
- **Checks:**
  - `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target` then `cd windows && cargo test --workspace` (0 warnings; `-j 1` if memory is short);
  - `npx tsc --noEmit`;
  - `npx vite build`;
  - the Node checks in `C:/tmp/mayak-spike/`.
  - `windows/target/release/mayak-hook.exe` must exist for the app build (tauri resource): build it with `cargo build -p mayak-hook --release` and copy it there if it is missing.

## Review Focus

1. **A huge `AskUserQuestion` payload** (questions over 2000 chars) must echo `questions` byte-for-byte. The truncated copy sent over the pipe must never be the one printed back (Task 1 test).
2. **A click that arrives after a release or timeout** must not print anything and must not throw. The server ignores unknown or expired ids (Task 2 test).
3. **Two requests at once.** The second is released immediately and the first keeps its card (Task 2/5).
4. **Codex installed with the old `timeout: 10`** while the island answering flag is on: the server must not route it to the island, so the card does not show and then die at 10 s (Task 2: route only when the entry has 600).
5. **"Always" with no `permission_suggestions`:** no Always button, and `always` from the island degrades to `allow` (Tasks 1/5).

---

### Task 1: The relay holds Claude questions and Codex permissions, and prints the replies

**Files:** `windows/hook/src/lib.rs`, `windows/hook/src/main.rs` (and its tests).

**Interfaces:**
- `fn waits_for_answer(agent, event, payload) -> bool`: true for Claude `PermissionRequest`, Claude `PreToolUse` with `tool_name == "AskUserQuestion"`, and Codex `PermissionRequest`.
- `ANSWER_BUDGET: Duration = 595 s`. It replaces `DECISION_BUDGET` for every waiting case and applies to Codex `PermissionRequest` too.
- `enum Reply { Allow, Always, Deny, Release, Answer(serde_json::Value) }` and `fn parse_reply(line: &str) -> Reply`.
- `fn reply_json(event, agent, reply, raw: &RawInput) -> Option<String>`.
  - `RawInput` keeps the untruncated `tool_input.questions` and `permission_suggestions` from stdin, taken before `MAX_FIELD_LEN` truncation.
  - Returns `None` for Release, for Always on Codex, and for Answer on any event other than PreToolUse.
  - Always with missing or empty suggestions prints the allow JSON.
  - Answer requires the answers object.
- The D87 filter stays: `AskUserQuestion` arriving as `PermissionRequest` → empty output and no pipe.
- Other `PreToolUse` events (another tool) → exit at once with no pipe and no output.

- [ ] **Step 1: Failing tests.**
  - Exact JSON for allow, always (with suggestions), always (no suggestions → allow), deny, Codex allow and deny, answer single, answer multi (array), and answer for two questions.
  - A question text of 5000 chars is echoed untruncated in `questions`.
  - Release and garbage reply → None.
  - `parse_reply` covers each form; an `answer` with invalid JSON → Release.
  - `waits_for_answer` covers each case.
- [ ] **Step 2:** Implement. Keep existing behaviour for every other event, and keep the 300 ms connect timeout.
- [ ] **Step 3:** `cargo test -p mayak-hook` and then the workspace.
- [ ] **Step 4: Commit:** `feat(hook): hold Claude questions and Codex permissions, print their answers`.

---

### Task 2: Server routes, decisions and expiry

**Files:** `windows/src-tauri/src/pipe.rs`, `windows/src-tauri/src/lib.rs` (commands, the hook event sink), `windows/src/core/bridge.ts` (types and commands), plus where the island receives the `hook` event (`windows/src/island/hooks.ts`), only for the payload shape.

**Interfaces:**
- **Routes:**
  - `route(agent, event, payload, codex_answers: bool) -> Route`;
  - `("claude","PermissionRequest") → Approval`;
  - `("claude","PreToolUse")` with tool AskUserQuestion `→ Question`;
  - `("codex","PermissionRequest") → Approval` when `codex_answers`, else `Observe`;
  - other Codex events `→ Observe`;
  - the rest `→ Ignore`.
  - A Codex `PermissionRequest` routed to `Approval` is also sent to the observer, as `Observe` does today, so the session shows as waiting.
- `DECISION_TIMEOUT = 590 s`.
- **Kinds:** the island event payload gains `kind: "permission" | "question"` and `agent: "claude" | "codex"`. Claude permissions also carry `canAlways: bool` (non-empty `permission_suggestions`).
- **Commands:**
  - `approval_decision(request_id, decision: "allow"|"always"|"deny")`: `always` for Codex or without suggestions becomes `allow`;
  - `question_answer(request_id, answers: serde_json::Value)`: validated as an object of string → string or array of strings, else rejected;
  - `approval_decline(request_id)` writes `release`.
  - Unknown or expired ids are ignored silently: no panic and no log of payload.
- **Codex flag:** `codex_answers` is shared state, set from `codex_hooks::answers_enabled(&hooks_json)` at launch and after each Codex install or remove. True when Mayak's `PermissionRequest` entry has `timeout == 600`.

- [ ] **Step 1: Failing tests.**
  - The route table, including Codex with the flag on and off.
  - The reply line written for each decision, and `answer` with the compact JSON.
  - Answers validation.
  - An expired id is ignored.
  - A second request while one card is pending is released (if that logic lives in the server; otherwise this is tested in Task 5).
- [ ] **Step 2:** Implement, with commands registered and bridge functions added: `Bridge.approvalDecision(id, d)`, `Bridge.questionAnswer(id, answers)`, `Bridge.approvalDecline(id)`.
- [ ] **Step 3: Checks.**
- [ ] **Step 4: Commit:** `feat(pipe): route Claude questions and Codex permissions to the island`.

---

### Task 3: Installers — Claude two entries at 600 s, Codex permission at 600 s

**Files:** `windows/src-tauri/src/hooks.rs`, `windows/src-tauri/src/codex_hooks.rs`, `windows/src-tauri/src/hookfile.rs` (only if a helper is needed), `windows/src-tauri/src/lib.rs` (commands for the Codex answers toggle, refreshing the `codex_answers` flag).

**Interfaces:**
- **Claude** (`hooks.rs`):
  - the planned entries are `PermissionRequest` `{type: "command", command: <same form>, timeout: 600}` and `PreToolUse` `{matcher: "AskUserQuestion", hooks: [{type: "command", command: "<exe> PreToolUse", timeout: 600}]}`;
  - a Mayak entry with `timeout: 120` or without the `PreToolUse` entry is reported as outdated (Update);
  - removal strips both;
  - the shape check also covers `hooks.PreToolUse` being an array.
- **Codex** (`codex_hooks.rs`):
  - `answers_enabled(json) -> bool`;
  - `plan_answers(on: bool)` returns a preview/apply plan that changes only Mayak's `PermissionRequest` group, to `timeout: 600` plus `statusMessage: "Waiting for your answer in Mayak"` (on), or back to `timeout: 10` with no `statusMessage` (off);
  - other events are untouched;
  - if Mayak's Codex hooks are not installed, return a `Message` asking to install the Codex integration first.
- **Commands:** `codex_answers_preview(on)` and `codex_answers_apply(target fingerprint)`, following the existing preview/apply pattern. After apply, refresh the server's `codex_answers` flag.

- [ ] **Step 1: Failing tests** (temp files only):
  - Claude fresh install writes both entries;
  - an old 120 s entry → outdated, and Update fixes it;
  - removal strips both;
  - a foreign `PreToolUse` entry is kept;
  - a bad `PreToolUse` shape → refusal;
  - Codex on/off changes only `PermissionRequest` timeout and `statusMessage`;
  - `answers_enabled`;
  - "not installed" → Message.
- [ ] **Step 2:** Implement, keeping the existing BOM/CRLF/backup/fingerprint behaviour.
- [ ] **Step 3: Checks.**
- [ ] **Step 4: Commit:** `feat(hooks): install Claude permissions and questions, and Codex answers, at 600 s`.

---

### Task 4: Settings — the "Answers from the island" section

**Files:** `windows/src/settings/main.ts`, `windows/src/settings/settings.css` if needed, the seven dictionaries.

- [ ] Replace the `<details>` "Advanced — … (legacy)" block with a section titled "Answers from the island" (translated). It has two rows:
  - **Claude:** status (off / on / outdated), plus Enable / Update / Disable through the existing `integrationSection("claude")` preview → diff → Apply flow, and the note "Questions need Claude Code 2.1.85 or newer".
  - **Codex:** status (needs the Codex integration / off / on), and Enable / Disable through `codex_answers_preview/apply` with a diff and Apply. After Apply on, show the trust hint: "Codex will ask you to trust its hooks again: run /hooks in Codex (Codex app: Settings → Hooks) and choose Trust."
- [ ] Every text goes into the seven dictionaries. Remove the now-unused `set.advanced*` keys from all seven.
- [ ] **Checks**, plus a look at `http://localhost:1420/settings.html` (a dev server may be running).
- [ ] **Commit:** `feat(settings): answers from the island for Claude and Codex`.

---

### Task 5: Island — permission and question cards, release on fold

**Files:** `windows/src/island/hooks.ts`, `windows/src/views/views.ts` (`buildApproval`, a new `buildQuestion` card or a reuse of the existing `question` view if it fits), `windows/src/island/island.ts` (pinning, decide, collapse → release), `windows/src/core/state.ts`, `windows/src/core/layout.ts` if the question card needs its own height, a new pure module `windows/src/island/answers.ts`, `windows/src/style.css`, and the seven dictionaries.

**Interfaces:**
- **`island/answers.ts`** (pure, Node-importable):
  - `parseQuestions(toolInput) -> Question[] | null`, with 1–4 questions and 2–4 options, else null;
  - `buildAnswers(questions, picks) -> Record<string, string | string[]>`;
  - `canAnswer(questions, picks) -> boolean`.
- **Island behaviour:**
  - the card is shown with `island.alert(...)` and **not pinned**, so it folds by `autoCloseInterval` like other alerts, and the cursor over the island keeps it open;
  - when the island leaves the card (fold, other view, auto-close), call `Bridge.approvalDecline(id)` once;
  - buttons:
    - Claude permission: Allow / Always (only if `canAlways`) / Deny / In terminal;
    - Codex permission: Allow / Deny / In terminal;
    - Question: options, plus Answer for multi-select or several questions, plus In terminal;
  - pause and pomodoro mute rules stay as in spec §2.1.
- **Existing tie-ins:** the phase-card release after a decision (`releaseHeldCard`) and the language-reload decline keep working.

- [ ] **Step 1: Node check** `C:/tmp/mayak-spike/answers-check.mjs`:
  - `parseQuestions` accepts and rejects the right shapes;
  - `buildAnswers` single, multi (array) and two questions;
  - `canAnswer`.
  - Run it and see it fail first.
- [ ] **Step 2:** Implement the cards and the release-on-fold rule.
- [ ] **Step 3: Checks**, then live in the dev app over CDP (`C:/tmp/mayak-spike/cdp.mjs`, `shot.mjs`). Emit a synthetic `hook` event through `window.__TAURI_INTERNALS__.invoke("plugin:event|emit", …)` with fake request ids, for each kind, and check:
  - the card renders;
  - the buttons call the right Bridge commands (wrap `Bridge` methods to count the calls);
  - auto-close calls `approvalDecline` once.

  Restore the user's state afterwards.
- [ ] **Step 4: Commit:** `feat(island): answer permissions and questions from the island`.

---

### Task 6: A card whose session moved on goes away (spec §2.6)

**Files:** `windows/src/island/island.ts` (`onSnapshot`, `handleAlerts`, the card views' subject `cardSubject()`), `windows/src/core/state.ts`, possibly a new pure helper in `windows/src/island/alert-queue.ts`, Node check `C:/tmp/mayak-spike/alert-queue-check.mjs`.

- [ ] Read how cards know their session: `State.alertKey`, `State.alertFallback`, `cardSubject()`, the alert ranks (`Alert.rank`, `cardRank`), `sessionAck`, and how the observer marks sessions waiting, finished or error, and which of those are "unacknowledged".
- [ ] Pure helper (Node-checked):
  - `staleCard(card: {key, kind, since}, snapshot) -> boolean`: true when that session's state shows it moved on after the alert, i.e. a newer turn or no longer waiting or finished for that alert;
  - `nextAlert(snapshot, exceptKey) -> {key, kind} | null`: the most important other unacknowledged alert, by the existing ranks.
- [ ] In `onSnapshot`:
  - if the island is expanded on a session card (`waiting`, `finished`, `error`, or the answer cards once released) and `staleCard` is true, show `nextAlert(...)` as a card (silent, no sound), or else collapse;
  - leave a view the user opened (sessions, home, tabs, settings) alone;
  - no new timers.
- [ ] **Checks**, plus a live check over CDP with a synthetic snapshot sequence if the snapshot can be injected. Otherwise check the logic only through the Node check, and say so.
- [ ] **Commit:** `feat(island): a card whose session moved on gives way to the next alert or folds`.

---

### Task 7: Docs and live check with the user (controller)

- [ ] Update:
  - `windows/README.md`: how to enable, what Codex asks, the Claude Code version for questions;
  - the "legacy" comments in `hooks.rs`, `pipe.rs` and `island/hooks.ts`;
  - the night features spec §G: replaced by this spec.
- [ ] **Real install needs the user.** Ask the user to click Enable in Settings for Claude, confirm the diff, then ask Claude Code something that needs permission, and an `AskUserQuestion`. For Codex: Enable, then trust in `/hooks`, then a command needing approval. The controller watches and checks:
  - the card;
  - the agent's reaction to each button;
  - auto-close → the prompt in the agent's window;
  - the "waiting for you" row disappears after answering in the window.
