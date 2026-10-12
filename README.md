<div align="center">

<img src="windows/src-tauri/icons/128x128.png" width="96" alt="Mayak icon">

# Mayak

**An island at the top of your Windows screen that keeps an eye on your AI coding agents.**

Mayak is an assistant for the work you are doing right now and a control panel for
your AI coding agents and their sessions. It is not a chat agent of its own: it does
not write code or talk to a model for you (the chat tab stays off).

Luch (Луч, “ray of light”), a small fire spirit, lives in it. He shows your Claude Code,
Codex and Antigravity sessions and which of them wait for you, lets you answer
permission requests and questions with a click, brings up the right window, and keeps
your plan limits, music and focus timer one glance away.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows&logoColor=white)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

</div>

---

## What Mayak is

Mayak is a fork of [Coucou](https://github.com/Louis-CFM/coucou) by
[Louis Raillé](https://louisraille.fr), a notch companion for macOS that also has a
Tauri version for Windows and Linux. Mayak takes that Tauri version and works on
Windows only:

- the character is Luch, a fire spirit drawn in code (a WebGL shader and a 2D face),
  instead of Coucou's Mochi;
- the 28 sounds and the app and tray icons are Mayak's own, generated in code;
- the island watches several agent sessions at once, grouped by project, and adds
  limits, providers, a player, a focus timer, stats and an app launcher.

The app lives in [`windows/`](windows/). The rest of the repository still holds
upstream's macOS app (`NotchBuddy/`), its docs (`docs/SPEC.md`, `docs/INTEGRATIONS.md`,
the `docs/*.html` site) and its design files (`design/`). Mayak does not maintain or
build them, and provides no macOS or Linux build.

## Features

Everything below is in the current Windows code.

**Agent sessions**

- **Claude Code** sessions show up on their own: Mayak reads Claude Code's session and
  transcript files, no hook needed.
- **Codex** sessions show up once you install the Codex integration (Settings → Agents).
  A Codex session whose hooks never reach Mayak still shows, as a muted "no hooks" row
  with the reason (folder gone, hooks not installed, hooks not trusted).
- **Antigravity** (the IDE agent and the `agy` CLI) shows up once its hook is installed
  from Settings. Mayak only watches it: Antigravity keeps asking for permissions in its
  own window.
- Sessions are grouped by project, with who is working, who is waiting for you and who
  is done, the last tool or answer, and how full the context is: a ring and a percent
  (for Claude against the model's real context window, with the model and the window in
  the tooltip).
- **Waiting sessions**: the tray icon counts them, and its tooltip lists their projects
  and how long each has waited. A wait nobody answered sounds again and Luch peeks out
  (after 10 minutes by default, at most three times; Settings → Alerts → Remind: off, 3,
  10 or 30 minutes).
- **Turn summary** for Claude Code: +N −M lines on the row, and the changed files as
  chips on the session and finished cards; a chip opens the file in Cursor or VS Code at
  its first changed line.
- **Compaction**: one notice when a session's context reaches 85 %, before
  auto-compaction, and a "compacted HH:MM" mark after it (Settings → Alerts → Context).
- **Git state** in each project header: branch or detached commit, uncommitted changes,
  a rebase or merge in progress, conflicts, and commits to push and pull. Mayak only
  reads git.
- **Recent · 24 h** under the list: the Claude Code and Codex sessions of the last day.
  **Resume** opens a new console running `claude --resume` or `codex resume` in the
  session's folder; **Copy** puts the PowerShell command on the clipboard.
- **Right-click a row** (or press the menu key on it) to copy the last reply, the project
  path or the resume command, or to open the folder.
- Each row shows its host's icon: Cursor, VS Code, Claude Desktop, Codex, Antigravity,
  or the terminal it runs in (Windows Terminal, PowerShell, Command Prompt, Git Bash and
  more).
- **Open** switches to the session's window: Cursor or VS Code (multi-root workspaces
  too), Claude Desktop (its own chat), Codex Desktop, Antigravity, or the terminal.
  A waiting card the island let go offers ↗ **Open** next to OK, to answer in that
  window.
- A **↗** button on a hovered or selected session row does the same as **Open** in one
  press; its tooltip names the host.

**Answers from the island** (off until you turn them on, per agent)

