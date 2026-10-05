# Mayak — подпроект 1: ядро «много сессий × много проектов»

- **Дата:** 2026-10-01
- **Статус:** дизайн согласован по разделам. Спецификация доработана по итогам состязательного ревью (59 замечаний) и ждёт ревью пользователя. 2026-10-04 внесены поправки 0–5 после слияния апстрима Coucou v0.1.5 (§3, §5, §8, §10–§14).
- **Ветка:** `feature/multi-session-core`. Репозиторий `mahach666/mayak` — приватный и отвязанный от сети форков; оригинал `Louis-CFM/coucou` остаётся remote `upstream`.
- **База:** `main` = `upstream/main` = `2b387e4` (Coucou v0.1.5); ветка содержит его через слияние `d3a96de`. Номера строк в этом документе даны по `2b387e4`.
- **Платформа:** только Windows (`windows/`). Linux-порт апстрима остаётся в дереве, но не собирается и не тестируется (§5, §12). macOS-приложение `NotchBuddy/` не развивается.

## 1. Цель и контекст

Mayak — форк Coucou, который становится самостоятельным приложением. Это «островок» поверх всех окон вверху экрана. Он показывает, что делают ИИ-агенты в нескольких проектах одновременно, и сообщает, кто из них ждёт пользователя.

**Как работает пользователь:**
- **Claude Code**: вкладка Code в Claude Desktop и расширение Claude Code внутри Cursor. Обычный чат Claude событий не отдаёт.
- **Codex**: расширение `openai.chatgpt` внутри Cursor и десктопное приложение Codex (MSIX).
- **Cursor**: редактор-хост. Собственным агентом Cursor пользователь почти не пользуется.
- **Visual Studio 2022**: появится в подпроекте 2.
- Обычно параллельно идут несколько проектов. На момент исследования было 5 живых сессий Claude в 5 разных репозиториях.

**Приоритеты подпроекта 1**, по убыванию:
1. **Кто меня ждёт.** Какой агент, в каком проекте остановился: запрос разрешения, вопрос, завершённый ход.
2. **Живой обзор.** Все сессии, сгруппированные по проектам: что делает каждая и сколько времени.
3. **Уведомления** о завершении и ошибке с кратким итогом.

Allow/Deny прямо с островка — не приоритет.

**Подпроекты:**
1. **Ядро** — этот документ.
2. **Окна и связывание.** Окна Cursor, решения VS 2022, Claude Desktop, Codex Desktop привязываются к проектам, появляется «перейти к окну». В подпроекте 1 строятся только точки расширения (§13).
3. **Визуальный ребрендинг.** Иконка, новый персонаж вместо Mochi, свои звуки, удаление ассетов Coucou. Обязателен до любой публикации сборок (`LICENSE-ASSETS.md`).

## 2. Принятые решения

| Решение | Значение |
|---|---|
| Подход | **Гибрид-наблюдатель.** Claude читается пассивно, без какой-либо настройки. Codex — опциональные хуки на уровне хода. Решения о состоянии принимает одна чистая модель на Rust с тестами. |
| Имя | Mayak. slug `mayak`, bundle id `io.github.mahach666.mayak` |
| Проект | Корень git-репозитория, найденный подъёмом от пути. Без git — сама папка. Worktree и submodule считаются отдельными проектами. |
| Охват | Только локальные сессии. Облако, WSL и SSH не входят. |
| Хуки собственного агента Cursor | Нет |
| Codex на островке | Только наблюдение. Островок **никогда** не отвечает Codex. |
| Старый Allow/Deny для Claude | Раздел «Advanced», по умолчанию выключен. Ставит только `PermissionRequest`. |
| Клик «Open» в подпроекте 1 | Корень проекта открывается в редакторе-хосте: `cursor`, `code` или Проводник. |
| Алерты | Карточка + звук, затем сворачивание. Пока есть непросмотренное ожидание, компактный островок не прячется. Типы алертов включаются по отдельности. |
| Язык интерфейса | Английский, как сейчас. Локализация вне подпроекта 1. |
| Данные Coucou | Одноразово копируется `%APPDATA%\Coucou\settings.json`, если файла Mayak ещё нет: там пользователь выбрал плашки интеграций. Старые папки не удаляются. Конфиги инструментов Coucou не трогал, мигрировать там нечего. |
| Интеграции (Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com) | Без изменений |

## 3. Факты, на которых стоит дизайн

Ниже факты помечены так: **[проверено]** — наблюдалось вживую на этой машине; **[док]** — официальная документация или исходники; **[код]** — чтение исходников, в рантайме не проверено. Всё непроверенное собрано в §15.

### Claude Code (2.1.280–2.1.286)

- **Файлы сессий.** `~/.claude/sessions/<pid>.json` — один файл на каждый живой процесс `claude.exe`, и в Desktop (`entrypoint: claude-desktop`), и в расширении (`claude-vscode`) **[проверено]**.
  - Ключи: `pid`, `sessionId`, `cwd`, `procStart`, `version`, `kind`, `entrypoint`, `name`, `nameSource`, `status`, `waitingFor`, `statusUpdatedAt`, `updatedAt`, `pidDomain` и другие.
  - Формат не документирован. Документировано только назначение: «обнаружение параллельных сессий и падений» **[док]**. Документированный аналог — `claude agents --json` (research preview, CLI на этой машине нет).
  - Рядом в той же папке лежат `<pid>.<hash>.key` с `peerToken`. Они отсекаются регулярным выражением `^[0-9]+\.json$` и никогда не открываются.
- **Spike S1, Desktop [проверено 2026-10-01].**
  - Во время вопроса пользователю файл становится `status: waiting`, `waitingFor: "input needed"`, после ответа — `busy`, в конце хода — `idle`.
  - Каждую перезапись сообщил `FileSystemWatcher` с `NotifyFilter = LastWrite | Size | FileName`.
- **`procStart`** — десятичная строка, равная FILETIME создания процесса из `GetProcessTimes`. Совпало во всех 5 живых сессиях **[проверено]**. Проверка живости нужна, потому что PID переиспользуются, а файл после падения может остаться.
- **Транскрипт** `~/.claude/projects/<slug>/<sessionId>.jsonl` дописывается с открытием и закрытием файла на каждую запись, и уведомления о дозаписи приходят **[проверено]**. Файл появляется только с первым сообщением сессии.
  - Значения `message.stop_reason` у записей assistant в 32 локальных транскриптах **[проверено]**: `tool_use` (16012), `end_turn` (601), `stop_sequence` (23), `refusal` (14). `max_tokens` — только по документации API.
  - Ошибки API помечены `isApiErrorMessage`, например `error: "rate_limit"`, `apiErrorStatus: 429`.
  - Прерывание — запись user с текстом `[Request interrupted by user`.
- **Путь exe** из `QueryFullProcessImageNameW` **[проверено]**:
  - Desktop (MSIX): `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude\claude-code\<ver>\claude.exe`. После папки версии могут идти ещё папки: в 2.1.286 это хеш-папка, `…\claude-code\2.1.286\635c1867224a\claude.exe`. Виртуализированные представления, например `Get-Process.Path`, показывают `%APPDATA%\Claude\claude-code\…`.
  - Cursor: `c:\Users\…\.cursor\extensions\anthropic.claude-code-*\resources\native-binary\claude.exe`, буква диска в нижнем регистре.
- **`~/.claude/settings.json`** — обычный файл. Его копирует `~/dotfiles/sync.ps1`: `pull` затирает, `push` коммитит и пушит. `~/.claude/skills` — junction в dotfiles.
- **`~/.claude/ide/*.lock`** содержат `authToken`.
- **Нет хука на прерывание.** `PermissionRequest` не несёт `tool_use_id`. С версии 2.1.286 в одной сессии может ждать несколько запросов **[док]**.

### Codex

Версии на машине: Desktop MSIX 26.924 бандлит codex-cli 0.158.0-alpha.2.1, расширение в Cursor — 0.146.0-alpha.3, в VS Code — 0.144/0.155.

- **Хуки — стабильная функция**, по умолчанию включена, работает в ядре **[док/код]**.
  - Конфиг `~/.codex/hooks.json`. Пользовательские хуки грузятся и в недоверенных проектах **[док]**.
  - События: `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, `Interrupt`.
  - В 0.144 нет `SessionEnd` и `Interrupt`, в 0.146 нет `Interrupt`. Неизвестные ключи событий игнорируются **[код]**.
- **Trust** **[док/код]**. Неуправляемые хуки выполняются только после подтверждения в UI Codex.
  - Ключ доверия позиционный: `<файл>:<событие>:<индекс группы>:<индекс обработчика>`.
  - Хеш покрывает `command`, `timeout`, `matcher`, `statusMessage`.
- **Формат `hooks.json`** **[код]**:
  - на верхнем уровне в 0.144/0.146 допустимы только `description` и `hooks` (`deny_unknown_fields`);
  - `mcp_tool` ломает разбор в 0.144/0.146, поэтому используется только `type: "command"`;
  - асинхронные хуки 0.144/0.146 не выполняют.
- **Вход `PermissionRequest`** (схема 0.153.4 и main) **[код]**: `agent_id`, `agent_type`, `cwd`, `hook_event_name`, `model`, `permission_mode`, `session_id`, `tool_input`, `tool_name`, `transcript_path` (nullable), `turn_id`.
  - Идентификатора вызова нет. Суффикс `:retry` внутренний и в хук **не передаётся**.
  - `Stop` несёт `last_assistant_message`.
  - Явного поля-источника нет. Пара `turn_id` + `model` в `PermissionRequest` есть только у Codex (у Claude этих полей там нет) **[док]**.
- **Запуск хука на Windows** **[код]**: через `powershell -NoProfile -Command`, в некоторых версиях и путях — через `cmd /C`.
  - Замеры **[проверено]**: `powershell -NoProfile` ~0.28–0.31 с, `cmd /C` ~0.04 с.
  - 0.144/0.146 запускают хук без `CREATE_NO_WINDOW` **[код]**.
- **Родной запрос Codex не показывается**, пока выполняется хук `PermissionRequest` **[код]**.
- **При ошибке хода `Stop` не вызывается** **[код]**: в main ошибка идёт через `emit_turn_error_lifecycle`.
- **Rollout** `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Формат legacy наблюдался локально **[проверено]**: `session_meta`, затем `event_msg` (`task_started`, `task_complete`, `turn_aborted`, …) и `response_item`.
  - Запросы разрешения туда не пишутся **[код]**.
  - В main writer держит файл открытым и не вызывает `FlushFileBuffers` **[код]**.
  - Эксперимент **[проверено]**: три дозаписи через открытый handle не дали ни одного `Changed` за время наблюдения. Значит, уведомления о такой дозаписи **не гарантированы**.
  - В main `TurnAbortedEvent.turn_id` имеет тип `Option` **[код]**.
- **`notify` в `config.toml` занят** computer-use десктопного Codex **[проверено]**. В `config.toml` есть блок, которым управляют dotfiles. Каждое окно IDE и Desktop держит свой приватный stdio app-server **[проверено]**.

### Текущий код Windows-приложения (`2b387e4`)

