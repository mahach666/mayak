# Ответы из острова: разрешения Claude, AskUserQuestion, разрешения Codex. Исследование

Дата: 2026-10-08. Ветка `main` (854e2ee), `upstream/main` = 89477b4.
Только чтение: в репозитории ничего не менялось.

Метки:
- **documented**: есть в официальной документации или в changelog.
- **observed in upstream code**: видно в коде Coucou (upstream) или в исходниках Codex.
- **observed in Mayak code**: видно в коде этого репозитория.
- **unknown**: ни один источник этого не говорит.

Источники (скачаны в `scratchpad/web/`):
- Claude Code hooks: https://code.claude.com/docs/en/hooks.md (сюда перенаправляет `docs.claude.com/en/docs/claude-code/hooks`, ответ 301). Ниже `hooks.md:<строка>`.
- Claude Code CHANGELOG: https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md (верх: 2.1.293). Ниже `cc-changelog:<строка>`.
- Codex hooks: https://developers.openai.com/codex/hooks.md. Ниже `codex-hooks.md:<строка>`.
- Исходники Codex, тег `rust-v0.161.0` (последний релиз): `codex-rs/hooks/src/events/permission_request.rs`, `codex-rs/hooks/src/engine/command_runner.rs`, `codex-rs/core/src/tools/approvals.rs`, `codex-rs/core/src/hook_runtime.rs`, `codex-rs/core/src/session/mod.rs`, `codex-rs/core/src/tools/handlers/request_permissions.rs`.
- Upstream: `git show upstream/main:NotchBuddy/Sources/App/HookServer.swift` (ниже `HookServer.swift:<строка>`), `AskQuestion.swift`, PR #165 (c767db9), PR #130 (123e713), PR #295 (89477b4).
- MemPalace: поиск по «Mayak AskUserQuestion PermissionRequest hook island Codex approval» ничего относящегося к делу не вернул (только заметки FlowWorker/Obsidian).

---

## A. Mayak сейчас

### A.1 Существующий путь Allow/Deny для Claude, от начала до конца

**Установка хука** (observed in Mayak code)
- UI: блок `<details>` «Advanced — Claude Code Allow/Deny in the island (legacy)» (`windows/src/settings/main.ts:372-391`, текст `windows/src/core/i18n/en.ts:277-279`).
  - Файл `~/.claude/settings.json` читается, только когда блок раскрыт (`main.ts:381-383`, `main.ts:131-134`).
  - Внутри общий `integrationSection("claude")` (`main.ts:114-200`): статус, Install / Reinstall / Update, Remove…, затем превью диффа и Apply.
  - Если ретранслятора нет, Install отключён (`main.ts:177-182`).
- Что записывается: одна запись `PermissionRequest` с `timeout: 120`, без `matcher` (`windows/src-tauri/src/hooks.rs:29-32`, `hooks.rs:162-181`).
  - Команда: путь к exe в кавычках, с прямыми слэшами, плюс имя события: `"C:/…/Mayak/bin/mayak-hook.exe" PermissionRequest` (`hooks.rs:80-83`, тест `hooks.rs:544-552`).
  - Причина такой формы: на Windows Claude Code запускает команды хуков через Git Bash (`hooks.rs:13-15`).
- Слияние (`merged`, `hooks.rs:164-181`):
  - сначала удаляются все наши записи и записи Coucou в любых событиях (`strip_ours`, `hooks.rs:118-138`; маркеры `entry_is_ours`, `hooks.rs:96-101`);
  - затем добавляется наша запись;
  - чужие хуки не трогаются.
