# Обзор upstream (Louis-CFM/coucou) для Mayak на Windows, часть 4

Дата: 2026-10-09. Диапазон: `a7bd575..upstream/main` (`c946650`): 23 коммита, из них 10 на первом родителе и 10 не-слияний. Теги в диапазоне: `v0.2.3` (Mac), `windows-v0.3.0`, `linux-v0.3.0` (и плавающий `linux-latest`). Изменения: 62 файла, +6 701/−245, почти всё в `windows/`, но по смыслу период линуксовый: из 7 PR в `windows/` четыре только для Linux (#367, #371, #372, #379), а для Windows новые только «открытие по наведению» и «свои звуки» (#366) и шорткаты #370. Предыдущий обзор: `2026-10-08-upstream-review.md` (`89477b4..a7bd575`). Тексты PR (`gh pr view 353 355 366 367 370 371 372 373 376 379`) использованы только как данные. Ничего не менялось и не коммитилось, кроме этого файла; сборка, `cargo test` и `npm run check` не запускались. Номера строк Mayak — по `main` на `f496928`.

Что стало с топ-5 прошлого обзора: пункт 1 сделан (`ingest_file` в `lib.rs` больше нет, файлы чата только из inbox), пункт 2 сделан (`vite.config.ts:54`, `watch.ignored`), пункт 3 сделан (`focus/mod.rs:90,112`: `pick_terminal` для `HostKind::Unknown`, обход предков с проверкой времени создания в `focus/win.rs:73-92` — строже, чем у upstream). Сделаны и мелочи оттуда: AltGr-шорткаты, перезапуск отсчёта автозакрытия (`fsm.ts:170-175`, проверка `scripts/checks/fsm-delay.mjs`), симлинки конфигов, `--locked` в CI (`windows-test.yml:65`). Открыты пункт 4 (лимиты Codex через `codex app-server`: в коде Mayak нет ни `app-server`, ни `account/rateLimits/read`) и пункт 5 (свои звуки). По пункту 5 в этом периоде у upstream появилась готовая Windows-реализация (см. топ-1). MemPalace по запросу «Mayak sounds open on hover upstream coucou» ничего относящегося к Mayak не вернул.

## Топ-5 рекомендаций

| # | Рекомендация | Действие | Связь |
|---|---|---|---|
| 1 | **Свои звуки (#366, 2261334): у upstream теперь есть код для Windows.** `src-tauri/src/sounds.rs` (100 строк с тестами): папка `config_dir()/sounds`, имя звука — только строчные ASCII-буквы до 32 символов (никаких `..`, `/`, расширений), 10 расширений по порядку (`wav`, `mp3`, `ogg`, `oga`, `m4a`, `aac`, `flac`, `aiff`, `aif`, `caf`), файл не пустой и не больше 8 МБ, байты уходят в страницу через `tauri::ipc::Response` (без asset-протокола и без расширения CSP). В `core/sound.ts` файл пользователя декодируется через `decodeAudioData`; если не вышло, остаётся встроенный звук. В Settings — «Open sounds folder», «Reload sounds» и счётчик заменённых. В Mayak звуки берутся только из `/sounds/<name>.wav` (`core/sound.ts:40`), а сборка — из `../NotchBuddy/Resources/sounds` (`vite.config.ts:9`), то есть звуки upstream под «all rights reserved» | **take** (код MIT, копирайт Louis Raillé уже в нашем `LICENSE`): `sounds.rs` почти без правок (`platform::config_dir()` у Mayak уже даёт `%APPDATA%\Mayak`, `platform/windows.rs:35`; `reveal_folder` есть, строка 82), загрузчик из `sound.ts`, кнопки в существующем блоке звука Settings (`settings/main.ts:1028`), строки в 7 словарях. Это первый шаг подпроекта Б (§26): своё переопределение работает уже сейчас, а когда появится свой набор WAV, сменится только `SOUNDS_DIR`. Пустые `catch {}` из upstream заменить записью в лог (`Bridge.log`) | #366, прошлый топ-5, ideas §26 |
| 2 | **Порядок `wasInIsland` (тот же класс, что исправлен в 2261334; у Mac это `previouslyInIsland`).** В `onCursor` флаг пишется после вызовов FSM (`island/island.ts:1405-1420`), а обработчики переходов `petit` и `home` читают его (`island.ts:450, 461`: `if (!this.wasInIsland) this.fsm.mouseLeft()`). Если переход происходит внутри `mouseEntered`, остров видит курсор «ещё снаружи» и сразу запускает отсчёт закрытия. Сейчас это не проявляется: из `hidden` Mayak будит полоска (`onStripTouch`, `island.ts:1325`), а пока остров скрыт, опрос курсора остановлен (`PollGate`, `src-tauri/src/island.rs:87-130`), так что `mouseEntered` с переходом из `onCursor` не приходит. Ошибка станет живой при любом переходе по наведению (топ-3) | **take** (3 строки): `const wasIn = this.wasInIsland; this.wasInIsland = inIsland;` до вызовов FSM, как у upstream. Обязателен перед топ-3 | 2261334 |
| 3 | **Открытие по наведению (#366, 2261334), по желанию.** Настройка `openOnHover` (по умолчанию выкл.): наведение на `hidden`/`petit` сразу открывает `home`, но не при `pinned`; после ухода курсора остров сворачивается через 0,6 с; клик внутри (`mousedown` → `fsm.userInteracted()`) делает остров обычным, и короткий отсчёт заменяется обычным; `forceHome`/`forcePetit`/`forceHidden` и истёкший таймер сбрасывают флаг. Шесть проверок в `tests/fsm.test.mjs`. У Mayak та же FSM (`island/fsm.ts:53-70, 177-184`) | **adapt**: перенести поле `byHover`, ветку в `mouseEntered`, `userInteracted`, задержку в `scheduleHomeCollapse`. Своё у Mayak: (а) полоска обратного отсчёта считает `autoCloseInterval` сама (`island.ts:267, 1416`), а нужна фактическая задержка FSM (у upstream для этого есть `homeCollapseDueAt`); (б) решить, открывает ли касание полоски `home` сразу (у верхнего края экрана возможны случайные касания); (в) при открытии страница по-прежнему из `openOnClick` (`State.openView()`); (г) шесть случаев — в `scripts/checks/` на поддельных таймерах, как `fsm-delay.mjs` | #366, прошлый 2(b) |
| 4 | **Плеер: позиция, перемотка, shuffle и repeat по образцу карточки Spotify (#372).** Линуксовая часть (MPRIS) Mayak не нужна, но страница upstream даёт готовые правила: позиция — `position` + `positionAt` и экстраполяция по времени без опроса (`spotifyPosition`), ограничение длительностью, `withPlaying` замораживает часы сразу по клику; громкость отправляется не чаще раза в 120 мс; полоса перерисовывается дважды в секунду только пока карточка видна и музыка играет. У Mayak плеер через SMTC знает только название, исполнителя, приложение, play/pause, next/prev и обложку (`media.rs:23-33`) | **idea only**: если расширять плеер — `TimelineProperties` и `TryChangePlaybackPositionAsync`/`TryChangeShuffleActiveAsync`/`TryChangeAutoRepeatModeAsync` у сессии SMTC, на странице правила `spotifyPosition`. Громкости приложения в SMTC нет. Перед работой проверить, что отдаёт Spotify desktop через SMTC | #372, прошлый 2(f) |
| 5 | **Шорткаты внутри острова (#370).** Upstream добавил `Ctrl+↓`/`Ctrl+↑` по списку GitHub с подсветкой, `Ctrl+O` открывает выделенную строку, `Ctrl+E` — дифф; в текстовом поле стрелки остаются полю. У Mayak внутри острова есть только `Esc` (`island.ts:1289-1292`), списков GitHub и диффа нет | **idea only**: тот же разбор `islandKeyAction` для списка сессий Mayak (стрелки по сессиям, `Ctrl+O` = «Открыть»), если остров получает клавиатуру. Отдельный вопрос, потому что Mayak неактивирующее окно; разумно только вместе с хоткеем, который открывает остров | #370 |

Сводно: для Mayak в этом периоде два пункта — свои звуки (готовый код, прямо под подпроект Б) и порядок `wasInIsland` (дешёвый и обязательный перед открытием по наведению). Остальное — Linux (Wayland-портал, «Open terminal» через `/proc`, KWin и EWMH, Spotify через MPRIS), десктопный Mochi, Mac (диктовка, релиз 0.2.3, тесты) и версии с документацией. В Windows-части upstream нет исправлений багов, которые Mayak ещё несёт: изменения `session_window.rs` и `platform/windows.rs` в #371 — это перестройка (`Vec<u32>` на сессию), а не исправление, а `anywhere` в `desktop.rs` касается только Mochi на рабочем столе, которого у Mayak нет.

## 1. Коммиты и PR upstream

Первый родитель: `0571a28` (#353) → `5cb2a27` (#355, 0.2.3) → `892ffb3` (#366) → `9dbad74` (#367) → `af6c4ed` (#370) → `067c719` (#371) → `873f39d` (#372) → `637b576` (#373, Windows & Linux 0.3.0) → `fb9674d` (#376) → `c946650` (#379).

### 1.1. Windows & Linux 0.3.0 (#373) по частям

| Коммит / PR | Что делает | Часть | Есть в Mayak? | Вердикт |
|---|---|---|---|---|
| 2261334 (#366), свои звуки | `sounds.rs`: файл с именем звука в `config_dir()/sounds` заменяет встроенный; проверка имени (только строчные буквы), 10 расширений, ≤ 8 МБ; команды `custom_sounds`, `custom_sound` (сырые байты), `reveal_sounds_folder`, `reload_sounds` (событие `sounds-changed`); `sound.ts` декодирует с откатом на встроенный; Settings: папка, перечитать, счётчик; 2 Rust-теста | Windows, Linux | Нет: только встроенные WAV upstream (`core/sound.ts:26-50`, `vite.config.ts:9`) | **take**: топ-1 |
| 2261334 (#366), открытие по наведению | `openOnHover` в настройках (Rust и TS), `byHover`/`userInteracted`/`hoverCloseDelay = 0.6` в `fsm.ts`, `mousedown` на острове снимает режим, `wasInIsland` записывается до FSM; 6 тестов FSM | Windows, Linux | Нет. Есть выбор страницы при открытии (`openOnClick`, `core/state.ts:145,174`) и полоска скрытого острова | **adapt** (по желанию): топ-3; порядок `wasInIsland` — **take**: топ-2 |
| 2261334 (#366), строки | Шесть новых строк в Mac-каталоге на 10 языков, перегенерирован `strings.json` | Mac, Windows | Своё i18n (7 языков, типизированные ключи) | skip; свои строки писать в `core/i18n/*.ts` |
| 97f9d66 (#367) | Linux/Wayland: глобальные шорткаты через XDG GlobalShortcuts portal (`portal.rs`, 908 строк, libdbus, поток в `poll()` без таймаута), поколения привязок, статусы `pending`/`refused`, «Desktop: …» в Settings | Linux | Хоткей Mayak только для Windows (`hotkey.rs:1-4`: macOS и Linux отвечают «не поддерживается») | skip; если Linux вернётся — брать первым, это единственный способ шорткатов на Wayland |
| 322e2ab (#370), `desktopToggle` | Глобальный `Ctrl+Alt+D`: Mochi на рабочий стол и обратно (`flyOutOrHome`), ворчание там, где он не может выйти (GNOME/Wayland); `desktop_mochi_fly_out(anywhere)` — угол экрана, если сохранённое место на отключённом дисплее | Windows, Linux | Персонажа на рабочем столе у Mayak нет (Луч живёт в острове) | skip |
| 322e2ab (#370), клавиши острова | `Ctrl+↓`/`Ctrl+↑` по спискам GitHub (подсветка белым 12 %, `.gh-row.selected`), `Ctrl+O` — открыть строку (с проверкой github.com), `Ctrl+E` — дифф в обзоре; в поле чата стрелки остаются полю; 9 тестов | Windows, Linux | Внутри острова только `Esc` (`island.ts:1289-1292`); списков GitHub и диффа нет | **idea only**: топ-5 |
| 574deec (#371), Linux | «Open terminal» на Linux: PID релея через `SO_PEERCRED`, предки из `/proc/<pid>/stat` (имя до последней `)`), `LINUX_TREE_TOPS`; подъём окна KWin-скриптом по D-Bus, EWMH через x11rb, `kitten @ focus-window`; иначе VS Code. Цепочка перечитывается перед использованием, чтобы не поднять чужой переиспользованный PID | Linux | `platform/linux.rs` есть, но не сопровождается | skip |
| 574deec (#371), Windows-часть | `session_window`: сессия хранит `Vec<u32>` вместо одного PID, `window_owners`, `focus_session_window`; поведение на Windows то же (в PR отмечено, что на Windows это не компилировалось) | Windows | Своё в `focus/`: обход предков с проверкой, что родитель старше ребёнка (`focus/win.rs:70-92`), — от переиспользованных PID защищено | skip |
| c810767 (#372), Spotify | Линукс-пилюля Spotify через MPRIS (`spotify.rs`, 1 503 строки): без опроса, обложки скачивает Rust (только CDN Spotify, без редиректов, ≤ 3 МБ, проверка типа по байтам, кэш 12), управление seek/shuffle/repeat/volume; на странице `core/spotify.ts`, `views/spotify.ts`; поддержка `support: "linux"` в каталоге | Linux | Плеер Mayak через SMTC видит любой плеер, обложка из SMTC без сети (`media.rs`, `media/win.rs`) | skip (MPRIS); **idea only** (позиция, перемотка, shuffle/repeat по правилам страницы): топ-4 |
| c810767 (#372), танец | `BotEngine.setDancing`/`applyDance` из Mac: 112 BPM, покачивание, прыжок, наклон, «счастливые глаза»; правила: компактный остров при музыке, раскрытый — только на обзоре с пилюлей Spotify; на Windows «никто не сообщает о музыке», так что ничего не меняется | Windows, Linux | У Луча своя реакция на музыку: наушники и ноты (`island/activity.ts:15-18`, `luch/engine.ts:730`) | skip (код Mochi и его анимация); **idea only**: лёгкое покачивание пламени Луча в такт (без анализа звука, просто ритм при `playing`) — решает дизайн Луча |
| 60adb44 (#373) | Версии 0.3.0 в `package.json`, `package-lock.json`, `tauri.conf.json`, `Cargo.toml/lock`, PKGBUILD; раздел CHANGELOG | Windows, Linux, docs | Свои версии и CHANGELOG | skip |

### 1.2. Остальные PR периода

| Коммит / PR | Что делает | Часть | Есть в Mayak? | Вердикт |
|---|---|---|---|---|
| c475a21 (#353) | Mac-диктовка в чате: до трёх распознавателей на устройстве (языки Mac, раскладки, английский), берётся самая уверенная расшифровка; правый клик по микрофону — фиксированный язык | Mac | Чат выключен (`CHAT_ENABLED = false`); на Windows есть системная Win+H | skip |
| fa8747a (#355, 0.2.3) | Mac 0.2.3: версия, CHANGELOG (терминальные сессии, Spotify, цвета, диктовка, открытие по наведению, свои звуки, Antigravity `ask`) | Mac, docs | Содержимое разобрано в прошлом обзоре (#326, #330, #331, #332, #333, #329) | skip |
| 0e6b83f (#376) | Mac-тест автозакрытия: два случая с гонкой короткого таймера и короткого `sleep` разнесены, ожидания 5 с вместо 1 с | Mac, CI | Проверки Mayak идут на поддельных таймерах (`scripts/checks/fsm-delay.mjs`), гонок с реальным временем нет | skip; правило: проверки открытия по наведению (топ-3) тоже писать на поддельных таймерах |
| 301fbfb (#379) | README и сайт: ссылки на Linux 0.3.0 (AppImage, .deb, .rpm, `linux-latest`), таблица версий Windows и Linux, страницы `index.html` и `support.html` | docs, site | Свои README; сайт upstream Mayak не использует | skip |
| 5cb2a27, 892ffb3, 9dbad74, af6c4ed, 067c719, 873f39d, 637b576, fb9674d, c946650, 0571a28, 52f2d37, a7c7ca6, 608f559 | Служебные слияния | — | — | skip |

## 2. Детали

### (a) Общие с upstream баги в `windows/` Mayak

1. **`wasInIsland` пишется после FSM (топ-2).** `island.ts:1405-1420`:
   - `crossed` и обе ветки читают старое значение, `this.wasInIsland = inIsland` стоит в строке 1420;
   - обработчик перехода в `petit` (`island.ts:450`) и в `home` (`island.ts:461`) вызывает `fsm.mouseLeft()`, если `!this.wasInIsland`;
   - переход внутри `fsm.mouseEntered()` сейчас возможен только `hidden` → `petit` (`fsm.ts:53-60`). Через `onCursor` он не происходит, потому что опрос курсора останавливается, пока остров скрыт, а `onStripTouch` (`island.ts:1325`) вызывается при курсоре на полоске, не на острове. Поэтому сегодня ошибка спит.

   Исправление то же, что у upstream: запомнить старое значение, записать новое, потом звать FSM. `crossed` считать от сохранённого значения. Проверить, что ничего в ветке ухода (`island.ts:1413-1418`) не зависит от старого значения поля после вызова.
2. **Других исправлений Windows-багов в диапазоне нет.** Просмотрены диффы `2261334`, `322e2ab` (Rust и TS), Windows-часть `574deec` (`lib.rs`, `pipe.rs`, `platform/windows.rs`, `session_window.rs`). `anywhere` в `desktop.rs` чинит случай «сохранённое место на отключённом дисплее» только для десктопного Mochi.

### (b) Свои звуки (топ-1) для Mayak

- **Rust.** `windows/src-tauri/src/sounds.rs` из upstream почти без правок: `folder()` = `platform::config_dir().join("sounds")` (у Mayak `%APPDATA%\Mayak\sounds`), четыре команды в `invoke_handler`, оба теста. `reveal_sounds_folder` у upstream глотает ошибку `create_dir_all` (`let _ =`) — у Mayak записать её в лог (`crate::log::line`).
- **Страница.** `SoundEngine.loadAll(ctx)` из upstream: `Bridge.customSounds(SOUND_NAMES)` → для каждого имени из списка `Bridge.customSound(name)` → `decodeAudioData`; при ошибке — встроенный `/sounds/<name>.wav`. `reload()` перечитывает, событие `sounds-changed` в `main.ts`. У upstream `reload()` при первом вызове грузит всё дважды (`preload()`, потом `loadAll`) — мелочь, можно не повторять.
- **CSP.** Ничего менять не нужно: байты идут через IPC (`connect-src ipc:` уже есть, `tauri.conf.json:15`), `decodeAudioData` не трогает `media-src`.
- **Settings.** В существующий блок звука (`settings/main.ts:1028`): две кнопки и счётчик, подсказка про имена файлов. Стиль — как у соседних строк, новых классов не нужно. Строки — в 7 словарях `core/i18n/*.ts` (проверка ключей `i18n-keys.mjs`).
- **Подпроект Б.** Это не заменяет свой набор звуков: встроенные по-прежнему звуки upstream. Но (1) у пользователя сразу появляется своя замена, (2) когда будет свой набор WAV, остаётся сменить `SOUNDS_DIR` (`vite.config.ts:9`), (3) сборка без встроенных звуков уже поддерживается (`vite.config.ts:32-36`), так что до набора можно выпускать сборку только со звуками пользователя. Имя `greeting` (29-й звук у upstream) Mayak не нужно, у нас 28 имён (`core/sound.ts:6-11`); звук приветствия Луча — часть подпроекта Б.

### (c) Открытие по наведению (топ-3) в FSM Mayak

| | Upstream (Windows, 2261334) | Что учесть в Mayak |
|---|---|---|
| Вход | `mouseEntered`: при `openOnHover`, состоянии `hidden`/`petit` и `!pinned` → `cancelTimers`, `byHover = true`, `home` | `collapsedByButton = false`, как в обычной ветке `hidden` (`fsm.ts:55-57`); `holdVisible` не мешает |
| Задержка | `scheduleHomeCollapse`: `byHover ? 0.6 : homeDelay` | В Mayak таймер не ставится при `pinned` или `heldOpen` (`fsm.ts:179`) — это сохраняется |
| Клик | `mousedown` → `userInteracted()`: идущий короткий отсчёт становится обычным | Тот же `mousedown` на `islandEl` |
| Сброс | `click`, `forceHome`, `forcePetit`, `forceHidden`, истёкший таймер | В Mayak добавить и `launch` |
| Полоска отсчёта | `homeCollapseDueAt` в FSM | `island.ts:267, 1416` считают `autoCloseInterval` — брать фактическое время из FSM |
| Полоска скрытого острова | Нет | `onStripTouch` → `mouseEntered`: с опцией сразу `home`. Решить отдельно (возможно, из полоски только `petit`) |
| Страница | `State.openView()` | То же (`openOnClick`, ideas §28) |

Проверки: шесть случаев upstream (`выкл. — только peek`, `уход → 0,6 с`, `возврат до 0,6 с`, `клик → обычная задержка`, `ожидающая карточка держит`, `открыт тревогой → обычная задержка`) в `scripts/checks/fsm-hover.mjs` на поддельных таймерах, как `fsm-delay.mjs`.

### (d) Шорткаты (#367, #370)

- У Mayak один глобальный хоткей (`hotkey.rs`, по умолчанию `Alt+M`) показывает и скрывает остров; модель «одно действие» выбрана сознательно (spec `2026-10-06-mayak-hotkey-design.md`). Каталог из 10 действий upstream (`shortcuts.rs`) переносить незачем.
- `Ctrl+Alt+D` проверен upstream как не-AltGr «на проверенных раскладках»; Mayak это не касается, но наша проверка AltGr уже есть.
- Wayland-портал (#367) нужен, только если Mayak когда-нибудь будет поддерживать Linux.

### (e) Музыка: Spotify (#372) и плеер Mayak

- Mayak уже лучше покрывает Windows: SMTC видит любой плеер без сети. Upstream на Windows «ничего не сообщает о музыке» — Mochi там не танцует, пилюли Spotify нет.
- Брать правила страницы, не код: позиция с экстраполяцией (`spotifyPosition`), заморозка часов при play/pause до ответа плеера (`withPlaying`), склейка громкости раз в 120 мс, таймер перерисовки только при видимой карточке и играющей музыке (0 % CPU при скрытом острове сохраняется).
- Загрузка обложек в Rust с белым списком хостов, без редиректов и с проверкой типа по байтам — хороший образец, если Mayak когда-нибудь будет брать обложки по сети. Сейчас не нужно: обложка приходит из SMTC (`media.rs:89`, `cover_data_url`).

### (f) Linux (#367, #371, #372, #379)

Все четыре — только Linux. Если Linux в Mayak оживёт, порядок: портал шорткатов (#367), «Open terminal» (#371: `parse_stat`, `LINUX_TREE_TOPS`, KWin-скрипт, EWMH), MPRIS (#372). Зависимости `dbus` 0.9 и `x11rb` 0.13 у upstream уже были в `Cargo.lock` через `keyring` и `global-hotkey`; у Mayak своё дерево зависимостей, это надо проверить заново.

## Что не проверялось

- Сборка, `cargo test`, `npm run check` не запускались; выводы по коду Mayak сделаны чтением.
- Топ-2 не воспроизводился: вывод «сейчас ошибка спит» опирается на то, что опрос курсора останавливается при скрытом острове (`PollGate` в `island.rs`) и что из `hidden` будит только полоска; путь `followPageCursor` (запуск вне Tauri) не разбирался.
- `decodeAudioData` в WebView2 для `ogg`, `flac`, `aiff`, `caf` не проверялся; у upstream проверены только Rust-тесты папки (по описанию PR: `cargo test` 204/204 на Linux).
- Методы SMTC для позиции, перемотки, shuffle и repeat на живом Spotify не проверялись.
- Код Swift (#353, #376) прочитан только по сообщениям коммитов и описаниям PR; `portal.rs`, `linux_focus.rs`, `spotify.rs` (Rust) — по описаниям и спискам файлов, без чтения целиком; `views/spotify.ts` не читался, `core/spotify.ts` прочитан.
