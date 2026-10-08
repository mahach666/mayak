# Claude Desktop: как открыть конкретную Code-сессию по deep link

Дата: 2026-10-08. Claude Desktop `Claude_2.26454.2.0_x64__pzs8sxrjxfjjc` (MSIX), Windows 11.
Метки: **[наблюдено]** — видно в коде asar или в данных на диске; **[документировано]** — есть в официальной справке; **[неизвестно]** — не проверялось или проверить не удалось.

Код Claude Desktop не запускался. Asar распакован в `scratchpad\claude-asar`. Обработчик ссылок лежит в `.vite\build\index.chunk-CJN9s-60.js` (функция `fqn`, ветка `case fu.Code` и `case pu.Resume`). Менеджер сессий лежит в `.vite\build\index.chunk-CFhdElXw.js`.

## 1. Маршруты `claude://`, где есть id сессии, проекта или папки

| URL | Что делает | Метка |
|---|---|---|
| `claude://code/continue?session=local_<id>` | Ищет неархивную сессию, у которой `sessionId === <id>` (точное совпадение), и переходит на её страницу `/epitaxy/local_<id>`. Если сессия не найдена, открывает `/epitaxy` (главная страница Code). `session=last` открывает сессию с самой поздней `lastActivityAt`. | [наблюдено] |
| `claude://code/needs-input?session=local_<id>` | Ищет сессию только среди тех, у которых есть `pendingToolPermissions`. Если среди них её нет, открывает **первую** ожидающую сессию, то есть может открыть чужую. | [наблюдено] |
| `claude://resume?session=<uuid CLI-сессии>` | Вызывает `importCliSession(uuid)`. Если сессия уже принадлежит Desktop, возвращает её id. Если нет, **импортирует** её как новую запись. Затем переходит на `/epitaxy/<id>`. | [наблюдено] |
| `claude://code/<cse_… или session_…>` | Удалённая (облачная, bridge) сессия claude.ai. Ищет локального «двойника» через `findSessionIdByBridgeSessionIdLoaded`. Для `local_…` не подходит, регулярка `^(cse\|session)_`. | [наблюдено] |
| `claude://code/project/<projectId>?thread=&msg=` | Проект Code. | [наблюдено] |
| `claude://code/new?q=&prompt=&folder=&file=` | Новая сессия. | [документировано], [наблюдено] |
| `claude://code/search` | Поиск. | [наблюдено] |
| `claude://claude.ai/chat/{conversation-id}` | Обычный чат, не Code. | [документировано] |

Фрагменты кода [наблюдено]:

```js
hqn=/^local_[A-Za-z0-9-]{1,64}$/
function vqn(e){return e==="last"||e!==null&&hqn.test(e)}
if(a.pathname==="/continue"||a.pathname==="/needs-input"){ … n=a.searchParams.get("session"); … yqn(e,n,o,t,i) … }
async function Sqn(e){ … n=(await t.getAllSessions()).filter(e=>!e.isArchived),
  r=e==="last"?…:n.find(t=>t.sessionId===e); return r?t.getSessionRoute(r.sessionId):void 0}
function Dj(e){return`${Cj}/${encodeURIComponent(e)}`}   // Cj="/epitaxy"
```

Claude Desktop сам формирует такие ссылки для своих точек входа, например «Continue Last Claude Code Session» в списке переходов Windows [наблюдено]:

```js
url:qI(fu.Code,"/continue",r,{session:c.sessionId})   // → claude://code/continue?session=local_…&source=…
```

### Ограничения маршрута `continue` [наблюдено]
- Функция `yqn` выполняется, только когда `hKn()` возвращает true. Это так, когда не включено `authentication.disableDeepLinks`, не активен режим стороннего провайдера (`Zu()`, не `1p`) и пользователь вошёл в аккаунт. Иначе в лог пишется «code entry deep link ignored (entry points off)».
- Публичный issue claudeissues.com #90035 (Desktop 1.37937.3, macOS) пишет, что в той версии маршрут был закрыт флагом («gated off») и приложение только выходило на передний план. В текущей сборке 2.26454.2.0 такого флага в коде нет, есть только проверка `hKn()` [наблюдено]. Работает ли маршрут во время выполнения, **не проверялось** [неизвестно].
- Параметр `source` необязателен. Неизвестные значения превращаются в `url_external` (это только для телеметрии самого Desktop) [наблюдено].
- Повторный запуск передаёт ссылку через событие `second-instance`. Перед обработкой вызывается `gY()`, которая восстанавливает и фокусирует окно (`restore()`/`focus()`/`show()`) [наблюдено]. Отдаст ли Windows фокус при запуске из Mayak, неизвестно [неизвестно].