- **Ретранслятор** `windows/hook/src/main.rs`:
  - argv `[--agent <name>] [<EventName>]`: флаг может стоять где угодно, его значение поглощается всегда, первый из остальных аргументов — событие. Непустое имя агента вставляется в payload как `coucou_agent` (`main.rs:103-122`);
  - stdin JSON → `\\.\pipe\coucou-<SID>`; подключение 300 мс, сервер должен принадлежать тому же пользователю. `ERROR_PIPE_BUSY`, `pipe_path` и `connect` лежат в `hook/src/win.rs:22-56`; Linux-транспорт (Unix-сокет) — в `hook/src/unix.rs`;
  - ждёт ответа на любой `PermissionRequest`, от какого бы агента он ни пришёл (`main.rs:50`);
  - выкидывает `transcript_path` и `tool_response`, обрезает строки до 2000 символов;
  - снимает только один BOM.
- **`windows/src-tauri/src/pipe.rs`**: задача на каждое подключение, событие `hook` во фронтенд, состояния сессий нет. Маршрутизации по агенту нет: всё, кроме `PermissionRequest`, сразу уходит во фронтенд (`pipe.rs:198-203`). `pipe_name` и `start` под `#[cfg(windows)]`, рядом Linux-`start` и `trait Relay` (`pipe.rs:62-166`).
- **`src-tauri/src/platform/`** (4175ebc; `win_user.rs` удалён). Всё, что зависит от ОС, вызывается как `platform::…`; Windows-реализация — `platform/windows.rs`:
  - `HOOK_EXE = "coucou-hook.exe"` (`:26`), `HOME_VAR` (`:29`);
  - `config_dir`/`local_dir` — `%APPDATA%\Coucou` и `%LOCALAPPDATA%\Coucou` (`:36-50`); `settings::config_dir`/`local_dir` — их реэкспорт (`settings.rs:50`);
  - `ensure_private_dir` (`:53`), `local_time` (`:60`), `no_console` (`:75`), `find_on_path` (`:91`), `current_user_sid` (`:113`), `CURSOR_POLL = true` (`:151`);
  - `home_dir` и `LocalTime` — в `platform/mod.rs`, Linux-реализация — `platform/linux.rs`.
- **Фронтенд** сворачивает все события Claude в одну задачу `integration_claude` с одним `pendingApproval`.
  - Событие с допустимым `coucou_agent` (`^[a-z0-9-]{1,24}$`, кроме `claude`) создаёт пилюлю `agent_<name>`, а её `PermissionRequest` сразу отклоняется (`hooks.ts:157-183`, `:273-280`; `state.ts:231-252`).
  - `BootInfo.cursorPoll` (`bridge.ts:29-30`): при `false` включается `Island.followPageCursor` (`main.ts:25`).
  - `fsm.ts:14` содержит `petitToHiddenDelay = 60`. Пауза делает `island.fsm.forceHidden()` (`main.ts:48`).
  - Анимации внутри невидимых видов стоят на паузе: `.view:not(.on) * { animation-play-state: paused; }` (`style.css:274-279`, eb1241f).
- **`src-tauri/src/hooks.rs`** — установщик для `~/.claude/settings.json`: дифф, бэкап, fingerprint, маркер `coucou-hook`.
  - Дифф строится по отформатированному разбору, а не по записываемым байтам.
  - Путь бэкапа считается заново при записи (`hooks.rs:267` и `:294`).
  - BOM и CRLF не сохраняются.
  - `ensure_hook_exe` строит всех кандидатов и сообщения в лог из `platform::HOOK_EXE` (`ensure_hook_exe` и `#[cfg(windows)] install_relay`, `:357-410`).
  - `boot()` в `lib.rs:51` на каждом запуске вызывает `hooks::status()`, то есть читает `~/.claude/settings.json`.
- **`Settings`** (`settings.rs`): `serde(rename_all = "camelCase")`, без `deny_unknown_fields`, поле `hooks_installed`. `save_settings` целиком заменяет структуру копией из UI (`lib.rs:68`).
- **Linux-порт апстрима:** `platform/linux.rs`, `hook/src/unix.rs`, `src-tauri/tauri.linux.conf.json`, секции `[target.'cfg(target_os = "linux")'.dependencies]` в обоих `Cargo.toml`, ветки `#[cfg(unix)]`/`target_os = "linux"` в `hook/src/main.rs`, `pipe.rs`, `log.rs`, `hooks.rs`, раздел `## Linux` в `windows/README.md`. Слияние добавило в `Cargo.lock` 32 пакета, все для Linux; версии остальных пакетов не изменились.
- **Тесты.** Rust-тесты есть в `hooks.rs`, `files.rs`, `claude.rs`, `hook/main.rs`. TS-тестов нет.
  - CI: `.github/workflows/windows.yml` только собирает по тегу; `.github/workflows/linux.yml` собирает и тестирует под Ubuntu каждый pull request, который затрагивает `windows/**`.
  - **Базовая сборка на Windows [проверено 2026-10-04, копия `git archive d3a96de`]:** `npm run build` проходит; `cargo test --workspace --offline` — 8 тестов `coucou_lib` (claude 1, hooks 6, files 1), 0 в бинарнике, 3 в `coucou_hook`, ни одного предупреждения. Два теста `#[cfg(unix)]` в `hooks.rs` на Windows не компилируются.
  - Тест `files::ingest_copies_and_never_overwrites` пишет и удаляет файлы в настоящей `%LOCALAPPDATA%\Coucou\inbox`.

## 4. Архитектура

```
 ┌ claude-io (std::thread) ─────────────────────────────┐
 │ DirWatcher(sessions\) ─┐ сырые уведомления           │
 │ DirWatcher(projects\) ─┴► разбор файла, доказ. процесса│
 │ хвосты транскриптов, ожидаемые имена, ProcRegistry    │──┐
 └───────────────────────────────────────────────────────┘  │
 ┌ codex-io (std::thread) ──────────────────────────────┐   │   Signal (std::mpsc, unbounded, FIFO)
 │ HookEvent от pipe-router ─► доказ. процесса, проект,  │   ├──────────────► ┌ store (std::thread) ─────────┐
 │ хвосты rollout (по пути), тикер 1 Гц (только видим)   │──┤                 │ Store::apply(s, now) →       │
 └───────────────────────────────────────────────────────┘  │                 │   {alerts, effects}  (чистая)│
 ┌ exit-waiters (по потоку на процесс) ─ ProcExit ───────────┘                 │ публикация snapshot          │
                                                                              └──┬────────────────┬──────────┘
                     Effect (TrackTranscript, Untrack, ReadRollout, Ticker…) ◄───┘                │
                     к claude-io / codex-io                                    emit_to("island") + Arc<Mutex<Snapshot>>
                                                                                                   ▼
                                                              островок (TS): только отрисовка
```

**Принципы:**
- **Решения о состоянии принимает только `Store::apply`.** Это чистая функция без ввода-вывода, тестируется табличными тестами. Ввод-вывод — в потоках источников. Они присылают сигналы с уже вычисленными фактами и исполняют эффекты, которые возвращает модель.
- **Порядок.** Каждый поток источника отправляет связанные сигналы подряд в один FIFO-канал: сначала хвост, потом изменение файла или хук. Поэтому для одной сессии модель видит их в правильном порядке. `since` фазы никогда не уменьшается: берётся `max(prev, new)`.
- **В скрытом состоянии нет таймеров.** Работают наблюдатели папок (`ReadDirectoryChangesW`), ожидание выхода процессов (`WaitForSingleObject`) и хуки. Единственный периодический таймер — тикер хвоста rollout Codex раз в секунду. Он живёт, только пока островок **виден** и есть Codex-сессия в `Working`/`Waiting`; в этом состоянии и так работает опрос курсора ~60 Гц (`island.rs`).
- **Потоки — `std::thread`, а не tokio.** Блокирующие вызовы Win32 и чтение файлов не мешают tokio-рантайму Tauri. Канал без ограничения, поэтому отправка с главного потока (`set_collapsed`) никогда не блокируется.
- **Новых crate-ов нет.** К `windows` 0.61 добавляются features `Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_System_Diagnostics_ToolHelp`.

## 5. Компоненты и файлы

| Компонент | Файлы |
|---|---|
| Общие идентификаторы | NEW `windows/ident.rs`. Подключается через `#[path = "../../ident.rs"] mod ident;` в `hook/src/main.rs`, `hook/src/lib.rs` (с T3, для `win::pipe_path`) и `src-tauri/src/lib.rs`. Единственный источник имён: `platform/windows.rs` берёт из него `HOOK_EXE` и `APP_DIR`. |
| Платформенные помощники апстрима | CHANGED только `src-tauri/src/platform/windows.rs` (имена из `ident.rs`, комментарии); `platform/mod.rs` не меняется. Код Mayak вызывает существующие `platform::…` и не копирует их (§5.8). |
| Ретранслятор `mayak-hook.exe` | `windows/hook/src/main.rs` (+ `lib.rs` для тестируемого ядра), `hook/src/win.rs`, `hook/Cargo.toml` |
| Pipe-сервер и router | `windows/src-tauri/src/pipe.rs`, `lib.rs` |
| Модель | NEW `src-tauri/src/observe/model.rs` (чистая), `observe/text.rs` (тексты алертов) |
| Поток store и публикация | NEW `src-tauri/src/observe/mod.rs` |
| Источник Claude | NEW `observe/claude.rs` (поток `claude-io`, разбор файла сессии), `observe/claude_transcript.rs` (факты из записей), `observe/fixtures/claude/*` |
| Источник Codex | NEW `observe/codex.rs` (поток `codex-io`, разбор rollout), `observe/fixtures/codex/*` |
| Хвост JSONL, пути | NEW `observe/tail.rs`, `observe/paths.rs` |
| Примитивы Win32 | NEW `observe/win.rs` |
| Установщик конфигов | NEW `src-tauri/src/hookfile.rs`, NEW `src-tauri/src/codex_hooks.rs`, CHANGED `hooks.rs`, `settings.rs`, `lib.rs` |
| Фронтенд: хранилище | NEW `windows/src/core/sessions.ts`, NEW `src/island/observer.ts`, CHANGED `src/core/bridge.ts`, `src/main.ts` |
| Фронтенд: виды | NEW `src/views/sessions.ts`, CHANGED `views.ts`, `layout.ts`, `state.ts`, `island.ts`, `fsm.ts`, `hooks.ts`, `style.css` |
| Окно настроек | `src/settings/main.ts`, `settings.css`, `settings.html` |
| Деинсталлятор, CI | `src-tauri/nsis/hooks.nsh`, NEW `.github/workflows/windows-test.yml` — единственный тестовый CI; DELETED `.github/workflows/linux.yml` |
| Linux-порт апстрима | Не собирается и не тестируется. Побайтно без изменений остаются `src-tauri/src/platform/linux.rs`, `hook/src/unix.rs`, `src-tauri/tauri.linux.conf.json` и секции `[target.'cfg(target_os = "linux")'.dependencies]` в обоих `Cargo.toml`: так будущие слияния апстрима проходят без конфликтов modify/delete. Linux-ветки общих файлов уходят, когда Mayak их переписывает: `hook/src/main.rs`, `pipe.rs`, `log.rs`, `hooks.rs`. Из `windows/README.md` удаляется раздел `## Linux`. |

### 5.1 `windows/ident.rs`

```rust
pub const APP_SLUG: &str = "mayak";
pub const APP_NAME: &str = "Mayak";
pub const APP_DIR: &str = "Mayak";             // без пробелов: путь попадает в команду Codex без кавычек
pub const HOOK_EXE: &str = "mayak-hook.exe";
pub const PIPE_PREFIX: &str = "mayak-";        // \\.\pipe\mayak-<SID>
pub const MARKER: &str = "mayak-hook";
pub const LEGACY_MARKERS: &[&str] = &["coucou-hook"];   // только для статуса и удаления
pub const LEGACY_DIR: &str = "Coucou";                   // только для одноразового копирования settings.json
pub const BUNDLE_ID: &str = "io.github.mahach666.mayak";
```