- Claude Code permission requests: **Allow / Always / Deny**, or **In terminal** to
  answer in the agent's own window.
- Claude Code questions (`AskUserQuestion`): pick the options, several questions at
  once, or an "Other" answer.
- Codex permission requests: **Allow / Deny**.
- Nothing is ever allowed, denied or answered without your click. Folding the island
  hands the question back to the agent. The agent waits 10 minutes at most.

**Home page: limits, providers and integrations**

- Claude and Codex plan limits as four rings (5 h and 7 d) in a 2 × 2 table, amber from
  80 % and red from 95 %. Claude's come from Claude Code's own cache; fresh Codex limits
  are read from the Codex CLI app server when the page opens with old data and right
  after a window resets (on by default, Settings → Agents → Codex).
- Every detail is in a popover: hover a ring or reach it with Tab, click to pin it,
  `Esc` closes it. It shows the window, the percent, when it resets, and the pace: when
  the limit would run out at your current rate.
- Alerts at 80 % and 100 % of a limit, once per window.
- OpenRouter credits and DeepSeek and Kimi (Moonshot) balances, from your API keys, on
  one line under the rings.
- Your active integrations as tiles, up to four, two per row: GitHub and GitLab counts
  and CI dots, Vercel, Stripe and the others. A row opens its link; with nothing
  connected, the page points to Settings.

**Luch**

- Catches from a lighter's sparks at launch, wakes, waves and settles into the compact
  island.
- Looks at the pointer, blinks, gets annoyed when clicked and dizzy after three clicks,
  shows hearts when the pointer rests on him for two seconds.
- Dances inside his fire while music plays.
- Shows the focus timer in the fire: a steady flame for work, a softer one for a short
  break, a smouldering ember for a long break.
- Changes colour like a fire: a short flare, then the flame settles in the new colour.
  Mini Luch in the pills burns in the pill's colour.

**Island pages**

- **Sessions** and **Home** (limits, providers and integration tiles).
- **Stats**: tokens for 7 and 30 days, by day, project, agent and model, counted from
  the agents' own files.
- **Focus**: a pomodoro, and a focus mode that holds "done" and "error" alerts until it
  ends.
- **Apps**: open Cursor, VS Code, Windsurf, Zed, Claude, Codex and Antigravity, add your
  own apps, reorder, rename, remove or hide them.
- **Player** under every page, for any app that reports to Windows media controls: cover,
  play/pause, previous/next, a seek bar, shuffle and repeat when the player offers them.

**Getting around**

- The hidden island is a thin line at the top centre of the screen. It stretches towards
  the pointer and opens on the first touch; it hides over fullscreen apps.
- `Alt+M` (changeable in Settings) opens and folds the island from any app and gives it
  the keyboard: ↑/↓ walk the sessions, Enter opens one, ←/→ switch pages, `Esc` closes.
- `Alt+Shift+M` (changeable in Settings) brings up the window of the session waiting
  longest, without opening the island; press it again for the next waiting one.
