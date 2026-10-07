# Mayak: the chat tab as an agent launcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The island's chat tab (view `prompt`) becomes a launcher: the user types a question, optionally keeps a dropped file, picks an installed agent (Claude Desktop, Claude or Codex in Cursor / VS Code, Codex Desktop) and a project, presses Send, and that agent opens in that project with the question in its input field, never sent; "Claude API (answers here)" keeps the old in-island chat when the Anthropic key is saved.

**Architecture:** A new Rust module `launcher` (pure logic in `launcher/mod.rs`, Win32 in `launcher/win.rs`, like `focus`) finds the installed targets from the registry (`HKEY_CLASSES_ROOT\<scheme>` with a `URL Protocol` value), the editors' extension folders (minus `extensions\.obsolete`) and `%LOCALAPPDATA%\Packages\OpenAI.Codex_<publisher>`, and opens one: Claude targets through a `claude://` / `cursor://` / `vscode://` link handed to `ShellExecuteW` (the editor's project window first found with `focus::pick_window`, raised with `focus::win::raise`, or opened with `cursor <root>` and waited for 15 s / 250 ms); Codex targets through the clipboard, the AppX activation `explorer shell:AppsFolder\<family>!App` or the editor link `cursor://openai.chatgpt/`, and, for Codex Desktop only, `Ctrl+V` through `SendInput` when the foreground window's process image lies in the Codex package folder. Two Tauri commands: `launcher_targets` and `launcher_open` (its work on its own thread, like `session_open`, with the outcome sent back over a `tokio::sync::oneshot`). The front end gets a DOM-free `views/launcher-model.ts` (checked with Node's type stripping) and a rebuilt `buildPrompt` in `views/chat.ts`.

**Tech Stack:** Rust (serde, serde_json, tokio `sync`, `windows` 0.61 with five more Win32 features; no new crates), TypeScript (no new packages), Tauri 2, Node 22 `--experimental-strip-types` for the out-of-repo check script.

**Spec:** `docs/superpowers/specs/2026-10-07-mayak-launcher-design.md` (Russian; LF line endings in the working tree).

## Global Constraints

- Values from the spec, verbatim:
  - Never send: «**Сам вопрос не отправляется**: последнее слово за пользователем в окне агента.»; «Ничего не отправляется агенту без явного действия пользователя в окне агента.»; Codex: «**Enter не нажимается никогда.**»
  - «Нет опроса в покое.» and, for the editor, «до 15 секунд с шагом 250 мс ожидается появление окна с этим проектом. Опрос идёт только в этот момент, после него нет»; Codex Desktop: «в течение 5 секунд ожидается, пока на переднем плане окажется окно процесса Codex»
  - «Существующие виды, кроме вкладки чата, не меняются.»; «Новые элементы — в стиле приложения, существующие виды не меняются.»
  - «Выбор агентов ограничен тем, что установлено на компьютере.» Order: «Claude Desktop; Claude в Cursor; Claude в VS Code; Codex Desktop; Codex в Cursor; Codex в VS Code; Claude API, если ключ сохранён.» Label «Claude API (answers here)».
  - «Последний выбранный агент **запоминается** в настройках (`launcherAgent`).»
  - «**Проект по умолчанию** — проект последней активной сессии острова» «(по `since` в снимке)»; «Если сессий нет — «No project»»; «**Claude Desktop без папки:** `claude://code/new?q=…`»; «**Редактор без проекта:** URI уходит в последнее активное окно редактора.»
  - Links: `claude://code/new?q=<вопрос>&folder=<папка>[&file=<путь>]…`, `cursor://anthropic.claude-code/open?prompt=<вопрос>`, `vscode://anthropic.claude-code/open?prompt=`, `cursor://openai.chatgpt/`; «каждый параметр в `encodeURIComponent`»; «вопрос ограничить 8000 символами, файлов — не больше 10»; «открыть его через системный обработчик протокола (`ShellExecuteW` / `open`)».
  - Codex Desktop: «приложение открывается через `explorer shell:AppsFolder\OpenAI.Codex_<publisherId>!App` (имя берётся из установленного пакета)»; «если оказалось — `Ctrl+V` через `SendInput`. Если нет — ничего не вставляется.» Codex in the editor: «**Без автовставки**: фокус может оказаться в редакторе кода, а вставка туда изменила бы файл.»
  - Files: «для Claude Desktop — параметром `file=`; для Claude в редакторе — `@<путь>` в начале вопроса; для Codex — путь отдельной строкой в тексте вопроса.»
  - Result: «`Opened | OpenedPasteYourself | Failed(reason)`. Остров сворачивается, как после «Open». Для `OpenedPasteYourself` и для Codex без автовставки показывается короткая подсказка «Paste the question with Ctrl+V».»
  - Log: «одна строка без текста вопроса и путей: `launcher: agent=… outcome=…`.»
  - View: «поле ввода»; «приложенный файл (если перетащен) с крестиком «убрать»»; «строка из двух выпадающих списков: агент и проект»; «кнопка «Send», она же `Enter`; `Shift+Enter` — новая строка.»; «При выборе «Claude API» вид работает как прежний чат: ответы в острове, история. Перетаскивание файла по-прежнему ведёт в этот вид. Кнопка «Ask a question» на карточке файла остаётся.»
- Machine facts (measured 2026-10-07): Claude Desktop accepts `claude://code/new?q=<prompt>&folder=<path>&file=<path>` and prefills without sending; `cursor://anthropic.claude-code/open?prompt=<text>` (and `vscode://`) prefills Claude in the last active editor window; Codex Desktop is AppX `OpenAI.Codex`, publisher id `2p2nqsd0c76g0`, app id `App` (registry `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\OpenAI.Codex_26.930.7945.0_x64__2p2nqsd0c76g0\App`), no deep link for a new thread with a prompt; the Codex extension folder is `openai.chatgpt-*` (`openai.codex-audio-*` is a different extension); `cursor://openai.chatgpt/` opens its panel; extension folders live under `%USERPROFILE%\.cursor\extensions` and `%USERPROFILE%\.vscode\extensions`, and both hold an `.obsolete` file naming folders of uninstalled versions still on disk; `HKCR\codex` has a `URL Protocol` value and no `shell\open\command`.
- No new crates, no new packages. Only `windows`-crate features are added: `Win32_System_Registry` (Task 1); `Win32_System_Com`, `Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_UI_Shell` (Task 2).
- No polling at rest. The only wait loops are the bounded ones during a launch: 15 s / 250 ms for the editor window, 5 s / 250 ms for the Codex foreground. No retry loop anywhere else (the clipboard is tried once).
- Never press Enter in another app. `Ctrl+V` only into a verified Codex Desktop foreground window. Tests never send keys, never touch the real clipboard and never open a link.
- Never log the prompt or file paths: the only launcher log line is `launcher: agent=<id> outcome=<Name>`; `Failed` reasons are Mayak's own texts and are not logged.
- Do not restyle existing views except the prompt view: CSS changes are new selectors (`.launch-*`, `.chip-x`, `textarea.chat-input`, `.chat-card.launch …`, `.send-btn:disabled`); the existing rules stay as they are.
- Build environment: always `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target` before cargo. Never touch `windows/target/release`.
- Baseline measured on `feature/launcher` at `07feea2` (2026-10-07): `cargo test --workspace` = lib **482 passed, 1 ignored**; hook **32 passed**; **0 warnings**; `npx tsc --noEmit` exits 0.
- Front-end checks: `npx tsc --noEmit`, `npx vite build` (there is no front-end test framework; do not add one; the Node check script lives outside the repo, in `C:\tmp\mayak-spike`).
- Keep each file's line endings: every source file touched here is CRLF in the working tree (`git ls-files --eol`: `i/lf w/crlf`, `core.autocrlf=true`); the spec is LF. Edit in place; a new or wholly rewritten source file is converted with `unix2dos` before its commit.
- The installed Mayak 0.2.0 may be running and the dev app may be stopped. Never stop or start either; the single-instance plugin lets only one run, so the user switches them (Task 4).
- Commits: conventional, one line, **no trailers** (no Co-Authored-By, no «Generated with»); never mention an AI assistant anywhere (code, comments, docs, PR text). Never push without the user's explicit yes (Task 4 asks); merge only on a separate explicit request.

## Review Focus

1. Enter held down or auto-repeating when Send fires: the field loses the focus to the agent within milliseconds, and a repeated Enter keydown landing in Claude Desktop or Cursor would send the question. The island sends on the release of Enter, ignores repeats and a press it did not see start, and Rust never sends `Ctrl+V` while Enter, Shift, Alt or Win is down (Task 3 check script: the `enterStep` cases and the 30-repeat sequence; Task 2: `paste_only_into_the_app_window_with_no_key_held`).
2. An extension or editor that is no longer installed: on this machine `.cursor\extensions\.obsolete` lists `anthropic.claude-code-2.1.288` and `2.1.289`, whose folders are still on disk, and an uninstalled VS Code leaves `%USERPROFILE%\.vscode`. Neither may produce an agent in the list (Task 1: `a_folder_listed_as_obsolete_does_not_count`, `a_target_needs_its_protocol_and_the_api_needs_the_key`).
3. A long or non-Latin question: 8000 Cyrillic characters encode to 48 000 link characters, past the 32 767-character command line the protocol handler gets. The question must never be cut and the launch must not fail: the link opens without it and the whole question goes to the clipboard with the paste hint (Task 2: `a_question_too_long_for_the_link_goes_to_the_clipboard_whole`, `the_limits_count_characters_not_bytes`).
4. The paste target: Codex did not come forward, the island or another app is in front, or a process outside `WindowsApps\OpenAI.Codex_…__2p2nqsd0c76g0\` owns the foreground window. None of them gets `Ctrl+V`, and the keys sent are exactly Ctrl down, V down, V up, Ctrl up (Task 2: `only_an_image_inside_the_codex_package_folder_counts`, `paste_only_into_the_app_window_with_no_key_held`, `ctrl_v_is_four_key_events_and_never_enter`).
5. Input from the webview: a file path outside Mayak's inbox (`inbox\..\secret.txt`, a folder, a missing file, a relative path) or a root like `--x` must never reach a link, an editor command line or the clipboard (Task 2: `prepare_refuses_files_outside_the_inbox`, `prepare_refuses_a_root_that_is_not_an_existing_absolute_folder`).

---

## Decisions (where the spec leaves a choice)

- **Four tasks instead of the suggested five.** `launcher` is a private module of the lib and the baseline has 0 warnings: a pure function or a Win32 helper that no command uses yet is a `dead_code` warning in the `mayak` bin build that `cargo test --workspace` also compiles. So each Rust task is one end-to-end slice: Task 1 = discovery (pure folder scans + registry lookup) + `launcher_targets` + the `launcherAgent` setting; Task 2 = launch (pure links, plans, checks, paste decision, editor step, waits + the Win32 link, clipboard, keys, foreground, AppX + `launcher_open`). Task 3 is the whole front end, Task 4 the live acceptance with the user and the push question.
- **Too long for a link:** the 8000-character limit is applied as "the link carries at most 8000 characters", plus a 30 000-character cap on the whole link (`MAX_URI_LEN`). Past either, the link opens without the question (Claude Desktop keeps `folder=` and `file=`; Claude in the editor opens `…/open` without `prompt`), the whole question goes to the clipboard and the outcome is `OpenedPasteYourself`. Nothing is truncated.
- **Protocol lookup:** `HKEY_CLASSES_ROOT\<scheme>` (HKCU and HKLM classes merged) with a `URL Protocol` value; `shell\open\command` is not required (`codex` has none). Claude Desktop needs `claude`. An editor target needs its extension folder **and** its editor's protocol (`cursor`, `vscode`). Codex Desktop needs the package folder `%LOCALAPPDATA%\Packages\OpenAI.Codex_<publisher>` **and** `codex`.
- **Extension folder:** a directory named `<prefix><ASCII digit>…` (`anthropic.claude-code-`, `openai.chatgpt-`), not marked `true` in `extensions\.obsolete`; an unreadable or torn `.obsolete` hides nothing.
- **Codex window check:** the foreground window's process image has a path component `WindowsApps` followed by `<Name>_<Version>_<Arch>_<ResourceId>_<Publisher>` of the family found at launch. Checked every 250 ms for 5 s, then again after a 300 ms settle right before `Ctrl+V`, together with the held keys.
- **Editor window:** `editor_step` over the editor process's windows (`Cursor.exe`, `Code.exe`): the foreground window is the project's window → ready; else one project window → raise it; else (none, or two of one folder name) → wait. A raise that ends `Focused` is ready; otherwise `cursor <root>` / `code <root>` starts and the step repeats every 250 ms for 15 s; a window that only flashed is not raised again. Timeout → `Failed("The project window did not open in time.")` and **no link** (the editor would hand it to whichever window was last active, possibly another project).
- **Input checks (`prepare`):** a non-blank question; `root` empty or absent = no project, else an existing absolute folder; at most 10 files, each an existing file directly inside `files::inbox_dir()`. Anything else is `Failed` with a reason and the question stays in the field.
- **Formats:** Claude in the editor: `@<path>` per file, `@"<path>"` when the path has whitespace, separated by spaces, then the question. Codex: the question, a blank line, one path per line. The clipboard text uses `\n`.
- **The command waits for the outcome:** `launcher_open` is an `async` command; the launch runs on a `std::thread` (like `session_open`) and returns over a `tokio::sync::oneshot`; COM is initialised on that thread for `ShellExecuteW`. The island shows Mochi thinking meanwhile; `Opened` → collapse; `OpenedPasteYourself` → note «Paste the question with Ctrl+V» for 2.4 s, then collapse; `Failed` → note `<agent label>: <reason>` for 2.4 s, then back to the prompt with the question kept.
- **Island details:** the targets are read each time the prompt view becomes active (`setActive(true)`); the default project is fixed at the first draw after that and does not move while sessions change; a project that leaves the snapshot falls back to "No project". `launcherAgent` is stored as text (default `""`); a value not in the list gives the first agent. The view's height grows with the chat history only while "Claude API" is selected (`State.promptCount`). The textarea replaces the single-line input in both modes.
- **The clipboard is not restored** after a launch: the question stays there for the user to paste.

## File Structure

- Create `windows/src-tauri/src/launcher/mod.rs` — `AgentId`, `Editor`, discovery (Task 1); `LaunchOutcome`, links, plans, `prepare`, `should_paste`, `is_package_image`, `editor_step`, `wait_for`, `open` and the per-agent flows (Task 2); tests.
- Create `windows/src-tauri/src/launcher/win.rs` — `protocol_registered` (Task 1); `Com`, `open_uri`, `set_clipboard_text`, `ctrl_v_inputs`, `send_ctrl_v`, `keys_held`, `foreground`, `image_path`, `launch_appx` (Task 2); tests.
- Modify `windows/src-tauri/src/focus/win.rs` — `app_windows` + test (Task 2).
- Modify `windows/src-tauri/src/settings.rs` — `launcher_agent` + tests (Task 1).
- Modify `windows/src-tauri/src/lib.rs` — `mod launcher`, commands `launcher_targets` (Task 1) and `launcher_open` (Task 2), `generate_handler!`.
- Modify `windows/src-tauri/Cargo.toml` — `windows` features (Tasks 1-2).
- Modify `windows/src/core/bridge.ts`, `windows/src/core/state.ts`, `windows/src/views/views.ts`, `windows/src/island/island.ts`, `windows/src/style.css`; rewrite `windows/src/views/chat.ts`; create `windows/src/views/launcher-model.ts` (Task 3).
- Create `C:\tmp\mayak-spike\launcher-model-check.mjs` (Task 3), outside the repository.
- Modify `docs/superpowers/specs/2026-10-07-mayak-launcher-design.md` — clarifications (Task 4).

---

### Task 1: Installed targets → `launcher_targets`, and the `launcherAgent` setting

**Files:**
- Create: `windows/src-tauri/src/launcher/mod.rs`, `windows/src-tauri/src/launcher/win.rs`
- Modify: `windows/src-tauri/Cargo.toml` (`windows` features, ~line 31-46)
- Modify: `windows/src-tauri/src/lib.rs` (module list ~line 14-15; new command after `usage_refresh` ~line 304; `generate_handler!` ~line 624)
- Modify: `windows/src-tauri/src/settings.rs` (`Settings` ~line 42; `Default` ~line 94; tests module end ~line 263)
- Test: `launcher/mod.rs` and `launcher/win.rs` (modules `tests`), `settings.rs` (module `tests`)

**Interfaces:**
- Consumes: `crate::testutil::TempDir { path }`; `crate::secrets::present(key: &str) -> bool`; `crate::platform::home_dir() -> PathBuf`; `app_paths().localappdata` (lib.rs).
- Produces (later tasks rely on these exact names):
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub enum AgentId { ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode, ClaudeApi }` — wire names `claudeDesktop`, `claudeCursor`, `claudeVsCode`, `codexDesktop`, `codexCursor`, `codexVsCode`, `claudeApi`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Editor { Cursor, VsCode }` with `pub fn scheme(self) -> &'static str` and `pub fn extensions_dir(self, home: &Path) -> PathBuf`
  - `pub const CLAUDE_EXTENSION: &str = "anthropic.claude-code-"`, `pub const CODEX_EXTENSION: &str = "openai.chatgpt-"`, `pub const CODEX_PACKAGE: &str = "OpenAI.Codex"`, `pub const API_KEY: &str = "anthropic-api-key"`
  - `pub fn has_extension(dir: &Path, prefix: &str) -> bool`, `pub fn codex_family(packages: &Path) -> Option<String>`, `pub fn installed_targets(home: &Path, localappdata: &Path, protocol: impl Fn(&str) -> bool, api_key: bool) -> Vec<AgentId>`
  - `launcher::win::protocol_registered(scheme: &str) -> bool`
  - Tauri command `launcher_targets` (no arguments) → `Vec<AgentId>`; Task 3 calls it as `Bridge.launcherTargets()`
  - `Settings.launcher_agent: String` (wire `launcherAgent`, serde default `""`)

