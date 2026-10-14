<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Mayak icon">

# Mayak for Windows

**Luch, Mayak's fire spirit, doesn't get a notch on a PC — so it lives at the top of your screen instead.**

Watch your Claude Code, Codex and Antigravity sessions, answer their permission requests and questions, keep an eye on your limits and services — without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

</div>

---

## Install

Download the setup, MSI or portable build from
[Releases](https://github.com/mahach666/mayak/releases). The builds are not
code-signed: SmartScreen warns about an unknown publisher (**More info → Run
anyway**), and Microsoft Defender has flagged earlier unsigned builds of this app
as `Trojan:Win32/Wacatac.H!ml`, a machine-learning false positive.

Or [build it yourself](#build-it-yourself): it takes a few minutes and installs for
the current user only — no admin prompt.

## Using it

| What you do | What happens |
|---|---|
| Move the mouse to the very top-centre of the screen | Luch peeks out |
| Bring the mouse near the thin line at the top centre | It stretches towards the pointer; touch it and Luch peeks out |
| Click ⌃ in the island header, or on the small island | It folds away, down to the line |
| Click the bell in the island header | Quiet mode — the same as the tray's Pause, but the island stays open |
| `Alt+M` in any app (change or turn it off in Settings → General) | The island opens on the session list; press it again and it folds away to the line |
| `Alt+Shift+M` in any app (change or turn it off in Settings → General) | The window of the session waiting longest for you comes to the front, the island stays closed; press it again for the next one. A session whose window cannot be reached opens the island on its card |
| Click the small island | It opens |
| Click Luch | It gets annoyed. Three times in a row and it goes dizzy |
| Rest the pointer on Luch for two seconds | Hearts |
| `Esc` | Closes the island |
| Put a file named like one of Luch's sounds (`finish.wav`, `approval.mp3`, `greet.m4a`…) in the sounds folder | It replaces that sound after **Settings → General → Reload sounds**. **Open sounds folder** shows the folder, `%APPDATA%\Mayak\sounds`. Files of up to 8 MB; one that does not play falls back to the built-in sound |
| Tray icon | Open, Settings…, Pause, Quit |
| Right-click a session row (or press the menu key on the highlighted row) | Copy last reply, Copy project path, Copy resume command, Open folder |
| Click a file chip on a session or finished card | The file opens in Cursor or VS Code at its first changed line |
| ↗ Open on a waiting card (a permission or question the island let go) | The session's window comes to the front, to answer there |
| ↗ on a hovered or highlighted session row | The session's window comes to the front and the island folds, like Open on its card; the tooltip names the host. Recent rows have none |
| Hover a limit ring on the home page, or reach it with `Tab` | A popover with the window, the percent, the reset time, the pace and the balances; click pins it, `Esc` closes it |
| **Settings → Integrations → Connect** on GitHub or GitLab | A code to enter on the service's page; once you approve, the token goes to Credential Manager. **Disconnect** deletes it. If GitLab refuses sign-in by code, **Create a token on GitLab** opens its token page filled in for Mayak (`read_api`) |
| Sessions wait for an answer | The tray icon shows how many; hover it for the projects and how long each has waited. A dismissed card still counts while the agent waits. A wait nobody answered sounds again after 10 minutes, at most three times (Settings → Alerts → Remind: off, 3, 10 or 30 minutes); silent while paused; in a muted pomodoro interval it goes into the phase summary, or only sounds when waits get through |

**Recent · 24 h** at the bottom of the session list lists the Claude Code and
Codex sessions of the last day that are no longer on the list. The files are read
only when you open it. **Resume** (or `Enter` on the row) opens a new Command
Prompt in the session's folder that runs `claude --resume <id>` or
`codex resume <id>`. The CLI is run by the full path found on `PATH`, never from
the project folder. Without the CLI on `PATH` the command is copied instead.
**Copy** puts PowerShell text on the clipboard (`Set-Location -LiteralPath '<folder>'; claude --resume <id>`),
which does not work in Command Prompt. A session started by a program through
the Agent SDK, `claude -p` included, is not listed. A session on the list whose
turn ended on an error or an interrupt has **Copy resume** on its card. These
buttons work on Windows only: on Linux they say the command could not be resumed
or copied.

Everything else happens on its own: a waiting or finished session shows in the
island, a permission request or a question opens it with **Allow / Deny** or the
options once answers from the island are enabled (below), and your integrations
sit in the coloured pills next to Luch and as tiles on the home page, next to the
plan-limit rings. The GitHub tile shows your open pull requests, the ones waiting
for your review and their CI, through MayakIslandApp, a read-only GitHub App that
sees only the repositories you install it on; the GitLab tile (gitlab.com or a
self-managed https host) shows your merge requests, the ones waiting for your
review, CI and your issues. A Claude Code row shows the lines its last
turn changed (+N −M) and a context ring against the model's real window; a session
whose context reaches 85 % gets one notice before auto-compaction and a "compacted"
mark after it (Settings → Alerts → Context); each project header shows its git
branch, uncommitted changes, conflicts and commits to push and pull, read-only.

## Answers from the island

Mayak sees your Claude Code sessions without any hook, and Codex sessions once the
Codex integration is installed (**Settings… → Agents → Codex**). To allow, deny
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
A session in a terminal shows that terminal's icon, told once from the agent's
parent processes. A PowerShell or Command Prompt started from Start or Run that
Windows 11 hands over to Windows Terminal (the default terminal setting) has no
Windows Terminal among its parents, so it shows the shell's icon.

## Antigravity

Google Antigravity (the IDE agent and the `agy` CLI) only reaches Mayak through
its hooks. **Settings… → Agents → Antigravity → Install** adds one hook group,
`mayak`, to `%USERPROFILE%\.gemini\config\hooks.json` (PreInvocation,
PostToolUse, Stop), through the same diff, dated backup and **Apply**; your own
groups stay as they are, and **Remove…** takes out only `mayak`. Start a new
Antigravity conversation afterwards.

Its sessions then show as working, then finished (with the last answer, read
from the conversation's `transcript.jsonl`) or failed, under the conversation's
first workspace, labelled Antigravity or agy. They stay listed until Antigravity
exits; **Open** raises the Antigravity window, or starts it. Mayak only watches:
no PreToolUse hook is installed, Antigravity keeps asking for permissions in its
own window, and an Antigravity session never shows as waiting. The relay answers
every Antigravity event with `{}` at once, whether Mayak runs or not.

## The Agents tab

A chat with your own agent CLIs, in the island, with no API key: Claude Code, Codex,
Gemini CLI and Cursor Agent. Pick a CLI, a model, a mode and a project folder, and Mayak
runs that CLI there (`src-tauri/src/chat`). It uses the CLI's own login: Mayak makes no
network request for it, never signs in for you and stores no key for it. Luch sits on the
left of the chat and follows the run: it thinks while the CLI answers, works while a tool runs,
waits when a request needs your click, and is pleased when a turn ends well. Answers are drawn
as Markdown (code blocks have a Copy button, links open in your browser), the tools the agent
runs as small chips. What the next message runs with sits on the bottom row of the input box:
the agent (fixed once the chat has started), the access mode as a coloured pill, the model, and
the round send / stop button; the project is the chip at the top right.

- A CLI that is not installed is listed as "not installed"; with none found the tab says so.
  One that is not signed in is listed as "not signed in" and a message gives the command
  to run in a terminal (`claude auth login`, `codex login`…); send again after.
- A permission request of the CLI shows in the island (and opens it) with **Allow / Deny**.
  Nothing is approved without your click. Claude Code's modes are ask and plan.
- Codex is the exception: its chat runs `codex exec`, which has no approval prompts. It runs
  under the sandbox chosen in the view (read-only, or edit project files) and what is outside
  that sandbox fails instead of asking; Mayak never lifts the sandbox.
- Gemini CLI and Cursor Agent were written from their documentation only and never run:
  the picker marks them "unverified".
- The chat is given the Mayak MCP server (below) only while **Settings… → Agents → Mayak
  MCP server** is on, and only Claude Code and Codex. That switch is off by default and is
  read again at each turn that starts a CLI process.
- Tested on Windows. The code for macOS and Linux is the same app, but a CLI process is
  held in a process group there (a job object on Windows) and that path has not been run.

## Mayak MCP server

Optional, off by default (**Settings… → Agents → Mayak MCP server**). It lets agents
— the chat Mayak starts, or your own Claude Code or Codex — talk to Mayak over
[MCP](https://modelcontextprotocol.io): read what the island shows, and put a
question on it that only your click answers.

| Tool | What it does |
|---|---|
| `list_sessions`, `get_session` | The sessions the island lists (agent, host, phase, activity, model, context fill and, in `get_session`, the last outcome's summary and the files the last turn changed) — only what the island itself shows; never the last reply in full, a session's working folder beyond its project root, or a process id |
| `get_limits` | The plan windows of Claude Code and Codex, and the OpenRouter, DeepSeek and Moonshot balances when saved |
| `get_stats` | The Stats tab's tokens for 7 or 30 days (the biggest 20 projects) |
| `list_apps` | The Apps tab's tiles |
| `ask_user` | A question with 2–4 options on the island, the same card Claude Code's own questions use; answered by your click or a typed answer. Waits 45 s by default (5 to 240 s on request); closing the island or **In terminal** hands it back as "no answer" |

The server speaks Streamable HTTP on `127.0.0.1` only (port 47831; `mcpPort` in
`settings.json` changes it, and a busy port is shown in Settings, never replaced). Every
request needs `Authorization: Bearer <token>`: a random 256-bit token made when you
switch it on and kept in the **Credential Manager**. A request with another `Host`
(DNS rebinding) or a foreign `Origin` is refused before its body is read. It is
stateless and has no other tools: nothing is approved, sent or written for anyone.

**Copy command for Claude Code / Codex** puts the line that adds the server to your own
agent on the clipboard (the token is in it, so Rust copies it; the page never has it).
Mayak writes into none of the agents' settings: you paste the line. The Codex line sets
the user variable `MAYAK_MCP_TOKEN` (`setx`) before `codex mcp add`. **New token** cuts
off every agent that holds the old one. For a CLI Mayak starts itself, `mcp_server::attach`
gives the arguments (`--mcp-config` for Claude Code, `-c mcp_servers.mayak.…` for Codex) and
the environment, so the token is named on the command line, never written in it.

Claude Code ends a tool call of an HTTP server after 60 s unless the server's entry sets a
larger `timeout`, so a longer `ask_user` wait only works there with that entry, which
`attach` writes for the chat. Claude Code and Codex were run against the server on this
machine; Cursor, Gemini CLI and others were not.

## Keys

The chat tab with its API key and the file drop are switched off in this version
(`CHAT_ENABLED` in `src/core/features.ts`). The Agents tab (`CLI_CHAT_ENABLED`, see
above) needs no key: it uses each CLI's own login.

Provider and integration keys, the GitHub and GitLab
tokens from **Connect**, and the MCP server's token live in the **Windows Credential
Manager**, never on disk and never in the interface — the island can only ask whether a
key exists.

No telemetry. The only network requests Mayak makes are to the services you
configure yourself: for GitHub, `github.com` and `api.github.com`; for GitLab,
only the address you saved.

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

Before a commit:

```powershell
npm run check          # the front end's checks (scripts/checks/), straight from the TypeScript
cargo test --workspace # the Rust tests and the relay's
```

`npm run check -- i18n` runs only the checks whose name contains `i18n`. The
checks need Node 22.6 or newer.

The CLI chat's tests start fake CLIs (PowerShell scripts, Windows only). A few more run the
real `claude` and `codex` on your PC with your own logins, so they are skipped unless asked:
`cargo test --lib chat::tests::real -- --ignored --test-threads=1 --nocapture`. Gemini CLI and
Cursor Agent were written from their documentation only and never run (see the headers of
`src-tauri/src/chat/gemini.rs` and `cursor.rs`). `dev/agent-chat-preview.html` (with `npm run
dev`) shows the Agents tab on a fake backend.

The app installs the relay from `target/release/mayak-hook.exe`, so after a
change under `hook/` (or to `ident.rs`) rebuild it with
`cargo build --release -p mayak-hook` (`npm run dev` and `npm run build` do it
first). A stale relay silently drops what it doesn't know yet, such as a new
agent's events: a debug build runs `mayak-hook.exe --version` at launch and
writes a `WARNING: stale relay` line to stderr and `mayak.log` when it doesn't
report the `RELAY_VERSION` the app was built with, and the `relay-fresh` check
warns when the exe is older than its sources (`MAYAK_STRICT_RELAY=1` makes it
fail; it is only a warning by default because a checkout rewrites mtimes).

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
tray icon in the notification area are the whole app, and Quit lives in its menu.

The 28 sounds are Mayak's own, synthesised in code like Luch (`scripts/sounds/sfx.mjs`)
and committed as WAVs in `sounds/`. The folder is declared once, in `SOUNDS_DIR` at
the top of `vite.config.ts`. After changing `sfx.mjs`, render them again; `npm run check`
fails while a committed WAV differs from the script:

```powershell
npm run sounds         # regenerates sounds/*.wav from scripts/sounds/sfx.mjs
```

The app icon and the tray icon are drawn in code, like Luch itself (`src/luch/`, `scripts/gen-icons.mjs`):

```powershell
npm run icons          # regenerates src-tauri/icons from scripts/gen-icons.mjs
```

### Layout

```
windows/
  src/                 island front end (TypeScript, no framework)
    luch/              Luch, the fire spirit (WebGL shader + 2D face), and the sparks greeting
    island/            state machine, hooks, integrations
    views/             every island view
    settings/          the settings window
  src-tauri/           Rust backend: window, named pipe, Claude API, pollers
    src/chat/          the CLI chat (Agents tab): adapters, conversations, processes
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
- Not in this version: sending a file by email, and dragging Luch onto a window
  to attach it as context.
- **Open** on a session started in a terminal brings that terminal's window
  forward (the one whose title names the project folder, if there are several),
  but not a tab inside it. When no window of the terminal is found, the project
  folder opens in Explorer instead.
- Cal.com shows the next bookings as a list rather than the Mac's calendar.

## On a Mac (this same app)

The same Tauri app runs on macOS 13 or newer; the Swift app in `NotchBuddy/` is
upstream's and is not developed here.

```bash
cd windows
npm install
npm run tauri dev      # development build
npx tauri build        # target/release/bundle/macos/Mayak.app and dmg/Mayak_<v>_aarch64.dmg
```

You need Rust (`brew install rustup && rustup default stable`), Node 20+ and the
Xcode command line tools. The bundle is signed ad hoc, not with a Developer ID:
a copy downloaded from elsewhere is stopped by Gatekeeper until you allow it
(**System Settings → Privacy & Security → Open Anyway**).

What is different on a Mac:

- The island sits in the notch, sized to the real notch of the screen, and wakes
  when the pointer enters it. On a screen without a notch it stays at the top
  centre, as on Windows.
- Files live in `~/Library/Application Support/Mayak` (settings, sounds) and
  `~/.local/share/mayak` (`bin/mayak-hook`, the inbox, `mayak.log`). The relay
  reaches the app over a socket in your private temporary folder.
- **Open** on a Terminal or iTerm2 session brings its very tab forward (macOS asks
  once to let Mayak control that app); any other terminal comes forward as a whole.
  Claude Desktop opens the session's chat; Cursor and VS Code open the window of
  the project folder through their own `cursor` / `code` commands.
- **Resume** opens a new window of the terminal you have running (iTerm2,
  Ghostty, WezTerm, kitty, Alacritty), else Terminal; **Copy** puts a
  `cd … && claude --resume …` line for zsh or bash on the clipboard.
- The player reads the system's Now Playing (any app, browsers included) through
  a small helper, `mayak-nowplaying.dylib`, run inside `/usr/bin/perl`: macOS
  keeps that information to Apple's own processes. This is not an Apple API and
  a later macOS may close it; the player then falls back to Apple Music and
  Spotify (no cover, Automation asked on the first button press).
- The shortcuts are the same settings: `Alt` is ⌥ and `Win` is ⌘, so the
  defaults are ⌥M and ⌥⇧M.
- The Apps tab's **+** picks an app from Applications.
- The session file drop and the API chat tab are switched off, as on Windows. The Agents tab is on
  (see above); its macOS code has not been run on a Mac.