- Проверка формы: если `"hooks"` не объект или `hooks.PermissionRequest` не массив, установка отказывается (`check_claude_shape`, `hooks.rs:144-160`).
- Согласие, бэкап, дифф (`windows/src-tauri/src/hookfile.rs:1-14`, `preview`/`apply` в `hooks.rs:326-387`):
  - превью считает точные байты и `unified_diff`;
  - план хранится в Rust; webview возвращает только target и fingerprint (`hookfile.rs:282-372`);
  - перед записью файл читается снова и должен совпасть с fingerprint (`hookfile.rs:390-400`);
  - затем байт-в-байт бэкап `<config>.mayak-backup-<stamp>` (`hookfile.rs:252-268`, `hookfile.rs:417-425`), временный файл и rename;
  - BOM, CRLF и конечный перевод строки сохраняются (`hookfile.rs:60-155`).
- Ретранслятор копируется при запуске в `%LOCALAPPDATA%\Mayak\bin\` (`hooks.rs:389-420`, `ensure_hook_exe`).

**Ретранслятор** `windows/hook` (observed in Mayak code)
- Порядок в `main.rs:25-89`:
  1. stdin; пустой → выход 0.
  2. Неизвестный `--agent` → выход 0 (D86).
  3. Убираются BOM; JSON-объект, иначе выход 0.
  4. `resolve_event`: берётся `hook_event_name`, иначе argv.
  5. `detect_agent`: `PermissionRequest` с `turn_id` считается Codex (`lib.rs:121-131`).
- Фильтр D87 (`main.rs:50-54`, `lib.rs:137-148`): Claude `PermissionRequest` без непустого `session_id`, или с `tool_name == "AskUserQuestion"` → выход 0 без обращения к pipe и без stdout.
- Обогащение (`lib.rs:180-219`):
  - добавляются `agent`, `relay: 2`, `relay_ts`, `agent_pid` (из `CLAUDE_PID`), `cwd`, терминальный контекст (`TERM_CONTEXT`, `lib.rs:43-49`);
  - удаляются `tool_response` и (у Claude) `transcript_path`;
  - каждая строка обрезается до `MAX_FIELD_LEN = 2000` (`lib.rs:28`, `lib.rs:221-239`).
- Отправка: одна строка compact JSON + `\n` (`lib.rs:241-246`) в `\\.\pipe\mayak-<SID>`.
  - Ждём ответа, только если `waits_for_answer` = Claude && `PermissionRequest` (`lib.rs:133-135`).
  - Ответ читается до первого `\n` (`lib.rs:265-287`).
- Что печатается в stdout (`decision_json`, `lib.rs:289-310`; тест `lib.rs:621-635`):
  - на `allow` или `always`:
    `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`
  - на `deny`:
    `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Mayak"}}}`
  - на что угодно другое или без ответа: ничего, выход 0 (`main.rs:82-88`).
  - `updatedPermissions` («Always») и `interrupt` не отправляются никогда.

**Тайм-ауты** (observed in Mayak code)

| Константа | Значение | Где | Что при срабатывании |
|---|---|---|---|
| `CONNECT_TIMEOUT` | 300 мс | `hook/src/lib.rs:31` | Нет pipe (Mayak закрыт) → сразу `None`, ничего не печатается (тест `lib.rs:695-700`) |
| `DECISION_BUDGET` | 110 с | `hook/src/lib.rs:37` | Весь прогон ретранслятора для Claude `PermissionRequest`, считается от старта процесса (`main.rs:26-28`, `main.rs:79`); по истечении `recv_timeout` → `None` → пустой stdout (`lib.rs:250-263`) |
| `FIRE_AND_FORGET_BUDGET` | 2 с | `lib.rs:35` | Остальные события Claude (они всё равно `Ignore`) |
| `CODEX_BUDGET` | 1 с | `lib.rs:33` | Весь прогон для Codex |
| `ACK_TIMEOUT` | 800 мс | `src-tauri/src/pipe.rs:47-50` | Остров не подтвердил, что карточка показана → соединение закрывается без ответа → терминал (`pipe.rs:323-340`) |
| `DECISION_TIMEOUT` | 108 с | `pipe.rs:51-52` | После ack никто не кликнул → закрыть без ответа (`pipe.rs:342-355`) |
| таймер карточки | 110 000 мс | `src/island/hooks.ts:118-129` | Карточка снимается сама |
| `timeout` в settings.json | 120 с | `hooks.rs:32` | Claude Code отменяет хук и отбрасывает его вывод (см. B.1) |