- [ ] **Step 1: Write the failing tests**

Create `windows/src-tauri/src/launcher/mod.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::AgentId::*;
    use super::*;
    use crate::testutil::TempDir;
    use serde_json::json;

    const ALL: [AgentId; 7] = [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode, ClaudeApi];

    fn mkdirs(root: &Path, dirs: &[&str]) {
        for dir in dirs {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    /// A profile with both extensions in both editors and the Codex package: (home, localappdata).
    fn machine(tmp: &TempDir) -> (PathBuf, PathBuf) {
        let home = tmp.path.join("home");
        let local = tmp.path.join("local");
        mkdirs(
            &home,
            &[
                r".cursor\extensions\anthropic.claude-code-2.1.291-win32-x64",
                r".cursor\extensions\openai.chatgpt-26.721.30844-win32-x64",
                r".vscode\extensions\anthropic.claude-code-2.1.289-win32-x64",
                r".vscode\extensions\openai.chatgpt-26.930.51102-win32-x64",
            ],
        );
        mkdirs(&local, &[r"Packages\OpenAI.Codex_2p2nqsd0c76g0"]);
        (home, local)
    }

    #[test]
    fn agent_ids_have_stable_wire_names() {
        let names: Vec<_> = ALL.iter().map(|a| serde_json::to_value(a).unwrap()).collect();
        assert_eq!(
            names,
            [
                json!("claudeDesktop"),
                json!("claudeCursor"),
                json!("claudeVsCode"),
                json!("codexDesktop"),
                json!("codexCursor"),
                json!("codexVsCode"),
                json!("claudeApi"),
            ]
        );
        assert_eq!(serde_json::from_value::<AgentId>(json!("claudeVsCode")).unwrap(), ClaudeVsCode);
        assert!(serde_json::from_value::<AgentId>(json!("cursor")).is_err());
    }

    #[test]
    fn an_extension_folder_of_any_version_is_found() {
        let tmp = TempDir::new("launcher-ext");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION));
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION));
    }

    #[test]
    fn no_folder_another_extension_or_a_file_is_no_target() {
        let tmp = TempDir::new("launcher-ext-none");
        assert!(!has_extension(&tmp.path.join("missing"), CLAUDE_EXTENSION), "no extensions folder");
        mkdirs(&tmp.path, &["openai.codex-audio-26.930.41038", "anthropic.claude-code-insiders", "anthropic.claude-code-"]);
        std::fs::write(tmp.path.join("openai.chatgpt-26.930.51102-win32-x64"), b"").unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "no version after the prefix");
        assert!(!has_extension(&tmp.path, CODEX_EXTENSION), "codex-audio is another extension, a file is no folder");
    }

    #[test]
    fn a_folder_listed_as_obsolete_does_not_count() {
        // Both editors keep the folders of uninstalled versions and list them in `.obsolete`.
        let tmp = TempDir::new("launcher-ext-obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.288-win32-x64", "openai.chatgpt-26.623.31443-win32-x64"]);
        std::fs::write(
            tmp.path.join(".obsolete"),
            r#"{"anthropic.claude-code-2.1.288-win32-x64":true,"openai.chatgpt-26.623.31443-win32-x64":false}"#,
        )
        .unwrap();
        assert!(!has_extension(&tmp.path, CLAUDE_EXTENSION), "uninstalled, its folder not yet removed");
        assert!(has_extension(&tmp.path, CODEX_EXTENSION), "false is not obsolete");
        mkdirs(&tmp.path, &["anthropic.claude-code-2.1.291-win32-x64"]);
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "a newer version is installed");
        std::fs::remove_dir_all(tmp.path.join("anthropic.claude-code-2.1.291-win32-x64")).unwrap();
        std::fs::write(tmp.path.join(".obsolete"), b"{ torn").unwrap();
        assert!(has_extension(&tmp.path, CLAUDE_EXTENSION), "an unreadable list hides nothing");
    }

    #[test]
    fn codex_family_is_the_package_folder_name() {
        let tmp = TempDir::new("launcher-packages");
        assert_eq!(codex_family(&tmp.path.join("missing")), None);
        mkdirs(&tmp.path, &["Claude_pzs8sxrjxfjjc", "OpenAI.CodexBeta_2p2nqsd0c76g0", "OpenAI.Codex_", "OpenAI.Codex_a_b"]);
        std::fs::write(tmp.path.join("OpenAI.Codex_filenotfolder"), b"").unwrap();
        assert_eq!(codex_family(&tmp.path), None);
        mkdirs(&tmp.path, &["OpenAI.Codex_2p2nqsd0c76g0"]);
        assert_eq!(codex_family(&tmp.path).as_deref(), Some("OpenAI.Codex_2p2nqsd0c76g0"));
    }

    #[test]
    fn installed_targets_follow_the_spec_order() {
        let tmp = TempDir::new("launcher-targets");
        let (home, local) = machine(&tmp);
        assert_eq!(installed_targets(&home, &local, |_| true, true), ALL);
    }

    #[test]
    fn a_target_needs_its_protocol_and_the_api_needs_the_key() {
        let tmp = TempDir::new("launcher-targets-partial");
        let (home, local) = machine(&tmp);
        assert_eq!(
            installed_targets(&home, &local, |s| s != "claude", false),
            [ClaudeCursor, ClaudeVsCode, CodexDesktop, CodexCursor, CodexVsCode],
            "no claude protocol, no key"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "codex", true),
            [ClaudeDesktop, ClaudeCursor, ClaudeVsCode, CodexCursor, CodexVsCode, ClaudeApi],
            "a Codex package folder without its protocol"
        );
        assert_eq!(
            installed_targets(&home, &local, |s| s != "vscode", false),
            [ClaudeDesktop, ClaudeCursor, CodexDesktop, CodexCursor],
            "a profile folder left by an uninstalled VS Code"
        );
        let nobody = tmp.path.join("nobody");
        assert_eq!(installed_targets(&nobody, &nobody, |_| true, false), [ClaudeDesktop], "only the protocol");
    }
}
```

Create `windows/src-tauri/src/launcher/win.rs` with only its test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_protocol_is_found_and_an_unknown_one_is_not() {
        // `http` is registered on every Windows; the second name exists nowhere.
        assert!(protocol_registered("http"));
        assert!(!protocol_registered("mayak-no-such-protocol"));
        assert!(!protocol_registered(""), "the root of the classes is no protocol");
        assert!(!protocol_registered(r"http\shell"), "a path is no scheme");
    }
}
```

`lib.rs`, after `mod island;`:

```rust
mod launcher;
```

`settings.rs`, at the end of module `tests` (before its closing `}`):

```rust

    #[test]
    fn an_older_settings_file_loads_with_no_launcher_agent() {
        // Exactly the keys a settings.json from before the launcher has.
        let old = r#"{"soundEnabled":true,"soundVolume":0.12,"autoCloseInterval":15,"absenceInterval":180,"activeIntegrations":[],"screen":"primary","autostart":false,"model":"claude-opus-5","alertWaiting":true,"alertFinished":true,"alertError":true,"hotkey":"Alt+M","hotkeyEnabled":true}"#;
        let s = serde_json::from_str::<super::Settings>(old).expect("an older settings.json must still load");
        assert_eq!(s.launcher_agent, "");
        assert_eq!(super::Settings::default().launcher_agent, "");
    }

    #[test]
    fn the_launcher_agent_round_trips_in_camel_case() {
        let mut s = super::Settings::default();
        s.launcher_agent = "codexDesktop".into();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["launcherAgent"], "codexDesktop");
        let back: super::Settings = serde_json::from_value(v).unwrap();
        assert_eq!(back.launcher_agent, "codexDesktop");
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib launcher 2>&1 | grep -E "^error" | sort -u | head -8`
Expected: compile errors, among them `unresolved import \`super::AgentId\``, `cannot find function \`has_extension\` in this scope` and `no field \`launcher_agent\` on type \`Settings\``. (`win.rs` is not compiled yet: `pub mod win;` comes with Step 4.)

- [ ] **Step 3: The `windows` feature**

`Cargo.toml`, in the `windows` features list, replace

```toml
  "Win32_System_Ole",
  "Win32_System_SystemInformation",
```

with

```toml
  "Win32_System_Ole",
  "Win32_System_Registry",
  "Win32_System_SystemInformation",
```

- [ ] **Step 4: Implement discovery**

`launcher/mod.rs`, above `#[cfg(test)]`:

```rust
//! The chat tab as a launcher (spec docs/superpowers/specs/2026-10-07-mayak-launcher-design.md):
//! which agents are installed (§2, §3) and opening one with the question in its field (§4).
//! Everything but `open` and its flows is pure or reads only the folders given to it; the Win32
//! part is in `win.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod win;

/// A target of the prompt view (spec §3). Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentId {
    ClaudeDesktop,
    ClaudeCursor,
    ClaudeVsCode,
    CodexDesktop,
    CodexCursor,
    CodexVsCode,
    /// The island chat over the saved Anthropic key; never launched.
    ClaudeApi,
}

/// The editors that host the Claude and Codex extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    Cursor,
    VsCode,
}

impl Editor {
    /// URL scheme of the editor (`cursor://…`, `vscode://…`), also its protocol key.
    pub fn scheme(self) -> &'static str {
        match self {
            Editor::Cursor => "cursor",
            Editor::VsCode => "vscode",
        }
    }

    /// `%USERPROFILE%\.cursor\extensions` / `%USERPROFILE%\.vscode\extensions` (spec §2).
    pub fn extensions_dir(self, home: &Path) -> PathBuf {
        let profile = match self {
            Editor::Cursor => ".cursor",
            Editor::VsCode => ".vscode",
        };
        home.join(profile).join("extensions")
    }
}

/// Folder name prefix of the Claude Code extension (`anthropic.claude-code-2.1.291-win32-x64`).
pub const CLAUDE_EXTENSION: &str = "anthropic.claude-code-";
/// Folder name prefix of the Codex extension (`openai.chatgpt-26.930.51102-win32-x64`).
pub const CODEX_EXTENSION: &str = "openai.chatgpt-";
/// AppX package name of Codex Desktop; its family is `OpenAI.Codex_<publisher id>`.
pub const CODEX_PACKAGE: &str = "OpenAI.Codex";
/// Credential Manager key of the Anthropic API key (secrets.rs).
pub const API_KEY: &str = "anthropic-api-key";

/// True when `dir` holds a folder of the extension `prefix` (`<prefix><digit>…`, any version)
/// that the editor has not marked `true` in `.obsolete` (uninstalled, folder not removed yet).
pub fn has_extension(dir: &Path, prefix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let obsolete = obsolete_folders(dir);
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return false };
        is_extension_folder(name, prefix)
            && !obsolete.contains(name)
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
    })
}

fn is_extension_folder(name: &str, prefix: &str) -> bool {
    name.len() > prefix.len()
        && name.is_char_boundary(prefix.len())
        && name[..prefix.len()].eq_ignore_ascii_case(prefix)
        && name.as_bytes()[prefix.len()].is_ascii_digit()
}

/// The folder names `.obsolete` marks `true`; empty when the file is missing or not a JSON object.
fn obsolete_folders(dir: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(".obsolete")) else { return HashSet::new() };
    let Ok(map) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) else { return HashSet::new() };
    map.into_iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k).collect()
}

/// The package family folder of Codex Desktop under `%LOCALAPPDATA%\Packages`
/// (`OpenAI.Codex_2p2nqsd0c76g0`), as it is named on disk.
pub fn codex_family(packages: &Path) -> Option<String> {
    let entries = std::fs::read_dir(packages).ok()?;
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find(|name| is_family_of(name, CODEX_PACKAGE))
}