## 2. Как Desktop хранит Code-сессии [наблюдено]

Путь: `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude-code-sessions\<accountId>\<orgId>\local_<desktopUuid>.json`. На этой машине 14 файлов и `scheduled-tasks.json`.

Ключи верхнего уровня (значения не читались, кроме id и cwd): `sessionId`, `cliSessionId`, `priorCliSessionIds`, `bridgeSessionIds`, `cwd`, `originCwd`, `title`, `titleSource`, `isArchived`, `isStarred`, `lastActivityAt`, `lastFocusedAt`, `createdAt`, `model`, `permissionMode`, `effort` и другие.

- `sessionId` = `local_<uuid>`. Этот uuid **не совпадает** с id сессии CLI. Это проверено на всех 14 файлах: `local_<cliSessionId>` не встретился ни разу.
- `cliSessionId` = `sessionId` Claude Code, то есть имя транскрипта `~/.claude/projects/<proj>/<cliSessionId>.jsonl`.
- `priorCliSessionIds` — прежние CLI-id той же Desktop-сессии. Например, у `local_da7117cd-…` есть `cliSessionId=3235e946-…` и `priorCliSessionIds=["907f89fc-…"]`.

## 3. Сопоставление сессии Mayak и сессии Desktop

### Способ A, прямой (рекомендуется): `hostSessionId` в `~/.claude/sessions/<pid>.json` [наблюдено]
У сессий с `entrypoint == "claude-desktop"` в файле есть поле `hostSessionId`. Его значение равно `sessionId` Desktop-сессии:

| pid-файл | `sessionId` (CLI) | `hostSessionId` | Файл Desktop с тем же `cliSessionId` |
|---|---|---|---|
| 30024.json | 3235e946-0d64-4356-98c9-f7b8c8949416 | local_da7117cd-3665-4872-af75-dd782d0be925 | local_da7117cd-….json |
| 41664.json | a56e55b7-f40d-4d39-a11d-8951e704a4e5 | local_50ddee22-a91b-4dbc-9f62-6b261a0aa8f0 | local_50ddee22-….json |
| 17472.json | 6f03ac8a-6d53-4031-b00e-7ebeebe9ab15 | local_02ccf4e4-c75e-4472-86e1-db97e790f431 | local_02ccf4e4-….json |

У сессий `claude-vscode` (pid 26808 и 49076) поля `hostSessionId` нет.

Откуда берётся значение [наблюдено]: Desktop при запуске CLI задаёт переменную окружения:
```js
async buildSessionEnv(e, …){ … return d.CLAUDE_CODE_HOST_SESSION_ID=e, … }
```
Здесь `e` — id Desktop-сессии. Сама запись `hostSessionId` в pid-файл происходит в бинарнике Claude Code, его код я не смотрел. Связь подтверждается только совпадением данных в трёх парах [наблюдено].

Сейчас Mayak это поле не читает. В `windows/src-tauri/src/observe/claude.rs` разбираются `sessionId`, `cwd`, `entrypoint` и другие поля, `hostSessionId` среди них нет [наблюдено].

Итоговая ссылка: `claude://code/continue?session=<hostSessionId>`, без дополнительного URL-кодирования. Значение подходит под `^local_[A-Za-z0-9-]{1,64}$`. Перед отправкой стоит проверить его той же регуляркой.

### Способ B, запасной: файлы Desktop [наблюдено]
Найти `local_*.json`, у которого `cliSessionId == sessionId` из Mayak (или этот id есть в `priorCliSessionIds`), и взять оттуда `sessionId`. Минусы: файлы большие (0,4–0,8 МБ), в путь входят `accountId`/`orgId`, а формат внутренний и без документации.

