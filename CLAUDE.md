# Mayak — guide for AI coding agents

Mayak is an island at the top of the screen, for programmers who work with AI coding agents: it shows every Claude Code, Codex and Antigravity session, lets the user answer permission requests and questions with a click, brings up the right window, runs a chat over the user's own agent CLIs, and keeps limits, a player and a focus timer in view. Luch, a small fire spirit drawn in code, lives in it. It is a work assistant and a control panel for agents, not an agent of its own: it never writes code or talks to a model for the user.

It runs on Windows and, from the same code, on macOS 13+ (Apple silicon). It is built on [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé (MIT); nothing of Coucou's native app or artwork is in this repository.

## Where things are
- `windows/src-tauri/` — Rust: the app, the session observer (`observe/`), the agent chat (`chat/`), the MCP server (`mcp_server/`), integrations, platform code (`platform/`).
- `windows/src/` — TypeScript, no framework: the island (`island/`), views (`views/`), pure models (`core/`), Luch (`luch/`), Settings (`settings/`), dictionaries (`core/i18n/`, seven languages).
- `windows/hook/` — `mayak-hook`, the relay the agents' hooks run.
- `windows/scripts/checks/` — the front end's checks, run straight from the TypeScript.
- `windows/README.md` — behaviour, settings, the Agents tab, the MCP server, Agent Orchestrator.
- `docs/superpowers/specs/` — the design of the features; `docs/superpowers/ideas/` — reviews of upstream and ideas. `docs/*.html` — the GitHub Pages site.

## Build and check
```
cd windows
npm install
npm run tauri dev        # development build
npx tsc --noEmit         # types
npm run check            # front-end checks
cargo test --workspace   # Rust tests, the relay's included
npm run pack             # installers in windows/release/
```

## Rules
- No third-party dependencies unless truly unavoidable. Luch is drawn in code (WebGL + Canvas), no Rive/Lottie/images.
- Secrets live in the OS credential store (Windows Credential Manager, the Keychain on a Mac), never on disk or in git, and never reach the page: it can only ask whether a key exists.
- No telemetry. Network calls only to services the user configured.
- Never block an agent: if the app does not answer within the relay's budget, the hook exits at once.
- Never overwrite an agent's config (`~/.claude/settings.json`, the Codex and Antigravity files): a dated backup, a merge, the diff shown, written only after the user confirms.
- Never send an email, and never approve a Claude Code or Codex permission, without an explicit click. A request waits for its click; nothing has a default or a timeout that approves.
- Performance: no CPU to speak of while the island is hidden.
- Stored identifiers are contract values: the settings keys, the pill ids, the credential-store keys, the bundle identifier `io.github.mahach666.mayak`. Never rename one.
- Never restyle what already ships (views, Settings, chat) unless asked; a new view follows the existing style.
- What was verified against a real CLI is written down as such; an adapter written from documentation only is marked unverified in the UI and the README.
- Every release adds its CHANGELOG.md section and a row in the README Versions table.