/// `<package>_<publisher id>`, the publisher id ASCII letters and digits only.
fn is_family_of(name: &str, package: &str) -> bool {
    let Some((head, publisher)) = name.split_once('_') else { return false };
    head.eq_ignore_ascii_case(package) && !publisher.is_empty() && publisher.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The installed targets in the spec's order (§3). `protocol` tells whether a URL scheme is
/// registered (`win::protocol_registered`); `api_key` whether the Anthropic key is saved.
pub fn installed_targets(home: &Path, localappdata: &Path, protocol: impl Fn(&str) -> bool, api_key: bool) -> Vec<AgentId> {
    let in_editor = |editor: Editor, prefix: &str| {
        protocol(editor.scheme()) && has_extension(&editor.extensions_dir(home), prefix)
    };
    let mut found = Vec::new();
    if protocol("claude") {
        found.push(AgentId::ClaudeDesktop);
    }
    if in_editor(Editor::Cursor, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeCursor);
    }
    if in_editor(Editor::VsCode, CLAUDE_EXTENSION) {
        found.push(AgentId::ClaudeVsCode);
    }
    if protocol("codex") && codex_family(&localappdata.join("Packages")).is_some() {
        found.push(AgentId::CodexDesktop);
    }
    if in_editor(Editor::Cursor, CODEX_EXTENSION) {
        found.push(AgentId::CodexCursor);
    }
    if in_editor(Editor::VsCode, CODEX_EXTENSION) {
        found.push(AgentId::CodexVsCode);
    }
    if api_key {
        found.push(AgentId::ClaudeApi);
    }
    found
}

```

`launcher/win.rs`, above `#[cfg(test)]`:

```rust
//! The Win32 of the launcher: the registry lookup of URL protocols.

use ::windows::core::HSTRING;
use ::windows::Win32::Foundation::ERROR_SUCCESS;
use ::windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};

/// True when `scheme` is a registered URL protocol: `HKEY_CLASSES_ROOT\<scheme>` (HKCU and HKLM
/// classes merged) has a `URL Protocol` value. Only the presence of that value is read.
pub fn protocol_registered(scheme: &str) -> bool {
    if scheme.is_empty() || scheme.contains(['\\', '/']) {
        return false;
    }
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CLASSES_ROOT, &HSTRING::from(scheme), Some(0), KEY_READ, &mut key) != ERROR_SUCCESS {
            return false;
        }
        let found = RegQueryValueExW(key, &HSTRING::from("URL Protocol"), None, None, None, None) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        found
    }
}

```

- [ ] **Step 5: The setting and the command**

`settings.rs`, in `pub struct Settings`, after `pub hotkey_enabled: bool,`:

```rust
    /// The prompt view's last chosen agent (launcher spec §3), a wire name of `launcher::AgentId`;
    /// "" until one is chosen. Kept as text so a name this build does not know still loads.
    #[serde(default)]
    pub launcher_agent: String,
```

In `impl Default for Settings`, after `hotkey_enabled: true,`:

```rust
            launcher_agent: String::new(),
```

`lib.rs`, after the `usage_refresh` command:

```rust

/// The agents the prompt view can open (launcher spec §2-§3), in the spec's order, "claudeApi"
/// last when the Anthropic key is saved. Reads folders, the registry and the Credential Manager,
/// so it runs on its own thread.
#[tauri::command]
async fn launcher_targets() -> Vec<launcher::AgentId> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let targets = launcher::installed_targets(
            &platform::home_dir(),
            &app_paths().localappdata,
            launcher::win::protocol_registered,
            secrets::present(launcher::API_KEY),
        );
        let _ = tx.send(targets);
    });
    rx.await.unwrap_or_default()
}
```

In `tauri::generate_handler![ … ]`, after `usage_refresh,`:

```rust
            launcher_targets,
```

- [ ] **Step 6: Line endings, the tests and the suite**

Run: `cd /c/Users/HP/source/repos/mayak/windows && unix2dos src-tauri/src/launcher/mod.rs src-tauri/src/launcher/win.rs && file src-tauri/src/launcher/mod.rs src-tauri/src/launcher/win.rs src-tauri/src/lib.rs src-tauri/src/settings.rs src-tauri/Cargo.toml`
Expected: every file `with CRLF line terminators`.

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib 2>&1 | grep -E "^test (launcher::|settings::tests::(an_older_settings_file_loads_with_no_launcher_agent|the_launcher_agent_round_trips))"` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: the first run lists exactly 10 lines (7 `launcher::tests::`, 1 `launcher::win::tests::`, 2 settings tests), each ending `... ok`; suite: lib **492 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/Cargo.toml windows/src-tauri/src/launcher/mod.rs windows/src-tauri/src/launcher/win.rs windows/src-tauri/src/lib.rs windows/src-tauri/src/settings.rs && git commit -m "feat(launcher): find the installed agents and remember the chosen one"
```

---

### Task 2: Opening an agent → `launcher_open`

**Files:**
- Modify: `windows/src-tauri/src/launcher/mod.rs` (the `use` block at the top; new code between `installed_targets` and `#[cfg(test)]`; tests module end)
- Modify: `windows/src-tauri/src/launcher/win.rs` (the `use` block; new code between `protocol_registered` and `#[cfg(test)]`; tests module end)
- Modify: `windows/src-tauri/src/focus/win.rs` (after `host_windows` ~line 146; tests after `the_enumeration_runs_and_only_the_host_windows_have_titles` ~line 273)
- Modify: `windows/src-tauri/Cargo.toml` (`windows` features)
- Modify: `windows/src-tauri/src/lib.rs` (new command after `launcher_targets`; `generate_handler!`)
- Test: `launcher/mod.rs`, `launcher/win.rs`, `focus/win.rs` (modules `tests`)

**Interfaces:**
- Consumes: Task 1's `AgentId`, `Editor`, `codex_family`, test helpers `ALL`, `mkdirs`; `focus::{pick_window, Pick, WindowInfo}`, `focus::workspace::{load, Workspaces}`, `focus::win::{raise, Raise, process_table, raw_windows (private), window_title (private)}`; `crate::platform::{find_on_path, no_console}`; `crate::files::inbox_dir() -> PathBuf`; `crate::log::line`.
- Produces:
  - `impl AgentId { pub fn as_str(self) -> &'static str }` (= the wire name)
  - `impl Editor { pub fn host(self) -> HostKind; pub fn cli(self) -> &'static str /* "cursor" | "code" */; pub fn exe(self) -> &'static str /* "Cursor.exe" | "Code.exe" */ }`
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize)] #[serde(tag = "kind", rename_all = "camelCase")] pub enum LaunchOutcome { Opened, OpenedPasteYourself, Failed { reason: String } }` — wire `{ "kind": "opened" }`, `{ "kind": "openedPasteYourself" }`, `{ "kind": "failed", "reason": "…" }`; `pub fn name(&self) -> &'static str`
  - `pub const MAX_PROMPT_CHARS: usize = 8000; MAX_FILES: usize = 10; MAX_URI_LEN: usize = 30_000; EDITOR_WAIT = 15 s; CODEX_WAIT = 5 s; WAIT_STEP = 250 ms`
  - `pub fn encode_component(text: &str) -> String`, `claude_desktop_uri(prompt: Option<&str>, root: Option<&str>, files: &[String]) -> String`, `editor_claude_uri(editor: Editor, prompt: Option<&str>) -> String`, `editor_codex_uri(editor: Editor) -> String`, `mention_prompt(prompt: &str, files: &[String]) -> String`, `codex_text(prompt: &str, files: &[String]) -> String`, `prompt_fits(prompt: &str) -> bool`
  - `pub struct UriPlan { pub uri: String, pub clipboard: Option<String> }`; `claude_desktop_plan(prompt: &str, root: Option<&str>, files: &[String]) -> UriPlan`, `editor_claude_plan(editor: Editor, prompt: &str, files: &[String]) -> UriPlan`
  - `pub struct Request { pub prompt: String, pub root: Option<String>, pub files: Vec<String> }`; `prepare(prompt: &str, root: Option<String>, files: &[String], inbox: &Path) -> Result<Request, String>`
  - `should_paste(window_pid: Option<u32>, app_pids: &[u32], keys_down: bool) -> bool`, `is_package_image(image: &str, family: &str) -> bool`
  - `pub enum EditorStep { Ready, Raise(isize), Wait }`; `editor_step(host: HostKind, root: &str, foreground: Option<isize>, windows: &[WindowInfo], workspaces: impl FnMut(&[String]) -> Workspaces) -> EditorStep`
  - `wait_for<T>(limit: Duration, step: Duration, sleep: impl FnMut(Duration), probe: impl FnMut() -> Option<T>) -> Option<T>`
  - `log_line(agent: AgentId, outcome: &LaunchOutcome) -> String`
  - `pub fn open(agent: AgentId, prompt: &str, root: Option<String>, files: &[String], localappdata: &Path, inbox: &Path) -> LaunchOutcome` (blocking; writes the log line)
  - `launcher::win::{Com::init() -> Com, open_uri(uri: &str) -> Result<(), String>, utf16_nul(text: &str) -> Vec<u16>, set_clipboard_text(text: &str) -> Result<(), String>, ctrl_v_inputs() -> [INPUT; 4], send_ctrl_v() -> Result<(), String>, keys_held() -> bool, foreground() -> Option<(isize, u32)>, image_path(pid: u32) -> Option<String>, launch_appx(family: &str) -> Result<(), String>}`
  - `focus::win::app_windows(exe_name: &str) -> Vec<WindowInfo>`
  - Tauri command `launcher_open(agent: AgentId, prompt: String, root: Option<String>, files: Vec<String>) -> LaunchOutcome` — Task 3 calls it as `Bridge.launcherOpen(agent, prompt, root, files)`

- [ ] **Step 1: Write the failing tests**

`launcher/mod.rs`, module `tests`: after `use serde_json::json;` add

```rust
    use crate::observe::keys::HostKind;
    use std::time::Duration;
```

and at the end of the module (after `a_target_needs_its_protocol_and_the_api_needs_the_key`, before the closing `}`):

