# Coucou — guide for AI coding agents

Coucou is a native macOS app (`NotchBuddy/`); `windows/` is the Tauri version for Windows and Linux. Mochi, a small animated character living in the MacBook notch, shows AI coding agent sessions (Claude Code, Gemini CLI, Antigravity and more) and a few integrations, and lets the user approve, answer, chat and drop files from the notch.

## Where things are
- `NotchBuddy/Sources/App/` — all Swift code. `NotchBuddy/Resources/sounds/` — the 28 WAV sounds. `NotchBuddy/project.yml` — XcodeGen project (never edit the `.xcodeproj` by hand).
- `NotchBuddy/Sources/App/PillCatalog.swift` — single source of truth for all declared pills (workspace tools, agents, AI providers, services). Every pill ID, color, category and subtitle lives here.
- `docs/SPEC.md`, `docs/INTEGRATIONS.md` — behaviour, views, states, integrations (in French).
- `design/prototype/notch-buddy.html` — original prototype, the visual source of truth. `design/captures/` — target screenshots.
- `windows/` — the Tauri app for Windows and Linux: Rust in `src-tauri/`, TypeScript in `src/`, the `coucou-hook` relay in `hook/`. `windows/README.md` lists what differs from the Mac.
- `docs/*.html` — the GitHub Pages site (privacy, terms, support, legal notice).

## Build
```
cd NotchBuddy && xcodegen && xcodebuild -scheme NotchBuddy -configuration Debug build
```
Windows and Linux: `cd windows && npm install && npm run tauri dev`

## Rules
- Swift 6, SwiftUI + AppKit. No third-party dependencies unless truly unavoidable. The character is drawn in code (`Canvas` + `TimelineView`), no Rive/Lottie/images.
- Secrets live in the Keychain, never on disk or in git.
- No telemetry. Network calls only to services the user configured.
- Never block Claude Code: if the app doesn't answer, the hook exits immediately.
- Never overwrite `~/.claude/settings.json`: dated backup, merge, show the diff, write only after the user confirms.
- Never send an email or approve a Claude Code or Codex permission without an explicit click.
- Performance: 0 % CPU when the island is hidden.
- Keep the bundle identifier `fr.louisraille.NotchBuddy` (Keychain items, preferences and permissions depend on it).
- Never restyle what already ships (pills, cards, Settings, chat…): existing views stay exactly as they are in `main`, which is the App Store build. Change the look of an existing view only when explicitly asked.
- Pill IDs are stable contract values (Keychain, UserDefaults, hook routing): never rename an existing pill ID.
- New views follow the existing app style. `design/prototype/notch-buddy.html` and `design/captures/` are references for new work, not a reason to change existing views.
- Every release adds its CHANGELOG.md section and a row in the README Versions table.

<!-- coord:start -->
## Team coordination

This repo is set up for team coordination between agents working in shared
sessions. Follow these rules:

- Before starting non-trivial work, publish a plan with `publish_plan`.
- Keep step status current as you go, using `update_step`.
- If the injected context shows an overlap warning, coordinate before you
  edit (spec Law 3, Rev 28) — never silently duplicate the work. If it is the
  same task and you have not started, join their session: register on their
  branch, or offer `propose_work_request` — the server auto-accepts only
  when every step is provably unstarted and unassigned, so a safe join needs
  no human. If the work is related, claim only unstarted, unassigned steps
  the same way. Never take started or assigned work. Stop and ask your human
  only when your permission mode requires approval or no safe action exists.
- When you discover something non-obvious (a gotcha, a decision, a fact
  someone else would need), write it down with `remember` so the team
  keeps it.
- When something is a teammate’s job rather than yours — their area, their
  decision, a question only they can answer — hand it to them with
  `assign_work` instead of asking your human to relay a message. Use
  `list_people` to turn a name into an assignee id, put the whole brief
  in the note, and tell your human who you handed it to. Their own agent is
  told at its next prompt. This is free: no model call, no sponsor, nothing
  to pay for.
- When you hit a bug, a blocker, or something that plainly wants a different
  specialist, open it with `report_opportunity` and close it with
  `resolve_opportunity` when it is handled. Nothing reads your transcript
  looking for these, so a problem you only describe in your answer reaches
  nobody; reporting it is what lets the team's Brain offer it to whoever fits.
  Reporting is not notifying — it may reach no one — so do not tell your human
  a teammate was alerted.
<!-- coord:end -->