### 5.2 Ретранслятор `mayak-hook.exe`

**argv:** `[--agent <name>] [<EventName>]` — грамматика ретранслятора апстрима; отличие одно: неизвестные `--флаги` пропускаются и событием не становятся (`--dump` только в debug).
- Флаг может стоять где угодно. Значение после `--agent` поглощается всегда и событием не становится. Первый аргумент не-флаг — событие.
- Одиночный `<EventName>` означает `claude`, как раньше.
- Непустой `payload.hook_event_name` главнее события из argv, как в Coucou.
- В debug-сборке (`#[cfg(debug_assertions)]`) есть флаг `--dump` для spike S2. Он дописывает строку в `%LOCALAPPDATA%\Mayak\dump.log`: длина stdin, SHA-256 stdin, мс от создания процесса, цепочка образов предков, имя shell-родителя. Содержимое stdin не пишется. В релизной сборке флага нет.

**Значение `--agent`:**
- `claude`, пустое значение или флаг без значения → Claude;
- `codex` → Codex;
- любое другое непустое значение — чужой агент, например `--agent cursor`. Ретранслятор дочитывает stdin, ничего не отправляет в pipe, ничего не печатает и выходит с `exit 0`.
- Поле `coucou_agent` апстрима ретранслятор не добавляет. Если оно пришло в payload, на агента и маршрут оно не влияет.

Чужое имя не должно становиться Claude: тогда его `PermissionRequest` ждал бы карточку до 110 с, а клик Allow напечатал бы JSON Claude (`hookSpecificOutput`) чужому инструменту. Апстрим такие запросы тоже не одобряет, а отклоняет (`hooks.ts:273-280`).

**Определение агента** (для `claude` и `codex`): `agent = if payload имеет turn_id && hook_event_name == "PermissionRequest" { codex } else { argv.agent }`.
- Это защита: Codex может импортировать хуки Claude (externalAgentConfig), и тогда импортированный старый Allow/Deny не должен ждать ответа или одобрять Codex.

**Для обоих агентов:**
- снимаются **все** ведущие `U+FEFF`;
- добавляются `agent`, `relay: 2`, `relay_ts` (мс Unix UTC);
- строки обрезаются до 2000 символов, как сейчас.

**Claude:** ждёт ответа только `PermissionRequest`, как раньше. `agent_pid` берётся из документированной `CLAUDE_PID`.
- Старый Allow/Deny получает только настоящие запросы разрешений Claude Code. Если у `PermissionRequest` `tool_name == "AskUserQuestion"` или нет непустого строкового `session_id`, ретранслятор ничего не отправляет и не печатает, `exit 0` (`legacy_approval_allowed(payload)`). Claude Code сам показывает свой запрос. Вопрос наблюдатель по-прежнему показывает как ожидание, detail берётся из транскрипта (§7).
- `session_id` — документированное общее поле входа хуков Claude Code, поэтому обычные запросы разрешений проходят как раньше.
- Причины. Карточка Allow/Deny не умеет отвечать на вопрос: Allow без `updatedInput` или Deny ломают ответ. Апстрим на такой `PermissionRequest` отвечает «ask» и карточку не показывает, чтобы вопрос не появлялся дважды (c767db9). Апстрим также заявляет (5332f9e), что собственный агент Cursor запускает хуки из `~/.claude/settings.json` с `conversation_id` вместо `session_id`; у нас это не проверено. Без отсева появилась бы карточка без сессии и проекта.
- Отсеивать нужно здесь, а не дальше: `route()` не видит `tool_name`, и любая пересланная строка claude + `PermissionRequest` становится карточкой Approval.

**Codex:**
- **никогда не ждёт и ничего не печатает в stdout**; всегда `exit 0`, бюджет 1 с, из них 300 мс на подключение;
- сохраняет `transcript_path`;
- обходит предков (Toolhelp32, до 6 шагов, у каждого шага время создания раньше, чем у потомка) до первого образа с именем `codex.exe` (сравнение без учёта регистра);
- записывает `agent_pid`, `agent_created` (FILETIME, u64) и `agent_image` (полный путь). Если предок не найден — `agent_pid: null`, событие всё равно отправляется.

`waits_for_answer = agent == claude && event == PermissionRequest`. Оба ранних выхода (чужой агент, отсев старого Allow/Deny) происходят до обращения к pipe.

Ядро вынесено в `hook/src/lib.rs`: `talk_to(pipe_name, payload, waits, budget)`, `legacy_approval_allowed(payload)`. Транспорт апстрима остаётся в `hook/src/win.rs`: `pipe_path()` становится `pub` и строит имя из `ident::PIPE_PREFIX`, `connect(path)` принимает имя pipe, `ERROR_PIPE_BUSY` остаётся там же; `talk_to` вызывает `win::connect`, поэтому у ретранслятора один `connect`. В `win.rs` также SID, проверка «сервер — тот же пользователь» и обход предков. `hook/src/unix.rs` не подключается. Тесты используют уникальные имена pipe и никогда не трогают настоящий `mayak-<SID>`.

### 5.3 Pipe-сервер и router

- `pipe::serve(name: &str, sink: impl RouterSink)`; `Pending` — обычная структура. Это шов для тестов: Tauri `AppHandle` подключается только в `lib.rs`.
- Чистая функция `route(agent, event) -> Route { Observe, Approval, Ignore }`:
  - `codex`, любое событие → `Observe` → `codex-io` получает `HookEvent`;
  - `claude` + `PermissionRequest` → `Approval`. Прежний поток Pending/ack (800 мс)/decision (108 с). К payload добавляются `session_key` и `project_name`; `session_key` строится через `model::claude_key(pid, created)` по `agent_pid` и `GetProcessTimes`. Команды `approval_ack`/`approval_decline`/`approval_decision` не меняются;
  - остальное → `Ignore`: для Claude источник истины — наблюдатель.
- Строка больше 1 MiB или невалидный JSON: в лог пишется длина и причина, строка **явно** отбрасывается.
- Лог: `hook <agent> <Event> s=<8 символов session>`. Содержимое payload не логируется.
- Имя pipe — `\\.\pipe\mayak-<SID>`, SID из `platform::current_user_sid()`. Linux-`start` и `trait Relay` апстрима из `pipe.rs` уходят.
- DACL только для текущего SID (тот же `platform::current_user_sid()`) — **отдельным коммитом после S4**.

### 5.4 Модель (`observe/model.rs`, чистая)

```rust
pub fn claude_key(pid: u32, created_ft: u64) -> SessionKey   // "claude:<pid>:<created_ft десятичный>"
pub fn codex_key(session_id: &str) -> SessionKey            // "codex:<session_id>"

pub struct ProcId { pub pid: u32, pub created_ft: u64, pub image: String }
pub enum Source { Claude, Codex }
pub enum HostKind { ClaudeDesktop, Cursor, VsCode, CodexDesktop, Unknown }

pub enum Signal {
    // claude-io отправляет ClaudeTranscript перед ClaudeFile для той же сессии
    // rebind_at = Some(o), когда транскрипт привязан заново: o = 0 для только что созданного файла,
    // o = EOF для уже существующего файла, найденного поиском
    ClaudeTranscript { key: SessionKey, rebind_at: Option<u64>, records: Vec<(u64, Value)>, eof: u64 },
    ClaudeFile { key: SessionKey, proc: ProcId, host: HostKind, project: ProjectRef,
                 file: ClaudeSessionFile, transcript_eof: Option<u64> },
    ClaudeGone { key: SessionKey },             // файл удалён или procStart сменился
    // codex-io отправляет CodexRollout перед CodexHook для той же сессии
    CodexRollout { key: SessionKey, path: String, records: Vec<(u64, Value)>, eof: u64 },
    CodexHook { key: SessionKey, event: HookEvent, proc: Option<ProcId>, host: HostKind,
                project: ProjectRef, rollout_eof: Option<u64> },
    CodexHooksInstalled(bool),
    ProcExit { pid: u32, created_ft: u64 },
    Ack(SessionKey),
    Visible(bool),
    AlertPrefs { waiting: bool, finished: bool, error: bool },
    SourceHealth { source: Source, health: Health },   // Ok | Missing | Error(String) | Degraded{unreadable: u32}
}

pub enum Effect {
    TrackTranscript { key: SessionKey, session_id: String },   // claude-io: найти/ждать <sessionId>.jsonl
    UntrackTranscript { key: SessionKey },
    UntrackRollout { key: SessionKey },                         // codex-io сам начинает хвост по transcript_path хука
    CodexTicker(bool),                                          // codex-io: тикер 1 Гц вкл/выкл
}

pub struct Applied { pub alerts: Vec<Alert>, pub effects: Vec<Effect> }
impl Store {
    pub fn apply(&mut self, s: Signal, now_ms: i64) -> Applied;
    pub fn snapshot(&self, now_ms: i64) -> SessionsSnapshot;   // rev += 1 при каждом изменении
}
```

**`ProjectRef { key, root }`** вычисляет поток источника через `paths::project_of`. Отображаемое имя считается в `snapshot()` по текущему набору проектов (§6).

**`ClaudeSessionFile`** (serde, все поля `Option`, неизвестные игнорируются):
- `pid: u32`, `sessionId: String`, `cwd: String`;
- `procStart: String` (десятичный u64), `version`, `kind`, `entrypoint`, `name`, `nameSource`;
- `status: String`, `waitingFor: String`, `statusUpdatedAt: i64` (мс), `updatedAt: i64`;
- `spare: bool`, `parkedJobId: String`, `pidDomain: String`.

**`HookEvent`:**
- `hook_event_name`, `session_id`, `turn_id: Option`, `cwd`, `transcript_path: Option`;
- `tool_name: Option`, `tool_input: Option<Value>`, `last_assistant_message: Option`;
- `agent_id: Option`, `agent_type: Option`;
- `relay_ts: i64`, `agent_pid: Option<u32>`, `agent_created: Option<u64>`, `agent_image: Option<String>`.

### 5.5 Поток `claude-io` (`observe/claude.rs`)

**Пути.** `$CLAUDE_CONFIG_DIR`, если задан в окружении приложения, иначе `%USERPROFILE%\.claude`. Базовый путь передаётся явно, окружение читается только в `lib.rs`.

**Наблюдатели:**
- `DirWatcher(sessions\)` — без рекурсии, маска `FILE_NAME | LAST_WRITE | SIZE`;
- `DirWatcher(projects\)` — рекурсивно, `FILE_NAME | LAST_WRITE | SIZE`.

Оба только пересылают уведомления в `claude-io`.

**Файл сессии** (только имена `^[0-9]+\.json$`):
1. Прочитать и разобрать.
   - Пустое или усечённое чтение и ошибка JSON у уже известного файла **не** считаются удалением. Последнее состояние сохраняется до следующего уведомления, в здоровье источника увеличивается `degraded`.
   - У нового файла ошибка разбора означает пропуск.