### Способ C: `claude://resume?session=<CLI sessionId>` [наблюдено, опасен]
Сопоставление выполняет сам Desktop. Сначала он ищет `sessions.get("local_"+id)`, потом `sessionWithLocalTranscriptOpen(id)`, то есть сессию, у которой `cliSessionId`/`unarchivedCliSessionId` равен id. Если такая сессия найдена, Desktop **разархивирует** её (`unarchiveSession`) и открывает. Риски описаны ниже.

## 4. Риски

### `code/continue?session=local_…`
- В коде нет отправки сообщения и не создаётся новый чат. Есть только поиск и `aL()` → `dispatchNavigate(route)`. Кроме того, `aL` очищает параметры `q`/`prompt` и удаляет `folder`/`file` [наблюдено].
- Если id не найден или сессия в архиве, открывается `/epitaxy`, главная Code. Это страница новой сессии с пустым полем ввода, но сама сессия не создаётся и ничего не отправляется [наблюдено: строка `aL(o??"/epitaxy",…)`]. Для пользователя это выглядит как «открылось не то».
- Если deep links выключены (managed `disableDeepLinks`), пользователь не вошёл или активен 3P-режим, ссылка тихо игнорируется [наблюдено].
- Запускает ли переход на страницу простаивающей сессии её CLI-процесс, неизвестно [неизвестно].
- Desktop шлёт свою телеметрию `desktop_os_surface_invoked` с `verb:"code_continue"` [наблюдено]. Это телеметрия Desktop, не Mayak.
- Формат внутренний и не документирован: в справке есть только `code/new` [документировано]. Маршрут может измениться или снова закрыться флагом, как в 1.37937.3 по #90035.

### `code/needs-input?session=…`
- Если у сессии нет ожидающих разрешений, открывается другая сессия, самая старая из ожидающих [наблюдено: `n.find(t=>t.id===e)??n[0]`]. Для «Открыть» не подходит.

### `resume?session=<uuid>`
- Если сессия Desktop не принадлежит (обычный CLI, VS Code), она **импортируется**: в боковой панели появляется новая запись. Рабочая папка помечается доверенной без диалога (`saveWorkspaceTrust(m,{withoutDialog:!0})`, `autoTrust` для deep link не задан). У транскрипта обновляется mtime (`lutimes`). Может зарегистрироваться worktree. Проверки «CLI-процесс ещё работает» (`liveOwnershipRefusal`) на этом пути нет, она есть только в `resumeCliSession` из выбора сессии [наблюдено].
- Если сессия в архиве, она разархивируется [наблюдено].
- Issue #90035 описывает дубликаты при `resume` в 1.37937.3. В текущем коде сначала ищется существующий владелец транскрипта [наблюдено], но во время выполнения это не проверялось [неизвестно].

## 5. Публичные источники
- Справка «Open Claude Desktop with a link» (support.claude.com/en/articles/14729294): `claude://code/new`, `claude://cowork/new`, `claude://claude.ai/new`, `claude://claude.ai/chat/{id}`, `claude://claude.ai/project/{id}`. Ссылок `continue`/`resume`/`needs-input` для Code нет [документировано].
- claudeissues.com #90035 «Un-gate claude://code/continue?session=<local_id>» (открыт, Desktop 1.37937.3): маршрут был закрыт флагом, а `resume` создавал дубликат. Это данные третьей стороны, а не документация.
- claudeissues.com #50345 (закрыт): запрос на открытие сессии по UUID.

## 6. Чего не проверял
- Ссылки не запускались: ни `continue`, ни `resume`. Поведение во время выполнения неизвестно.
- Код Claude Code (CLI), который пишет `hostSessionId`, не смотрел.
- Не открывал `*.key`, `ide/*.lock`, `auth.json`, `.credentials.json`, `claude_desktop_config.json`, `bridge-state.json`, `buddy-tokens.json`. Значения `title` и другие поля сессий, кроме id и cwd, не читал.