```rust

    // ── Opening (spec §4, §6) ──

    #[test]
    fn encode_component_matches_encode_uri_component() {
        // Expected values: Node, encodeURIComponent.
        assert_eq!(encode_component("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        assert_eq!(encode_component(r"a b&c=d?e#f+g/h\i:j%"), "a%20b%26c%3Dd%3Fe%23f%2Bg%2Fh%5Ci%3Aj%25");
        assert_eq!(encode_component("Привет"), "%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82");
        assert_eq!(encode_component("line\nnext\r\t"), "line%0Anext%0D%09");
        assert_eq!(encode_component("😀"), "%F0%9F%98%80");
        assert_eq!(encode_component(""), "");
    }

    #[test]
    fn the_claude_desktop_uri_carries_the_question_the_folder_and_each_file() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\b c.txt".to_string()];
        assert_eq!(
            claude_desktop_uri(Some("fix a & b #1"), Some(r"C:\src\app"), &files),
            "claude://code/new?q=fix%20a%20%26%20b%20%231&folder=C%3A%5Csrc%5Capp&file=C%3A%5Cin%5Ca.txt&file=C%3A%5Cin%5Cb%20c.txt"
        );
        assert_eq!(claude_desktop_uri(Some("hi"), None, &[]), "claude://code/new?q=hi", "no project: no folder");
        assert_eq!(claude_desktop_uri(Some("Привет"), None, &[]), "claude://code/new?q=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82");
        assert_eq!(claude_desktop_uri(None, Some(r"C:\a"), &[]), "claude://code/new?folder=C%3A%5Ca");
        assert_eq!(claude_desktop_uri(None, None, &[]), "claude://code/new");
    }

    #[test]
    fn the_editor_uris_name_the_extension_routes() {
        assert_eq!(
            editor_claude_uri(Editor::Cursor, Some("Привет & #")),
            "cursor://anthropic.claude-code/open?prompt=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%26%20%23"
        );
        assert_eq!(editor_claude_uri(Editor::VsCode, Some("x")), "vscode://anthropic.claude-code/open?prompt=x");
        assert_eq!(editor_claude_uri(Editor::Cursor, None), "cursor://anthropic.claude-code/open");
        assert_eq!(editor_codex_uri(Editor::Cursor), "cursor://openai.chatgpt/");
        assert_eq!(editor_codex_uri(Editor::VsCode), "vscode://openai.chatgpt/");
        let text = mention_prompt("why?", &[r"C:\in\a.txt".to_string(), r"C:\in\my doc.pdf".to_string()]);
        assert_eq!(
            editor_claude_uri(Editor::VsCode, Some(&text)),
            "vscode://anthropic.claude-code/open?prompt=%40C%3A%5Cin%5Ca.txt%20%40%22C%3A%5Cin%5Cmy%20doc.pdf%22%20why%3F"
        );
    }

    #[test]
    fn files_become_mentions_for_claude_in_the_editor() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\my doc.pdf".to_string()];
        assert_eq!(mention_prompt("why?", &files), r#"@C:\in\a.txt @"C:\in\my doc.pdf" why?"#);
        assert_eq!(mention_prompt("why?", &[]), "why?");
    }

    #[test]
    fn codex_text_puts_each_path_on_its_own_line() {
        let files = [r"C:\in\a.txt".to_string(), r"C:\in\b c.txt".to_string()];
        assert_eq!(codex_text("why?\nreally", &files), "why?\nreally\n\nC:\\in\\a.txt\nC:\\in\\b c.txt");
        assert_eq!(codex_text("why?", &[]), "why?");
    }

    #[test]
    fn the_limits_count_characters_not_bytes() {
        assert_eq!((MAX_PROMPT_CHARS, MAX_FILES, MAX_URI_LEN), (8000, 10, 30_000));
        assert!(prompt_fits(&"x".repeat(MAX_PROMPT_CHARS)));
        assert!(!prompt_fits(&"x".repeat(MAX_PROMPT_CHARS + 1)));
        assert!(prompt_fits(&"я".repeat(MAX_PROMPT_CHARS)), "16 000 bytes, 8000 characters");
    }

    #[test]
    fn a_question_too_long_for_the_link_goes_to_the_clipboard_whole() {
        // 8000 Cyrillic characters fit the character limit but encode to 48 000 link characters.
        let long = "я".repeat(MAX_PROMPT_CHARS);
        assert_eq!(
            claude_desktop_plan(&long, Some(r"C:\src\app"), &[]),
            UriPlan { uri: "claude://code/new?folder=C%3A%5Csrc%5Capp".into(), clipboard: Some(long.clone()) }
        );
        let over = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(claude_desktop_plan(&over, None, &[]), UriPlan { uri: "claude://code/new".into(), clipboard: Some(over) });
        assert_eq!(claude_desktop_plan(&"x".repeat(MAX_PROMPT_CHARS), None, &[]).clipboard, None);
        assert_eq!(
            editor_claude_plan(Editor::Cursor, &long, &[]),
            UriPlan { uri: "cursor://anthropic.claude-code/open".into(), clipboard: Some(long) }
        );
        let half = "я".repeat(4_000); // 24 000 encoded characters: fits
        let plan = editor_claude_plan(Editor::VsCode, &half, &[]);
        assert_eq!((plan.clipboard, plan.uri.len()), (None, "vscode://anthropic.claude-code/open?prompt=".len() + 24_000));
    }

    /// A temporary inbox (`<tmp>\inbox`) with one file in it.
    fn temp_inbox(tmp: &TempDir) -> (PathBuf, String) {
        let inbox = tmp.path.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let file = inbox.join("note (2).txt");
        std::fs::write(&file, b"x").unwrap();
        (inbox, file.to_string_lossy().into_owned())
    }

    #[test]
    fn prepare_accepts_a_question_a_folder_and_inbox_files() {
        let tmp = TempDir::new("launcher-prepare");
        let (inbox, file) = temp_inbox(&tmp);
        let root = tmp.path.to_string_lossy().into_owned();
        assert_eq!(
            prepare("why?", Some(root.clone()), &[file.clone()], &inbox),
            Ok(Request { prompt: "why?".into(), root: Some(root), files: vec![file] })
        );
        assert_eq!(prepare("why?", None, &[], &inbox).map(|r| r.root), Ok(None));
        assert_eq!(prepare("why?", Some(String::new()), &[], &inbox).map(|r| r.root), Ok(None), "\"\" is No project");
    }

    #[test]
    fn prepare_refuses_files_outside_the_inbox() {
        let tmp = TempDir::new("launcher-prepare-files");
        let (inbox, _) = temp_inbox(&tmp);
        let outside = tmp.path.join("secret.txt");
        std::fs::write(&outside, b"x").unwrap();
        std::fs::create_dir_all(inbox.join("folder")).unwrap();
        let refused = Err("The file is not in Mayak's inbox.".to_string());
        for file in [
            outside.to_string_lossy().into_owned(),
            format!(r"{}\..\secret.txt", inbox.display()),
            inbox.join("folder").to_string_lossy().into_owned(),
            inbox.join("gone.txt").to_string_lossy().into_owned(),
            "note (2).txt".to_string(),
        ] {
            assert_eq!(prepare("q", None, &[file.clone()], &inbox), refused, "{file}");
        }
        let eleven: Vec<String> = (0..11)
            .map(|i| {
                let path = inbox.join(format!("{i}.txt"));
                std::fs::write(&path, b"x").unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert_eq!(prepare("q", None, &eleven, &inbox), Err("At most 10 files can go with a question.".to_string()));
        assert!(prepare("q", None, &eleven[..10], &inbox).is_ok());
    }

    #[test]
    fn prepare_refuses_a_root_that_is_not_an_existing_absolute_folder() {
        let tmp = TempDir::new("launcher-prepare-root");
        let (inbox, file) = temp_inbox(&tmp);
        let gone = Err("The project folder is not there any more.".to_string());
        // bef6558: a launcher must never see a root like `--x`.
        for root in ["--x".to_string(), r"work\app".to_string(), tmp.path.join("missing").to_string_lossy().into_owned(), file] {
            assert_eq!(prepare("q", Some(root.clone()), &[], &inbox), gone, "{root}");
        }
    }

    #[test]
    fn prepare_refuses_an_empty_question() {
        let tmp = TempDir::new("launcher-prepare-empty");
        let (inbox, _) = temp_inbox(&tmp);
        for prompt in ["", "  \n\t"] {
            assert_eq!(prepare(prompt, None, &[], &inbox), Err("The question is empty.".to_string()));
        }
    }

    #[test]
    fn paste_only_into_the_app_window_with_no_key_held() {
        assert!(should_paste(Some(4242), &[4242], false));
        assert!(!should_paste(Some(4242), &[4242], true), "Enter, Shift, Alt or Win held");
        assert!(!should_paste(Some(77), &[4242], false), "another app is in front");
        assert!(!should_paste(None, &[4242], false), "no foreground window");
        assert!(!should_paste(Some(0), &[0], false), "no process");
        assert!(!should_paste(Some(4242), &[], false), "not a Codex window");
    }

    #[test]
    fn only_an_image_inside_the_codex_package_folder_counts() {
        let family = "OpenAI.Codex_2p2nqsd0c76g0";
        let app = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.930.7945.0_x64__2p2nqsd0c76g0\app\Codex.exe";
        assert!(is_package_image(app, family));
        assert!(is_package_image(&app.to_lowercase(), family), "any case");
        for other in [
            r"C:\Program Files\WindowsApps\Claude_2.19675.1.0_x64__pzs8sxrjxfjjc\app\Claude.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.930.7945.0_x64__otherpublisher\app\Codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_1.0.0.0_x64__2p2nqsd0c76g0\app\Codex.exe",
            r"C:\Users\dev\Downloads\OpenAI.Codex_26.930.7945.0_x64__2p2nqsd0c76g0\Codex.exe",
            r"C:\Users\dev\AppData\Local\Mayak\mayak.exe",
            "",
        ] {
            assert!(!is_package_image(other, family), "{other}");
        }
        assert!(!is_package_image(app, "OpenAI.Codex"), "a family without its publisher");
    }

    fn w(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), minimized: false }
    }

    fn no_workspaces(_: &[String]) -> Workspaces {
        Workspaces::new()
    }

    #[test]
    fn the_editor_step_is_ready_raise_or_wait() {
        let root = r"C:\src\app";
        let wins = [w(1, "a.rs - other - Cursor"), w(2, "b.rs - app - Cursor")];
        let step = |front: Option<isize>, wins: &[WindowInfo]| editor_step(HostKind::Cursor, root, front, wins, no_workspaces);
        assert_eq!(step(Some(2), &wins), EditorStep::Ready, "the project window is in front");
        assert_eq!(step(Some(1), &wins), EditorStep::Raise(2), "another project is in front");
        assert_eq!(step(Some(99), &wins), EditorStep::Raise(2), "the island is in front");
        assert_eq!(step(None, &wins), EditorStep::Raise(2));
        assert_eq!(step(Some(1), &wins[..1]), EditorStep::Wait, "the project is not open yet");
        let twins = [w(1, "a.rs - app - Cursor"), w(2, "b.rs - app - Cursor")];
        assert_eq!(step(Some(99), &twins), EditorStep::Wait, "two windows of one folder name: the launcher decides");
        assert_eq!(step(Some(2), &twins), EditorStep::Ready, "the launcher brought one forward");
        let ws = [w(3, "x - all (Workspace) - Cursor")];
        let known = |_: &[String]| Workspaces::from([("all".to_string(), vec![r"c:\src".to_string()])]);
        assert_eq!(editor_step(HostKind::Cursor, root, Some(3), &ws, known), EditorStep::Ready, "a workspace holding the root");
        assert_eq!(editor_step(HostKind::VsCode, root, Some(2), &wins, no_workspaces), EditorStep::Wait, "Cursor titles are no VS Code window");
    }

    #[test]
    fn the_waits_are_bounded() {
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found: Option<()> = wait_for(EDITOR_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            None
        });
        assert_eq!((found, probes, slept), (None, 61, Duration::from_secs(15)), "15 s in 250 ms steps, then stop");
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found: Option<()> = wait_for(CODEX_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            None
        });
        assert_eq!((found, probes, slept), (None, 21, Duration::from_secs(5)));
        let (mut probes, mut slept) = (0, Duration::ZERO);
        let found = wait_for(EDITOR_WAIT, WAIT_STEP, |d| slept += d, || {
            probes += 1;
            (probes == 3).then_some(7)
        });
        assert_eq!((found, probes, slept), (Some(7), 3, Duration::from_millis(500)), "stops at the first hit");
    }

    #[test]
    fn outcomes_have_stable_wire_shapes_and_names() {
        assert_eq!(serde_json::to_value(LaunchOutcome::Opened).unwrap(), json!({ "kind": "opened" }));
        assert_eq!(serde_json::to_value(LaunchOutcome::OpenedPasteYourself).unwrap(), json!({ "kind": "openedPasteYourself" }));
        assert_eq!(
            serde_json::to_value(LaunchOutcome::Failed { reason: "x".into() }).unwrap(),
            json!({ "kind": "failed", "reason": "x" })
        );
        let all = [LaunchOutcome::Opened, LaunchOutcome::OpenedPasteYourself, LaunchOutcome::Failed { reason: String::new() }];
        let names: Vec<&str> = all.iter().map(LaunchOutcome::name).collect();
        assert_eq!(names, ["Opened", "OpenedPasteYourself", "Failed"]);
        for agent in ALL {
            assert_eq!(serde_json::to_value(agent).unwrap(), json!(agent.as_str()));
        }
    }

    #[test]
    fn the_log_line_has_only_the_agent_and_the_outcome() {
        let failed = LaunchOutcome::Failed { reason: r"C:\Users\dev\secret question".into() };
        assert_eq!(log_line(AgentId::CodexDesktop, &failed), "launcher: agent=codexDesktop outcome=Failed");
        assert_eq!(log_line(AgentId::ClaudeVsCode, &LaunchOutcome::Opened), "launcher: agent=claudeVsCode outcome=Opened");
    }
```

`launcher/win.rs`, at the end of module `tests` (after `a_registered_protocol_is_found_and_an_unknown_one_is_not`):

```rust

    #[test]
    fn ctrl_v_is_four_key_events_and_never_enter() {
        let keys: Vec<(VIRTUAL_KEY, bool)> = ctrl_v_inputs()
            .iter()
            .map(|input| {
                assert_eq!(input.r#type, INPUT_KEYBOARD);
                let ki = unsafe { input.Anonymous.ki };
                (ki.wVk, ki.dwFlags == KEYEVENTF_KEYUP)
            })
            .collect();
        assert_eq!(keys, [(VK_CONTROL, false), (VK_V, false), (VK_V, true), (VK_CONTROL, true)]);
        assert!(keys.iter().all(|(vk, _)| *vk != VK_RETURN));
    }

    #[test]
    fn clipboard_text_is_utf16_with_a_terminating_nul() {
        assert_eq!(utf16_nul("aя\n"), [0x61, 0x44F, 0x0A, 0]);
        assert_eq!(utf16_nul(""), [0]);
    }

    #[test]
    fn the_foreground_and_image_lookups_run() {
        // Read-only lookups; nothing here sends a key or touches the clipboard.
        let me = image_path(std::process::id()).expect("the test binary has an image path");
        assert!(me.to_ascii_lowercase().ends_with(".exe"), "{me}");
        assert_eq!(image_path(0), None, "the idle process has no image");
        let _ = foreground();
        let _ = keys_held();
    }
```

`focus/win.rs`, module `tests`, after `the_enumeration_runs_and_only_the_host_windows_have_titles`:

```rust

    #[test]
    fn an_app_without_processes_has_no_windows() {
        assert!(app_windows("mayak-no-such-app.exe").is_empty());
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib launcher 2>&1 | grep -E "^error" | sort -u | head -8`
Expected: compile errors, among them `cannot find function \`encode_component\` in this scope`, `cannot find function \`ctrl_v_inputs\` in this scope` and `cannot find function \`app_windows\` in this scope`.

- [ ] **Step 3: The `windows` features**

`Cargo.toml`, in the `windows` features list, replace

```toml
  "Win32_Storage_FileSystem",
  "Win32_System_Diagnostics_ToolHelp",
```

with

```toml
  "Win32_Storage_FileSystem",
  "Win32_System_Com",
  "Win32_System_DataExchange",
  "Win32_System_Diagnostics_ToolHelp",
```

replace

```toml
  "Win32_System_LibraryLoader",
  "Win32_System_Ole",
```

with

```toml
  "Win32_System_LibraryLoader",
  "Win32_System_Memory",
  "Win32_System_Ole",
```

and replace

```toml
  "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_WindowsAndMessaging",
```

with

```toml
  "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
```

- [ ] **Step 4: The Win32 pieces**

`focus/win.rs`, after `pub fn host_windows(..) { .. }`:

```rust

/// Visible, titled top-level windows of every process whose image is `exe_name` (any case),
/// topmost first: the windows of an editor when no agent pid leads to it (the launcher).
pub fn app_windows(exe_name: &str) -> Vec<WindowInfo> {
    let pids: Vec<u32> =
        process_table().into_iter().filter(|r| r.exe_name.eq_ignore_ascii_case(exe_name)).map(|r| r.pid).collect();
    if pids.is_empty() {
        return Vec::new();
    }
    raw_windows()
        .into_iter()
        .filter(|w| pids.contains(&w.pid))
        .filter_map(|w| {
            window_title(w.hwnd).map(|title| WindowInfo { hwnd: w.hwnd.0 as isize, title, minimized: w.minimized })
        })
        .collect()
}
```

`launcher/win.rs`: replace the doc comment and the `use` block at the top (everything above `/// True when \`scheme\` is a registered URL protocol`) with

```rust
//! The Win32 of the launcher: the registry lookup of URL protocols, links, the clipboard, keys,
//! the foreground window and AppX activation. Tests never open a link, never send a key and
//! never touch the clipboard: those calls are checked by hand, with real agent windows.

use std::process::Command;

use ::windows::core::{HSTRING, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::{CloseHandle, GlobalFree, ERROR_SUCCESS, HANDLE};
use ::windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use ::windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use ::windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use ::windows::Win32::System::Ole::CF_UNICODETEXT;
use ::windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};
use ::windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_V,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SW_SHOWNORMAL};

```

Then, after `pub fn protocol_registered(..) { .. }`:

```rust

/// COM on the launch thread for its lifetime: `ShellExecuteW` may hand a link to a shell
/// extension, which needs it. Uninitialised on drop when the init succeeded.
pub struct Com(bool);

impl Com {
    pub fn init() -> Com {
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Opens `uri` with the handler Windows has for its protocol (spec §4, `ShellExecuteW` "open").
pub fn open_uri(uri: &str) -> Result<(), String> {
    let code = unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    }
    .0 as isize;
    // ShellExecuteW reports success as a value above 32.
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Windows could not open the link (error {code})."))
    }
}

/// `text` as the NUL-terminated UTF-16 that CF_UNICODETEXT holds.
pub fn utf16_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn clipboard_busy() -> String {
    "The clipboard is busy. Try again.".to_string()
}

/// Puts `text` on the clipboard as Unicode text, replacing what is there. One attempt.
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    let wide = utf16_nul(text);
    unsafe {
        OpenClipboard(None).map_err(|_| clipboard_busy())?;
        let result = fill_clipboard(&wide);
        let _ = CloseClipboard();
        result
    }
}

/// The clipboard is open: empty it and hand it a moveable copy of `wide`, which the system owns
/// once `SetClipboardData` succeeds.
unsafe fn fill_clipboard(wide: &[u16]) -> Result<(), String> {
    unsafe {
        EmptyClipboard().map_err(|_| clipboard_busy())?;
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|_| clipboard_busy())?;
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(clipboard_busy());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        // FALSE with no error once the lock count is zero: nothing to check.
        let _ = GlobalUnlock(mem);
        if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
            return Err(clipboard_busy());
        }
        Ok(())
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Ctrl down, V down, V up, Ctrl up: the only keys Mayak ever sends (spec §1.4: never Enter).
pub fn ctrl_v_inputs() -> [INPUT; 4] {
    [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)]
}

/// Sends Ctrl+V to the foreground window. The caller has checked which window that is.
pub fn send_ctrl_v() -> Result<(), String> {
    let inputs = ctrl_v_inputs();
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows refused the key presses.".to_string())
    }
}

/// True while Enter, Shift, Alt or a Windows key is held: Ctrl+V then would be another shortcut,
/// or an Enter still going down would reach the agent.
pub fn keys_held() -> bool {
    [VK_RETURN, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|vk| (unsafe { GetAsyncKeyState(i32::from(vk.0)) } as u16 & 0x8000) != 0)
}

/// The foreground window and the pid of its process; None when no window is in front.
pub fn foreground() -> Option<(isize, u32)> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    Some((hwnd.0 as isize, pid))
}

/// Full path of the image of `pid`; None when the process cannot be opened.
pub fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        let text = buf.get(..len as usize)?;
        ok.then(|| String::from_utf16_lossy(text))
    }
}

/// Starts the AppX application `<family>!App` (spec §4: `explorer shell:AppsFolder\…`).
pub fn launch_appx(family: &str) -> Result<(), String> {
    Command::new("explorer")
        .arg(format!(r"shell:AppsFolder\{family}!App"))
        .spawn()
        .map(|_| ())
        .map_err(|_| "Explorer did not start.".to_string())
}
```