2. Пропустить, если `spare == true`, или `parkedJobId` непуст, или `pidDomain` есть и не начинается с `win32:`, или нет `procStart`. В последнем случае `Health::Degraded`.
3. **Доказательство процесса:**
   - `identify(pid)` → `OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION)`;
   - `WaitForSingleObject(h, 0) == WAIT_TIMEOUT`, то есть процесс жив, а не зомби с удержанным handle;
   - `GetProcessTimes` FILETIME `== procStart`;
   - `QueryFullProcessImageNameW`.
   - Отказ в доступе даёт `Degraded{unreadable += 1}`.
   - Несовпадение `procStart` с известной сессией этого pid → `ClaudeGone`.
4. Зарегистрировать ожидание выхода в `ProcRegistry`. Ключ `(pid, created_ft)`, не больше одного ожидающего на процесс, ожидающий поток забирает **тот же** handle. Выход процесса → `ProcExit`.
5. **Хост** — сравнение нормализованного пути в нижнем регистре (`paths::normalize().key`):
   - `…\packages\claude_*\localcache\roaming\claude\claude-code\*\**\claude.exe` или `%appdata%\claude\claude-code\*\**\claude.exe` → `ClaudeDesktop`, при `entrypoint == "claude-desktop"`. Папка версии обязательна, за ней допустимо любое число папок, в том числе ни одной; `claude-code\claude.exe` без папки версии не подходит;
   - `…\.cursor\extensions\anthropic.claude-code-*\…\claude.exe` → `Cursor`;
   - `…\.vscode\extensions\anthropic.claude-code-*\…\claude.exe` → `VsCode`;
   - иначе `Unknown`.
6. **Если сменился `status` или `statusUpdatedAt`:** сначала дочитать хвост транскрипта и отправить `ClaudeTranscript`, затем `ClaudeFile{transcript_eof}`. Иначе — только `ClaudeFile`.
7. **Удаление файла** → `ClaudeGone`.

