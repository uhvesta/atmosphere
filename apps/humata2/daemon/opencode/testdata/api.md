# OpenCode v2.3.0 server API (captured 2026-11-16)

Pin: `anomalyco/opencode` v2.3.0, published 2026-11-13T08:55:07Z.
Asset: https://github.com/anomalyco/opencode/releases/download/v2.3.0/opencode-darwin-arm64.zip
sha256: 7907df2b12720a840fbb49c5b978b9b82925b6bf75479a5fd216afa8932b6702

Server: `opencode serve --hostname 127.0.0.1 --port <n>` with `OPENCODE_SERVER_PASSWORD`.
Auth: HTTP basic, user `opencode` (or `OPENCODE_SERVER_USERNAME`). Without it every `/api/*`
call returns 401 `{"_tag":"UnauthorizedError","message":"Authentication required"}`.
`GET /health` and the web UI (`/`, `/doc`, any non-API path) need no auth; the UI HTML is served
for every unknown path, so a 200 on a non-`/api` path proves nothing.
OpenAPI document: `GET /openapi.json` (authenticated; the server maps it to `/api/openapi.json`
internally, which 404s from outside). `/doc` is the web UI, not Swagger.
Only `/api/...` routes exist. The old `/session`, `/event`, `/session/:id/prompt_async`,
`/session/:id/abort` and `/session/:id/permissions/:id` routes are gone (they fall to the HTML UI
or 404).

All bodies are JSON. Responses wrap in `{"data": ...}` (some add `"location":{"directory"}`).
Errors: `{"_tag": "...", "message": "..."}`.

| Op | Method and path | Request | Response |
|---|---|---|---|
| Info | `GET /api/info` | none | `{version, pid, hostname, urls[], paths, capabilities}` |
| Create session | `POST /api/session` | `{location?:{directory}, permissions?:[{action,resource,effect: allow\|deny\|ask}], agent?, model?:{id,providerID,variant?}, id?, parentID?, metadata?}` (all optional, no extra keys) | 200 `{data: Session.Info}` with `id` `ses_...` |
| List sessions | `GET /api/session?limit&order=asc\|desc&search&parentID&archived=true\|false&directory&project&subpath&cursor` | none | `{data:[Session.Info], cursor:{previous,next}}` |
| Get session | `GET /api/session/{id}` | none | `{data: Session.Info}`; has `outcome` (`succeeded\|failed\|interrupted`) once a run ended, and `time.busy`/`time.idle` |
| Active (busy) sessions | `GET /api/session/active` | none | `{data:{"<sessionID>":{"type":"running"}}}`; `{data:{}}` when all idle |
| Prompt, no wait | `POST /api/session/{id}/prompt` | `{text (required), files?, agents?, skills?, metadata?, id?, delivery?: "steer"\|"queue", resume?}` | 200 immediately `{data:{id:"msg_...",sessionID,time,type:"user",payload:{text},delivery:"steer"}}`; the run continues in the background (this is the async call) |
| Abort | `POST /api/session/{id}/interrupt[?resume=true\|false]` | none | 200 `{interrupted:true}` while running, `{interrupted:false}` when idle; 404 `SessionNotFoundError` |
| Event stream | `GET /api/event[?types=a&types=b]` | none | SSE, see below |
| List pending permissions (all) | `GET /api/permission/request[?location[directory]=...]` | none | `{location, data:[Permission.Request]}` |
| List pending permissions (session) | `GET /api/session/{id}/permission` | none | `{data:[Permission.Request]}` |
| Get one permission | `GET /api/session/{id}/permission/{requestID}` | none | `{data: Permission.Request}` |
| Reply to permission | `POST /api/session/{id}/permission/{requestID}/reply` | `{"decision":"once"\|"always"\|"reject","message"?:string\|null}` (`decision` required; `{"response":..}` gives 400 `Missing key ["decision"]`) | 204 empty. 404 `PermissionNotFoundError` for an unknown id |

Permission.Request: `{id: "per_...", sessionID, action, resources:[string], save?:[string], metadata?, source?:{type:"tool",messageID,id}, message?}`.
For a bash tool call `action` is `"shell"` (not `bash`), `resources` is the command line
(`["echo hi"]`), `save` is the pattern an `always` reply would store (`["echo *"]`).

Permission rules: `{action, resource, effect}`. Agent defaults (`GET /api/agent`) are
`*:*:allow` plus `read *.env* ask`. A session rule of `{action:"bash",resource:"*",effect:"ask"}`
did NOT ask (the action is `shell`); `{action:"*",resource:"*",effect:"ask"}` on the session did.
A project `opencode.json` `{"permission":{"bash":"ask"}}` was also not enough on its own.

## SSE

Each frame is `data: <json>\n\n`, plus `: heartbeat\n\n` comments. Response headers:
`content-type: text/event-stream`, `cache-control: no-cache, no-transform`. The stream is global
for the server (all sessions and projects); filter on `data.sessionID`. No replay: subscribe
before sending the prompt.

Envelope: `{id:"evt_...", created:<ms>, type, location?:{directory}, data:{...}, durable?:{aggregateID,seq,version}}`
(`data`, not `properties`). First frame: `{"id":"evt_...","type":"server.connected","data":{}}`
(no `created`).

Run lifecycle on this version (there is no `session.status` event in the stream, and none was
seen `session.idle` either):