- [ ] **Step 5: The pure launch logic and the flows**

`launcher/mod.rs`: replace the `use` block at the top (from `use std::collections::HashSet;` to `pub mod win;`) with

```rust
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::focus::{self, workspace::Workspaces, Pick, WindowInfo};
use crate::observe::keys::HostKind;
use crate::{log, platform};

pub mod win;
```

After `pub enum AgentId { .. }`:

```rust

impl AgentId {
    /// The wire name, also the one in the log line.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeDesktop => "claudeDesktop",
            AgentId::ClaudeCursor => "claudeCursor",
            AgentId::ClaudeVsCode => "claudeVsCode",
            AgentId::CodexDesktop => "codexDesktop",
            AgentId::CodexCursor => "codexCursor",
            AgentId::CodexVsCode => "codexVsCode",
            AgentId::ClaudeApi => "claudeApi",
        }
    }
}
```

In `impl Editor`, after `pub fn extensions_dir(..) { .. }`:

```rust

    /// The host kind `focus::pick_window` tells this editor's titles by.
    pub fn host(self) -> HostKind {
        match self {
            Editor::Cursor => HostKind::Cursor,
            Editor::VsCode => HostKind::VsCode,
        }
    }

    /// The command-line launcher found on PATH (`cursor <root>`, `code <root>`).
    pub fn cli(self) -> &'static str {
        match self {
            Editor::Cursor => "cursor",
            Editor::VsCode => "code",
        }
    }

    /// Image name of the editor's processes, whose windows are searched.
    pub fn exe(self) -> &'static str {
        match self {
            Editor::Cursor => "Cursor.exe",
            Editor::VsCode => "Code.exe",
        }
    }
```

After `pub fn installed_targets(..) { .. }` (before `#[cfg(test)]`):

```rust

// ── Opening an agent (spec §4) ──────────────────────────────────────────────────

/// Longest question a link carries (spec §4); a longer one goes through the clipboard whole.
pub const MAX_PROMPT_CHARS: usize = 8000;
/// Most files one question takes (spec §4).
pub const MAX_FILES: usize = 10;
/// Longest link handed to Windows. The protocol handler gets the link on its command line, which
/// holds 32 767 characters; 8000 Cyrillic characters alone encode to 48 000.
pub const MAX_URI_LEN: usize = 30_000;
/// How long the project's editor window is waited for after `cursor <root>` (spec §4).
pub const EDITOR_WAIT: Duration = Duration::from_secs(15);
/// How long the Codex Desktop window is waited for in the foreground (spec §4).
pub const CODEX_WAIT: Duration = Duration::from_secs(5);
/// Step of both waits.
pub const WAIT_STEP: Duration = Duration::from_millis(250);
/// After the Codex window came forward, before Ctrl+V: its field takes the focus meanwhile.
const PASTE_SETTLE: Duration = Duration::from_millis(300);

/// What the island gets back (spec §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LaunchOutcome {
    /// The agent is open with the question in its field.
    Opened,
    /// The agent is open; the question is on the clipboard for the user to paste.
    OpenedPasteYourself,
    /// Nothing usable opened. `reason` is Mayak's own text, never a path or the question.
    Failed { reason: String },
}

impl LaunchOutcome {
    /// The name in the log line.
    pub fn name(&self) -> &'static str {
        match self {
            LaunchOutcome::Opened => "Opened",
            LaunchOutcome::OpenedPasteYourself => "OpenedPasteYourself",
            LaunchOutcome::Failed { .. } => "Failed",
        }
    }
}

/// The only thing a launch writes to `mayak.log` (spec §4): no question, no path, no reason.
pub fn log_line(agent: AgentId, outcome: &LaunchOutcome) -> String {
    format!("launcher: agent={} outcome={}", agent.as_str(), outcome.name())
}

/// `encodeURIComponent`: every byte of the UTF-8 except ASCII letters, digits and `-_.!~*'()`
/// becomes `%XX`.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    if params.is_empty() {
        return base.to_string();
    }
    let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode_component(v))).collect();
    format!("{base}?{}", query.join("&"))
}

/// `claude://code/new?q=…&folder=…&file=…` (spec §2): a new Code session in the folder with the
/// question in its field; each part only when given.
pub fn claude_desktop_uri(prompt: Option<&str>, root: Option<&str>, files: &[String]) -> String {
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(q) = prompt {
        params.push(("q", q));
    }
    if let Some(folder) = root {
        params.push(("folder", folder));
    }
    for file in files {
        params.push(("file", file.as_str()));
    }
    with_query("claude://code/new", &params)
}

/// `cursor://anthropic.claude-code/open?prompt=…` (or `vscode://`): Claude in the last active
/// editor window with the question in its field; without a prompt, the panel alone.
pub fn editor_claude_uri(editor: Editor, prompt: Option<&str>) -> String {
    let base = format!("{}://anthropic.claude-code/open", editor.scheme());
    match prompt {
        Some(text) => with_query(&base, &[("prompt", text)]),
        None => base,
    }
}

/// `cursor://openai.chatgpt/` (or `vscode://`): the Codex panel of the editor.
pub fn editor_codex_uri(editor: Editor) -> String {
    format!("{}://openai.chatgpt/", editor.scheme())
}

/// The question for Claude in the editor: `@<path>` per file before it (spec §2), a path with
/// whitespace in quotes.
pub fn mention_prompt(prompt: &str, files: &[String]) -> String {
    let mut out = String::new();
    for file in files {
        if file.chars().any(char::is_whitespace) {
            let _ = write!(out, "@\"{file}\" ");
        } else {
            let _ = write!(out, "@{file} ");
        }
    }
    out.push_str(prompt);
    out
}

/// The clipboard text for Codex: the question, a blank line, one path per line (spec §2).
pub fn codex_text(prompt: &str, files: &[String]) -> String {
    if files.is_empty() {
        return prompt.to_string();
    }
    format!("{prompt}\n\n{}", files.join("\n"))
}

/// At most MAX_PROMPT_CHARS characters (not bytes).
pub fn prompt_fits(prompt: &str) -> bool {
    prompt.chars().count() <= MAX_PROMPT_CHARS
}

/// A link to open and, when the question did not fit in it, the whole question for the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriPlan {
    pub uri: String,
    pub clipboard: Option<String>,
}

fn plan(text: &str, with: String, without: impl FnOnce() -> String) -> UriPlan {
    if prompt_fits(text) && with.len() <= MAX_URI_LEN {
        UriPlan { uri: with, clipboard: None }
    } else {
        UriPlan { uri: without(), clipboard: Some(text.to_string()) }
    }
}

/// Claude Desktop: the question in `q=` when it fits, else the folder and files only.
pub fn claude_desktop_plan(prompt: &str, root: Option<&str>, files: &[String]) -> UriPlan {
    plan(prompt, claude_desktop_uri(Some(prompt), root, files), || claude_desktop_uri(None, root, files))
}

/// Claude in the editor: the question with its mentions in `prompt=` when it fits, else the panel.
pub fn editor_claude_plan(editor: Editor, prompt: &str, files: &[String]) -> UriPlan {
    let text = mention_prompt(prompt, files);
    let with = editor_claude_uri(editor, Some(&text));
    plan(&text, with, || editor_claude_uri(editor, None))
}

/// A checked launch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub prompt: String,
    pub root: Option<String>,
    pub files: Vec<String>,
}

/// Checks what the webview sent: a question that is not blank; `root` absent or "" (no project)
/// or an existing absolute folder; at most MAX_FILES files, each an existing file directly in
/// `inbox`. Nothing else reaches a link, a command line or the clipboard.
pub fn prepare(prompt: &str, root: Option<String>, files: &[String], inbox: &Path) -> Result<Request, String> {
    if prompt.trim().is_empty() {
        return Err("The question is empty.".to_string());
    }
    let root = match root.filter(|r| !r.is_empty()) {
        None => None,
        Some(r) => {
            let path = Path::new(&r);
            if !(path.is_absolute() && path.is_dir()) {
                return Err("The project folder is not there any more.".to_string());
            }
            Some(r)
        }
    };
    if files.len() > MAX_FILES {
        return Err(format!("At most {MAX_FILES} files can go with a question."));
    }
    for file in files {
        let path = Path::new(file);
        if path.parent() != Some(inbox) || !path.is_file() {
            return Err("The file is not in Mayak's inbox.".to_string());
        }
    }
    Ok(Request { prompt: prompt.to_string(), root, files: files.to_vec() })
}

/// Paste only when the foreground window belongs to the app (`app_pids`) and no key that would
/// change Ctrl+V, or reach the agent, is held (spec §4, §6).
pub fn should_paste(window_pid: Option<u32>, app_pids: &[u32], keys_down: bool) -> bool {
    !keys_down && window_pid.is_some_and(|pid| pid != 0 && app_pids.contains(&pid))
}

/// True when `image` lies in the install folder of the package `family` (`<Name>_<Publisher>`):
/// `…\WindowsApps\<Name>_<Version>_<Arch>_<ResourceId>_<Publisher>\…`, any case.
pub fn is_package_image(image: &str, family: &str) -> bool {
    let Some((name, publisher)) = family.split_once('_') else { return false };
    if name.is_empty() || publisher.is_empty() {
        return false;
    }
    let parts: Vec<&str> = image.split(['\\', '/']).collect();
    parts.windows(2).any(|pair| {
        let fields: Vec<&str> = pair[1].split('_').collect();
        pair[0].eq_ignore_ascii_case("WindowsApps")
            && fields.len() == 5
            && fields[0].eq_ignore_ascii_case(name)
            && fields[4].eq_ignore_ascii_case(publisher)
    })
}

/// One look at an editor's windows while its project window is brought forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorStep {
    /// The project's window is in the foreground: the link can go.
    Ready,
    /// The project's window exists: raise it.
    Raise(isize),
    /// No single window of the project: start or wait for the launcher.
    Wait,
}

/// `windows` are the editor's windows (topmost first), `foreground` the window in front. The
/// project's window is chosen as «Open» chooses it (`focus::pick_window`, workspaces included).
pub fn editor_step(
    host: HostKind,
    root: &str,
    foreground: Option<isize>,
    windows: &[WindowInfo],
    mut workspaces: impl FnMut(&[String]) -> Workspaces,
) -> EditorStep {
    if let Some(front) = foreground.and_then(|hwnd| windows.iter().find(|w| w.hwnd == hwnd)) {
        if focus::pick_window(host, root, std::slice::from_ref(front), &mut workspaces) == Pick::One(front.hwnd) {
            return EditorStep::Ready;
        }
    }
    match focus::pick_window(host, root, windows, &mut workspaces) {
        Pick::One(hwnd) => EditorStep::Raise(hwnd),
        Pick::Many | Pick::None => EditorStep::Wait,
    }
}

/// Probes now and then every `step` until `limit` has passed; the first Some wins. Bounded:
/// at most `limit / step + 1` probes. The only waits of the launcher (spec §4).
pub fn wait_for<T>(
    limit: Duration,
    step: Duration,
    mut sleep: impl FnMut(Duration),
    mut probe: impl FnMut() -> Option<T>,
) -> Option<T> {
    let mut waited = Duration::ZERO;
    loop {
        if let Some(found) = probe() {
            return Some(found);
        }
        if waited >= limit {
            return None;
        }
        sleep(step);
        waited += step;
    }
}

/// Opens `agent` with the question in its field (spec §4) and writes the one log line. Blocking
/// (up to EDITOR_WAIT, or CODEX_WAIT plus a settle): call it off the UI thread.
pub fn open(
    agent: AgentId,
    prompt: &str,
    root: Option<String>,
    files: &[String],
    localappdata: &Path,
    inbox: &Path,
) -> LaunchOutcome {
    let _com = win::Com::init();
    let outcome = match prepare(prompt, root, files, inbox) {
        Ok(req) => run(agent, &req, localappdata).unwrap_or_else(|reason| LaunchOutcome::Failed { reason }),
        Err(reason) => LaunchOutcome::Failed { reason },
    };
    log::line(log_line(agent, &outcome));
    outcome
}

fn run(agent: AgentId, req: &Request, localappdata: &Path) -> Result<LaunchOutcome, String> {
    match agent {
        AgentId::ClaudeDesktop => open_plan(&claude_desktop_plan(&req.prompt, req.root.as_deref(), &req.files)),
        AgentId::ClaudeCursor => claude_in_editor(Editor::Cursor, req),
        AgentId::ClaudeVsCode => claude_in_editor(Editor::VsCode, req),
        AgentId::CodexDesktop => codex_desktop(req, localappdata),
        AgentId::CodexCursor => codex_in_editor(Editor::Cursor, req),
        AgentId::CodexVsCode => codex_in_editor(Editor::VsCode, req),
        AgentId::ClaudeApi => Err("Claude API answers in the island.".to_string()),
    }
}

fn open_plan(plan: &UriPlan) -> Result<LaunchOutcome, String> {
    if let Some(text) = &plan.clipboard {
        win::set_clipboard_text(text)?;
    }
    win::open_uri(&plan.uri)?;
    Ok(if plan.clipboard.is_some() { LaunchOutcome::OpenedPasteYourself } else { LaunchOutcome::Opened })
}

fn claude_in_editor(editor: Editor, req: &Request) -> Result<LaunchOutcome, String> {
    let plan = editor_claude_plan(editor, &req.prompt, &req.files);
    if let Some(root) = &req.root {
        place_editor(editor, root)?;
    }
    open_plan(&plan)
}

fn codex_in_editor(editor: Editor, req: &Request) -> Result<LaunchOutcome, String> {
    win::set_clipboard_text(&codex_text(&req.prompt, &req.files))?;
    if let Some(root) = &req.root {
        place_editor(editor, root)?;
    }
    win::open_uri(&editor_codex_uri(editor))?;
    // No automatic paste: the focus may be in the code editor, and a paste there would change a file.
    Ok(LaunchOutcome::OpenedPasteYourself)
}