- Mayak закрывается, пока карточка ждёт: pipe рвётся, ретранслятор освобождается сразу (тест `lib.rs:681-693`).
- Смена языка сначала отдаёт карточку обратно (`approvalDecline`, `src/main.ts:131-137`).

**Протокол pipe** (`pipe.rs`; observed in Mayak code)
- Маршрут `route(agent, event)` (`pipe.rs:81-87`):
  - `("codex", _)` → `Observe`;
  - `("claude", "PermissionRequest")` → `Approval`;
  - всё остальное → `Ignore`.
- `Approval` (`pipe.rs:291-320`):
  1. `id = "<pid>-<n>"`.
  2. `approval_context` через `spawn_blocking`: `session_key` и `project_name` от наблюдателя (`pipe.rs:89-103`).
  3. Регистрация в `Pending`, затем `show_approval` эмитит `hook` в окно острова (`lib.rs` AppSink `src-tauri/src/lib.rs:560-578`).
  4. Ожидание `Reply::{Ack, Decision, Decline}` (`pipe.rs:54-62`).
  5. В pipe пишется только голое слово `allow` или `deny` + `\n` (`pipe.rs:313-318`); без решения не пишется ничего.
- Команды Tauri (`src-tauri/src/lib.rs:580-600`):
  - `approval_decision` → `pipe::answer`: всё, кроме `allow`/`always`, превращается в `deny` (`pipe.rs:384-391`);
  - `approval_ack`;
  - `approval_decline`.
- Безопасность pipe: DACL только для SID текущего пользователя (`pipe.rs:159-189`).

**Карточка в острове** (observed in Mayak code)
- `src/island/hooks.ts:77-131`. Принимается только `hook_event_name == "PermissionRequest"` с `request_id`.
- Сразу `approvalDecline`, если:
  - `shouldOpenForAlert("permission", …)` запрещает показ (пауза, mute; `hooks.ts:81-90`);
  - уже есть другая карточка: одна карточка — один запрос (`hooks.ts:91-97`).
- Иначе:
  - заполняется `State.pendingApproval`, строка «что разрешаем» через `approvalTarget` (`command` / `file_path` / `path` / `url` / `query` / `pattern` / `prompt`; `hooks.ts:53-71`);
  - синхронно `approvalAck`, звук, `island.alert("approval")`.
- `buildApproval` (`src/views/views.ts:318-350`):
  - имя проекта + «needs permission»;
  - строка кода;
  - две кнопки Deny (N) и Allow (Y); «Always» убран сознательно (`views.ts:338-343`).
- Клик → `decide` (`src/island/island.ts:197-209`) → `Bridge.approvalDecision`, затем карточка закрывается.

**Чего не хватает, чтобы это стало обычной функцией, а не «legacy»** (observed in Mayak code; это перечень по факту, не предложение)
1. Функция спрятана и помечена устаревшей:
   - `<details>` «Advanced … (legacy)» (`en.ts:277`, `main.ts:372-391`);
   - предупреждение про `sync.ps1` dotfiles (`en.ts:279`);
   - в 7 локалях свои строки `set.advanced*` (`src/core/i18n/*.ts`).
2. Комментарии и спецификации называют путь legacy:
   - `hooks.rs:4-5`;
   - `pipe.rs:6`;
   - `island/hooks.ts:1`;
   - D87 (`docs/superpowers/plans/2026-10-01-mayak-multi-session-core-contract.md:2330-2340`).