- `session.execution.started {sessionID}` = busy.
- `session.execution.succeeded {sessionID}` = idle, turn ended normally.
- `session.execution.interrupted {sessionID, reason: user|shutdown|superseded|inactivity}`.
- `session.execution.failed {sessionID, error}`.
- `permission.asked {id, sessionID, action, resources, save, source}`.
- `permission.replied {sessionID, requestID, reply: once|always|reject}`.

Fixtures here: `events-session-created.sse`, `execution-*.json`, `permission-*.json`,
`tool-called.json`, `step-failed-aborted.json`, `session-active-*.json`.
Paths are rewritten to `/work/proj`.

## Driver follow-up (v2.3.0, captured 2026-11-16)

Server: `OPENCODE_SERVER_PASSWORD=<pw> opencode serve --hostname 127.0.0.1 --port 0`. `--port 0` is
allowed; stdout prints one line `server listening on http://127.0.0.1:<port>` (see
`serve-port0-stdout.txt`). `--print-logs` does not apply to `serve`. `--stdio` printed
`{"url":"http://127.0.0.1:4096"}` (ignores `--port 0`; not explored further).

TUI attached to that server: `OPENCODE_PASSWORD=<pw> opencode --server http://127.0.0.1:<port> -s <sessionID>`.
- Password sources that worked: env `OPENCODE_PASSWORD`, env `OPENCODE_SERVER_PASSWORD`, URL userinfo
  `http://opencode:<pw>@host:port` (screen reached the normal UI; server-side proof not logged).
- No env and no userinfo: exits with "Server ... requires a password; set OPENCODE_PASSWORD".
  Wrong password: "rejected the password". There is no password flag. Username env not tested.
- `-s <id>` accepts an API-created session id; the id is reused, not duplicated. An unknown id starts
  the TUI on a blank "New session" and creates nothing until a prompt is sent (not verified past that).
- With the TUI attached: API prompt runs and appears on the TUI screen; a `*` ask rule on the
  session raises `permission.asked`, listed by `GET /api/session/{id}/permission`, shown in the TUI
  ("Permission required / Allow once / Always allow / Reject"); API reply 204 clears the ask, emits
  `permission.replied`, run finishes (`session.execution.succeeded`). Text typed into the TUI's pty
  plus Enter (`\r`) submitted a prompt to the same session (new `session.execution.started` and
  `succeeded`; screen showed the reply).
- Sessions persist across a server restart with the same XDG_DATA_HOME (messages and metadata intact;
  `GET /api/session/{id}` and `/message` worked, TUI resumed with `-s`).

## Shared store, replies, env, events, ids (v2.3.0, captured 2026-11-16)

Shared store: `opencode serve --service`, plus two plain `serve --port 0`, ran at once in one XDG set.
All three share the single SQLite file `$XDG_DATA_HOME/opencode/opencode.db` (+ `-wal`, `-shm`) and one
`log/opencode.log`. A session created and prompted through each showed in all three `GET /api/session`
lists, all `succeeded`. No lock errors in the log. Live SSE is per process (not cross-checked).
Files written: plain `serve` writes only the db files and the log; no port, URL or service
registration. `serve --service` additionally writes `$XDG_CONFIG_HOME/opencode/service.json`
(`{password}`, 0600) and `$XDG_STATE_HOME/opencode/service.json` (`{id,version,url,pid,password}`,
0600) and ignores `OPENCODE_SERVER_PASSWORD` (see `service-registration.json`).
A default TUI (no `--server`) under the same XDG connected to the `--service` server only (lsof:
two connections to its port; none to the plain servers). It reads the state `service.json`.

Replies:
- `{"decision":"reject","message":"use staging"}`: 204; `permission.replied` has `reply:"reject"` and
  NO message; the message is in `session.tool.failed {error:{type:"permission.rejected",
  message:"use staging"}, executed:false}`. The model saw it and quoted it. The run ended
  `outcome:"succeeded"` (the model continued after the rejected tool).
- `{"decision":"always"}`: 204; `permission.replied reply:"always"`. It saved a rule
  (`GET /api/permission/saved` -> `{action:"shell",resource:"echo *"}`, id `psv_...`), so later
  `echo ...` commands in any session of that store do NOT ask. Use a different command when testing.
- Reply to an id the TUI already answered (TUI Enter = Allow once): 404 `PermissionNotFoundError`
  (`errors.json`).

Env leak: the shell tool inherits the server's environment, including `OPENCODE_SERVER_PASSWORD`
(`env-leak.json`, values redacted, names only). Any model with a shell tool can read the password.

Events for one prompted session (in order, first seen): session.created, inbox.enqueued,
execution.started, instructions.updated, inbox.delivered, step.started, step.first-token,
reasoning.started/delta/ended, tool.input.started/ended, tool.called, step.streamed, permission.asked,
usage.updated, session.renamed (title generation), permission.replied, tool.failed|success, step.ended,
text.started/delta/ended, execution.succeeded. Non-session events on the same stream: project.updated,
shell.created, shell.exited. Title generation fires only `session.renamed` on the SAME session; no
separate or child session and no extra `session.execution.*` was seen. Subagent/task tool not tried.

Custom id: `POST /api/session {"id":"ses_humata0000000000000000000"}` accepted and kept (GET works);
posting it again returned 200 with the same session; an id not starting with `ses` -> 400
`Expected a string starting with "ses"`.

RSS (ps, KB): plain `serve` about 146000 idle after several sessions, 215000 right after a prompt;
`serve --service` 247000; a TUI attached to a server 178000.