fn codex_desktop(req: &Request, localappdata: &Path) -> Result<LaunchOutcome, String> {
    let family =
        codex_family(&localappdata.join("Packages")).ok_or_else(|| "Codex Desktop is not installed.".to_string())?;
    win::set_clipboard_text(&codex_text(&req.prompt, &req.files))?;
    win::launch_appx(&family)?;
    let came = wait_for(CODEX_WAIT, WAIT_STEP, std::thread::sleep, || codex_in_front(&family, false).then_some(()));
    if came.is_none() {
        return Ok(LaunchOutcome::OpenedPasteYourself);
    }
    std::thread::sleep(PASTE_SETTLE);
    // Checked again right before the keys: the window may have changed meanwhile.
    if !codex_in_front(&family, win::keys_held()) || win::send_ctrl_v().is_err() {
        return Ok(LaunchOutcome::OpenedPasteYourself);
    }
    Ok(LaunchOutcome::Opened)
}

/// The foreground window belongs to the Codex package and, with `keys_down` false, may get Ctrl+V.
fn codex_in_front(family: &str, keys_down: bool) -> bool {
    let window_pid = win::foreground().map(|(_, pid)| pid);
    let app: Vec<u32> = window_pid
        .filter(|pid| win::image_path(*pid).is_some_and(|image| is_package_image(&image, family)))
        .into_iter()
        .collect();
    should_paste(window_pid, &app, keys_down)
}

/// Brings the project's editor window to the foreground (spec §4 steps 1-3): raised when found,
/// else `cursor <root>` / `code <root>` and up to EDITOR_WAIT in WAIT_STEP steps. Err when it never
/// came forward: the link would then land in whichever window was active last.
fn place_editor(editor: Editor, root: &str) -> Result<(), String> {
    let host = editor.host();
    let look = || {
        let windows = focus::win::app_windows(editor.exe());
        let front = win::foreground().map(|(hwnd, _)| hwnd);
        editor_step(host, root, front, &windows, |names| focus::workspace::load(host, names))
    };
    let mut flashed: Option<isize> = None;
    if settle(look(), &mut flashed) {
        return Ok(());
    }
    let launcher =
        platform::find_on_path(editor.cli()).ok_or_else(|| format!("`{}` is not on PATH.", editor.cli()))?;
    let mut cmd = Command::new(launcher);
    cmd.arg(root);
    platform::no_console(&mut cmd).spawn().map_err(|_| format!("`{}` did not start.", editor.cli()))?;
    wait_for(EDITOR_WAIT, WAIT_STEP, std::thread::sleep, || settle(look(), &mut flashed).then_some(()))
        .ok_or_else(|| "The project window did not open in time.".to_string())
}

/// Acts on one look: true when the project's window is in front. A window that was raised but
/// only flashed (or was stale) is not raised again, or it would flash at every step.
fn settle(step: EditorStep, flashed: &mut Option<isize>) -> bool {
    match step {
        EditorStep::Ready => true,
        EditorStep::Raise(hwnd) if *flashed != Some(hwnd) => {
            if focus::win::raise(hwnd) == focus::win::Raise::Focused {
                return true;
            }
            *flashed = Some(hwnd);
            false
        }
        EditorStep::Raise(_) | EditorStep::Wait => false,
    }
}

```

- [ ] **Step 6: The command**

`lib.rs`, after the `launcher_targets` command:

```rust

/// The prompt view's «Send» (launcher spec §4): opens `agent` in `root` with the question in its
/// field and never sends it. The work runs on its own thread, like `session_open`'s; the island
/// gets the outcome when it is done (an editor that has to start takes up to 15 s).
#[tauri::command]
async fn launcher_open(
    agent: launcher::AgentId,
    prompt: String,
    root: Option<String>,
    files: Vec<String>,
) -> launcher::LaunchOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let localappdata = app_paths().localappdata;
        let inbox = crate::files::inbox_dir();
        let _ = tx.send(launcher::open(agent, &prompt, root, &files, &localappdata, &inbox));
    });
    rx.await.unwrap_or_else(|_| launcher::LaunchOutcome::Failed { reason: "The launch stopped.".to_string() })
}
```

In `tauri::generate_handler![ … ]`, after `launcher_targets,`:

```rust
            launcher_open,
```

- [ ] **Step 7: Line endings, the tests and the suite**

Run: `cd /c/Users/HP/source/repos/mayak/windows && file src-tauri/src/launcher/mod.rs src-tauri/src/launcher/win.rs src-tauri/src/focus/win.rs src-tauri/src/lib.rs src-tauri/Cargo.toml`
Expected: every file `with CRLF line terminators` (all were edited in place; if one shows LF only, run `unix2dos` on it).

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test -p mayak --lib 2>&1 | grep -E "^test (launcher::|focus::win::tests::an_app_without_processes_has_no_windows)"` then `cargo test --workspace 2>&1 | grep -E "^test result|^warning|^error"`
Expected: the first run lists exactly 29 lines — 24 `launcher::tests::` (7 from Task 1, 17 new), 4 `launcher::win::tests::` (1 + 3) and `focus::win::tests::an_app_without_processes_has_no_windows` — each ending `... ok`; suite: lib **513 passed, 1 ignored**, hook 32 passed, 0 failed, no `warning` line.

- [ ] **Step 8: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src-tauri/Cargo.toml windows/src-tauri/src/launcher/mod.rs windows/src-tauri/src/launcher/win.rs windows/src-tauri/src/focus/win.rs windows/src-tauri/src/lib.rs && git commit -m "feat(launcher): open an agent with the question in its field"
```

---

### Task 3: The prompt view as a launcher (bridge, state, pure model, view, CSS)

**Files:**
- Modify: `windows/src/core/bridge.ts` (`Bridge` after `usageRefresh` ~line 78; types after `DroppedFile` ~line 143)
- Modify: `windows/src/core/state.ts` (`Settings` ~line 115; `DEFAULT_SETTINGS` ~line 133; `AppState` after `chatHistory` ~line 163)
- Create: `windows/src/views/launcher-model.ts`
- Rewrite: `windows/src/views/chat.ts`
- Modify: `windows/src/views/views.ts` (registry ~line 533)
- Modify: `windows/src/island/island.ts` (`targetSize` ~line 640; `chatHeight` ~line 1186-1188)
- Modify: `windows/src/style.css` (after `.send-btn:active { .. }` ~line 947; new selectors only)
- Create: `C:\tmp\mayak-spike\launcher-model-check.mjs` (outside the repository)

**Interfaces:**
- Consumes: commands `launcher_targets` (Task 1) and `launcher_open` (Task 2) with the wire names above; `Settings.launcherAgent` (Task 1); `ViewActions.setView`, `ViewActions.collapse`; `ViewHost.setActive` (called by `island.ts` `syncDom` only when "expanded and on screen" changes); `Sessions.snapshot`; `State.droppedFile`, `State.promptContext`, `State.chatHistory`, `State.noteMessage`, `State.stateOverride`.
- Produces:
  - `bridge.ts`: `export type LauncherAgent = "claudeDesktop" | "claudeCursor" | "claudeVsCode" | "codexDesktop" | "codexCursor" | "codexVsCode" | "claudeApi"`, `export type LaunchOutcome = { kind: "opened" } | { kind: "openedPasteYourself" } | { kind: "failed"; reason: string }`, `Bridge.launcherTargets(): Promise<LauncherAgent[] | null>`, `Bridge.launcherOpen(agent: LauncherAgent, prompt: string, root: string | null, files: string[]): Promise<LaunchOutcome | null>`
  - `state.ts`: `Settings.launcherAgent: string`, `State.launcherApi: boolean`, `get State.promptCount(): number`
  - `launcher-model.ts`: `API_AGENT`, `AGENT_LABEL: Record<LauncherAgent, string>`, `NO_PROJECT = "No project"`, `PASTE_HINT = "Paste the question with Ctrl+V"`, `interface ProjectChoice { root: string; name: string }`, `projectChoices(snapshot: SessionsSnapshot | null): ProjectChoice[]`, `defaultRoot(snapshot: SessionsSnapshot | null): string | null`, `keptRoot(chosen: string | null, choices: ProjectChoice[]): string | null`, `chooseAgent(targets: LauncherAgent[], saved: string): LauncherAgent | null`, `interface KeyInfo`, `interface EnterStep { armed; send; prevent }`, `enterStep(e: KeyInfo, armed: boolean): EnterStep`, `interface FollowUp { note: string | null; then: "collapse" | "prompt"; clear: boolean }`, `launchFollowUp(outcome: LaunchOutcome | null, agent: LauncherAgent): FollowUp`
  - `chat.ts`: `export function buildPrompt(actions: ViewActions, onHeightChange: () => void): ViewHost`
  - CSS classes `.launch-row`, `.launch-select`, `.chip-x`, `.chat-card.launch`

- [ ] **Step 1: The check script (write it first)**

Create `C:\tmp\mayak-spike\launcher-model-check.mjs`:

```js
// Checks the pure parts of the Mayak launcher (launcher spec §3, §5) with Node's type stripping.
// Run: node --experimental-strip-types C:/tmp/mayak-spike/launcher-model-check.mjs
import assert from "node:assert/strict";

const m = await import("file:///C:/Users/HP/source/repos/mayak/windows/src/views/launcher-model.ts");
const project = (key, root, name, sinces) => ({ key, root, name, sessions: sinces.map((since) => ({ since })) });
const snap = (...projects) => ({ projects });

// default project: the latest `since`; the first of equal ones; none without a session
assert.equal(m.defaultRoot(snap(project("c:\\a", "C:\\a", "a", [5, 9]), project("c:\\b", "C:\\b", "b", [7]))), "C:\\a");
assert.equal(m.defaultRoot(snap(project("c:\\a", "C:\\a", "a", [7]), project("c:\\b", "C:\\b", "b", [7]))), "C:\\a");
assert.equal(m.defaultRoot(snap(project("c:\\a", "C:\\a", "a", []))), null);
assert.equal(m.defaultRoot(null), null);
// choices: projects with a session, each key once, snapshot order
assert.deepEqual(
  m.projectChoices(snap(project("c:\\a", "C:\\a", "a", [1]), project("c:\\e", "C:\\e", "e", []), project("c:\\a", "C:\\A", "a", [2]))),
  [{ root: "C:\\a", name: "a" }],
);
assert.deepEqual(m.projectChoices(null), []);
// the chosen root stays while it is a choice
const choices = [{ root: "C:\\a", name: "a" }];
assert.equal(m.keptRoot("C:\\a", choices), "C:\\a");
assert.equal(m.keptRoot("C:\\gone", choices), null);
assert.equal(m.keptRoot(null, choices), null);
// agents: the remembered one while installed, else the first
assert.equal(m.chooseAgent(["claudeDesktop", "codexDesktop"], "codexDesktop"), "codexDesktop");
assert.equal(m.chooseAgent(["claudeDesktop", "codexDesktop"], "claudeCursor"), "claudeDesktop");
assert.equal(m.chooseAgent(["claudeDesktop"], ""), "claudeDesktop");
assert.equal(m.chooseAgent([], "claudeDesktop"), null);
// Enter (plan Review Focus 1): the question goes on the release, once
const key = (type, o = {}) => ({ type, key: "Enter", shiftKey: false, repeat: false, isComposing: false, ...o });
assert.deepEqual(m.enterStep(key("keydown"), false), { armed: true, send: false, prevent: true }, "a press arms and sends nothing");
assert.deepEqual(m.enterStep(key("keydown", { repeat: true }), true), { armed: true, send: false, prevent: true }, "auto-repeat sends nothing");
assert.deepEqual(m.enterStep(key("keyup"), true), { armed: false, send: true, prevent: true }, "the release sends");
assert.deepEqual(m.enterStep(key("keyup"), false), { armed: false, send: false, prevent: false }, "a release of a press not seen here");
assert.deepEqual(m.enterStep(key("keydown", { repeat: true }), false), { armed: false, send: false, prevent: true }, "Enter already held when the field got the focus");
assert.deepEqual(m.enterStep(key("keydown", { shiftKey: true }), false), { armed: false, send: false, prevent: false }, "Shift+Enter is a new line");
assert.deepEqual(m.enterStep(key("keydown", { isComposing: true }), false), { armed: false, send: false, prevent: false }, "an IME commit");
assert.deepEqual(m.enterStep({ ...key("keydown"), key: "a" }, true), { armed: true, send: false, prevent: false });
let armed = false;
let sends = 0;
for (const e of [key("keydown"), ...Array.from({ length: 30 }, () => key("keydown", { repeat: true })), key("keyup")]) {
  const s = m.enterStep(e, armed);
  armed = s.armed;
  if (s.send) sends++;
}
assert.equal(sends, 1, "press, 30 repeats, release: one send");
// what follows a launch
assert.deepEqual(m.launchFollowUp({ kind: "opened" }, "claudeDesktop"), { note: null, then: "collapse", clear: true });
assert.deepEqual(m.launchFollowUp({ kind: "openedPasteYourself" }, "codexDesktop"), { note: "Paste the question with Ctrl+V", then: "collapse", clear: true });
assert.deepEqual(
  m.launchFollowUp({ kind: "failed", reason: "The project window did not open in time." }, "claudeCursor"),
  { note: "Claude in Cursor: The project window did not open in time.", then: "prompt", clear: false },
);
assert.deepEqual(m.launchFollowUp(null, "codexVsCode"), { note: "Codex in VS Code could not be opened.", then: "prompt", clear: false });
assert.equal(m.AGENT_LABEL.claudeApi, "Claude API (answers here)");
assert.equal(Object.keys(m.AGENT_LABEL).length, 7);
assert.equal(m.NO_PROJECT, "No project");
console.log("launcher-model: all checks passed");
```

Run: `node --experimental-strip-types C:/tmp/mayak-spike/launcher-model-check.mjs`
Expected: FAIL with `ERR_MODULE_NOT_FOUND` (the module does not exist yet).

- [ ] **Step 2: Bridge and state**

`bridge.ts`, after `usageRefresh: () => call<void>("usage_refresh"),`:

```ts

  // ── Launcher (prompt view, launcher spec §3-§4) ───────────────────────────
  /** The installed agents in the spec's order; "claudeApi" last when the Anthropic key is saved. */
  launcherTargets: () => call<LauncherAgent[]>("launcher_targets"),
  /** Opens the agent with the question in its field, never sends it; resolves when the launch is done. */
  launcherOpen: (agent: LauncherAgent, prompt: string, root: string | null, files: string[]) =>
    call<LaunchOutcome>("launcher_open", { agent, prompt, root, files }),
```

After `export interface DroppedFile { .. }`:

```ts

/** A target of the prompt view (launcher spec §3); wire names of Rust `launcher::AgentId`. */
export type LauncherAgent =
  | "claudeDesktop" | "claudeCursor" | "claudeVsCode"
  | "codexDesktop" | "codexCursor" | "codexVsCode"
  | "claudeApi";
/** What a launch gave (launcher spec §4); `reason` is Mayak's own text. */
export type LaunchOutcome =
  | { kind: "opened" }
  | { kind: "openedPasteYourself" }
  | { kind: "failed"; reason: string };
```

`state.ts`, in `interface Settings`, after `hotkeyEnabled: boolean;`:

```ts
  /** The prompt view's last chosen agent (a LauncherAgent name), "" until one is chosen. */
  launcherAgent: string;
```

In `DEFAULT_SETTINGS`, after `hotkeyEnabled: true,`:

```ts
  launcherAgent: "",
