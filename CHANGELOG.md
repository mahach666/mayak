# Changelog

## Mayak for Windows 0.5.0 — October 12, 2026

### Home page

- The home page is rebuilt: Luch on the left, the Claude and Codex plan limits as four rings (5 h and 7 d) in a 2 × 2 table, amber from 80 % and red from 95 %, and the provider balances on one line (#53)
- Every detail of a limit, window, percent, reset time, pace, balance and source, is in a popover: hover a ring or reach it with Tab, click to pin it, `Esc` closes it (#53)
- Your integrations are on the page again, as tiles: up to four, two per row, with GitHub and GitLab counts and CI dots, Vercel, Stripe and the others; a row opens its link and a click on a tile clears its badge (#53)
- With no integration connected, the page shows a hint to connect services in Settings, and the hint no longer flashes at startup (#53, #54)

### GitHub and GitLab

- Connect GitHub and GitLab with a code: Settings → Integrations → Connect shows a code to enter on the service's page, Disconnect deletes the token; pasting a token still works (#52)
- GitLab, on gitlab.com or a self-managed https host: merge requests assigned to you and waiting for your review, CI of your merge requests and of the default branches of your 5 most recent projects, and issues assigned to you, with events for a new review request and pipeline changes; the token goes only to that host (#52)
- GitLab sign-ins renew themselves, and Settings says when one needs reconnecting; a self-managed GitLab connects with an application of your own (Application ID) (#52)
- Connect refusals read plainly in all seven languages; when GitLab refuses sign-in by code, Create a token on GitLab opens its token page with the name Mayak and the `read_api` scope filled in (#52)
- The GitHub tile shows your open pull requests, the ones waiting for your review and their CI (failed, pending, passed), with events for a new review request or a CI change (#55)
- GitHub connects through MayakIslandApp, a read-only GitHub App that sees only the repositories you install it on; Settings shows the connection mode and offers Install the app on repositories. A pasted token or an earlier connection keeps the repository and star counts and asks you to connect the Mayak app for pull requests (#55)

### Sessions

- A ↗ button on a hovered or selected session row opens the session's window in one press, like Open on its card; its tooltip names the host (#54)

## Mayak for Windows 0.4.0 — October 11, 2026

### Waiting sessions

- The tray icon counts the sessions waiting for you; its tooltip lists their projects and how long each has waited (#48)
- A wait nobody answered sounds again and Luch peeks out, after 3, 10 or 30 minutes and at most three times (Settings → Alerts → Remind); silent while paused, and in the summary during a muted pomodoro (#48)
- `Alt+Shift+M` brings up the window of the session waiting longest without opening the island; press it again for the next one; change or turn it off in Settings (#48)
- The waiting card of a permission or question the island let go offers ↗ Open next to OK, to answer it in the session's own window (#48)

### Sessions

- Turn summary for Claude Code: +N −M lines on the row and the changed files as chips on the session and finished cards; a chip opens the file in Cursor or VS Code at its first changed line (#48)
- Claude sessions show a context ring and percent against the model's real context window, with the model and the window in the tooltip (#48)
- One notice when a session's context reaches 85 %, before auto-compaction, and a "compacted HH:MM" mark after it, with the tokens before and after; the notice can be turned off in Settings → Alerts → Context (#48)
- Project headers show the git state: branch or detached commit, uncommitted changes, a rebase or merge in progress, conflicts, and commits to push and pull; git is only read (#48)
- Recent · 24 h under the session list: Claude Code and Codex sessions of the last day, with Resume (a new console running `claude --resume` or `codex resume` in the session's folder) and Copy (the PowerShell command) (#48)
- Right-click a session row, or press the menu key on it, to copy its last reply, its project path or its resume command, or to open its folder (#48)

## Mayak for Windows 0.3.0 — October 11, 2026

### Luch

- Luch, a fire spirit drawn in code, replaces Mochi: he catches from a lighter's sparks at launch, wakes, waves and settles into the compact island; mini Luch in the pills burns in the pill colour, and the app and tray icons come from the same fire (#19)
- Luch dances inside his fire while music plays (#20)
- The pomodoro shows in the fire itself: a gathered, steady flame for work, a softer flame for a short break, a smouldering ember for a long break (#14, #22)
- Colour changes look like a fire: a short flare hides the swap, then the flame settles in the new colour (#24)
- Mayak's own 28 sounds, warm tones with crackle, whoosh and hiss; put a file named after a sound in `%APPDATA%\Mayak\sounds` to replace it, with open folder and reload in Settings (#21)

### Agents

- Answer from the island: Allow, Always, Deny or In terminal for Claude permissions, Allow or Deny for Codex, and Claude's questions with their options, several questions and an "Other" answer. Turn it on per agent in Settings; nothing is answered without a click (#15)
- Antigravity sessions in the list, its hooks installed from Settings, Open raises its window; answering its permission prompts is not possible (#39, #40)
- Each session row shows its host's icon: Cursor, VS Code, Claude Desktop, Codex, Antigravity, or the terminal it runs in (Windows Terminal, PowerShell, Command Prompt, Git Bash and more) (#41, #46)
- Open on a terminal session raises its terminal or editor window; a session in a VS Code or Cursor terminal opens the editor (#17, #46)
- Open on a Claude Desktop session goes to its own chat (#16)

### Limits and providers

- Fresh Codex limits: Mayak asks the Codex CLI app server when the home page opens with old data and right after a limit window resets; on by default, toggle in Settings → Codex (#30, #45)
- OpenRouter credits and DeepSeek and Kimi balances on the home page, from keys kept in Credential Manager (#32, #35)
- Pace: a limit row says when it would run out if you use it faster than the window passes (#32)
- Alerts at 80 % and 100 % of a limit, once per window (#32)
- Limits refresh when a Claude turn ends and when a window resets, without polling (#35)

### Island

- Player for Windows media sessions with the cover, under every tab, with bigger buttons, a progress bar to seek, and shuffle and repeat when the player offers them (#14, #20, #28)
- Apps tab: open Cursor, VS Code, Windsurf, Zed, Claude, Codex and Antigravity, add your own apps, drag to reorder, rename, remove or hide them, with Catppuccin-style icons (#14, #29, #31, #39)
- Focus tab with a pomodoro, and a focus mode that holds "done" and "error" alerts until it ends (#14)
- Stats tab: tokens for 7 and 30 days by day, project, agent and model (#14)
- Keyboard: ↑/↓ walk the sessions, Enter opens one, ←/→ switch pages; the hotkey gives the island the keyboard and gives it back on fold (#27)
- Switch pages with a horizontal touchpad swipe or a tilted mouse wheel (#43)
- Optional: open the island on hover (#34)
- Choose the page the island opens on (#16)
- Seven languages: English, Russian, German, French, Spanish, Chinese and Japanese (#14)

### Settings

- A sidebar with one page per group: Island, Sound, Alerts, Agents, Limits and providers, Integrations, System (#36)
- Settings follows changes made in the island while it is open (#33)
- Launch at login follows Windows both ways, including the Task Manager startup switch (#23)
- The hook preview shows a readable summary and a coloured diff (#16)
- Settings files survive a bad field or a BOM, are copied aside before a broken one is replaced, and are saved atomically (#14)

### Performance and fixes

- Less CPU: the cursor poll reads the window directly and slows down away from the island; the fire runs at 20 fps open and 15 in the compact bar; fewer redraws from integrations and cheaper frames (#19, #25, #44)
- The compact island's animations run at real speed (#44)
- The island page can no longer make the app read arbitrary files (#17)
- A Ctrl+Alt shortcut that types a character (AltGr) is refused with a clear message (#17)
- Hook installs keep a symlinked config file in place (#17)
- The relay answers `--version`, and debug builds warn when the installed relay does not match the app (#42)


## Mayak for Windows 0.2.0 — October 7, 2026

- Every Claude Code and Codex session at once, grouped by project, with who is working, who waits for you and who is done; the island holds open while an agent waits
- Click a session to switch to its window: Cursor, VS Code (also multi-root workspaces), Claude Desktop or Codex Desktop; Open on a card folds the island
- A Codex session whose hooks never reach Mayak (folder gone, hooks off or untrusted) shows a muted "no hooks" row with the reason, and its cards say so
- The hidden island is a thin line at the top centre that stretches towards the pointer and wakes on the first touch; it hides over fullscreen apps
- Fold the island with ⌃, mute it with the bell (the tray's Pause), or toggle it from anywhere with Alt+M (changeable in Settings)
- Each session row shows how full its context is (Codex: a ring and a percent, Claude: tokens)
- The home page shows your Claude and Codex plan limits and when they reset
- The fold countdown runs without moving the mouse


## 0.1.5 — October 4, 2026

- Dress Mochi up: right-click him to open the wardrobe and pick a party hat, beanie, crown, witch hat, Santa hat, bunny ears, bow, sunglasses, round glasses, scarf or pumpkin, all drawn in code (#195)
- Auto mode dresses Mochi for the seasons on his own (#195)
- Outfits follow his head in 3D, glasses stay on his eyes, soft parts react when you tap or move him, and outfits come and go with a transition. Only the main Mochi wears them (#195)
- A new launch greeting: Mochi drops into the island, bounces, slides to the side and waves hello with a quick little hand, then comes back, with a new soft whisper of a sound (#196)
- Mochi's body is no longer clipped at two corners during the greeting (#196)

## 0.1.4 — October 3, 2026

- See what Claude is editing, live: each file edit shows up in the session ticker with its +N −M lines, and a click opens the diff right in the notch (#177)
- When Claude finishes, the session card shows its final message instead of the last step, without the shimmer (#177, #179)
- GitHub pill: your open pull requests with their CI status, the pull requests waiting for your review, and the CI of the default branch of your recent repos. Click a row for the list, then an item to open it on github.com (#181)
- GitHub alerts: a badge and a sound when the CI of one of your pull requests turns red or green, when a default branch breaks, or when someone requests your review. Fast CI runs are caught too, and the card refreshes when you open it (#181, #185)
- Your GitHub contribution grid: the last 7 days in the GitHub card header, click it for the past 23 weeks, and click a day for its count (#187)
- The GitHub token needs read access to pull requests and CI: a classic token with the repo scope, or a fine-grained token with read access to Pull requests, Commit statuses and Actions (#181)
- The finished view no longer overflows the card (#179)

## 0.1.3 — October 3, 2026

- Answer Claude's questions from the notch: when Claude Code asks a multiple-choice question, pick an option or type your own answer right in the island, and Reply in terminal hands it back. Update your hooks in Settings to turn it on (#165) — thanks @Vega8991 for the idea (#94)
- Claude plan usage (GitHub build): turn on Settings → Agents → Plan usage to see your 5-hour and weekly limits in a small pill in the notch header, and click it for the details and reset times. Pro and Max plans; your current status line keeps working (#159)
- Chat with local models through Ollama or LM Studio, no API key needed: connect them in Settings → Chat → Local models. Answers stream in, and thinking blocks stay hidden (#156)
- Markdown in chat answers: bold, lists, headings, quotes, and code blocks with a copy button. Links open only when they are web links (#156)
- Apple Music (GitHub build): see what is playing in the notch, play, pause and skip on hover, and Mochi dances along (#144, #153)
- Settings are now organized in a sidebar (#153)
- The chat greets you by your own first name (#154)

## 0.1.2 — October 2, 2026

- Codex support (GitHub build): sessions show up live on the Codex pill, and permission requests get Allow and Deny in the notch. Install from Settings → Codex Hooks, then trust the hooks once with /hooks in Codex (#130) — thanks @lacatu5
- Cursor: Claude Code started in Cursor's terminal shows up on the Cursor pill, and you can answer its permission requests from the notch (#120).
- Pick your main coding tool in Settings → Active pills: VS Code, Cursor, Codex or Antigravity (Codex and Antigravity: GitHub build). It stays on and no longer takes one of the 4 slots (#120).
- The permission card stays in the notch until you answer it: the mouse no longer folds it, and reopening the island shows the request again (#117).
- The permission card also shows when the island is already open, and the pill you were on comes back once you answer (#120).

## 0.1.1 — October 2, 2026

- Declare the tools you use in Settings: Gemini CLI, Antigravity, Anthropic, Google AI and OpenAI pills join the existing ones (Cursor and Codex pills are coming soon), and you pick the main pill.
- Chat now supports Google AI (Gemini) and OpenAI in addition to Anthropic; switch provider and model by clicking the model name in the chat view, on macOS.
- Linux version: the Tauri app now builds for Linux too (AppImage, .deb, .rpm), with the island as a layer-shell overlay on Wayland and Claude Code hooks over a private Unix socket (#21) — thanks @Davy133
- Compact island on screens without a notch (#22) — thanks @Kamasoutra
- Only web links (http/https) open from the notch; other kinds of links from Claude or integrations are ignored (#16) — thanks @Cris1670
- Hook socket limited to your own user account, with size and time limits; logs no longer keep commands, n8n data or full URLs, and stay under 1 MB (#16) — thanks @Cris1670 and @Vignesh-Thangamariappan
- The island always reopens after folding, and Settings opens below it, resizable — thanks @rouderz
- Choose the Claude model for the chat in Settings; the list comes from your Anthropic account, and Claude Sonnet 4.6 stays the default — thanks @rouderz
- Windows build artifacts are now downloadable from a manual CI run — thanks @MysJofR
- Any agent can talk to Mochi: tag a hook payload with `coucou_agent` (e.g. `nb-hook --agent my-agent`) and it gets its own pill in the island (#7, #9) — thanks @lacatu5
- Gemini CLI and Antigravity (agy) hook support on macOS: install from Settings and their sessions show up in the island — thanks @corefusiion

## 0.1.0 — September 27, 2026

- First release: Mochi lives in your notch, breathing, blinking, with eyes that follow your cursor
- Claude Code sessions: live steps, approve permissions, answer questions, jump to the terminal
- Chat with Claude from the notch
- Drop a file on the notch to ask a question about it or send it by email
- Drag Mochi onto any window to attach it as context
- Integrations: Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com
- 28 handcrafted sounds
- Hides when idle, peeks out when you hover
