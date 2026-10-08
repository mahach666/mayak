<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Mayak icon">

# Mayak for Windows

**Mochi doesn't get a notch on a PC — so it lives at the top of your screen instead.**

Approve Claude Code permissions, watch your session work, drop a file, chat with Claude, keep an eye on your services — without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

</div>

<img src="screenshots/greeting.png" width="640" alt="Mochi waving hello at launch">

---

## Install

Mayak does not publish an installer yet: it is not code-signed, and Microsoft
Defender has flagged unsigned builds of this app as malware
(`Trojan:Win32/Wacatac.H!ml`, a machine-learning false positive).

[Build it yourself](#build-it-yourself): it takes a few minutes and installs for
the current user only — no admin prompt.

## Using it

<img src="screenshots/compact.png" width="292" alt="The compact island, with the integration pills as mini Mochis">
<img src="screenshots/overview.png" width="640" alt="The overview: the focused integration on the left, the other pills on the right">
<img src="screenshots/approval.png" width="640" alt="A Claude Code permission request, with Deny and Allow">
<img src="screenshots/chat.png" width="640" alt="Chatting with Claude from the island">
<img src="screenshots/drop.png" width="640" alt="Mochi turned into a box, waiting for a file">

| What you do | What happens |
|---|---|
| Move the mouse to the very top-centre of the screen | Mochi peeks out |
| Bring the mouse near the thin line at the top centre | It stretches towards the pointer; touch it and Mochi peeks out |
| Click ⌃ in the island header, or on the small island | It folds away, down to the line |
| Click the bell in the island header | Quiet mode — the same as the tray's Pause, but the island stays open |
| `Alt+M` in any app (change or turn it off in Settings → General) | The island opens on the session list; press it again and it folds away to the line |
| Click the small island | It opens |
| Click Mochi | It gets annoyed. Three times in a row and it goes dizzy |
| Rest the pointer on Mochi for two seconds | Hearts |
| Drag a file onto the island | Mochi turns into a box, swallows it, then offers to answer questions about it |
| `Esc` | Closes the island |
| Tray icon | Open, Settings…, Pause, Quit |

Everything else happens on its own: a waiting or finished session shows in the
island, a permission request or a question opens it with **Allow / Deny** or the
options once answers from the island are enabled (below), and your integrations
sit in the coloured pills next to Mochi.

## Answers from the island

<img src="screenshots/settings.png" width="562" alt="The settings window">

Mayak sees your Claude Code and Codex sessions without any hook. To allow, deny
and answer them from the island, open **Settings… → Answers from the island**:

- **Claude Code → Enable…** adds two entries to `%USERPROFILE%\.claude\settings.json`:
  permission requests (**Allow / Always / Deny**) and the questions Claude asks
  (`AskUserQuestion`: pick the options).
- **Codex → Enable…** switches Mayak's permission entry in
  `%USERPROFILE%\.codex\hooks.json` to wait for the island (**Allow / Deny**). The
  Codex integration has to be installed first.

Each one shows you the exact diff of what will change and the path of the dated
backup that will be taken, and nothing is written until you click **Apply**. Your
own hooks are never touched, and **Disable…** takes back only Mayak's change.

While a permission or question card is up, **the agent waits for your click**.
Folding the island, letting it close on its own, or **In terminal** hands the
question back to the agent, which then asks in its own window as usual. Nothing
is ever answered or denied without a click. The agent waits 10 minutes at most.

- Claude Code questions need Claude Code 2.1.85 or newer, and questions with
  several answers to tick need 2.1.136 or newer.
- After you enable Codex answers, Codex asks you to trust its hooks again: run
  `/hooks` in Codex (Codex app: **Settings → Hooks**) and choose Trust.
- Codex's own **Permissions** dialog (`request_permissions`) does not go through
  its hooks, so it can't be answered from the island; the session still shows as
  waiting.

The relay is a tiny executable, `mayak-hook.exe`, copied to
`%LOCALAPPDATA%\Mayak\bin\` at launch. It is given 300 ms to reach Mayak and
exits cleanly if the app is closed, slow or crashed, and it holds an agent only
while a card is on screen: a paused island, an event without a card, or a card
that doesn't make it to the screen in time lets the agent go on at once.

It works from any terminal — Windows Terminal, PowerShell, VS Code, Git Bash.

## Chat and keys

**Settings… → Claude** takes your Anthropic API key. Keys live in the **Windows
Credential Manager**, never on disk and never in the interface — the island can
only ask whether a key exists. Same for every integration key.

No telemetry. The only network requests Mayak makes are to the services you
configure yourself.

## Build it yourself

You need [Rust](https://rustup.rs), [Node 20+](https://nodejs.org), and the
**MSVC build tools** (Visual Studio Build Tools with "Desktop development with
C++"). WebView2 ships with Windows 10/11.

```powershell
cd windows
npm install
npm run tauri dev      # live-reloading development build
npm run pack           # builds the installer and drops it in windows/release/
```

`npm run dev` alone serves the front end in an ordinary browser, which is enough
to work on the island's looks. It also serves `dev/upload-preview.html`, which
replays the whole file-drop choreography on a loop — the one part of the UI that
otherwise needs a real drag from Explorer to see. Neither page ships in the app.

`npm run pack` leaves four files in `windows/release/`:

```
Mayak-Windows-X.Y.Z-setup.exe    the versioned installer
Mayak-Windows-setup.exe          the same file under the rolling name
Mayak-Windows-X.Y.Z.msi          the same app as an MSI package
Mayak-Windows.msi                the MSI under the rolling name
```

The MSI installs for all users (Program Files, asks for administrator rights),
the setup.exe for the current user only. Its uninstaller does not have the NSIS
hooks: it leaves `%LOCALAPPDATA%\Mayak\{bin,inbox,mayak.log}` behind and does
not remind about the Codex integration. Install one of the two, not both.

Installing is optional — `target/release/mayak.exe` runs on its own. There is no
window in the taskbar and no console: the island at the top of the screen and the
Mochi in the notification area are the whole app, and Quit lives in its menu.

The 28 sounds are the macOS app's own files; they are never duplicated in this
folder. The path is declared once, in `SOUNDS_DIR` at the top of
`vite.config.ts` — when they move to `shared/sounds/`, change that one line.

The app icon and the tray icon are drawn in code, like Mochi itself:

```powershell
npm run icons          # regenerates src-tauri/icons from scripts/gen-icons.mjs
```

### Layout

```
windows/
  src/                 island front end (TypeScript, no framework)
    mochi/             Mochi and the launch greeting, in Canvas 2D
    island/            state machine, hooks, integrations
    views/             every island view
    settings/          the settings window
  src-tauri/           Rust backend: window, named pipe, Claude API, pollers
  hook/                mayak-hook.exe, the Claude Code relay
  scripts/             icon generator
```

### Log

`%LOCALAPPDATA%\Mayak\mayak.log` — hook events, permission decisions, poller
problems. It stays on your machine.

## What's different from the Mac version

- No notch, so the island lives at the top centre of the screen and retracts into
  the top edge instead of hiding in a notch.
- Permission approval works from **any** terminal; the Mac build only listens to
  VS Code sessions.
- Not in this version: sending a file by email, and dragging Mochi onto a window
  to attach it as context.
- **Open** on a session started in a terminal brings that terminal's window
  forward (the one whose title names the project folder, if there are several),
  but not a tab inside it. When no window of the terminal is found, the project
  folder opens in Explorer instead.
- Cal.com shows the next bookings as a list rather than the Mac's calendar.