```

In `class AppState`, after `chatHistory: ChatMessage[] = [];`:

```ts
  /** The prompt view's agent is "Claude API": the view is the island chat (launcher spec §5). */
  launcherApi = false;
```

and after `get otherTasks(): AgentTask[] { .. }`:

```ts

  /** Messages the prompt view's height counts: the chat's while it answers here, none while it launches. */
  get promptCount(): number {
    return this.launcherApi ? this.chatHistory.length : 0;
  }
```

- [ ] **Step 3: The pure model**

Create `windows/src/views/launcher-model.ts`:

```ts
// Pure parts of the prompt view's launcher (launcher spec §3, §5). No DOM and no runtime imports,
// so Node can run these functions as they are (C:\tmp\mayak-spike\launcher-model-check.mjs).

import type { LaunchOutcome, LauncherAgent, SessionsSnapshot } from "../core/bridge";

/** The one agent that answers in the island instead of being opened. */
export const API_AGENT: LauncherAgent = "claudeApi";

export const AGENT_LABEL: Record<LauncherAgent, string> = {
  claudeDesktop: "Claude Desktop",
  claudeCursor: "Claude in Cursor",
  claudeVsCode: "Claude in VS Code",
  codexDesktop: "Codex Desktop",
  codexCursor: "Codex in Cursor",
  codexVsCode: "Codex in VS Code",
  claudeApi: "Claude API (answers here)",
};

export const NO_PROJECT = "No project";
export const PASTE_HINT = "Paste the question with Ctrl+V";

export interface ProjectChoice {
  root: string;
  name: string;
}

/** The snapshot's projects that have a session, each key once, in snapshot order. */
export function projectChoices(snapshot: SessionsSnapshot | null): ProjectChoice[] {
  const seen = new Set<string>();
  const out: ProjectChoice[] = [];
  for (const p of snapshot?.projects ?? []) {
    if (p.sessions.length === 0 || !p.root || seen.has(p.key)) continue;
    seen.add(p.key);
    out.push({ root: p.root, name: p.name });
  }
  return out;
}

/** Root of the project of the session with the latest `since` (spec §3); the first of equal ones; null without a session. */
export function defaultRoot(snapshot: SessionsSnapshot | null): string | null {
  let best: { root: string; since: number } | null = null;
  for (const p of snapshot?.projects ?? []) {
    if (!p.root) continue;
    for (const s of p.sessions) {
      if (best === null || s.since > best.since) best = { root: p.root, since: s.since };
    }
  }
  return best?.root ?? null;
}

/** The chosen root while it is still a choice, else no project. */
export function keptRoot(chosen: string | null, choices: ProjectChoice[]): string | null {
  return chosen !== null && choices.some((c) => c.root === chosen) ? chosen : null;
}

/** The remembered agent while it is still installed, else the first one; null when there is none. */
export function chooseAgent(targets: LauncherAgent[], saved: string): LauncherAgent | null {
  return targets.find((t) => t === saved) ?? targets[0] ?? null;
}

export interface KeyInfo {
  type: "keydown" | "keyup";
  key: string;
  shiftKey: boolean;
  repeat: boolean;
  isComposing: boolean;
}

export interface EnterStep {
  armed: boolean;
  send: boolean;
  prevent: boolean;
}

/**
 * Enter sends, Shift+Enter is a new line (spec §5). The question goes on the release of Enter,
 * never on its press: the agent takes the focus within milliseconds, and a repeated keydown of a
 * held Enter landing there would send the question. A repeat or a press this field did not see
 * start never arms.
 */
export function enterStep(e: KeyInfo, armed: boolean): EnterStep {
  if (e.key !== "Enter") return { armed, send: false, prevent: false };
  if (e.type === "keyup") return { armed: false, send: armed, prevent: armed };
  if (e.isComposing || e.shiftKey) return { armed: false, send: false, prevent: false };
  return { armed: armed || !e.repeat, send: false, prevent: true };
}

export interface FollowUp {
  /** Note to show, null to collapse at once. */
  note: string | null;
  /** After the note: collapse, or back to the prompt with the question kept. */
  then: "collapse" | "prompt";
  /** The question and the file were handed over: clear them. */
  clear: boolean;
}

/** What the island does after a launch (spec §4); null is a call that never reached Rust. */
export function launchFollowUp(outcome: LaunchOutcome | null, agent: LauncherAgent): FollowUp {
  if (outcome === null) return { note: `${AGENT_LABEL[agent]} could not be opened.`, then: "prompt", clear: false };
  switch (outcome.kind) {
    case "opened":
      return { note: null, then: "collapse", clear: true };
    case "openedPasteYourself":
      return { note: PASTE_HINT, then: "collapse", clear: true };
    case "failed":
      return { note: `${AGENT_LABEL[agent]}: ${outcome.reason}`, then: "prompt", clear: false };
  }
}
```

Run: `node --experimental-strip-types C:/tmp/mayak-spike/launcher-model-check.mjs`
Expected: `launcher-model: all checks passed` (plus Node's `ExperimentalWarning` line on stderr).

- [ ] **Step 4: The view**

Replace the whole of `windows/src/views/chat.ts` with:

```ts
// Chat view — DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift, rebuilt as the launcher (launcher spec §5): the question opens the
// chosen agent in the chosen project with the question in its field, never sent. "Claude API
// (answers here)" keeps the island chat as it was.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type ChatContext, type LauncherAgent } from "../core/bridge";
import { Sessions } from "../core/sessions";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import {
  AGENT_LABEL, API_AGENT, NO_PROJECT, chooseAgent, defaultRoot, enterStep, keptRoot, launchFollowUp, projectChoices,
} from "./launcher-model";
import type { ViewActions, ViewHost } from "./views";

let nextId = 1;

/** How long the launcher's hint or error stays up before the island moves on. */
const NOTE_MS = 2400;

function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (a dropped file), with its ✕ (spec §5). */
function contextChip(label: string, onRemove: () => void): HTMLElement {
  const remove = h("button", { class: "chip-x", title: "Remove", onclick: () => onRemove() }, svg(ICONS.xmark, 9));
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }), remove);
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(actions: ViewActions, onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("textarea", {
    class: "chat-input",
    rows: 1,
    placeholder: "Ask me anything…",
    spellcheck: "false",
  });
  const send = h("button", { class: "send-btn", title: "Send" }, svg(ICONS.arrowUp, 11));
  const bar = h("div", { class: "chat-bar" }, input, send);
  const agentSelect = h("select", { class: "launch-select", title: "Agent" });
  const projectSelect = h("select", { class: "launch-select", title: "Project" });
  const launchRow = h("div", { class: "launch-row" }, agentSelect, projectSelect);
  const chatCard = h(
    "div",
    { class: "card wash chat-card launch" },
    h("div", { class: "chat-body" }, chipRow, log, bar, launchRow),
  );
  chatCard.style.setProperty("--wash", "rgba(99,102,241,0.5)");
  const el = h("div", { class: "view" }, chatCard);
  agentSelect.append(h("option", { value: "", text: "…" }));

  let sending = false;
  let launching = false;
  let renderedCount = -1;
  /** Agents found when the view opened; null until the answer comes. */
  let targets: LauncherAgent[] | null = null;
  let agent: LauncherAgent | null = null;
  /** The project of this opening of the view; undefined until the first draw picks the default. */
  let chosenRoot: string | null | undefined;
  let root: string | null = null;
  /** Enter went down in the field; the question goes on its release (enterStep). */
  let armed = false;

  function setAgent(next: LauncherAgent | null) {
    agent = next;
    State.launcherApi = next === API_AGENT;
    agentSelect.value = next ?? "";
  }

  function rebuildAgents() {
    const list = targets ?? [];
    clear(agentSelect);
    if (list.length === 0) agentSelect.append(h("option", { value: "", text: "No agent found" }));
    for (const id of list) agentSelect.append(h("option", { value: id, text: AGENT_LABEL[id] }));
    setAgent(chooseAgent(list, State.settings.launcherAgent));
  }

  /** "Claude API (answers here)": the island chat, unchanged. */
  async function submitApi() {
    const query = input.value.trim();
    if (!query || sending) return;
    input.value = "";
    sending = true;
    Sound.play("send");

    State.chatHistory.push({ id: nextId++, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    const file = State.droppedFile;
    const context: ChatContext | null =
      State.chatHistory.length === 1 && file ? { kind: "file", name: file.name, path: file.path } : null;

    try {
      const reply = await Bridge.chatSend(query, context);
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      State.stateOverride = null;
      Sound.play("finish");
    } catch (err) {
      State.stateOverride = null;
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      sending = false;
      State.notify();
      onHeightChange();
      input.focus();
    }
  }

  /** Any other agent: Rust opens it with the question in its field (spec §4). */
  async function launch(target: LauncherAgent) {
    const prompt = input.value.trim();
    if (!prompt || launching) return;
    launching = true;
    Sound.play("send");
    State.stateOverride = "thinking";
    State.notify();
    const files = State.droppedFile ? [State.droppedFile.path] : [];
    const outcome = await Bridge.launcherOpen(target, prompt, root, files);
    launching = false;
    State.stateOverride = null;
    const next = launchFollowUp(outcome, target);
    if (next.clear) {
      input.value = "";
      State.droppedFile = null;
      State.promptContext = null;
    }
    if (next.note === null) {
      State.notify();
      actions.collapse();
      return;
    }
    State.noteMessage = next.note;
    if (next.then === "prompt") Sound.play("error");
    actions.setView("note");
    window.setTimeout(() => {
      if (State.view !== "note") return;
      if (next.then === "collapse") actions.collapse();
      else actions.setView("prompt");
    }, NOTE_MS);
  }

  function submit() {
    if (agent === API_AGENT) void submitApi();
    else if (agent) void launch(agent);
  }

  send.addEventListener("click", () => submit());
  const onKey = (e: KeyboardEvent) => {
    const step = enterStep(
      {
        type: e.type === "keyup" ? "keyup" : "keydown",
        key: e.key,
        shiftKey: e.shiftKey,
        repeat: e.repeat,
        isComposing: e.isComposing,
      },
      armed,
    );
    armed = step.armed;
    if (step.prevent) e.preventDefault();
    if (step.send) submit();
    e.stopPropagation(); // Escape closes the island, not the chat
  };
  input.addEventListener("keydown", onKey);
  input.addEventListener("keyup", onKey);
  agentSelect.addEventListener("change", () => {
    const picked = (targets ?? []).find((t) => t === agentSelect.value);
    if (!picked) return;
    setAgent(picked);
    State.settings.launcherAgent = picked;
    void Bridge.saveSettings(State.settings);
    State.notify();
    onHeightChange();
  });
  projectSelect.addEventListener("change", () => {
    chosenRoot = projectSelect.value || null;
    State.notify();
  });

  return {
    el,
    sync() {
      const file = State.droppedFile;
      const wantChip = file?.name ?? "";
      if (chipRow.dataset.label !== wantChip) {
        chipRow.dataset.label = wantChip;
        clear(chipRow);
        if (wantChip) {
          chipRow.append(contextChip(wantChip, () => {
            State.droppedFile = null;
            State.promptContext = null;
            State.notify();
          }));
        }
      }

      const api = agent === API_AGENT;
      chatCard.classList.toggle("launch", !api);
      projectSelect.hidden = api;

      const snapshot = Sessions.snapshot;
      const choices = projectChoices(snapshot);
      const key = choices.map((c) => c.root).join("\n");
      if (projectSelect.dataset.key !== key) {
        projectSelect.dataset.key = key;
        clear(projectSelect);
        projectSelect.append(h("option", { value: "", text: NO_PROJECT }));
        for (const c of choices) projectSelect.append(h("option", { value: c.root, text: c.name, title: c.root }));
      }
      if (chosenRoot === undefined) chosenRoot = defaultRoot(snapshot);
      root = keptRoot(chosenRoot, choices);
      projectSelect.value = root ?? "";

      const thinking = State.stateOverride === "thinking";
      const count = api ? State.chatHistory.length + (thinking ? 0.5 : 0) : -1;
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        if (api) {
          for (const m of State.chatHistory) log.append(bubble(m));
          if (thinking) log.append(typingDots());
          log.scrollTop = log.scrollHeight;
        }
      }

      input.placeholder = api && State.chatHistory.length > 0 ? "Continue…" : "Ask me anything…";
      input.disabled = sending || launching;
      send.disabled = agent === null || sending || launching;
      agentSelect.disabled = launching || (targets ?? []).length === 0;
      projectSelect.disabled = launching;
    },
    focus() {
      input.focus();
      input.select();
    },
    setActive(on) {
      armed = false;
      if (!on) {
        chosenRoot = undefined;
        return;
      }
      // Read each time the tab opens (spec §3): what is installed may have changed.
      void Bridge.launcherTargets().then((list) => {
        targets = list ?? [];
        rebuildAgents();
        State.notify();
        onHeightChange();
      });
    },
  };
}
```

`views.ts`, in `buildViews`, replace

```ts
  map.set("prompt", buildPrompt(onChatHeightChange));
```

with

```ts
  map.set("prompt", buildPrompt(actions, onChatHeightChange));
```

`island.ts`, in `targetSize`, replace

```ts
    const { w, h } = islandSize(State.mode, State.view, State.chatHistory.length, Sessions.rowCount());
```

with

```ts
    const { w, h } = islandSize(State.mode, State.view, State.promptCount, Sessions.rowCount());
```

and in `get chatHeight()`, replace

```ts
    return chatPromptHeight(State.chatHistory.length);
```

with

```ts
    return chatPromptHeight(State.promptCount);
```

- [ ] **Step 5: CSS (new selectors only)**

`style.css`, after

```css
.send-btn:active {
  transform: scale(0.94);
}
```

insert:

```css

.send-btn:disabled {
  opacity: 0.4;
}

/* ── Launcher (prompt view, launcher spec §5) ───────────────────────────── */

textarea.chat-input {
  resize: none;
  height: 18px;
  line-height: 18px;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.18) transparent;
}

.chat-card.launch .chat-log {
  display: none;
}

.chat-card.launch .chat-bar {
  flex: 1 1 auto;
  min-height: 0;
  align-items: flex-end;
  padding: 8px 10px;
}

.chat-card.launch textarea.chat-input {
  align-self: stretch;
  height: auto;
}

.chip-x {
  display: grid;
  place-items: center;
  width: 14px;
  height: 14px;
  margin-right: -4px;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--dim-2);
  cursor: default;
}

.chip-x:hover {
  background: rgba(255, 255, 255, 0.12);
  color: var(--ink);
}

.launch-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.launch-select {
  min-width: 0;
  max-width: 50%;
  padding: 3px 8px;
  border: 0;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.07);
  color: var(--ink-2);
  font: 400 11.5px var(--font);
  outline: none;
  cursor: default;
}

.launch-select:disabled {
  color: var(--dim-3);
}

