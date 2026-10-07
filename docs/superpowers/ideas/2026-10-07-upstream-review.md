# Обзор upstream (Louis-CFM/coucou) для Mayak на Windows, часть 2

Дата: 2026-10-07. Диапазон: `83708fe..upstream/main` (`89477b4`): 46 коммитов, из них 10 на первом родителе (upstream вышел на 0.2.1 и влил два пакета community-PR, #294 и #295). Предыдущий обзор: `2026-10-06-upstream-review.md` (`2b387e4..83708fe`). Тексты PR (`gh pr view`) использованы только как данные. Ничего не менялось и не коммитилось, кроме этого файла; сборка и тесты не запускались. Номера строк Mayak взяты из рабочего дерева на момент чтения (ветка на `7fed610`); во время обзора в рабочем дереве шли чужие правки `hooks.rs`, `hookfile.rs`, `codex_hooks.rs`, `hotkey.rs`, `lib.rs` (новый `message.rs`), так что номера в этих файлах могут сдвинуться на несколько строк. Выводы перепроверены на рабочем дереве.

## Топ-5 рекомендаций

| # | Рекомендация | Действие | Связь |
|---|---|---|---|
| 1 | **Настройки Mayak теряются при одном испорченном поле (баг общий с upstream, там исправлен в d8d2939).** `settings::load` (`windows/src-tauri/src/settings.rs:213`) делает `from_slice::<Settings>(..).unwrap_or_default()`: BOM от Блокнота, поле не того типа или обрезанный файл дают **все** значения по умолчанию, а первое же сохранение затирает файл (язык, Alt+M, помодоро, тревоги). Запись не атомарная (`std::fs::write`, строка 223), ошибка сохранения уходит в `eprintln!` (`lib.rs:104`), которого GUI-приложение не показывает | **adapt**: перенести логику d8d2939 (снять BOM, `#[serde(default)]` на структуру, поштучное спасение полей, копия испорченного файла рядом, запрет перезаписи нечитаемого файла, temp + rename, `log::line`). В Mayak готовы `hookfile::strip_bom` и шаблон `write_synced` + rename. Сохранить копирование настроек Coucou и `migrated()` | баг, d8d2939 (#243) |
| 2 | **Защита формы `"hooks"` в `~/.claude/settings.json`** (из #243, `ClaudeSettingsFile.unexpectedHooks`). `hooks.rs::merged` (строки 141-158) при `"hooks"`, который не объект, или списке события, который не массив, молча подставляет пустое значение и записывает своё. У Codex такая проверка уже есть (`codex_hooks::check_shape`), у Claude нет. Дифф показывается до записи, так что это не тихая порча, но правило проекта звучит как «не перезаписывать» | **adapt** (~20 строк + тест): проверка формы до `make_plan`, при ошибке заметка и отказ, как у Codex | правило CLAUDE.md про settings.json |
| 3 | **Неотменяемый таймер после ошибки загрузки** (того же класса, что 3491a2b «cancel stale Stop timers»). `island.ts:886`: `window.setTimeout(() => this.setView(State.defaultView()), 2400)` не проверяет, что на экране ещё `note`. Если за 2,4 с пришла карточка (ожидание, finished, фаза помодоро, legacy Allow/Deny) или пользователь свернул остров, таймер заменит карточку и через `fsm.forceHome()` раскроет остров снова. В `integrations.ts:65-78` и `views/chat.ts:163` такая проверка есть | **take** по смыслу (1 строка: `if (State.view === "note")`). Не воспроизводилось, найдено чтением кода | 3491a2b |
| 4 | **Лимиты Codex напрямую через `codex app-server`** (#244, `CodexPlanGauge.swift`): JSON-RPC `initialize`, затем `account/rateLimits/read`; отдаёт `primary`/`secondary` с `usedPercent`, `windowDurationMins`, `resetsAt`, а также `planType` и `rateLimitResetCredits`. Mayak читает те же окна пассивно из `token_count` в rollout (`observe/codex.rs:239-256`), поэтому после сброса окна показывает «—» до следующего ответа Codex (`usage-format.ts:123`) | **adapt, после spike**: только по запросу (кнопка обновления или открытие главной при данных старше N минут), таймаут 15 с, без постоянного процесса. Проверить на Windows: как запускать npm-шим `codex.cmd`, существует ли метод в установленной версии Codex, сколько стоит запуск. Пассивное чтение оставить основным | ideas §1, usage-home spec §2.2 |
| 5 | **Лицензия ассетов перед ребрендингом §26** (#275 поменял только формулировки README и сайта; `LICENSE-ASSETS.md` уже есть в нашем дереве). Сейчас Mayak собирается с ассетами под «all rights reserved»: звуки берутся из `NotchBuddy/Resources/sounds` (`windows/vite.config.ts:9`, 29 wav), иконки `windows/src-tauri/icons` совпадают с upstream побайтно, персонаж Mochi (`windows/src/mochi/`) и имя «Mochi» в строках (`en.ts:301` и переводы) | **checklist для §26**: свой персонаж, свои звуки, свои иконки, убрать «Mochi»/«Coucou» из видимых строк. Код MIT, его можно оставлять с сохранением копирайта. Функциональные метки (`LEGACY_MARKERS`, миграция `Coucou\settings.json`) не бренд, их оставить | ideas §26 |

Сводно: в `windows/` upstream за период правил только три вещи: настройки (d8d2939), таймеры Stop в `hooks.ts` (3491a2b) и тесты с CI (78815b5). Первая — общий с Mayak баг, вторая в Mayak неприменима (этого пути больше нет), но рядом есть похожий таймер, третья полезна как инструмент. Остальное — macOS, где для Mayak есть идеи (лимиты Codex, итоги недели, новые агенты), но не готовый код.

## 1. Коммиты и PR upstream

Первый родитель: `ec1a4f2` (#263) → `9b144d5` (#264) → `3b21e59` (#268) → `02ef42c` (0.2.0) → `8b3322b` (#275) → `20e1f8b` (#276) → `595092b` (#288) → `721d34f` (0.2.1) → `0a633f9` (#294) → `89477b4` (#295). Внутри #294 и #295 сохранены коммиты авторов PR.

| Коммит / PR | Что делает | Часть | Есть в Mayak? | Вердикт |
|---|---|---|---|---|
| ec1a4f2 «More agents: Copilot CLI, Muse Code, OpenCode, Amp» (#263) | Установщики хуков и плагинов для четырёх агентов, нормализация имён событий в Python-релее, таблица агентов в `windows/README.md` (на Windows только Copilot и Muse через `--agent`, OpenCode и Amp «Mac-only») | Swift, docs (+ README `windows/`) | Нет. Релей Mayak отбрасывает чужой `--agent` (`hook/src/lib.rs:10,68`), наблюдатель знает только `Source::Claude`/`Codex` (`observe/model.rs:30`) | **idea only**: см. (e). Плагин Amp всегда возвращает `{action:'allow'}`, это противоречит правилу «не одобрять без клика», не копировать |
| 9b144d5 «Weekly recap» (#264) | `RecapStore` (recap.json, 12 недель): ходы, время, файлы и строки, команды, решения по разрешениям; карточка в понедельник, картинка 1080×1920 для шаринга | Swift | Частично: вкладка Stats считает токены по дням, проектам, агентам и моделям из файлов агентов (`stats.rs`) | **idea only**: см. (f) |
| 3b21e59 «Coucou in 10 languages» (#268) | `Localizable.xcstrings`, 305 ключей × 10 языков (en, ar, bn, es, fr, hi, id, pt-BR, ru, zh-Hans); выбор языка в Settings. В ветке есть дубли коммитов #263/#264 | Swift | Своё i18n на 7 языков (`windows/src/core/i18n`: en, ru, de, fr, es, zh, ja) | **idea only**: см. (g) |
| 02ef42c, 721d34f «Prepare 0.2.0 / 0.2.1» | Версии, CHANGELOG, README, `docs/AGENTS.md` (Hermes), строка в `windows/README.md` | docs, релиз | Свой CHANGELOG | skip |
| 8b3322b «MIT covers the code, not Mochi or the sounds» (#275) | Бейдж «Code: MIT», формулировки README и подвала сайта, ссылка на `LICENSE-ASSETS.md` | docs | `LICENSE-ASSETS.md` уже в дереве (bca501d, до диапазона) | **idea only**: топ-5, см. (h) |
| 20e1f8b «Demo mode for App Review» (#276) | `DemoEngine.swift`: фальшивые сессии и данные для ревью App Store | Swift | Неприменимо | skip |
| 595092b «Hermes Agent support» (#288) | Python-плагин в `~/.hermes/plugins/coucou/`, слияние `~/.hermes/config.yaml`, события `on_session_start`, `post_llm_call`, `pre_approval_request` (только наблюдение) | Swift | Нет | **idea only**: см. (e); YAML-слияние сложное (свой парсер), на Windows не заявлено |
| d05f22b «Fix crash an hour after a file edit» (#286) | Таймер очистки диффов сессии через час запускал `@MainActor`-код на глобальной очереди; Swift 6 ловил нарушение изоляции и завершал приложение. Теперь `DispatchQueue.main` | Swift | Такого таймера нет; класс ошибки в Rust невозможен (`Send`/`Sync`), но см. (d) про `panic = "abort"` | skip (код), см. (d) |
| 74984f2 … 2a24913 «Respect the configured auto-close delay» (#25) | FSM острова игнорировала задержку автозакрытия из Settings (всегда 15 с) | Swift | Есть: `fsm.homeToPetitDelay = State.settings.autoCloseInterval` (`island.ts:350,1478`), применяется сразу | skip |
| ea244a7 «Reveal the island when reopening» (#270) | `applicationShouldHandleReopen` открывает остров | Swift | Есть: single-instance → событие `tray`/`open` → `setPaused(false)` + `island.alert(...)` (`lib.rs:759`, `main.ts:104`) | skip |
| 918d30e «protect Claude settings during hook installation» (#243) | `ClaudeSettingsFile.swift`: не начинать с `{}` при нечитаемом файле, бэкап до записи, отказ если файл изменился после превью, атомарная запись с сохранением прав и симлинков, отказ при неожиданном типе `"hooks"` | Swift | Почти всё есть в `hookfile.rs` (`parse_object`, отпечаток, бэкап байт в байт через `create_new`, temp + rename). Нет: проверки формы `"hooks"` для Claude и следования симлинку | **adapt** (только проверка формы): топ-2, см. (c) |
| 78815b5 «test(windows): add test suite and CI» (#243) | `node --test` над TS-исходниками без сборки: `tests/setup.mjs` (резолв без расширений, `window = globalThis`), `tests/tauri.mjs` (подмена `__TAURI_INTERNALS__`, запись вызовов, отправка событий), `fsm.test.mjs`; `windows-ci.yml` (tsc, npm test, сборка релея, cargo test) | `windows/` | CI есть (`.github/workflows/windows-test.yml`: `npm run build` с `tsc`, `cargo test`), тестов TS нет | **adapt**: взять `setup.mjs` + `tauri.mjs` (~50 строк) и скрипт `test`, переписать тесты под нашу `fsm.ts`. Нужен Node ≥ 22.18 |
| 3491a2b «cancel stale Stop timers» (#243) | В `hooks.ts` таймер возврата в idle после Stop (5,2 с) не отменялся следующим ходом и сбрасывал новое состояние пилюли | `windows/` | Неприменимо: в Mayak `hooks.ts` обслуживает только legacy PermissionRequest, состояние сессий выводится наблюдателем без таймеров. Похожий таймер есть в `island.ts:886` | **idea**: топ-3, см. (a) |
| d8d2939 «make preferences resilient to corrupt files» (#243) | См. топ-1: BOM, `serde(default)`, поштучное спасение полей, копии `.corrupt-<дата>` / `.unread-<дата>`, запрет перезаписи непрочитанного, temp + rename, `log::line` | `windows/` | **Нет, баг общий** (`settings.rs:206-224`, `lib.rs:103-105`) | **adapt**: топ-1 |
| efc80c3 «Preserve all Claude response text» (#67) | Чат склеивает все текстовые блоки ответа без разделителей (у ответов с веб-поиском цитаты режут фразу на блоки) | Swift | Mayak уже берёт все блоки, но склеивает через `"\n"` (`claude.rs:148-155`): фразы с цитатами будут рваться по строкам | skip, пока `CHAT_ENABLED = false` (`core/features.ts:8`); при включении чата заменить `join("\n")` на `concat()` |
| 5c3fd8f, 0a633f9 (#294) | Слияние пакета и пустая строка в `build.yml` | CI | — | skip |
| 40e3ba8 «Allow pending approvals to collapse» (#290), 6012900 «Escape in another app never hides a pending approval» | Карточку разрешения можно свернуть (Escape острова, горячая клавиша), запрос остаётся ждать; глобальный Escape из другого приложения её больше не прячет | Swift | Частично: Alt+M сворачивает и закреплённый остров (`island.ts:1065-1075`), Escape в webview не сворачивает закреплённый (`island.ts:1019`); глобального перехвата Escape у Mayak нет. Повторный Alt+M открывает список сессий, а не ждущую карточку | skip (путь legacy и выключен); при пересмотре ideas §4 учесть «вернуться к ждущей карточке» |
| 52b1562 «Question card: full question and option descriptions» (#249) | Карточка AskUserQuestion показывает весь вопрос и описания вариантов | Swift | Сознательно нет (D87, `hooks.rs:30`) | skip |
| 4eb661d «Stop asking for a key on pills that run on hooks» (#183) | Пилюли агентов на хуках не просят API-ключ и показывают, установлены ли хуки | Swift | Неприменимо (у Mayak нет пилюль агентов с ключами) | skip |
| 4c4831d, ef7ca85 «Choose which screen Mochi lives on» (#236) | Выбор экрана: экран с вырезом, экран с меню, следовать за мышью, конкретный дисплей по UUID; взгляд Mochi на любой раскладке | Swift | Частично: `screen: "primary" \| "cursor"` (`core/state.ts:106`) | **idea only**: выбор конкретного монитора, если попросят |
| 344dc12, 77a7ef1 «Claude Desktop pill» (#191) | Сессии из Claude Desktop (`CLAUDE_CODE_ENTRYPOINT=claude-desktop`) получают свою пилюлю | Swift | Есть: `HostKind::ClaudeDesktop` в `observe/claude.rs` | skip |
| fff9649, 907d63a, 1add8e3, d1aabbd «Codex plan usage» (#244) | См. топ-4. Только GitHub-сборка, выключено по умолчанию, `app-server` запускается, пока видна пилюля, кэш 60 с; поиск `codex` в nvm, Volta, Bun, pnpm | Swift | Аналог есть, но пассивный (rollout) | **adapt, после spike**: топ-4, см. (b) |
| 2bdefe5, 89477b4 (#295) | Исправление сборки App Store и слияние пакета | Swift | — | skip |
| 8729a8d, 74dfec8, 2a24913, 7004bfb, e01c85f, 1450d82, 54511d2, f4285d0, 544884c, fd88480, 38e6b77, 801bf52, 60697cb, c0475b1 | Служебные слияния веток PR с main | — | — | skip |

Общий дифф `windows/ .github` за период: 11 файлов, +1470/-21 (`settings.rs` +618, из них около 400 строк тестов; `hooks.test.mjs` +493).

## 2. Детали по пунктам особого внимания

### (a) Изменения в `windows/` upstream и общие баги

1. **d8d2939, настройки (общий баг, высокий приоритет).** Код Mayak:
   - `settings.rs:213`: `serde_json::from_slice::<Settings>(&bytes).unwrap_or_default().migrated()`. Поля `sound_enabled`, `sound_volume`, `auto_close_interval`, `absence_interval`, `active_integrations`, `screen`, `autostart` обязательны (нет `#[serde(default)]`), поэтому файл без одного из них целиком сбрасывается. Новые поля Mayak (`alert_*`, `hotkey`, помодоро, `language`) защищены пофайлово, старые нет.
   - BOM: `serde_json` его не принимает; файл, сохранённый Блокнотом или PowerShell 5, даст значения по умолчанию.
   - `settings.rs:218-224`: `std::fs::write` без temp + rename; при сбое питания остаётся половина файла, что при следующем запуске опять даёт сброс.
   - `lib.rs:104`: `eprintln!` вместо `log::line`.
   - Что взять из upstream: `parse` (BOM, пустой файл, только объект), `salvage` (поле за полем), `load_from` с `protect`, `keep_what_was_not_loaded`, `set_aside` (без дублей одинаковых копий), `save_to` (temp + rename; ветку с симлинком под `#[cfg(unix)]` можно опустить). Тесты upstream подходят почти без правок (меняется набор полей в `CUSTOM`). Учесть порядок в Mayak: сначала `copy_legacy_settings`, затем разбор.
2. **3491a2b, таймеры Stop.** В Mayak этот обработчик удалён (переход на наблюдателя, `hooks.ts` 131 строка, только PermissionRequest). Пересмотр остальных `setTimeout` в `windows/src`: `integrations.ts` (по `clearTimers`, проверяет состояние), `views/chat.ts:163` (проверяет `State.view`), FSM (`fsm.ts` через `clear`) защищены. **Не защищён `island.ts:886`** (топ-3).
3. **78815b5, тесты TS.** Подход upstream запускает TS-исходники в Node 22.18+ без сборки и без изменений в коде, через подмену `__TAURI_INTERNALS__`. В Mayak `i18n-core.ts` уже рассчитан на запуск в Node (комментарий в строке 1-2). Перенос даёт тесты `fsm.ts`, `usage-format.ts`, `launcher-model.ts` в CI.

### (b) Лимиты Codex: #244 и Mayak

| | Upstream (#244) | Mayak |
|---|---|---|
| Источник | `codex app-server` (stdio JSON-RPC): `initialize`, `initialized`, `account/rateLimits/read` | `event_msg/token_count` в rollout-файлах, `rate_limits.primary/secondary` с `limit_id == "codex"` (`observe/codex.rs:231-256`) |
| Когда обновляется | Пока видна пилюля, кэш 60 с, таймаут 15 с | Только когда Codex отвечает в любой сессии; после `resets_at` окно показывается «—» и «ждём свежих данных» (`usage-format.ts:122-125`) |
| Окна | По `windowDurationMins` (≤ 1440 → 5h, иначе неделя) | По `window_minutes` (300 → 5h, 10080 → 7d, иначе Nh/Nd), то же по смыслу |
| Сверх окон | `planType`, `rateLimitResetCredits.availableCount`, ближайший `expiresAt` | В rollout есть `plan_type` и `credits` (видно в фикстуре `codex.rs:758-759`), но не читаются |
| Стоимость | Запуск процесса Codex (Node) на каждое обновление | Ноль: новых чтений файлов нет |

Вывод: пассивное чтение Mayak — правильная основа (0 % CPU, ничего не запускаем). Что можно добавить:
- дёшево: показывать `plan_type` из того же `token_count` (ни одного нового чтения), как подзаголовок «Codex · plus»;
- по запросу: `usage_refresh` (`lib.rs:327`) сейчас обновляет только Claude; для Codex можно запускать `codex app-server` один раз по кнопке. Сначала spike: путь к `codex` на Windows (npm ставит `codex.cmd`, его нельзя запустить напрямую через `CreateProcess` без `cmd /c` или без разрешения пути к JS-скрипту), поддерживает ли установленная версия метод `account/rateLimits/read` (известно только из кода upstream, нами не проверено), не открывает ли `app-server` окно консоли (`CREATE_NO_WINDOW`). В spec ядра (стр. 103) уже отмечено, что окна IDE держат свои приватные stdio app-server, так что такой запуск согласуется с устройством Codex.

### (c) Безопасность settings.json: #294 (#243) и писатель хуков Mayak

| Требование #243 | Mayak (`hookfile.rs`, `hooks.rs`, `codex_hooks.rs`) |
|---|---|
| Нечитаемый или невалидный файл не заменяется на `{}` | Есть: `parse_object` возвращает ошибку, `read_value` делает из неё заметку (`hooks.rs:182-196`); тест `unreadable_content_is_an_error_never_an_empty_object` |
| Пустой файл и пробелы = пустой объект | Есть (тест `empty_and_whitespace_files_start_from_nothing`) |
| BOM | Есть: снимается и сохраняется при записи (`render`) |
| Отказ, если файл изменился после превью | Есть: отпечаток в `write_plan` (`hookfile.rs:380`) |
| Бэкап обязателен до записи и никогда не перезаписывает предыдущий | Есть: `write_new` с `create_new`, имя с секундами |
| Атомарная запись | Есть: `write_synced` + `rename`, temp удаляется при ошибке |
| Неожиданный тип `"hooks"` или списка события → отказ | **Codex да** (`check_shape`), **Claude нет**: `merged` берёт `as_object().unwrap_or_default()` и `as_array().unwrap_or_default()` (`hooks.rs:142-149`) |
| Симлинк сохраняется | Нет: `rename` поверх симлинка заменит ссылку файлом. На Windows симлинк на `settings.json` редкость (нужны права или режим разработчика), низкий приоритет; если делать, то через `std::fs::canonicalize` перед записью |
| Права файла сохраняются | Неприменимо (ACL Windows наследуются от каталога при rename внутри одного каталога) |

Итог: писатель Mayak строже upstream во всём, кроме проверки формы для Claude. Это топ-2.

### (d) Падение через час после правки файла

Причина в upstream: Swift-специфичная (код `@MainActor` на глобальной очереди, проверка изоляции Swift 6). В Mayak:
- таймеров «через час» нет (`grep 3600|60 * 60` по `src-tauri/src` и `src` находит только константы `files.rs`);
- выполнение кода «не на том потоке» Rust не пропускает на этапе компиляции, а `Mutex` защищает общее состояние;
- **но** в `windows/Cargo.toml:16` стоит `panic = "abort"`: любая паника в любом фоновом потоке (наблюдатель, `stats_query`, опрос курсора) завершает всё приложение, как и ловушка Swift. Это сознательный выбор ради быстрого старта релея, но он же применяется к приложению. Аналог класса ошибки — `unwrap()`/`expect()`/индексация на внешних данных в долгоживущих потоках. Беглая проверка срезов строк (`[..n]`) нашла защищённые места (`is_char_boundary` в `focus/`). Полного аудита `unwrap` не делалось (в не-тестовом коде около 47 `.lock().unwrap()`; при `abort` отравления мьютекса не бывает, процесс завершается раньше).
- Идея: отдельный профиль для приложения (`panic = "unwind"` только у `mayak`, `abort` у `mayak-hook`) не настраивается в Cargo для отдельных пакетов одного профиля; альтернатива — `std::panic::set_hook` с записью в `mayak.log`, чтобы падение хотя бы оставляло след. Это идея, не общий баг.

### (e) Новые агенты: что нужно наблюдателю Mayak

Сейчас Mayak знает два источника: `Source::Claude`, `Source::Codex` (`observe/model.rs:30`), релей знает `Agent::Claude`/`Codex` и отбрасывает остальные `--agent` (`hook/src/lib.rs:52-55, 68`). Модель сессий строится из файлов агентов (транскрипты Claude, rollout Codex) плюс хуки Codex.

| Агент | Как его подключил upstream | Что нужно Mayak |
|---|---|---|
| Copilot CLI | `~/.copilot/hooks/coucou.json`: camelCase-события (`sessionStart`, `userPromptSubmitted`, `preToolUse`, `permissionRequest`, `agentStop`, `sessionEnd`), записи `{"bash": …, "timeoutSec": N}`, `version: 1`; на Windows заявлен через `--agent copilot` | Новый `Source::Copilot`: либо только по хукам (сессия живёт от `sessionStart` до `sessionEnd`, этапы из `preToolUse`), либо парсер его файлов сессии, если они есть (не проверялось). В релей: `Agent::Copilot` и нормализация имён событий. Писатель отдельного файла (`hookfile` подходит: превью, бэкап, отпечаток). Ответ на `permissionRequest` не давать (у Copilot «fail-closed»: на таймаут upstream отвечает `{"permissionDecision":"ask"}`; Mayak должен всегда сразу выходить, ничего не решая) |
| Muse Code | Слияние в `~/.config/muse/settings.json`, PascalCase-события как у Claude, `schema_version: 1` для нового файла | Похоже на Claude: тот же формат хуков, но другой файл и, вероятно, свой формат транскрипта. Продукт нами не проверялся, всё по описанию PR |
| OpenCode | JS-плагин `~/.config/opencode/plugins/coucou.js` (`session.created`, `session.idle`, `tool.execute.before/after`, `permission.asked`), запуск релея fire-and-forget. Upstream пишет, что на Windows не работает (`/bin/sh` и пути macOS) | Генерация плагина, который запускает `mayak-hook.exe` (Windows-путь), плюс хук-источник. Плагин — исполняемый код в чужом конфиге: только с превью и меткой «generated by Mayak», как upstream проверяет перед удалением |
| Amp | TS-плагин `~/.config/amp/plugins/coucou.ts`; `tool.call` всегда возвращает `{action:'allow'}` | Тот же путь, что OpenCode. **Не копировать `allow`**: если API Amp требует ответа, выяснить нейтральный ответ, иначе это одобрение без клика |
| Hermes | Python-плагин + слияние YAML (`~/.hermes/config.yaml`) своим парсером; одобрение из острова ждёт `register_approval_transport`, которого в Hermes 0.15 нет | Самый дорогой: YAML-слияние без зависимостей. На Windows upstream не поддерживает. Пропустить, пока нет спроса |

Общая оценка: каждый агент = отдельный подпроект (spec → план): вариант модели «сессия только по хукам» (у Mayak сейчас каждая сессия опирается на файл), источник в `observe/`, событие в релее, писатель конфига с превью. Дешевле всех Copilot CLI (обычные хуки JSON, Windows поддерживается). Начать стоит со spike: какие файлы сессий пишет каждый агент на Windows.

### (f) Итоги недели и вкладка Stats

| | Upstream (#264) | Mayak Stats (`stats.rs`, `views/stats.ts`) |
|---|---|---|
| Данные | Свой журнал `recap.json` из хуков: ходы (агент, проект, начало, конец), изменённые файлы, строки ±, команды Bash, вопросы, решения по разрешениям; 12 недель | Токены (input, output, cache read/write, запросы) по дням, проектам, агентам и моделям; считается из файлов агентов, кэш `stats-cache.json`; 7 или 30 дней |
| История | Только с момента включения | Задним числом, сколько хранят агенты |
| Показ | Карточка в понедельник ≥ 8:00, картинка для шаринга, «скрыть имена проектов» | Вкладка по запросу |

Что взять как идеи (без кода): метрики «время в работе», «сессий», «файлов и строк изменено», «команд», «самый загруженный день», «самая долгая сессия». Mayak может посчитать их из тех же транскриптов, без нового журнала: длительность хода — по временным меткам записей, файлы и строки — по входам `Edit`/`Write`/`apply_patch`. Карточка «итоги недели» по понедельникам — по желанию, с соблюдением тихого режима. Картинку для шаринга не делать (нужен рендер в PNG, ценность низкая).

### (g) 10 языков upstream и 7 языков Mayak

- Upstream: en, ar, bn, es, fr, hi, id, pt-BR, ru, zh-Hans; 305 ключей, все заполнены. Mayak: en, ru, de, fr, es, zh, ja; около 282 ключей в `en.ts`. Общие: en, ru, fr, es, zh.
- Строки почти не пересекаются: у upstream тексты macOS-интерфейса (интеграции, плагины, Music), у Mayak свои (наблюдатель, помодоро, Stats, Apps). Переносить файлы нельзя, можно использовать как глоссарий для общих понятий: «Итоги недели», «План Claude», «5 часов», «Неделя», «через %d ч %d мин», «только что».
- Русские формулировки upstream не лучше наших: «Отклонить» против нашего «Запретить» — оба допустимы; в upstream формы множественного числа упрощены («сессии» для любого числа ≥ 2), у нас таких строк со счётчиком пока нет. Если появятся, нужен `Intl.PluralRules` (в `i18n-core.ts` его нет).
- Языки, которые можно добавить дёшево: **pt-BR**, **id** (латиница, без RTL). hi и bn потребуют проверки шрифтов в WebView2. **ar** — RTL, нужен `dir="rtl"` и проверка всей вёрстки острова; не стоит без запроса.
- Перевод выглядит машинным (не проверялось носителями); брать как черновик, не как эталон.

### (h) Лицензия и ассеты (#275) и ребрендинг §26

- #275 только уточнил тексты (README, сайт, бейдж «Code: MIT»). Условия задаёт `LICENSE-ASSETS.md`, который у нас уже есть: под MIT только код; имена «Coucou» и «Mochi», персонаж Mochi («design, look, expressions and animations»), иконки, звуки (`NotchBuddy/Resources/sounds/`), медиа в `docs/media/` и `design/` — «all rights reserved». Распространять форк с ними нельзя без письменного разрешения; собирать для себя можно.
- Что в Mayak сейчас попадает под это:
  - звуки: `windows/vite.config.ts:9` берёт `../NotchBuddy/Resources/sounds` и кладёт в `dist/sounds` (29 файлов);
  - иконки: `windows/src-tauri/icons` побайтно совпадают с upstream `83708fe`;
  - персонаж: `windows/src/mochi/` (рисуется кодом, но защищён именно облик персонажа, а не способ рисования);
  - имя: «Mochi» в видимых строках (`set.integrationsNote` во всех словарях и, возможно, ещё где-то: полный поиск не делался).
- Для §26: свой персонаж (новый движок вместо `mochi/engine.ts`), свои звуки (и смена `SOUNDS_DIR`), свои иконки (`npm run icons` от нового исходника), чистка видимых «Mochi»/«Coucou». Сохранить `LICENSE` с копирайтом Louis Raillé для кода, добавить свой копирайт на новые ассеты. Функциональные упоминания Coucou (миграция настроек, распознавание старых хуков) — не бренд, оставить.

## Что не проверялось

- Сборка, `cargo test`, `npm test` не запускались; выводы по коду Mayak сделаны чтением.
- Таймер `island.ts:886` (топ-3) и сброс настроек при BOM/неверном поле (топ-1) не воспроизводились на живом приложении; второе прямо следует из `unwrap_or_default` и известного поведения `serde_json`.
- `codex app-server` и `account/rateLimits/read` на Windows не запускались.
- Форматы Copilot CLI, Muse, OpenCode, Amp, Hermes известны только из PR upstream.
- Полный аудит `unwrap`/`expect` при `panic = "abort"` не делался.
- Код Swift прочитан выборочно: CodexPlanGauge, ClaudeSettingsFile, RecapStore (начало), диффы d05f22b, 40e3ba8, ea244a7, efc80c3; остальное по описаниям PR и спискам файлов.