3. `windows/README.md:62-65` пишет «Settings… → Claude Code → Install hooks…», а в коде установка только в Advanced. README расходится с кодом.
4. Нет «Always»: `always` превращается в `allow` (`lib.rs:293-294`, `pipe.rs:385-387`); `updatedPermissions` не отправляется (upstream отправляет, B.1).
5. Только одна карточка: второй одновременный запрос сразу уходит в терминал (`hooks.ts:91-97`). Очереди нет.
6. Нет фильтра по месту запуска. Upstream отвечает `ask` для сессий не из VS Code или Cursor (`HookServer.swift:697-703`) и для Claude Desktop (PR #295, #191). Mayak показывает карточку для любой сессии с `session_id` (так задумано: `windows/README.md:153-154`).
7. Текст отказа фиксирован, `"Denied from Mayak"` (`lib.rs:295`). Он виден модели (B.1).
8. Ограничения времени согласованы (120 > 110 > 108; карточка 110), менять ничего не нужно.

### A.2 Что сейчас делают хуки Codex в Mayak (observed in Mayak code)
- Устанавливаемые события (`src-tauri/src/codex_hooks.rs:27-34`): `UserPromptSubmit` 10 с, `PermissionRequest` 10 с, `Stop` 10 с, `Interrupt` 2 с, `SessionEnd` 2 с.
  - Группа содержит только `type` / `command` / `timeout`, без `statusMessage` и `matcher`.
  - Команда: короткий (8.3) путь к exe без кавычек + `--agent codex` (`codex_hooks.rs:11-16`).
  - Codex доверяет хуку по позиции и хешу command/timeout/matcher/statusMessage (`codex_hooks.rs:3-9`; documented: `codex-hooks.md:62-70`, изменённый хук «marked for review and skipped until trusted»).
- Ретранслятор:
  - для Codex никогда не ждёт (`waits_for_answer` только Claude, `lib.rs:133-135`) и никогда ничего не печатает (`stdout_for`, `lib.rs:303-310`; тест `lib.rs:637-646`);
  - бюджет 1 с на весь прогон (`lib.rs:33`).
- Сервер: `route("codex", _) = Observe` (`pipe.rs:83`). Payload уходит наблюдателю, соединение сразу закрывается (`pipe.rs:280-283`).
  - Наблюдатель переводит сессию в `Waiting{Permission}` и поднимает алерт (`observe/model.rs:1558-1575`).
- Может ли ретранслятор держать хук и вернуть решение: механика есть (тот же `talk_to` с `waits=true`), но для Codex она выключена в трёх местах: `waits_for_answer`, `budget` → `CODEX_BUDGET`, `stdout_for`.
  - Ещё блокирует `route` (Codex → `Observe`, не `Approval`) и `timeout: 10` в `hooks.json`.
  - Кроме того, `detect_agent` переводит в Codex любой `PermissionRequest` с `turn_id`, чтобы импортированная из Claude запись Allow/Deny не отвечала Codex (`lib.rs:121-131`). Сам факт импорта хуков Claude в Codex взят из комментария Mayak; в документации Codex я его не нашёл (**unknown**).

---

## B. Контракты хуков Claude Code

### B.1 `PermissionRequest` (documented)
- Когда срабатывает: «Runs when Claude Code is about to ask you for permission to use a tool» (`hooks.md:1918`).
  - Для сетевого запроса команды в песочнице не запускается (`hooks.md:1923`).
  - Matcher: имя инструмента (`hooks.md:1925`).
- Вход (`hooks.md:1927-1955`): общие поля (`session_id`, `transcript_path`, `cwd`, `permission_mode`, `hook_event_name`), `tool_name`, `tool_input` (без `tool_use_id`), для MCP ещё `mcp_server`, необязательный `permission_suggestions` (массив permission update entries).
- Выход (`hooks.md:1958-1984`): `hookSpecificOutput.hookEventName = "PermissionRequest"` и `hookSpecificOutput.decision`:
  - `behavior`: `"allow"` или `"deny"`. Правила deny/ask всё равно применяются, то есть allow хука не перекрывает deny-правило;
  - `updatedInput` (только allow): заменяет весь ввод и снова проверяется на deny/ask;
  - `updatedPermissions` (только allow): например, эхо одного из `permission_suggestions` (`hooks.md:2014`). Это и есть «Always»;
  - `message` (только deny): «tells Claude why the permission was denied», то есть **текст отказа видит модель**;
  - `interrupt` (только deny): `true` останавливает Claude.
- Exit 2 для `PermissionRequest` не учитывается, поток не меняется (`hooks.md:1970`, `hooks.md:857`).
- Тайм-аут:
  - по умолчанию 600 с для command (`hooks.md:426`);
  - при тайм-ауте хук отменяется, вывод отбрасывается, «on most events a timed-out hook renders no decision» (`hooks.md:843-845`), значит обычный запрос в терминале.
- Пустой stdout:
  - exit 0 = успех без решения (`hooks.md:782-786`; пример «exit 0 # No decision: the normal permission flow applies», `hooks.md:812-820`);
  - пустой stdout с ненулевым кодом = non-blocking error с уведомлением (`hooks.md:836`).
- Все подходящие хуки запускаются параллельно (`hooks.md:412`).
- Показывает ли Claude Code диалог в терминале, пока хук `PermissionRequest` ещё работает: **unknown**, документация явно не говорит.
  - Mayak исходит из того, что терминал ждёт и «takes over» после тайм-аута (`pipe.rs:12-18`).
  - `statusMessage` = «spinner message displayed while the hook runs» (`hooks.md:427`).
- Версии (documented, changelog):
  - хук `PermissionRequest` появился в 2.0.45 (`cc-changelog:7603`);
  - `updatedPermissions` / «always allow» в 2.0.54 (`cc-changelog:7554`);
  - `updatedInput` перепроверяется на deny-правила с 2.1.110 (`cc-changelog:5538`).
- Upstream (observed in upstream code, `HookServer.swift:3277-3336`):
  - тот же `allow` / `deny` (`"Denied from Coucou"`);
  - для `always` (кроме Codex): `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":<permission_suggestions из входа>}}}`;
  - socket timeout 118 с; запись в settings.json `PermissionRequest` 120 с (`HookServer.swift:1263-1272`).

### B.2 `AskUserQuestion`: может ли хук ответить
- **Да, через `PreToolUse`** (documented):
  - `AskUserQuestion` есть в списке matcher для PreToolUse (`hooks.md:1580`);
  - `permissionDecision: "allow"` сам по себе не работает для `AskUserQuestion` и `ExitPlanMode`, нужен `updatedInput` (`hooks.md:1807`, `hooks.md:1840-1850`: «Returning "allow" alone is not sufficient»);
  - формат: эхо исходного массива `questions` + объект `answers` «question text → selected label»;
  - мультивыбор по документации: метки через запятую (`hooks.md:1787`).
- Документированный пример (`hooks.md:1854-1870`):
  ```json
  {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow",
    "updatedInput":{"questions":[{"question":"Which framework?","header":"Framework",
      "options":[{"label":"React","description":"Component library"},{"label":"Vue","description":"Progressive framework"}],
      "multiSelect":false}],
    "answers":{"Which framework?":"React"}}}}
  ```
- Другие исходы PreToolUse (documented, `hooks.md:1807-1810`):
  - `"deny"` + `permissionDecisionReason`: причину видит Claude, вопрос не задаётся;
  - `"ask"`: обычный запрос пользователю;
  - `"defer"`: только в `-p` режиме, в интерактиве игнорируется (`hooks.md:1876-1878`).
- Тайм-аут PreToolUse: command-хук по тайм-ауту не блокирует, вызов идёт обычным путём (`hooks.md:847-849`), то есть вопрос появится в терминале.
- Через `PermissionRequest`:
  - документация не говорит, что `AskUserQuestion` вызывает `PermissionRequest`, и не описывает ответ на вопрос через `decision.updatedInput` (**unknown** по документации);
  - upstream (observed in upstream code): старый путь Python-ретранслятора `decision == 'answer'` в ветке `PermissionRequest` печатает `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":{"questions":…,"answers":…}}}}` (`HookServer.swift:3327-3334`);
  - но сервер upstream сейчас на `PermissionRequest` с `AskUserQuestion` отвечает `ask` («If it still arrives here as a PermissionRequest, reply "ask"…», `HookServer.swift:726-735`). Значит, такой запрос может приходить;
  - Mayak его отфильтровывает (D87, A.1).
- Как это делает upstream (observed in upstream code, PR #165 → c767db9, слит 2026-10-03):
  - Установка: вторая запись `PreToolUse` с `"matcher":"AskUserQuestion"` и командой `<hook> --ask`, `timeout: 130` (`HookServer.swift:1279-1285`). Если записи нет, Settings показывает «outdated hooks» (`HookServer.swift:1180-1221`).
  - Ретранслятор `--ask` (`HookServer.swift:3179-3221`):
    - если `tool_name != AskUserQuestion`, выходит без вывода;
    - иначе socket timeout 125 с, ждёт `{"permissionDecision":"answer","answers":{…}}`;
    - печатает `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"questions":<questions из своего stdin>,"answers":<answers>}}}`;
    - на `ask` или что угодно другое не печатает ничего, и Claude Code спрашивает в терминале.
  - Сервер (`HookServer.swift:309-319`, `867-960`):
    - разбирает 1–4 вопроса по 2–4 варианта (`AskQuestion.swift`, `parse`);
    - непонятный payload → `ask`;
    - новый вопрос вытесняет старый (старому `ask`);
    - через 120 с `ask`;
    - кнопка «Reply in terminal», сворачивание и другой алерт → `ask` (`HookServer.swift:128-185`).
  - Формат ответов (`AskQuestion.swift`, `buildAnswers`): single-select / Other → строка; multi-select → **массив меток** (комментарий: «Claude Code 2.1.136+»).
- Документировано это или выведено опытным путём:
  - путь PreToolUse + `updatedInput{questions, answers}` **documented** (`hooks.md:1840-1870`);
  - мультивыбор массивом **documented только в changelog**: «Fixed `AskUserQuestion` discarding multi-select answers when supplied as an array» в 2.1.136 (`cc-changelog:5002`). Справочник называет метки через запятую (`hooks.md:1787`);
  - путь через `PermissionRequest` + `updatedInput` выведен опытным путём (только старый код upstream).
- Версии (documented):
  - 2.1.85: «PreToolUse hooks can now satisfy `AskUserQuestion` by returning `updatedInput` alongside `permissionDecision: "allow"`» (`cc-changelog:5971`; upstream ссылается на ту же версию, `HookServer.swift:1279`);
  - 2.1.136: массивы для мультивыбора (`cc-changelog:5002`);
  - 2.1.200: диалоги AskUserQuestion больше не продолжают сами по умолчанию (`cc-changelog:3835`).
- Важно для Mayak (observed in Mayak code): ретранслятор обрезает каждую строку до 2000 символов прямо в `payload` (`main.rs:76`, `lib.rs:221-239`).
  - `answers` надо строить по ключам точного текста вопроса, а `questions` надо эхом вернуть без изменений.
  - Значит, для `updatedInput` нужна необрезанная копия `tool_input.questions` из stdin. Upstream берёт её из своего stdin (`HookServer.swift:3216`).
- Пока PreToolUse-хук ждёт остров, диалога вопроса в терминале ещё нет: PreToolUse идёт «before processing the tool call», `hooks.md:1580`.
  - Это **inferred**, а не прямо сказано.
  - Upstream PR #165 пишет то же: «Reply in terminal, folding the island, another alert or a 120 s timeout all hand the question back to Claude Code».

---

## C. Контракты хуков Codex

### C.1 Ответ решением на `PermissionRequest` (documented + observed in Codex source)
- Когда срабатывает:
  - «runs when Codex is about to ask for approval, such as a shell escalation or managed-network approval. It can allow the request, deny the request, or decline to decide» (`codex-hooks.md:830-836`);
  - только для действий, которым нужно одобрение;
  - matcher: `Bash`, `apply_patch` (тоже `Edit|Write`), MCP-имена (`codex-hooks.md:420`, `838-840`).
- Вход: общие поля + `turn_id`, `tool_name`, `tool_input` (`tool_input.command` у Bash/apply_patch), `tool_input.description` (`codex-hooks.md:842-849`), `permission_mode` (`codex-hooks.md:484-487`).
- JSON ответа (`codex-hooks.md:855-882`):
  - allow: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`
  - deny: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"…"}}}`
  - это ровно тот формат, который уже печатает `decision_json` в Mayak (`lib.rs:289-301`).
- Поля `updatedInput`, `updatedPermissions`, `interrupt` «are reserved for future behavior and fail closed today» (`codex-hooks.md:886-888`), значит «Always» для Codex невозможен. Upstream тоже не шлёт `updatedPermissions` для Codex (`HookServer.swift:3312`; `docs/SPEC.md:124` upstream).
- Несколько хуков: любой deny побеждает, иначе allow; без решения обычный поток одобрения (`codex-hooks.md:884-885`; код `permission_request.rs:13-15`, `152-169`).
- Exit 2 с непустым stderr = deny с этим текстом (`permission_request.rs:244-252`). Plain text в stdout игнорируется (`codex-hooks.md:850`).
- Текст deny уходит модели: `ReviewDecision::denied(message)` → `ToolError::Rejected(rejection)` (`approvals.rs:517-520`, `approvals.rs:464`). observed in Codex source.
- Порядок: «Approval precedence is: 1. Hooks 2. … Guardian … Else, user» (`approvals.rs:505-523`). Модуль: «This event runs in the approval path, before guardian or user approval UI is shown» (`permission_request.rs:3-4`).
  - Значит, **пока хук ждёт, Codex не показывает свой запрос одобрения**. observed in Codex source.

### C.2 Тайм-аут Codex и что бывает без ответа
- По умолчанию 600 с для большинства хуков (`codex-hooks.md:182-184`). `SessionEnd` и `Interrupt`: 1 с по умолчанию, максимум 3 (`codex-hooks.md:184`, `1108-1109`). documented.
- По тайм-ауту (`command_runner.rs:276-320`): `error = "hook timed out after Ns"` → в `parse_completed` статус Failed, решения нет (`permission_request.rs:205-212`) → `None` → `request_reviewer_approval`, то есть обычный запрос пользователю (`approvals.rs:523`). observed in Codex source.
- Exit 0 и пустой stdout: решения нет, обычный поток (`permission_request.rs:214-217`; documented: `codex-hooks.md:517`).
- Mayak ставит `PermissionRequest` с `timeout: 10` (`codex_hooks.rs:31`), бюджет ретранслятора 1 с (`lib.rs:33`).
  - Ограниченное ожидание потребует поднять `timeout` в `hooks.json`.
  - Это меняет хеш, и Codex пропускает хук, пока пользователь снова не подтвердит доверие в `/hooks` (documented: `codex-hooks.md:62-70`; Mayak: `codex_hooks.rs:3-9`).
- Upstream ставит Codex `PermissionRequest` на 120 с со `statusMessage` «Waiting for your answer in the notch (Coucou)» (`HookServer.swift:1886-1898`), ретранслятор ждёт 118 с.
  - `statusMessage`: «Optional message shown while the hook runs» (`codex-hooks.md:270`); тоже входит в хеш доверия.

### C.3 Отвечает ли upstream на одобрения Codex из острова
- **Да, observed in upstream code**, с PR #130 (123e713, 2026-10-02 «Codex: live sessions and approvals from the notch»):
  - сервер: `isCodexRequest = rawAgent == "codex"` (только не-AppStore сборка), та же карточка, pill `agent_codex` (`HookServer.swift:672-700`);
  - ретранслятор печатает обычный allow/deny (`HookServer.swift:3315-3326`);
  - в SPEC upstream: «Approbations Codex … carte avec Allow et Deny seulement (pas Always) ; updatedPermissions jamais envoyé pour Codex» (`upstream docs/SPEC.md:124`);
  - README upstream: «Codex also gets Allow / Deny» (`upstream README.md:39`).
- PR #295 (89477b4) одобрения Codex не меняет.
  - В нём: #290 (сворачивание карточки одобрения), #249 (полный текст вопроса и описания вариантов), #191 (Claude Desktop: на permission requests отвечать `ask`), #244 (план Codex) и другое. Это видно в описании PR, текст PR считался данными.
- Upstream `windows/` (89477b4): `AskUserQuestion` не обрабатывает. Ретранслятор ждёт любой `PermissionRequest` (`upstream windows/hook/src/main.rs:50`), Codex-хуков в upstream `windows/` нет.

### C.4 Диалог `request_permissions` Codex и хуки
- **Ни один хук его не достаёт**, observed in Codex source v0.161.0:
  - `request_permissions_for_environment` (`session/mod.rs:2889-2963`) вызывает `request_guardian_approval` напрямую, а затем шлёт `EventMsg::RequestPermissions` в UI (`session/mod.rs:3012-3040`), минуя `Session::request_approval`;
  - `run_permission_request_hooks` вызывается только из `request_approval` (`approvals.rs:508`; поиск по репозиторию находит лишь `hook_runtime.rs` и `approvals.rs`);
  - у `ApprovalAction::RequestPermissions` есть `permission_request_payload` (`approvals.rs:225-235`), но этот путь его не использует.
- Mayak фиксировал это для Codex 0.160 (`docs/superpowers/specs/2026-10-01-mayak-multi-session-core-design.md:553`, `:962`). Наблюдатель видит его только по rollout (`observe/codex.rs:56-60`).
- В `PreToolUse` Codex: таблица Tool coverage говорит, что «Other local function tools» матчатся по имени (`codex-hooks.md:445-460`).
  - Перехватывает ли `PreToolUse` с matcher `request_permissions` этот вызов и что он может с ним сделать, кроме блокировки: **unknown**, не проверялось.
  - Ответить на диалог «разрешить» через PreToolUse документация не позволяет: в Codex у PreToolUse нет allow с `updatedInput`-ответом для этого инструмента.

---

## Итог по трём решениям пользователя

1. **Claude Allow/Deny.** Работает целиком, контракт documented. Не хватает только «нормального» UI: вынести из Advanced/legacy, поправить README, по желанию «Always» через `updatedPermissions` = `permission_suggestions` (documented). Тайм-ауты согласованы: 120 / 110 / 108 / 0,8.
2. **AskUserQuestion.** Возможно, documented с Claude Code 2.1.85: отдельный `PreToolUse` с `matcher: "AskUserQuestion"`, ответ `permissionDecision:"allow"` + `updatedInput{questions (эхо без изменений), answers{вопрос→метка}}`. Мультивыбор: массив (2.1.136+, changelog) или метки через запятую (справочник). Пока хук ждёт, вопрос в терминале не показан, значит нужен короткий потолок ожидания и быстрый возврат (upstream: 130 / 125 / 120 с, «Reply in terminal»). Ретранслятору нужна необрезанная копия `questions`.
3. **Codex Allow/Deny.** Возможно, documented: JSON тот же, что у Claude; только allow/deny, без «Always». По тайм-ауту или пустому выводу Codex показывает свой обычный запрос. Пока хук ждёт, Codex свой запрос не показывает. Нужно поднять `timeout` в `hooks.json` (сейчас 10 с), и это заставит пользователя заново подтвердить доверие хуку в `/hooks`. `request_permissions` хуками недоступен.