.launch-select option {
  background: var(--tab-on);
  color: var(--ink);
}
```

- [ ] **Step 6: Verify**

Run: `cd /c/Users/HP/source/repos/mayak/windows && unix2dos src/views/launcher-model.ts src/views/chat.ts && npx tsc --noEmit && npx vite build 2>&1 | tail -3` then `node --experimental-strip-types C:/tmp/mayak-spike/launcher-model-check.mjs` then `file src/core/bridge.ts src/core/state.ts src/views/launcher-model.ts src/views/chat.ts src/views/views.ts src/island/island.ts src/style.css`
Expected: `unix2dos` reports two converted files; `tsc` prints nothing and exits 0; `vite build` ends with `✓ built in …`; the check prints `launcher-model: all checks passed`; all seven files `with CRLF line terminators`. `cargo test --workspace` is unaffected (nothing Rust changed).

- [ ] **Step 7: Commit**

```bash
cd /c/Users/HP/source/repos/mayak && git add windows/src/core/bridge.ts windows/src/core/state.ts windows/src/views/launcher-model.ts windows/src/views/chat.ts windows/src/views/views.ts windows/src/island/island.ts windows/src/style.css && git commit -m "feat(island): the chat tab opens the chosen agent with the question"
```

---

### Task 4: Acceptance with the user, spec alignment, publishing

**Files:**
- Modify: `docs/superpowers/specs/2026-10-07-mayak-launcher-design.md` (append; LF)
- Modify: `.git/mayak-pr-notes.md`, create `.git/mayak-pr13-body.md` (local, untracked)

**Interfaces:** consumes the branch after Tasks 1-3 (`launcher_targets`, `launcher_open`, `launcherAgent`, the prompt view); produces the acceptance record.

The controller runs Steps 2-4 **with the user**: real Claude Desktop, Cursor, VS Code and Codex windows, and only the user clicks in them.

- [ ] **Step 1: Write the decisions into the spec**

Append at the end of the spec (Russian, LF line endings):

```markdown

## 7. Уточнения при реализации

- **Длинный вопрос не обрезается.** Если вопрос длиннее 8000 символов или адрес с ним длиннее 30 000 знаков (обработчик протокола получает адрес в командной строке, а она вмещает 32 767 знаков; 8000 кириллических символов дают 48 000), адрес открывается без вопроса (у Claude Desktop остаются `folder=` и `file=`, у Claude в редакторе — `…/open` без `prompt`), а вопрос целиком кладётся в буфер обмена: результат `OpenedPasteYourself` и подсказка.
- **Enter.** Вопрос уходит при отпускании Enter, а не при нажатии; автоповтор и нажатие, начатое не в поле, ничего не отправляют. Так нажатая клавиша не попадает в окно агента. `Ctrl+V` не посылается, пока зажаты Enter, Shift, Alt или Win.
- **Поиск целей.** Протокол считается зарегистрированным, если у `HKEY_CLASSES_ROOT\<схема>` есть значение `URL Protocol` (у `codex` нет `shell\open\command`). Расширение считается установленным, если есть папка `<префикс><цифра>…` и её нет с `true` в `extensions\.obsolete` (там остаются папки удалённых версий); кроме того нужен протокол редактора (`cursor`, `vscode`): папка профиля остаётся и после удаления редактора. Codex Desktop — папка `%LOCALAPPDATA%\Packages\OpenAI.Codex_<издатель>` и протокол `codex`; приложение пакета — `App` (подраздел `App` пакета в реестре).
- **Окно Codex** — переднее окно, процесс которого запущен из `WindowsApps\OpenAI.Codex_<версия>_<архитектура>__<издатель>\`. Через 300 мс после его появления это проверяется ещё раз прямо перед `Ctrl+V`.
- **Окно редактора.** Найденное окно проекта поднимается. Если окна нет, их два с одним именем папки или Windows не дала окну передний план, запускается `cursor <папка>` (`code <папка>`), и до 15 секунд с шагом 250 мс ждётся, пока окно проекта окажется на переднем плане (или найдётся и поднимется); мигающее окно повторно не поднимается. Если за 15 секунд не вышло — `Failed`, адрес не открывается: редактор отдал бы его окну другого проекта.
- **Проверка входа.** Вопрос не пустой; папка проекта — существующая абсолютная или «No project»; файлов не больше 10, каждый — существующий файл прямо в папке inbox Mayak. Иначе `Failed` с причиной, вопрос остаётся в поле.
- **Формат.** Для Claude в редакторе — `@<путь>` через пробел перед вопросом, путь с пробелом в кавычках: `@"C:\…\my doc.pdf"`. Для Codex — вопрос, пустая строка, по пути на строке.
- **Ответ острову.** Команда ждёт поток запуска и возвращает `{ kind: "opened" | "openedPasteYourself" | "failed", reason? }`. `opened` — остров сворачивается; `openedPasteYourself` — подсказка «Paste the question with Ctrl+V» на 2,4 с, затем остров сворачивается; `failed` — «<агент>: <причина>» на 2,4 с, затем снова вкладка чата с вопросом в поле.
- **Список агентов** читается при каждом открытии вкладки. **Проект по умолчанию** выбирается при открытии вкладки и сам не меняется; если проект пропал из снимка, выбран «No project».
- **`launcherAgent`** хранится строкой (`""` по умолчанию); неизвестное или неустановленное значение даёт первого агента списка.
- **Высота вкладки** растёт с историей чата только при «Claude API»; поле ввода многострочное в обоих режимах.
- **Буфер обмена** после запуска не восстанавливается: вопрос остаётся в нём для вставки. Буфер открывается одной попыткой, без повторов.
- **COM** инициализируется в потоке запуска (этого требует `ShellExecuteW`).
```

Commit:

```bash
cd /c/Users/HP/source/repos/mayak && git add docs/superpowers/specs/2026-10-07-mayak-launcher-design.md && git commit -m "docs: align the launcher spec with the implementation"
```

- [ ] **Step 2: The dev app**

Run: `curl -s --max-time 2 http://127.0.0.1:9222/json/list | grep -c "localhost:1420"`
Expected: at least 1 (the dev island page of this working tree is up). If it prints 0, the dev Mayak is not running: ask the user to quit the installed Mayak 0.2.0 themselves if it runs (the single-instance plugin lets only one Mayak run) and to start the dev app with `cd windows && npm run tauri dev`. Do not stop or start either app yourself; wait for the user to say it is up, then run the check again.

- [ ] **Step 3: Spec §6 live items with the user**

Record each as PASS/FAIL with what the user saw in `.git/mayak-pr-notes.md` under `## Launcher acceptance (2026-10-07)`. Every question typed in this step starts with the marker `mayak-launch-check` (Step 4 greps the log for it).
1. Claude Desktop: the user drops a small text file on the island, presses «Ask a question», picks «Claude Desktop» and the project `mayak`, types `mayak-launch-check what is in this file?` and presses Enter. Expected: a new Code session in `mayak`, the question in its field, the file attached, nothing sent; the island collapsed. Then once with «No project»: a session without a folder.
2. Claude in Cursor, three times with «Claude in Cursor»: (a) a project open in its own Cursor window; (b) a project that lies inside a multi-root workspace window; (c) a project not open in Cursor (Cursor opens it within 15 s). Expected each time: that window in front, the Claude tab with the question in its field, nothing sent.
3. Codex Desktop: «Codex Desktop», a question. Expected: Codex in front with the question pasted in its field and not sent, the island collapsed; or, if Codex did not come forward within 5 s, the note «Paste the question with Ctrl+V» and the question on the clipboard (the user pastes it).
4. Codex in Cursor: «Codex in Cursor» with a project. Expected: that project's window in front, the Codex panel open, the note «Paste the question with Ctrl+V», Ctrl+V in the panel pastes the question (with the file path on its own line when a file was dropped).
5. Claude API: only if the user has a key saved. «Claude API (answers here)»: the answer appears in the island and the history grows the view, as before this branch.
6. Nothing was sent by itself: the user confirms that in items 1-4 every agent showed the question unsent.
7. Long question (plan Review Focus 3): the user pastes into the field a text of more than 8000 characters (for example a long log) and sends it to Claude Desktop. Expected: Claude Desktop opens without the question, the note «Paste the question with Ctrl+V», and Ctrl+V pastes the whole text.

- [ ] **Step 4: Log and idle CPU**

Run: `grep "launcher:" "$LOCALAPPDATA/Mayak/mayak.log" | tail -12` then `grep -c "mayak-launch-check" "$LOCALAPPDATA/Mayak/mayak.log"` then `grep -c 'Mayak\\inbox' "$LOCALAPPDATA/Mayak/mayak.log"`
Expected: the first prints only lines ending `launcher: agent=<wire name> outcome=<Opened|OpenedPasteYourself|Failed>`, one per launch of Step 3; the two counts are `0`.

With the island hidden and no agent turn running: `powershell -File C:\tmp\mayak-accept\sample-cpu.ps1 -Seconds 60 -OutPath C:\tmp\mayak-accept\cpu-launcher-idle.log`
Expected: `tree_avg_percent_of_machine` < 0.100 (before this branch: 0.010, `.git/mayak-pr-notes.md`). Record both results in the notes.

- [ ] **Step 5: Full verification**

Run: `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=C:/tmp/mayak-target; cd /c/Users/HP/source/repos/mayak/windows && cargo test --workspace 2>&1 | grep -E "^test result|^warning"` and `npx tsc --noEmit && npx vite build 2>&1 | tail -3` and `node --experimental-strip-types C:/tmp/mayak-spike/launcher-model-check.mjs` and `cd /c/Users/HP/source/repos/mayak && git status --short && git log --oneline -5`
Expected: lib 513 passed (1 ignored), hook 32 passed, no `warning` line, build OK, `launcher-model: all checks passed`, `git status --short` empty, the four commits of this plan on top of `07feea2`, none with a trailer.

- [ ] **Step 6: Ask before publishing**

Write `.git/mayak-pr13-body.md` from the results of Steps 3-5: What (the chat tab as a launcher; the targets and how each opens; the clipboard rule for long questions; Enter on release; Ctrl+V only into a verified Codex window), Checks (local) with the test counts and every acceptance line, and no trailer. Ask the user, verbatim: «Запушить ветку `feature/launcher` в `origin` и открыть PR в `main`? Проверки на GitHub отключены, проверка локальная.» Only on an explicit yes:

```bash
cd /c/Users/HP/source/repos/mayak && git push -u origin feature/launcher && gh pr create --repo mahach666/mayak --base main --head feature/launcher --title "Mayak: the chat tab opens your agent with the question" --body-file "$(git rev-parse --git-dir)/mayak-pr13-body.md"
```

The merge stays with the user: do not merge unless the user asks for it in a separate, explicit message.

---

## Self-Review

- **Spec coverage:**
  - §1 the launcher, the question never sent, installed agents only, default project with a switch, API key optional, Codex clipboard + `Ctrl+V` without Enter → Tasks 1-3; never-send rules → `enterStep` (Task 3), `ctrl_v_inputs` / `should_paste` (Task 2); Task 4 Step 3 items 1-6.
  - §1 out of scope (no agent answers except the API, no task queue, no Codex in a terminal) → nothing added for them; rules (no polling at rest, other views unchanged) → `wait_for` bounds (Task 2 `the_waits_are_bounded`), targets read only on `setActive(true)`, CSS new selectors only; Task 4 Step 4 CPU.
  - §2 table: Claude Desktop protocol + link → `protocol_registered`, `claude_desktop_uri`; Claude in Cursor / VS Code → extension folders + `editor_claude_uri` + `place_editor`; Codex Desktop → `codex_family` + `protocol("codex")` + `launch_appx` + paste; Codex in the editor → `openai.chatgpt-` + `editor_codex_uri`, no paste; files → `file=`, `mention_prompt`, `codex_text`.
  - §3 order, remembered `launcherAgent`, default project by `since`, «No project», Claude Desktop without folder, editor without project → `installed_targets` order test, `Settings.launcher_agent` + `chooseAgent`, `defaultRoot` / `keptRoot`, `claude_desktop_uri(.., None, ..)`, `claude_in_editor` without `place_editor`.
  - §4 command arguments, own thread, encoding, 8000 / 10 limits, ShellExecuteW, editor flow 15 s / 250 ms, Codex 5 s + `SendInput`, no paste in the editor, outcomes, collapse, hint, log line → Task 2 (`launcher_open`, `prepare`, plans, `place_editor`, `codex_desktop`, `codex_in_editor`, `LaunchOutcome`, `log_line`) and Task 3 (`launchFollowUp`, note, collapse).
  - §5 field, file with ✕, two dropdowns, Send / Enter / Shift+Enter, API mode as before, drop flow and «Ask a question» unchanged → Task 3 (`buildPrompt`, `contextChip`, `.launch-row`, `enterStep`, `submitApi`); `swallow` and `buildChoose` are untouched.
  - §6 Rust tests (links: encoding, Cyrillic, `&`/`#`, several `file=`, limits; discovery on temp folders, several versions, none; clipboard text with paths; paste decision over pids) → Tasks 1-2; TS `tsc` + build → Task 3; live items 1-6 → Task 4 Step 3.
- **Placeholder scan:** none. Every code step carries its code and every command its expected output; the conditional steps name their action (no dev page: ask the user; no API key: item 5 skipped and recorded so; Codex not in front: the hint branch is the expected result).
- **Type consistency:** `AgentId` wire names (Task 1) = `LauncherAgent` (Task 3) = `AgentId::as_str` (Task 2, checked by `outcomes_have_stable_wire_shapes_and_names`); `LaunchOutcome` serializes to the `{ kind, reason? }` union `LaunchOutcome` in `bridge.ts`; the command arguments `agent`, `prompt`, `root`, `files` match `Bridge.launcherOpen`'s invoke keys; `Settings.launcher_agent` ↔ `launcherAgent` ↔ `State.settings.launcherAgent`; `buildPrompt(actions, onHeightChange)` matches its one caller in `views.ts`; `State.promptCount` replaces `State.chatHistory.length` in both `island.ts` height uses; `focus::win::app_windows`, `focus::win::raise` / `Raise::Focused`, `focus::pick_window` / `Pick`, `focus::workspace::load` / `Workspaces` are the names in `focus/`; `editor_step` passes `&mut workspaces` (an `FnMut` borrowed is an `FnOnce`) to `pick_window` twice.
- **Review Focus:** 1 → Task 3 check script (`enterStep` cases, 30-repeat sequence) and Task 2 `paste_only_into_the_app_window_with_no_key_held`; 2 → Task 1 `a_folder_listed_as_obsolete_does_not_count`, `a_target_needs_its_protocol_and_the_api_needs_the_key`; 3 → Task 2 `a_question_too_long_for_the_link_goes_to_the_clipboard_whole`, `the_limits_count_characters_not_bytes`, Task 4 Step 3 item 7; 4 → Task 2 `only_an_image_inside_the_codex_package_folder_counts`, `paste_only_into_the_app_window_with_no_key_held`, `ctrl_v_is_four_key_events_and_never_enter`; 5 → Task 2 `prepare_refuses_files_outside_the_inbox`, `prepare_refuses_a_root_that_is_not_an_existing_absolute_folder`.
- **Test counts:** Task 1 adds 7 `launcher` + 1 `launcher::win` + 2 settings tests (482 → 492); Task 2 adds 17 `launcher` + 3 `launcher::win` + 1 `focus::win` tests (492 → 513). No existing test is edited.