- Switch pages with a horizontal touchpad swipe or a tilted mouse wheel.
- Optional: open the island on hover. Choose the page it opens on.
- Fold it with ⌃; mute it with the bell (the same as the tray's **Pause**).

**Integrations**

- Up to 4 integrations, as coloured pills next to Luch and as tiles on the home page:
  Stripe, GitHub, GitLab, Vercel, n8n, Resend, Notion and Cal.com, each from a key you
  enter in Settings.
- **Connect** GitHub and GitLab with a code instead of a key: Settings → Integrations
  shows a code to enter on the service's page; **Disconnect** deletes the token. Pasting
  a token still works.
- **GitHub**: your open pull requests, the ones waiting for your review and their CI,
  with a sound and a badge for a new review request or a CI change. Mayak connects
  through MayakIslandApp, a read-only GitHub App that sees only the repositories you
  install it on (Settings → **Install the app on repositories**).
- **GitLab**, on gitlab.com or a self-managed https host: merge requests assigned to you
  and waiting for your review, CI of your merge requests and of the default branches of
  your recent projects, and issues assigned to you. If GitLab refuses sign-in by code,
  **Create a token on GitLab** opens its token page filled in for Mayak (`read_api`).

**Settings and sounds**

- A settings window with one page per group: Island, Sound, Alerts, Agents, Limits and
  providers, Integrations, System. It follows changes made in the island.
- Launch at login, kept in sync with Windows (including Task Manager's startup switch).
- 28 sounds. Put a file named after a sound (`finish.wav`, `approval.mp3`…) in
  `%APPDATA%\Mayak\sounds` and press **Reload sounds** to replace it.
- Seven languages: English, Russian, German, French, Spanish, Chinese and Japanese.

**Not in Mayak**

- The chat tab and the file drop are switched off (`CHAT_ENABLED = false` in
  `windows/src/core/features.ts`); the island refuses dropped files.
- Coucou's wardrobe, live diffs, email sending and dragging the character onto a window
  are macOS features and are not in the Windows app.
- No Gemini CLI sessions.

## Versions

See [CHANGELOG.md](CHANGELOG.md) for the full notes of each version. The older
`0.1.x` entries in the changelog are upstream Coucou's macOS releases.

| Version | Date | Highlights |
|---------|------|------------|
| Mayak for Windows 0.5.0 | Oct 12, 2026 | Home page with limit rings and integration tiles, Connect by code for GitHub and GitLab, GitLab, GitHub pull requests and CI, quick open on the session row |
| Mayak for Windows 0.4.0 | Oct 11, 2026 | Agent control: waiting sessions in the tray with reminders, Alt+Shift+M, turn summary, Claude context ring, git state, compaction notice, Recent · 24 h with Resume, row menu |
| Mayak for Windows 0.3.0 | Oct 11, 2026 | Luch the fire spirit, answers from the island, Antigravity, provider balances, own sounds, player, apps, seven languages |
| Mayak for Windows 0.2.0 | Oct 7, 2026 | All sessions by project, window switching, the strip, Alt+M, context fill, plan limits |

## Install

Download the latest build from [Releases](https://github.com/mahach666/mayak/releases):

| File | What it is |
|---|---|
| `Mayak-Windows-<version>-setup.exe` | Installer for the current user (recommended) |
| `Mayak-Windows-<version>.msi` | MSI package, for managed installs |
| `Mayak-Windows-<version>-portable.zip` | `mayak.exe` + `mayak-hook.exe`, no installation |

Each release lists the SHA-256 of its files in `SHA256SUMS.txt`.

The builds are **not code-signed**. Windows SmartScreen will warn about an unknown
publisher (**More info → Run anyway**), and Microsoft Defender has flagged earlier
unsigned builds of this app as `Trojan:Win32/Wacatac.H!ml` — a machine-learning false
positive. If you'd rather not run an unsigned binary, build it yourself.

### Build from source

You need:

- [Rust](https://rustup.rs) (stable, MSVC toolchain);
- [Node 20+](https://nodejs.org) (Node 22.6+ for `npm run check`);
- the **MSVC build tools** (Visual Studio Build Tools with "Desktop development with
  C++"). WebView2 ships with Windows 10/11.

```powershell
git clone https://github.com/mahach666/mayak.git
cd mayak/windows
npm install
npm run tauri dev      # development build with live reload
npm run tauri build    # release build: target/release/mayak.exe and the installers
npm run pack           # tauri build, then copies the installers to windows/release/
```

The relay `mayak-hook.exe` is built first by `npm run dev` and `npm run build`, which
`tauri dev` and `tauri build` call. After changing `windows/hook/` or `windows/ident.rs`,
rebuild it with `cargo build --release -p mayak-hook`; the app installs the relay from
`target/release/mayak-hook.exe`.

`target/release/mayak.exe` runs on its own; installing is optional. The setup `.exe`
installs for the current user (no admin prompt), the `.msi` for all users. Install one
of the two, not both.

Windows SmartScreen may warn about an unsigned build you made yourself, and Defender
may flag it the same way as above. Check what you built before allowing it.

There is no taskbar window and no console: the island at the top of the screen and the
tray icon are the whole app. **Quit** is in the tray menu.

More build details (checks, sounds, icons, layout, log) are in
[`windows/README.md`](windows/README.md).

## Setup

Open the tray icon → **Settings…**

| What | Why | What is written |
|---|---|---|
| Nothing (Claude Code) | Claude Code sessions | nothing: Mayak reads Claude Code's files |
| **Agents → Codex → Install** | Codex sessions and limits | Mayak's hook groups in `%USERPROFILE%\.codex\hooks.json`; then trust them in Codex (`/hooks`) |
| **Agents → Antigravity → Install** | Antigravity sessions | one `mayak` hook group in `%USERPROFILE%\.gemini\config\hooks.json` |
| **Answers from the island → Claude Code** | Allow / Deny and questions from the island | two entries in `%USERPROFILE%\.claude\settings.json` |
| **Answers from the island → Codex** | Allow / Deny from the island | switches Mayak's PermissionRequest entry in `hooks.json` to wait for the island |
| **Limits and providers** | OpenRouter, DeepSeek, Kimi balances | API keys in Windows Credential Manager |
| **Integrations** | Stripe, GitHub, GitLab, Vercel, n8n, Resend, Notion, Cal.com pills and tiles | keys and tokens (pasted or from **Connect**) in Windows Credential Manager |

Every change to an agent's config file shows you the exact diff and the path of the
dated backup first, and **nothing is written until you click Apply**. Your own hooks
are kept as they are, and **Remove…** / **Disable…** take back only Mayak's part.

If Mayak is not running, the relay exits at once and the agent goes on: Mayak never
blocks an agent unless a card is on screen waiting for your click.

## Privacy

- No telemetry, no account.
- Network requests go only to services you configured: the integrations you added keys
  for or connected (GitHub: `github.com` and `api.github.com`; GitLab: only the address
  you saved), and OpenRouter, DeepSeek or Moonshot when you entered a key. Codex limits
  are read by starting your local Codex CLI, which uses its own login.
- Keys and tokens live in the **Windows Credential Manager**, never on disk and never in
  the island's interface.
- The log, `%LOCALAPPDATA%\Mayak\mayak.log`, stays on your machine.

## How it works

- A [Tauri 2](https://tauri.app) app: Rust backend in `windows/src-tauri/`, TypeScript
  front end with no framework in `windows/src/`.
- The island is a transparent, always-on-top window that never takes focus unless you
  give it the keyboard.
- A session observer watches Claude Code's session and transcript files and Codex
  rollouts, and takes hook events from the relay; a pure model turns them into the
  session list.
- `mayak-hook.exe` (`windows/hook/`) is the relay the agents' hooks run. It is copied to
  `%LOCALAPPDATA%\Mayak\bin\` at launch and talks to the app over a named pipe. It gets
  300 ms to reach Mayak and exits cleanly if the app is closed or slow.
- Luch is a WebGL fire shader with a 2D face on top (`windows/src/luch/`).
- Integrations are small pollers, paused when the island is paused.

## Contributing

Issues and pull requests are welcome at
[mahach666/mayak](https://github.com/mahach666/mayak). Before a commit:

```powershell
cd windows
npm run check            # front-end checks (scripts/checks/)
cargo test --workspace   # Rust tests, the relay's included
```

Keep the house rules from [CLAUDE.md](CLAUDE.md): no telemetry, keys only in Credential
Manager, never block an agent, never write an agent's config without a backup, a diff
and the user's click. [CONTRIBUTING.md](CONTRIBUTING.md) is upstream's guide for the
macOS app.

## Credits

- Mayak is built on [Coucou](https://github.com/Louis-CFM/coucou) by
  [Louis Raillé](https://louisraille.fr). The island, the session cards, the hooks
  relay and the integrations come from there.
- Mayak's changes, Luch and the Windows work: [mahach666](https://github.com/mahach666).
- The Cursor and VS Code icons come from
  [Catppuccin vscode-icons](https://github.com/catppuccin/vscode-icons) (MIT); see
  [`windows/THIRD_PARTY_NOTICES.md`](windows/THIRD_PARTY_NOTICES.md).

## License

- **Code:** [MIT](LICENSE), copyright (c) 2026 Louis Raillé. Keep the notice when you
  copy or fork.
- **Coucou's name, the Mochi character, its icon, sounds and media** (`NotchBuddy/`
  assets, `docs/media/`, `design/`) remain © Louis Raillé, all rights reserved; see
  [LICENSE-ASSETS.md](LICENSE-ASSETS.md). Mayak does not use them in its app: it has its
  own name, character, icons and sounds.