**Транскрипт:**
- По `Effect::TrackTranscript{key, session_id}` ищется `projects\*\<sessionId>.jsonl`: правило slug не документировано, UUID уникален.
- Если файл найден, хвост начинается с окна (см. ниже), отправляется `ClaudeTranscript{rebind_at: Some(eof)}`.
- Если файла нет, имя `<sessionId>.jsonl` попадает в **набор ожидаемых имён**. Уведомления `projects\` фильтруются **по имени файла**, а не по полному пути. При `ADDED`/`MODIFIED` с ожидаемым именем файл привязывается, хвост начинается с 0, отправляется `ClaudeTranscript{rebind_at: Some(0)}`.
- Повторный поиск идёт при каждом `ClaudeFile` сессии без привязанного транскрипта.
- **Первое чтение уже существующего файла** — последние 256 KiB. Байты до первого `\n` отбрасываются.
- **Основная цепочка** — записи с `isSidechain != true` и `type ∈ {user, assistant}`. Записи заголовка (`custom-title`, `agent-name`, `ai-title`) учитываются, если попали в прочитанное. Иначе заголовок берётся из `name` файла сессии.

**Overflow любого наблюдателя:**
- перечислить `sessions\` заново;
- повторить поиск для всех ожидаемых имён;
- перечитать все хвосты.

### 5.6 Поток `codex-io` (`observe/codex.rs`)

**Вход:** `HookEvent` от router.

**Доказательство процесса:**
- `agent_pid`/`agent_created` → `identify` + сверка времени создания + `WaitForSingleObject(h,0)`;
- ожидание выхода регистрируется в общем `ProcRegistry`;
- если не удалось → `proc: None`.

**Хост** по `agent_image` (нормализованный путь в нижнем регистре):
- `…\.cursor\extensions\openai.chatgpt-*\…` → `Cursor`;
- `…\.vscode\extensions\openai.chatgpt-*\…` → `VsCode`;
- `…\windowsapps\openai.codex_*\…` или `%localappdata%\openai\codex\bin\*\codex.exe` → `CodexDesktop`;
- иначе `Unknown`.

**Rollout:**
- Состояние хвоста хранится **по пути**: `rollouts: Map<path, TailState>`.
- Главный rollout сессии — `transcript_path` из событий **без** `agent_id`. Файлы `.zst` и `null` не читаются.
- При первом знакомстве с путём хвост начинается **с EOF**: история не воспроизводится.

**Порядок на хуке:** дочитать хвост главного rollout → `CodexRollout` → `CodexHook{rollout_eof}`.

**Тикер.** По `Effect::CodexTicker(true)` хвосты отслеживаемых rollout читаются раз в 1 с, по `false` тикер останавливается. Модель включает его только при `Visible && есть Codex-сессия в Working|Waiting`.

**Записи rollout** (режим **legacy**, наблюдался локально):
- `Meta{id, cwd, parent_thread_id?, agent_role?}`;
- `TaskStarted{turn_id?}`;
- `TaskComplete{turn_id?, last_agent_message?, error?}`;
- `TurnAborted{turn_id?}`;
- `Call{call_id, label}` для `function_call` / `custom_tool_call` / `local_shell_call`;
- `PermissionAsk{call_id, label}` — вызов, который просит разрешения у пользователя (§7): `function_call` или `custom_tool_call` с именем `request_permissions`, либо `custom_tool_call` `exec`, чей `input` (строка) содержит `request_permissions`. Текст `arguments` у `function_call` и `input` у других инструментов не учитывается, сам текст не хранится;
- `CallDone{call_id}` — парный ответ на вызов: `function_call_output` / `custom_tool_call_output` со строковым `call_id`.

`session_meta` с `parent_thread_id` или `agent_role` — это субагент, сессией он не становится. Режим **paginated** (Codex 0.160 и Desktop, спайк S3, 2026-10-04: к legacy-записям добавляется `ordinal`, число или null) читается так же, как legacy; запись с другим `ordinal` неизвестна. Если записи неизвестны, они пропускаются, а здоровье становится `Degraded`. Начиная с Codex 0.160 записи `token_usage_record` и `inter_agent_communication_metadata` считаются известными и игнорируются, `Degraded` они не дают (спайк S3, 2026-10-04, решение пользователя).

**Заголовок** берётся из `~/.codex/session_index.jsonl` (`id` → `thread_name`). Файл читается при создании сессии.

### 5.7 Хвост JSONL и пути

`JsonlTail` (`tail.rs`):
- `new(path)`, `start_at_eof()`, `start_with_window(bytes)`, `read_new() -> io::Result<(Vec<(u64, String)>, u64 /*eof*/)>`;
- хранит смещение и неполную последнюю строку;
- если файл уменьшился, начинает с 0;
- каждое чтение — open/read/close с `FILE_SHARE_READ | WRITE | DELETE`, чтобы агенту не мешать.

`paths::normalize(raw) -> NormPath { display, key }` (`paths.rs`):
- trim;
- снять `file:///`;
- `/c:/` → `c:/`;
- разделители → `\`;
- снять `\\?\`;
- буква диска заглавная;
- схлопнуть повторы;
- лексически разрешить `.` и `..`;
- снять завершающий разделитель, кроме корня диска;
- `key = display.to_lowercase()`.

`paths::project_of(raw, probe: impl Fn(&Path) -> bool) -> ProjectRef { key, root }`:
- первый предок, включая сам путь, где `probe(dir\.git)` (папка или файл);
- без `.git` — сама папка;
- кеш на время жизни приложения в потоке источника;
- `probe` подставляется, поэтому тесты чистые.

### 5.8 Примитивы Win32 (`observe/win.rs`)

**То, что уже есть в `platform/` апстрима, вызывается оттуда, а не копируется:**
- `platform::current_user_sid` — имя pipe и DACL (§5.3);
- `find_on_path`, `no_console` — `session_open` (§8);
- `local_time`, `ensure_private_dir` — `log.rs` и метка времени бэкапа в `hookfile.rs`;
- `config_dir`/`local_dir` — через реэкспорт в `settings::`;
- `HOME_VAR`/`home_dir` — домашняя папка, когда `lib.rs` собирает базовые пути (§5.5).

**Собственный Win32-код Mayak** живёт в модулях Mayak, без Linux-двойников: `observe/win.rs` (ниже), `hook/src/win.rs` (транспорт апстрима `pipe_path`/`connect`, SID, предки), `codex_hooks.rs` (`GetShortPathNameW`), `pipe.rs` (DACL). `platform/mod.rs` не меняется.

- **`DirWatcher::spawn(dir, recursive, mask, tx)`.** Буфер 64 KiB. Отдаёт `Changes(Vec<(action, rel_path)>)`, `Overflow` или `Error(io::Error)`.
  - На `Error` поток завершается, источник шлёт `SourceHealth{Error}`.
  - Нет каталога при старте → `Missing`.
  - Восстановление — на `Visible(true)` (повторная попытка), без опроса.
- **`proc::identify(pid) -> Result<(ProcId, OwnedHandle), ProcError>`**: живость, время создания, образ (§5.5 п.3).
- **`proc::Registry::watch(ProcId, OwnedHandle, tx)`**: не больше одного потока на `(pid, created_ft)`, `WaitForSingleObject(INFINITE)` на переданном handle → `ProcExit`.
- **`proc::ancestors(pid, max)`** — только в ретрансляторе (`hook/src/win.rs`). Таблица процессов подставляется, поэтому функцию можно тестировать.

## 6. Модель данных

**Ключи:**
- Claude — `claude:<pid>:<procStart>`, это процесс. Ключ переживает `/clear`, `fork`, `resume`.
- Codex — `codex:<session_id>`. Субагенты сливаются с родителем.
- Проект — `ProjectRef.key`.

```text
Session {
  key, agent: Claude|Codex, host: HostKind, proc: Option<ProcId>,
  logical_id,                // Claude: sessionId; Codex: session_id
  cwd, project: ProjectRef, title: Option<String>,
  phase: Phase, since_ms: i64,
  activity: Option<String>, detail: Option<String>,
  outcome: Outcome, acked_at: Option<i64>,
  // Claude
  file_status_ts: i64, turn_start_offset: u64, idle_eof: u64, transcript_bound: bool,
  pending_tools: Vec<(String /*tool_use_id*/, String /*label*/)>,
  // Codex
  current_turn: Option<String>, turn_started_ts: i64, closed_turns: VecDeque<String> /*≤8*/,
  wait_key: Option<(String /*turn*/, String /*tool*/, u64 /*hash(tool_input)*/)>,
}
Phase   = Working | Waiting{ kind: Permission | Input | Other(String) } | Idle | Unknown(String)
Outcome = None | Pending | Finished{ summary, at_ms } | Error{ code: Option<String>, status: Option<u16>, text, at_ms } | Interrupted{ at_ms }
Project = { key, root, sessions: Vec<SessionKey> }
```

### Эпизоды, «просмотрено», живые переходы

- **Эпизод:** у `Waiting` начало — `since_ms`; у `Finished`/`Error` — `outcome.at_ms`.
- **Строка просмотрена**, если `acked_at >= episode_start`. «OK» и клик ставят `acked_at = now` и **никогда не меняют фазу**.
- **Новый эпизод ожидания** = новый `since_ms` (§7). Поэтому клик в `Working` не прячет будущий `Finished`: его `at_ms` больше.
- **Живой переход** — переход, применённый к сессии, которая уже была в модели до этого сигнала в текущем запуске. Алерты бывают только у живых переходов. Первое появление сессии алерта не даёт. Поздно найденный `TaskComplete` у известной сессии — живой переход, он даёт алерт.

### Внимание (вычисляется, не хранится; только непросмотренные эпизоды)

| Ранг | Условие | Порядок внутри |
|---|---|---|
| 0 | `Waiting(Permission)` | давние первыми |
| 1 | `Waiting(Input | Other)` | давние первыми |
| 2 | `Idle` + `Error` | новые первыми |
| 3 | `Idle` + `Finished` | новые первыми |
| 4 | `Working` | дольше работающие первыми |
| 5 | остальное | по `since` убыв. |

- Ничьи решаются по ключу.
- Проекты сортируются по лучшему рангу, затем по самой свежей активности.
- `counts`, `hold` и `aggregate` (§10) считаются **только** по непросмотренным эпизодам.
- **Отображаемое имя проекта** вычисляется в `snapshot()`: basename корня; при совпадении basename у разных ключей — `родитель\имя`.

## 7. Машины состояний

### Claude

Источник фазы — `ClaudeFile.file.status`, источник `since_ms` — `statusUpdatedAt`, не меньше предыдущего. `transcript_eof` присылается вместе с файлом; если транскрипт ещё не привязан (`None`), смещения не меняются.

Два смещения в транскрипте:
- **`idle_eof`** — конец транскрипта в момент последнего перехода в Idle, то есть граница между ходами;
- **`turn_start_offset`** — начало текущего хода: записи после него участвуют в вычислении итога.

Отсчёт от `idle_eof`, а не от EOF в момент `busy`, нужен, чтобы не потерять записи короткого хода, которые легли в транскрипт до того, как мы обработали `busy`.

| Из \ в | `busy` | `waiting` | `idle` | неизвестный `status` |
|---|---|---|---|---|
| **(нет сессии)** | Working | Waiting(kind) | Idle, outcome None | Unknown |
| **Working** | — (обновить activity) | Waiting(kind), алерт | Idle, Pending → вычислить; `idle_eof = transcript_eof` | Unknown |
| **Waiting** | Working | новый `statusUpdatedAt` или `waitingFor` → новый эпизод, алерт; иначе обновить detail | Idle, Pending → вычислить; `idle_eof = transcript_eof` | Unknown |
| **Idle / Unknown** | Working; `turn_start_offset = idle_eof`; outcome None | Waiting(kind), алерт; `turn_start_offset = idle_eof`; outcome None | новый `statusUpdatedAt` → свёрнутый ход: `turn_start_offset = idle_eof`, Pending → вычислить, затем `idle_eof = transcript_eof`; иначе — | Unknown |

- **Первое появление** (первая строка таблицы): `idle_eof = turn_start_offset = transcript_eof` (или 0, если транскрипт не привязан), алертов нет.
- **Привязка транскрипта** (`ClaudeTranscript.rebind_at = Some(o)`): `idle_eof = turn_start_offset = o`. Для нового файла `o = 0`, поэтому первый ход новой вкладки учитывается целиком.
- **`kind`:**
  - `"permission prompt"` → Permission;
  - `"input needed"` → Input;
  - иначе Other(`waitingFor`).
- **`detail`** — метка последнего неразрешённого `tool_use` в основной цепочке. Для `AskUserQuestion` — `input.questions[0].question`. Пересчитывается при **каждом** `ClaudeTranscript`. Если записи `tool_use` ещё нет, алерт уходит без detail, а карточка подхватит detail из следующего снимка.
**Вычисление итога.** Выполняется сразу и при каждом `ClaudeTranscript`, пока outcome = Pending. Смотрятся записи основной цепочки после `turn_start_offset`:
1. Последняя запись user — маркер `[Request interrupted by user` → `Interrupted`, без алерта.
2. Иначе последняя запись assistant с `isApiErrorMessage` → `Error{code: error, status: apiErrorStatus, text}`, алерт error.
3. Иначе последняя запись assistant с `stop_reason ∉ {null, "tool_use"}` (`end_turn`, `stop_sequence`, `max_tokens`, `refusal`, будущие значения), и после неё нет `tool_use`/`tool_result` → `Finished{первые 160 символов текста, пробелы схлопнуты}`, алерт finished.
4. Иначе остаётся `Pending`, без алерта и без таймера. Например, slash-команда без ответа модели.

Не больше одного алерта итога на ход. Выход из Idle сбрасывает Pending.

**Удаление сессии:** `ClaudeGone` или `ProcExit` её процесса → удалить, без алерта, `Effect::UntrackTranscript`. Новая сессия → `Effect::TrackTranscript`. Смена `sessionId` → `Untrack` + `Track`; смещения берутся из следующего `ClaudeTranscript{rebind_at}`.

### Codex

Источник `since_ms` — `relay_ts`, не меньше предыдущего.

**Создание.** Хук с неизвестным ключом создаёт сессию: `cwd`, проект, хост, `proc`, `title`, `current_turn = turn_id`. Затем **сам хук применяется как живой переход**: даёт фазу и алерт. История не воспроизводится: codex-io начинает хвост rollout с EOF.

**Начало хода** `start_turn(T, ts)`:
- `current_turn = T`;
- `turn_started_ts = ts`;
- outcome None;
- `wait_key = None`.

| Сигнал | Правило |
|---|---|
| `UserPromptSubmit(T)` | `start_turn(T)`, Working. Без алерта. |
| rollout `TaskStarted(T)`, `T ≠ current_turn` | `start_turn(T)`, Working. Без алерта. Нужен для набора хуков без `UserPromptSubmit`. |
| `PermissionRequest(T, tool, input)` | Если `T ≠ current_turn` → `start_turn(T)`. `k = (T, tool, hash(canonical_json(input)))`. Если фаза Waiting и `wait_key == k` → повтор (`:retry`): ничего не менять, без алерта. Иначе → Waiting(Permission), `since = relay_ts` (новый эпизод), `detail = "tool · target"`, `wait_key = k`, алерт permission. От субагента (`agent_id`) — то же, в подписи `agent_type`. |
| `Stop(T)` | Игнорировать, если `T ∈ closed_turns` или (`T ≠ current_turn` и `relay_ts < turn_started_ts`). Иначе: если `T ≠ current_turn` → `current_turn = T`; Idle, `Finished(last_assistant_message)`, `closed_turns += T`, алерт finished. |
| rollout `TaskComplete(T)` | Только если `T == current_turn` или `current_turn` — None. Если `T ∉ closed_turns` → Idle, `Finished(last_agent_message)` либо `Error(error)`, если поле есть; `closed_turns += T`; алерт. Если `T ∈ closed_turns`, outcome Finished и у записи есть `error` → поднять до Error, алерт error. |
| `Interrupt(T)` / rollout `TurnAborted(T?)` | `T` или (если нет) `current_turn` равно `current_turn` → Idle, Interrupted, без алерта. |
| rollout `Call` | `activity = label`. |
| `SessionEnd` | Удалить сессию, `Effect::UntrackRollout`. |
| `ProcExit` подтверждённого процесса | Удалить все его сессии. |
| `Ack` на Idle-сессии без `proc` | Удалить до следующего хука. |

**Снятие ожидания Codex — только детерминированные события:**
- следующий хук этой сессии с другим ключом (`UserPromptSubmit`, `Stop`, другой `PermissionRequest`, `Interrupt`, `SessionEnd`);
- rollout `TaskComplete`/`TurnAborted` текущего хода;
- «OK» (только скрывает, §6).

По активности хвоста ожидание **не снимается**: порядок записи rollout не гарантирован. Цена этого объявлена в §15.

**Диалог «Разрешения» без хука.** Для запроса `request_permissions` (диалог «Разрешения» Codex Desktop) Codex 0.160 не вызывает `PermissionRequest` и ничего не пишет в rollout, пока диалог открыт. Зато в rollout есть вызов, который его запрашивает: `function_call` или `custom_tool_call` с именем `request_permissions` либо, в тредах Codex Desktop без проекта (режим code), `custom_tool_call` `exec`, чей `input`, если это строка, содержит `request_permissions`. Текст `arguments` у `function_call` и `input` у других инструментов не читается: shell-команда или патч, которые лишь упоминают это слово, остаются обычным вызовом. Вызов без ответа с тем же `call_id` (`function_call_output` / `custom_tool_call_output`) переводит сессию из Working в Waiting(Permission): `detail = "request_permissions"`, `wait_key = (T, "request_permissions", hash(call_id))`, `since` — часы хранилища, алерт permission, как у хука. Из Idle и из ожидания, которое поставил хук, такая запись меняет только `activity`. В уже выведенное ожидание она добавляет свой `call_id` без второго алерта: ведётся множество из не более 8 `call_id`, самый старый забывается. Если ответ с тем же `call_id` пришёл в той же пачке записей, ожидания нет. Алерт поднимается по концу пачки и только если сессия всё ещё в этом ожидании: пачка, где после вызова идёт `TaskComplete`, `TurnAborted` или начало нового хода, алерта permission не даёт. Каждый парный ответ убирает свой `call_id`; Working (`detail = None`) возвращается, когда не осталось ни одного, и только из этого ожидания: ожидание от хука ответ не снимает. `Stop`, `Interrupt`, `TaskComplete`, `TurnAborted` и начало нового хода завершают такое ожидание, как любое другое, и забывают все `call_id`. Это единственное исключение из правила выше: вывод опирается на точное сопоставление `call_id`, а не на тайминг записей. Единственная догадка — что `exec`, чей `input` упоминает `request_permissions`, запрашивает разрешения (граница в §15). Как и любое чтение хвоста rollout, вывод работает только при видимом островке (тикер, §4): хук для этого диалога не вызывается, поэтому пока островок скрыт, вызов никто не читает. Компактный островок сам скрывается через 60 с, а Working его не удерживает (удерживает только Waiting). Ход, который дошёл до `request_permissions` позже этого срока, остаётся Working, пока островок не станет видимым; карточка, звук и счётчик появятся по первому чтению тикера (~1 с). Тикер при скрытом островке не включается: это нарушило бы §4.

Ход с ошибкой не вызывает `Stop`. Ошибка видна, только когда тикер или хук прочитает `TaskComplete`/ошибку из rollout, то есть при видимом островке или на следующем хуке. До этого строка остаётся Working (§15).

## 8. Конфиги

### По умолчанию в конфиги инструментов ничего не пишется

**Пишутся только собственные файлы:**
- `%APPDATA%\Mayak\settings.json`;
- `%LOCALAPPDATA%\Mayak\bin\mayak-hook.exe` (копируется при запуске);
- `%LOCALAPPDATA%\Mayak\mayak.log` — только ключи, фазы, счётчики, без текста;
- `%LOCALAPPDATA%\Mayak\inbox`;
- в debug-сборке — `dump.log`.

**Читаются:**
- `~/.claude/sessions/[0-9]+.json`;
- `~/.claude/projects/**/<sessionId>.jsonl`;
- rollout-пути из хуков Codex;
- `~/.codex/session_index.jsonl`;
- `~/.codex/hooks.json` (только для статуса установщика Codex).

**В подпроекте 1 не открываются:**
- `*.key`, `ide/*.lock`, `claude_desktop_config.json`, `~/.codex/auth.json`, `~/.codex/config.toml`;
- `~/.claude/settings.json` — только при раскрытии блока «Advanced», для статуса старого пути, и при его установке. **`boot()` больше не вызывает `hooks::status()`.**

`CLAUDE_CONFIG_DIR` и `CODEX_HOME` учитываются.

**Одноразово копируется** `%APPDATA%\Coucou\settings.json` → `%APPDATA%\Mayak\settings.json`, если второго ещё нет. Оригинал не трогается.

### Интеграция Codex (по желанию)

Настройки → Codex → Status → Preview → Install.

**Цель** — `~/.codex/hooks.json`. Сейчас файла нет, и dotfiles им не управляют.

**Запись** — одна группа на событие, добавляется **в конец** массива события:
```json
{ "hooks": [ { "type": "command",
               "command": "C:\\Users\\HP\\AppData\\Local\\Mayak\\bin\\mayak-hook.exe --agent codex",
               "timeout": 10 } ] }
```

**События по умолчанию:**
- `UserPromptSubmit`, `PermissionRequest`, `Stop` — `timeout: 10`;
- `Interrupt`, `SessionEnd` — `timeout: 2`.

Если S2 покажет мигание консоли или больше ~0.5 с на хук, по умолчанию останутся `PermissionRequest` + `Stop`. Модель поддерживает оба набора (§7, правило `TaskStarted`).

Итог S2 (2026-10-04, Codex 0.160): хуки запускаются через shell сессии — в VS Code это Windows PowerShell (~0.4 с на хук), в Codex Desktop встроенный PowerShell 7 `pwsh` (~0.5–0.58 с в тёплом состоянии, ~1.5 с при первом холодном запуске), окно консоли не мигает. Несмотря на строку ~0.5 с, пользователь решил оставить все пять хуков синхронными. `"async": true` учитывается только с Codex 0.148 (0.142–0.147 такие хуки пропускают), а Codex в Cursor — 0.146, поэтому `async` не используется, пока на всех поверхностях Codex не будет ≥ 0.148.

**Команда:**
- абсолютный путь без кавычек;
- **проверка по списку разрешённого** после `GetShortPathNameW`: `^[A-Za-z]:(\\[\p{L}\p{N}._~-]+)+$`, иначе отказ установки с объяснением;
- одинаково разбирается в `powershell -Command` и в `cmd /C`.

**Содержимое записи:**
- нет `matcher`, `statusMessage`, `commandWindows`, `async`, поэтому хеш доверия минимален;
- команда не меняется между версиями Mayak;
- если в существующем файле на верхнем уровне есть ключи кроме `description` и `hooks`, или JSON невалиден — отказ.

**Слияние** (доверие позиционное):
- **установка** — в конец; чужие группы не двигаются и не переписываются;
- **переустановка** — замена своей группы на месте (по `MARKER`); если запись идентична, файл не пишется;
- **удаление** — своя группа стирается, если она последняя, иначе заменяется заглушкой `{"hooks":[]}`.

### Защита записи (`hookfile.rs`, общая для обеих целей)

- **Дифф** строится между сырыми текущими байтами и **точными** байтами, которые будут записаны. Если файла нет, показывается целиком.
- **`hooks_preview`** запоминает в состоянии Rust `PendingPlan { target, fingerprint, bytes, backup_path }`. Путь бэкапа — `<config>.mayak-backup-YYYYMMDD-HHMMSS`, рядом с конфигом.
- **`hooks_apply(target?, install, fingerprint)`** берёт сохранённый план по `target` + `fingerprint`. **Путь от webview не принимается.** Перед записью fingerprint перепроверяется по текущим байтам, затем бэкап, временный файл и rename.
- **Формат** сохраняется: BOM, CRLF/LF, финальный перевод строки.

### Trust

После установки окно настроек объясняет: Codex → «hooks need review» → Trust → «Reload hooks» или новый тред. Или `/hooks` в Codex (в приложении Codex: Settings → Hooks). Mayak **никогда** не пишет `[hooks.state]` и `trusted_hash`.

Статус в `snapshot.sources.codexHooks` (только в памяти, без записи в настройки):
- `not_installed`;
- `installed_unseen`;
- `receiving{hosts, since}` с первого события в текущем запуске.

Модель узнаёт об установке через `Signal::CodexHooksInstalled`. Сигнал отправляется при старте (по чтению `hooks.json`) и после каждого `hooks_apply`.

### Старый Allow/Deny для Claude (Advanced, по умолчанию выключен)

- Прежний поток для `~/.claude/settings.json` на общем `hookfile.rs`, ставит **только** `PermissionRequest`.
- Пересылает только настоящие запросы разрешений Claude Code: `PermissionRequest` от `AskUserQuestion` и без непустого строкового `session_id` ретранслятор отсеивает (§5.2), и Claude Code показывает свой запрос сам.
- Распознаёт `MARKER` и `LEGACY_MARKERS`.
- Предупреждает: `sync.ps1 pull` сотрёт запись, `push` закоммитит машинный путь в dotfiles.

### Команды Tauri

| Команда | Сигнатура | Совместимость |
|---|---|---|
| `hooks_status` | `(target?: "claude"\|"codex")` → `{target, installed, outdated, settingsPath, hookPath, hookReady, notes[]}` | `settingsPath` сохраняется: это путь конфига цели. `target` по умолчанию — `"claude"`. |
| `hooks_preview` | `(target?, install)` → `{diff, backup, settingsPath, fingerprint, notes[]}` | как сейчас + `target`, `notes` |
| `hooks_apply` | `(target?, install, fingerprint)` → `Result<String, String>` | как сейчас + `target` |
| `sessions_snapshot` | `()` → `SessionsSnapshot` | новая |
| `session_ack` | `(key)` | новая |
| `session_open` | `(key)` → `bool` | новая. `cursor`/`code` через `platform::find_on_path` и `platform::no_console`, без shell, иначе Проводник. Корень передаётся, только если это абсолютный путь к существующему каталогу (правило `open_in_vscode` апстрима, bef6558, `lib.rs:144-152`); иначе `false` и ничего не запускается. |
| `set_collapsed` | `(collapsed)` | тело апстрима с `island::refresh_click_through` (`lib.rs:92-98`) сохраняется; добавляется только `Signal::Visible(!collapsed)` |

`Settings.hooks_installed` удаляется из Rust и TS. Старые `settings.json` с этим ключом читаются: неизвестные поля игнорируются.

Новые поля `alertWaiting`, `alertFinished`, `alertError` — `#[serde(default = "default_true")]` и добавляются в TS-интерфейс `Settings`. После `save_settings` Rust отправляет `Signal::AlertPrefs`.

### Удаление

- Настройки → Remove — дифф для каждой цели.
- NSIS удаляет только `%LOCALAPPDATA%\Mayak\{bin,inbox,mayak.log}`. Конфиги инструментов не трогает. Перед удалением подсказывает снять интеграцию Codex.

## 9. Публикация снимка

Поток store после каждой пачки сигналов:
1. обновляет `Arc<Mutex<SessionsSnapshot>>` — его читает `sessions_snapshot()`;
2. отправляет `emit_to("island", "session-alert", Alert[])`, если есть алерты;
3. отправляет `emit_to("island", "sessions", snapshot)`:
   - если островок **виден** — на каждое изменение;
   - если **скрыт** — только при изменении `counts`, `attention`, `hold`, `aggregate` или при алертах.

На `Visible(true)` всегда отправляется полный снимок.

Окно настроек запрашивает `sessions_snapshot()` при показе. TS отбрасывает снимки с `rev`, не большим последнего полученного.

## 10. Островок и уведомления

### Данные для фронтенда (serde: `rename_all = "camelCase"`, enum с `tag = "kind"`)

```ts
type Phase = { kind: "working" } | { kind: "waiting"; wait: "permission" | "input" | "other"; label: string }
           | { kind: "idle" } | { kind: "unknown"; label: string };
type OutcomeView = null | { kind: "finished" | "error" | "interrupted"; summary: string; at: number };
// Pending и None → null; error.summary = "<code>: <text>" или "<text>"
type Host = "claudeDesktop" | "cursor" | "vsCode" | "codexDesktop" | "unknown";
type AttentionKind = null | "permission" | "input" | "error" | "finished";
type BotState = "approval" | "question" | "error" | "working" | "finished" | "idle";

interface SessionView { key: string; agent: "claude" | "codex"; host: Host; title: string | null;
  phase: Phase; since: number; activity: string | null; detail: string | null;
  outcome: OutcomeView; attention: AttentionKind; acked: boolean; }
interface ProjectView { key: string; name: string; root: string; sessions: SessionView[]; }
interface SessionsSnapshot { rev: number; now: number; projects: ProjectView[]; attention: string[];
  counts: { waiting: number; error: number; finished: number; working: number };
  hold: boolean; aggregate: BotState;
  sources: { claude: Health; codex: Health;
             codexHooks: "not_installed" | "installed_unseen" | { receiving: { hosts: Host[]; since: number } } }; }
type Health = "ok" | "missing" | { error: string } | { degraded: { unreadable: number } };
interface Alert { key: string; kind: "permission" | "input" | "error" | "finished"; rank: number;
  agent: "claude" | "codex"; host: Host; project: string; title: string | null;
  detail: string | null; text: string; since: number; }
```

**Значения, которые вычисляет Rust:**
- **`Alert.text`** собирается в `observe/text.rs` и покрыт тестами. Шаблоны: `"<Claude|Codex> · <project> (<host>) needs permission: <detail>"` / `"… asks: <question>"` / `"… finished: <summary>"` / `"… error: <code>: <text>"`.
- **`aggregate`** — первый подходящий вариант в порядке approval → question → error → working → finished → idle, по непросмотренным эпизодам.
- **`hold`** = есть непросмотренный Waiting **и** `alertWaiting` включён.

### Виды (`IslandViewName`)

- Новые: `"sessions"`, `"waiting"`, `"session"` (карточка деталей любой фазы).
- Существующие `"finished"` и `"error"` привязаны к `State.alertKey`.

### Компактный вид (288×32)

- Персонаж показывает `snapshot.aggregate`.
- Если есть сессии, справа вместо мини-сетки ботов — счётчики: `!n` янтарный (waiting), `✕n` красный, `✓n` зелёный, `•n` синий.
- Без сессий — как сейчас.
- Клик открывает `"sessions"`.

### Вид `"sessions"`

Вкладка в шапке, открывается по умолчанию при наличии сессий. Обзор интеграций — соседняя вкладка без изменений. Панель 720×320, ширина 640, геометрия в Rust не меняется.

- **Высота:** `sessionsHeight(rows) = min(300, 72 + 24 * rows)`, по образцу `chatPromptHeight`. Дальше — прокрутка колесом.
- **Заголовок проекта:** цветная точка (`colorForProject`), имя, метки хостов (Desktop / Cursor / VS Code / Codex app), пустой правый слот для подпроекта 2.
- **Строка** (24 px): глиф агента (`C`/`X`), хост, фаза, деталь с многоточием, прошедшее время.
  - Время перерисовывается раз в секунду, только пока вид открыт.
  - Просмотренные `Idle` тусклые и стоят последними. Если их в проекте больше 3, они сворачиваются в «+N idle».
- **Клик по строке:** `session_ack(key)`. Открывается вид по фазе: `waiting` для Waiting, `finished`/`error` для непросмотренного итога, иначе `session`.
  - Карточка `session`: агент · проект · хост, фаза, активность, время, итог, кнопка «↗ Open» → `session_open(key)`.

### Карточки алертов (высота 160)

- **`waiting`** (янтарная, только чтение): кто, что (цель кодом или вопрос), сколько ждёт; «OK» (= ack) и листалка.
- **Листалка** идёт по `snapshot.attention`, то есть только по непросмотренным. Вид карточки меняется по `kind` элемента.
- **Новые алерты при открытой карточке:** если ранг строго выше текущего, карточка заменяется; иначе обновляется только счётчик листалки.
- **`approval`** (Allow/Deny) — только для старого пути Claude. Если живой approval есть для того же `session_key`, алерт `permission` от модели **не звучит и не открывает** карточку. Если approval пришёл позже, он заменяет карточку `waiting` на месте.

### Поведение алертов

| Алерт | Звук (существующие WAV до подпроекта 3) |
|---|---|
| permission, input | approval |
| error | error |
| finished | finish |
| interrupted | — (алерта нет) |

- **Островок скрыт или компактный:** `island.alert()` открывает карточку алерта с наибольшим `rank` в пачке. Один звук на пачку.
- **Карточка `waiting`** сворачивается в компактный вид через `autoCloseInterval`, если курсор не на ней.
- **Развёрнут вид `"sessions"`:** строки пересортировываются, звучит звук, вид не меняется.
- **Развёрнут другой вид:** обновляются только счётчики, если алерт не важнее карточки на экране.
- **Тосты Windows** не используются.

**Фильтр:**
- выключенный тип алерта не звучит и не открывает карточку, но строка и счётчики обновляются;
- «Пауза» в трее глушит всё, отслеживание продолжается.

### FSM: удержание

- `fsm.setHoldVisible(on)` вызывается из каждого снимка с `snapshot.hold`.
- Пока удержание включено, срабатывание `schedulePetitHide` ничего не делает.
- При переходе on → off в состоянии `petit` вызывается `schedulePetitHide()`, чтобы островок снова мог спрятаться (CPU 0% в скрытом состоянии).
- **Пауза главнее удержания** (`forceHidden`). После снятия паузы при включённом удержании вызывается `reveal()`.

### Что убирается

- Псевдо-задача `integration_claude` («VS Code»): `INTEGRATION_AGENTS` её теряет, `loadIntegrationTasks` её не добавляет.
- `hooks.ts` сводится к карточке старого `PermissionRequest` с подписью из `payload.project_name`.
- Пилюли сторонних агентов апстрима `agent_<name>` из `coucou_agent` не переносятся:
  - в `hooks.ts` — `validateAgent`, `agentColor`, `ensurePill` и отказ на внешние `PermissionRequest`;
  - в `state.ts` — `AgentSource "agent"`, `removeTask`, `upsertExternalAgent` и сортировка `integration_claude` → `agent_*`.
  - Причины: после router (§5.3) такие события до островка не доходят, и код был бы мёртвым. У пилюль нет доказательства процесса, корня проекта и ключа сессии; это был бы второй путь состояния в обход `Store::apply` (§4) с риском «призраков» (§15).
- `hooks.ts` сокращается в том же шаге, что и замена `state.ts` (хранилище фронтенда), а не позже вместе с видами: иначе `tsc` падает на вызовах удалённых методов.

### Что сохраняется без изменений

- `BootInfo.cursorPoll` и `Island.followPageCursor` (1000f44). На Windows `CURSOR_POLL = true` (`platform/windows.rs:151`), и `followPageCursor` не включается; поле нужно `main.ts:25`.
- CSS-пауза анимаций в невидимых видах: `.view:not(.on) * { animation-play-state: paused; }` (eb1241f, `style.css:274-279`). Новые строки и карточки сессий бесконечных CSS-анимаций не получают; если такая появится, она живёт внутри `.view`.

## 11. Переименование (шаг 1, механика)

- **Pipe:** `\\.\pipe\mayak-<SID>` — `pipe.rs` и `hook` через `ident.rs`; SID приложения — `platform::current_user_sid()`.
- **Папки:** `%APPDATA%\Mayak`, `%LOCALAPPDATA%\Mayak`, `bin\mayak-hook.exe`, `mayak.log`.
  - `platform/windows.rs`: `pub const HOOK_EXE: &str = crate::ident::HOOK_EXE;`, `config_dir`/`local_dir` строятся через `.join(crate::ident::APP_DIR)`, комментарии с именем продукта. `settings::config_dir`/`local_dir` остаются реэкспортом.
- **`ensure_hook_exe`** апстрима и его `#[cfg(windows)] install_relay` не переписываются: все кандидаты и сообщения в лог уже строятся из `platform::HOOK_EXE` (`hooks.rs:357-410`), а он равен `ident::HOOK_EXE` — ресурс, рядом с exe, `../release/`, `_up_/target/release/`.
- **Crate-ы:** `mayak`, `mayak-hook`, `mayak_lib`. `package.json`: `name`, `predev`/`prebuild` → `-p mayak-hook`.
- **`tauri.conf.json`:**
  - `productName: "Mayak"`;
  - `identifier: "io.github.mahach666.mayak"` — новый профиль WebView2 и ключ single-instance;
  - заголовок окна;
  - `bundle.resources` → `mayak-hook.exe`.
- **`scripts/pack.mjs`:** имена артефактов.
- **`hooks.rs`:** `MARKER`, временный суффикс `.json.mayak-<pid>`, имя временной папки в тестах.
- **`secrets.rs`:** `SERVICE = io.github.mahach666.mayak`.
- **Строки с именем продукта** «Coucou» → «Mayak»:
  - трей (id, подсказка, меню);
  - `settings.html`/`index.html` `<title>`;
  - `settings/main.ts:70, 87 (-p mayak-hook), 138, 443`;
  - `views.ts:335`;
  - `bridge.ts:18, 62, 138`;
  - `lib.rs` баннеры и `[coucou]`;
  - User-Agent в `integrations.rs`;
  - «Denied from …»;
  - `nsis/hooks.nsh`.
- **Одноразовое копирование настроек Coucou** (§8).
- **Не переименовываются:** Mochi и строки про него, звуки, иконки, внутреннее имя состояния FSM `"coucou"` (приветствие Mochi) — всё это подпроект 3. Также id плашек интеграций и `NotchBuddy/`.

## 12. Тестирование

### Rust, юнит-тесты (`cargo test --workspace`)

Все функции путей принимают базовый путь явно, переменные окружения в тестах не используются.

- **`paths.rs`:**
  - один ключ для `c:/Users/x`, `C:\Users\x\`, `/c:/Users/x`, `file:///c:/Users/x`, `\\?\C:\Users\x`, смешанного регистра и кириллицы;
  - `project_of` с подставленным `probe`: папка `.git`, файл `.git` (worktree), без `.git`, корень диска.
- **`claude.rs`:**
  - разбор: busy/idle/waiting с тремя видами `waitingFor`;
  - пропуск `spare`/`parkedJobId`/`pidDomain:"linux:x"`/без `procStart`;
  - `30584.abc.key` не подходит под фильтр;
  - мусорный JSON у известного файла сохраняет состояние;
  - классификация хоста по фикстурам путей: MSIX Desktop, `%APPDATA%` Desktop, Cursor с `c:\` в нижнем регистре, VS Code.
- **`claude_transcript.rs`:**
  - пары `tool_use`/`tool_result`, порядок полей цели, вопрос `AskUserQuestion`;
  - фильтр основной цепочки (`isSidechain`);
  - отброс неполной первой строки окна;
  - итоги: `end_turn`, `refusal`, `stop_sequence`+`isApiErrorMessage`, маркер прерывания, `tool_use` после `end_turn` → Pending, без assistant → Pending;
  - только записи после `turn_start_offset`.
- **`codex.rs`:**
  - legacy-фикстура: `task_started`, `task_complete` с `error` и без, `turn_aborted` с `turn_id` и без;
  - `session_meta` субагента игнорируется;
  - неизвестная запись → Degraded.
- **`model.rs`** — табличные последовательности `Signal`:
  - первое появление без алерта;
  - полная таблица переходов Claude из §7, включая свёрнутый ход (idle → idle с новым `statusUpdatedAt`) и waiting → waiting с новым эпизодом;
  - обратный порядок: `ClaudeFile(waiting)` до `tool_use` → detail появляется на следующем `ClaudeTranscript`;
  - транскрипт появился после файла сессии;
  - «просмотрено»: ack скрывает; клик в Working не прячет будущий Finished; новый эпизод ожидания снова виден;
  - Codex, первый хук `PermissionRequest`/`Stop` для неизвестного ключа даёт фазу и алерт;
  - Codex, оба набора хуков (с `UserPromptSubmit` и без, через `TaskStarted`);
  - `:retry` (тот же `wait_key` в Waiting) без алерта; другой `tool_input` в том же ходе — новый эпизод даже после OK на первом;
  - `Stop` + `TaskComplete` одного хода — один алерт; `TaskComplete` с `error` после `Stop` поднимает до Error;
  - `Stop` старого хода (`relay_ts < turn_started_ts`) игнорируется;
  - `TurnAborted` без `turn_id`;
  - `ProcExit` удаляет все сессии процесса;
  - `counts`/`hold`/`aggregate` только по непросмотренным;
  - `hold` при `alertWaiting = false`;
  - порядок проектов; имя проекта при коллизии basename (на уровне snapshot).
- **`text.rs`:** все шаблоны `Alert.text`, обрезка до 160 символов, схлопывание пробелов.
- **`hookfile.rs`/`codex_hooks.rs`:**
  - нет файла → новый;
  - чужие группы сохраняют индексы; своя последней; переустановка на месте; идентичная запись не пишется;
  - удаление последней / заглушка;
  - отказ при неизвестном ключе верхнего уровня, при невалидном JSON;
  - отказ по пути: `[`, `{`, `@`, `=`, пробел; кириллица разрешена;
  - BOM, CRLF и финальный `\n` сохраняются;
  - дифф = записанные байты;
  - устаревший fingerprint и неизвестный `target`/`fingerprint` → отказ;
  - бэкап побайтно равен оригиналу.
  - Шесть существующих тестов `hooks.rs` переезжают; добавляется тест, что старый путь Claude ставит только `PermissionRequest`. Два теста `#[cfg(unix)]` апстрима удаляются вместе с Linux-ветками `hooks.rs`; на Windows они и не компилировались.
- **`settings.rs`:**
  - файл без `alert*` → `true`;
  - файл со старым `hooksInstalled` читается;
  - копирование Coucou → Mayak только если файла Mayak нет.
- **Ретранслятор:**
  - обратная совместимость argv и грамматика апстрима: флаг где угодно, значение поглощается, одиночное событие — `claude`;
  - `--agent` без значения и `--agent ""` → Claude; `--agent cursor` → чужой агент (агент не определён), значение не становится событием;
  - `legacy_approval_allowed`: `{session_id:"s", tool_name:"Bash"}` → да; без `session_id`, с пустым `session_id`, с `tool_name:"AskUserQuestion"` → нет;
  - Codex не ждёт, ничего не печатает, `exit 0` в бюджете;
  - `turn_id` в `PermissionRequest` при `--agent claude` → Codex, не ждёт;
  - несколько BOM;
  - `transcript_path` только для Codex;
  - `ancestors` на подставленной таблице, включая переиспользованный PID.
- **`pipe.rs`:**
  - таблица `route()`;
  - `serve(уникальное_имя, тестовый sink)`: строка Codex → sink; Claude `PermissionRequest` → Approval; больше 1 MiB → лог и отброс.

### Интеграционные тесты Windows (`#[cfg(windows)]`)

- `identify()` текущего процесса совпадает с std.
- `Registry::watch` срабатывает на `cmd /c exit`; повторная регистрация того же `(pid, created)` не создаёт второй поток.
- `DirWatcher` уведомляет о записи с последующим закрытием файла и сообщает `Error`, если каталог удалён.
- Диагностика `#[ignore]`: дозапись через открытый handle с маской `FILE_NAME | LAST_WRITE` и окном 2 с. Она фиксирует наблюдение, а не гарантию ОС, и в CI не запускается.

### CI

NEW `.github/workflows/windows-test.yml` — `pull_request` и `workflow_dispatch` на `windows-latest`:
- `npm ci`;
- `npm run build` (`tsc --noEmit` + vite);
- `cargo test --workspace`.

Это единственный тестовый CI. `.github/workflows/linux.yml` апстрима удаляется: он запускается на каждый pull request с `windows/**`, вызывает `cargo build --release -p coucou-hook` и `cargo test --workspace` на Ubuntu, где Windows-only код Mayak не соберётся. Linux-сборка не проверяется ни в CI, ни локально: на машине установлен только target `x86_64-pc-windows-msvc`.

Тест в `files.rs` переходит на явную временную базовую папку.

### Проверки в начале реализации

| # | Когда | Что | Кто | Влияние |
|---|---|---|---|---|
| S1 | сразу, кода не требует | В Cursor при запросе разрешения файл сессии → `waiting`/`permission prompt` с уведомлением. `tool_use` в транскрипте до ответа. Завершающая запись хода около `idle`. (Desktop `input needed` уже подтверждён.) | агент + один запрос разрешения от пользователя в Cursor | **Блокирующая:** при отрицательном результате реализация останавливается, и путь для Claude перепроектируется вместе с пользователем. |
| S2 | после шагов «ретранслятор» и «router» | Хуки Codex в Cursor (0.146) и Desktop (0.158): shell, целостность stdin (кириллица), время хука, мигание консоли, цепочка до `codex.exe` и путь Desktop-образа, один ли Trust на все поверхности. | агент собирает debug-ретранслятор с `--dump`; пользователь вручную вставляет показанный фрагмент в `~/.codex/hooks.json` (бэкап — копия файла), делает Trust и несколько промптов; в конце запись удаляется по чек-листу | Набор хуков по умолчанию, правила хоста. |
| S3 | вместе с S2 | Rollout 0.146/0.158: режим legacy или paginated; есть ли `turn_id` в `task_started`/`task_complete`/`turn_aborted`; есть ли `error` в `task_complete`; пишется ли `turn_aborted` при прерывании. | агент (только ключи и типы) | Объём разбора rollout. Если paginated — добавить разбор, иначе Degraded. |
| S4 | после шага «router» | Pipe с DACL текущего SID: хук из MSIX Codex Desktop подключается и проходит проверку «тот же пользователь». | агент + пользователь | DACL-коммит или DACL по умолчанию с записанной причиной. |
| S5 | вместе с S2 | Переименованный exe ретранслятора: Codex не блокирует ход и показывает родной запрос. | агент + пользователь | Безопасность оставшейся записи после удаления. |

### Ручная приёмка (записывается в PR)

1. Без записей в конфиги: Desktop + две сессии Claude в Cursor в разных репозиториях — правильные проекты, хосты Desktop/Cursor, время.
2. Запрос разрешения в Cursor: строка янтарная за ~1 с с целью команды; ответ в агенте снимает ожидание.
3. Конец хода → зелёная карточка с итогом; ошибка лимита → красная; Esc → тишина.
4. Убийство `claude.exe` сразу убирает строку.
5. Скрытый островок:
   - агенты простаивают — CPU 0% в Диспетчере задач;
   - агенты работают — нет периодических пробуждений Mayak, только реакция на события файлов.
6. Codex: дифф, бэкап, Trust; ход в Cursor и в Desktop → работает → ждёт → готово; родной запрос появляется. При наборе из двух хуков «работает» видно, только пока островок открыт.
7. Закрытие окна Cursor убирает его строки Codex. Если не убирает, это фиксируется (§15) и строки держатся до `SessionEnd` или «OK».
8. S1–S5 пройдены, временные записи удалены.

## 13. Точки расширения для подпроекта 2

1. **Хост сессии** (`HostKind`) и `ProcId` агента уже есть. Цепочку родителей и окна подпроект 2 получает сам. Родитель агента в Cursor — extension-host без окон, поэтому окна ищутся через главный процесс Cursor/Code.
2. **Одна функция идентичности проекта** — `paths::project_of`. Через неё пройдут папки рабочих областей и пути `.sln` VS 2022.
3. **`Project.sessions`.** Подпроект 2 добавит участников-окна новым полем и новым `Signal::Window` от `WindowSource`; машины состояний сессий не трогаются.
4. **Граница источников:** источник — поток, который шлёт `Signal` и исполняет свои `Effect`.
5. **Точка действия:** `session_open(key)`. Меняется тело, сигнатура остаётся.
6. **Ретранслятор:** флаг `--agent` + таблица `route()`. Сейчас незнакомое значение `--agent` ничего не отправляет (§5.2), поэтому новый агент добавляется в разбор флага, строкой `route()` и источником модели (п. 4).
7. **UI:** пустой слот для приложений в заголовке проекта.

Если подпроект 2 решит читать `~/.claude/ide/*.lock`, то строго с ограничениями: разбирается только поле `workspaceFolders`, `authToken` не хранится, не логируется и не передаётся. Это решение принимается в спецификации подпроекта 2.

## 14. Вне подпроекта 1

- Окна, связывание, VS 2022 (подпроект 2).
- Визуальный ребрендинг (подпроект 3).
- Allow/Deny для Codex; переработка Allow/Deny для Claude.
- Хуки собственного агента Cursor, Agents Window.
- Сторонние агенты с `--agent <name>`/`coucou_agent` (Gemini CLI, Antigravity и др.) и пилюли апстрима для них (§10). Установщики Gemini CLI и Antigravity в апстриме есть только для macOS. Будущий агент добавляется значением `--agent`, строкой `route()` и источником модели (§13, п. 6).
- Сборка и тесты под Linux: Linux-порт апстрима остаётся в дереве как есть, но не собирается и не тестируется (§5).
- Обычный чат Claude, Cowork; облако, WSL, SSH.
- Любые записи в `~/.claude` по умолчанию: хуки, плагины, `CLAUDE_CODE_PLUGIN_DIRS`, `skills`.
- Наблюдающий хук Claude: нужен только при провале S1, тогда решается отдельно.
- Хуки Codex на каждый вызов: `PreToolUse`/`PostToolUse`, а также `SessionStart`/`SubagentStart`/`SubagentStop`.
- App-server Codex, daemon, `notify`, sqlite; Restart Manager.
- `claude agents --json`, OpenTelemetry, channels.
- Разбивка по субагентам.
- История и «просмотрено» между перезапусками.
- Тосты, локализация, TS-раннер тестов.

## 15. Риски и непроверенное

| Риск / непроверенное | Смягчение или запасной вариант |
|---|---|
| Основу для Claude составляют недокументированные файлы (`sessions/<pid>.json`, поля транскрипта), а 2.1.x меняется еженедельно. | Терпимый serde; `Unknown` без алертов; здоровье источника в настройках; фикстуры обновляются при выходе новых версий Claude; живость подтверждается процессом. Поломка формата даёт «меньше строк», а не призраков. |
| `permission prompt` в Cursor не наблюдался вживую. | S1, блокирующая. |
| Хуки Codex в IDE и Desktop работают «по архитектуре» [код], вживую не проверены. Стоят ~0.3 с на запуск через PowerShell; в 0.144/0.146 возможно мигание консоли. | S2; запасной набор `PermissionRequest` + `Stop`. |
| Codex не виден, пока пользователь не сделает Trust и не перезагрузит хуки. Загруженные треды не перечитывают `hooks.json` [код]. Ручное переупорядочивание файла требует повторного Trust. | Запись побайтно стабильна; заглушка при удалении; инструкция и статус `receiving`. |
| **Ожидание Codex после одобрения держится до следующего хука** (обычно `Stop` в конце хода) или «OK». Снятие по хвосту rollout отвергнуто как зависящее от таймингов. | Прошедшее время на строке; «OK». Улучшение возможно только с детерминированным признаком из S3, отдельным решением. |
| Хук `PermissionRequest` не вызывается для диалога «Разрешения» (`request_permissions`, Codex 0.160), а в rollout во время диалога ничего не пишется. Ожидание выводится из вызова без ответа: `request_permissions` по имени либо `exec`, чей `input` упоминает это слово (§7), то есть для `exec` из текста самой модели. | Объявленная граница. `exec`, который лишь упоминает `request_permissions` или продолжает работать после одобрения, показывается как Waiting до ответа на этот вызов или до конца хода. Несколько вызовов без ответа ведутся множеством из не более 8 `call_id`; ответ на забытый самый старый ничего не меняет. Пока островок скрыт, тикера нет (§4) и вызов никто не читает: ожидание, карточка и звук появляются после того, как островок стал виден (первое чтение тикера, ~1 с); Working островок не удерживает, он скрывается сам через 60 с. Запасного признака нет. |
| Ошибка хода Codex не вызывает `Stop` [код]: строка остаётся Working, пока хвост не прочитан (островок виден или следующий хук). | Объявленная граница. |
| Вопросы Codex к пользователю не покрываются. Ошибки Codex в версиях без `error` в `task_complete` — тоже (S3). | Объявленные границы. |
| После перезапуска Mayak сессии Codex появляются снова только со следующим хуком. | Объявленная граница. |
| Codex держит rollout открытым — по исходникам main [код], для 0.146/0.158 не проверено. | S3. Если пишет с закрытием, схема «хвост только при видимом островке» всё равно корректна. |
| Убивает ли закрытие окна Cursor его app-server, не проверено. | Приёмка №7; иначе строки держатся до `SessionEnd`/«OK». |
| Какой `codex.exe` запускает Desktop 26.924: бандл в WindowsApps или копию в `%LOCALAPPDATA%\OpenAI\Codex\bin`. | Оба пути классифицируются как CodexDesktop; S2. |
| Завершающая запись хода Claude опаздывает или отсутствует → алерта нет. | Без догадок по времени; S1 проверяет порядок записей. |
| Фронтенд меняет однo-сессионный код без TS-тестов. | Вся логика и вычисляемые поля (`aggregate`, `hold`, `rank`, `text`) — в Rust под `cargo test`; `tsc` в CI; ручной чек-лист. |
| Поддельные события Codex от локального процесса. | DACL (S4); события Codex дают только read-only карточки; Codex никогда не получает ответа. |
| Число потоков: по одному ожидающему на подтверждённый процесс агента (десятки) + 2 наблюдателя + 3 рабочих. | При необходимости — `RegisterWaitForSingleObject` за тем же интерфейсом. |

## 16. Порядок работ

1. Переименование и копирование настроек Coucou (§11).
2. **S1** — блокирующая, кода не требует.
3. Ретранслятор (§5.2).
4. Pipe и router (§5.3) → **S4**.
5. Примитивы Win32, хвост, пути (§5.7–5.8).
6. Модель и тексты (§5.4, §6, §7).
7. Поток `claude-io` (§5.5).
8. Поток `codex-io` (§5.6) → **S2, S3, S5**.
9. Поток store и публикация (§9), команды Tauri.
10. Установщик (`hookfile`, `codex_hooks`, правки `hooks.rs`, `boot`).
11. Фронтенд (§10).
12. Окно настроек.
13. CI.
14. Ручная приёмка.

**Объём:** ~4.5 тыс. изменённых строк, из них ~1.5 тыс. — Rust-тесты. Новых crate-ов нет.
