# Forma runtime protocol v1

Implementation contract, not a runtime QA receipt. **The desktop app owns and automatically launches Hermes under the hood.** Operators configure only their model/provider credentials, not a runtime URL, runtime token, Python installation or Hermes service. There is no direct-chat fallback. The optional generic HTTP control service is retained only for remote development/fixtures on miniforum, not product manual setup. App-owned native stdio sidecar QA is desktop functionality; unrelated backend tests still run on miniforum. Never reuse the bosgame company profile.

Hermes source is pinned to **349e6611a1c5d846a865368dd6c386b78edd1a54**, package 0.20.5, Python >=3.11,<3.14, MIT. This supersedes the older integration research pin for this adapter. Preserve Hermes's LICENSE when installing its separately pinned source.

## Transport and authority

Base `/v1`; JSON camelCase; UUID strings; RFC3339 UTC timestamps ending `Z`; omit optional fields. Integers exclude booleans. Unknown mutation fields fail validation. In the managed product, exclusively owned anonymous stdin/stdout pipes authenticate the native parent. There is **no TCP control listener and no runtime bearer to provision**. Provider keys pass from the native OS credential store through private stdin to the controller's memory only: no secret argv, environment, configuration files, logs, renderer or worker. HTTP remote/dev fixtures separately require their dedicated bearer. Neither mode permits redirects, implicit pairing, account token upload or provider fallback.

One operator owns one device-local library; the controller automatically creates stable library/device UUIDs in its locked profile SQLite database. The native app supplies its dedicated OS application-data `managed-hermes` directory. A different profile receives different identities; another controller cannot own the same database concurrently. Request IDs never establish authority. **Workspaces are chats, not tenants or separate authorization principals.** Native operator administration may inspect/manage proposals and schedules across their own library; a workspace filter is organization, not an ACL. Shared interfaces explicitly span chats. This does not grant a model cross-workspace authority: the supervisor binds each tool call to its admitted run; account grants, tool claims, deletion fences and scheduled targets are independently workspace/device/connection bound. Generated code and Hermes workers have no private control pipe. A model cannot create grants, enable schedules, delete interfaces, impersonate a native approval or bypass account consent.

### Managed lifecycle: `forma-managed-v1`

The native host verifies bundled resources and launches:

```text
<resources>/managed-hermes/python/bin/python3.12 -I -B \
  <resources>/managed-hermes/controller/forma_runtime/managed.py
```

`-I` ignores ambient Python settings; the script inserts only its owned controller root. The child environment is cleared. The bundled Python prefix supplies its installed dependencies; no runtime dependency downloads or user interpreter paths. `source-proof.json` is the exact SDK `{revision,files:[{path,sha256}]}` proof; `manifest.json` is distinct packaging metadata. Native owns packaging validation and OS credential access.

NDJSON, UTF-8, one request/response per UUID `id`. No unsolicited stdout. Native serializes pipe I/O and matches response IDs. Maximum request line 600000 bytes including newline; maximum response line 400000 bytes including newline. A partial input or blocked output has a15-second bound. Fatal framing/pipe errors terminate the owned controller; no unbounded buffering or raw exception text.

```text
Bootstrap request:
{id,op:"bootstrap",profileDir:absoluteDedicatedRuntimeProfile,
 sourceDir:absoluteBundledSource,pythonPath:absoluteBundledPython,
 manifestPath:absoluteSourceProof,modelConfig?:ModelConfig|null}

ModelConfig = {providerId:string,gatewayUrl:string,configEpoch?:UUID,
 gatewayKey?:string,models:[{id,name,available:boolean,reason?}],dailyLimit?:1..192}

Bootstrap/configure success:
{id,ok:true,result:{protocol:"forma-managed-v1",libraryId,deviceId,capabilities,enabledSchedules}}

Product request:
{id,op:"request",method:"GET"|"POST",path:"/v1/...",query?:Record<string,string>,
 idempotencyKey?:UUID,body?:object}
→ {id,ok:true,result:{status:integer,body:theExistingV1DTO}}

Model replacement/removal:
{id,op:"configureModel",modelConfig:ModelConfig|null}

Secret-free lifecycle observation:
{id,op:"lifecycle"} → {id,ok:true,result:{enabledSchedules:number}}

Shutdown:
{id,op:"shutdown"} → {id,ok:true,result:{stopped:true}}

Control error:
{id,ok:false,error:{code,message,retryable}}
```

`enabledSchedules` counts enabled schedules across the entire owned library whose endAt/maxRuns are not exhausted. The lifecycle query performs no tick, model invocation or credential read, so native can inspect background intent before accessing the keychain.

Normal API rejections remain `{status,body:{error:...}}` inside a successful control envelope. GET has no body or idempotency key. POST uses the existing durable idempotency UUID. Errors never echo key/config/request contents. Startup without `modelConfig` succeeds, exposing interfaces/history/schedules while `runtime.ready=false,reason="needs_model"`; it does not invoke a model. Missing/invalid SDK/sandbox fails closed; Windows has no verified worker adapter and remains a separate unavailable gate.

`gatewayUrl` is the native-validated operator-selected OpenAI-compatible `/v1` endpoint. Managed mode supports explicit HTTPS public providers, plus explicit private gateways for development; generic HTTP fixture mode remains private-gateway-only. No arbitrary worker-selected URL, redirects, proxies or hidden paid fallback. Higgsfield hosts remain explicitly blocked before DNS while the account freeze applies. A genuinely no-auth native-approved provider may omit the key; parent omits Authorization rather than fabricating a credential. The worker always sees only the dummy Unix-relay credential, never the upstream key.

Native persists a random nonsecret `configEpoch` with provider approval and rotates it on provider/credential changes. Parent persists only `{providerId,gatewayUrl,configEpoch}`, **not the key or a key hash**. On bootstrap with no key, enabled schedule intent remains stored but the supervisor neither ticks nor admits work. Providing the same approved epoch after restart restores execution under the original remaining limits; missed occurrences are skipped, not replayed in a burst. Paused schedules stay paused. An absent/different epoch, explicit model removal, or in-process credential change revokes grants, fences old runs/claims/proposals and pauses old enabled schedules before replacing configuration. Draining and reaping must finish before a new relay can use a new key. Catalog display/availability refresh under the same provider epoch does not reset approvals or budgets; a removed selected model is independently fenced. Model selection within a workspace is not a provider-identity change.

Shutdown acknowledgment follows supervisor/relay drain and worker reaping. EOF, broken output pipe and SIGTERM take the same cleanup path. Native retains process ownership and a bounded kill/reap fallback if the controller itself fails. Restart reconciles previously active runs as interrupted; it never retries uncertain execution as a fresh run.

`capabilities` additionally returns `modelOrigin` (the configured model gateway's scheme + authority, without credentials/path), and `deviceId`/`libraryId` for the automatically established managed profile identity (explicitly provisioned only in remote/dev fixtures). Native egress grants bind both the runtime origin and this model origin. Interface responses additionally contain host-derived `provenance:{dataMode:"synthetic"|"nonAccount",sourceRunId?:UUID}`; model input cannot declare live provenance.

For `POST /v1/runs`, the Idempotency-Key UUID is also the run ID. After a lost admission response, the native client may safely `GET /v1/runs/{requestUuid}` before retransmitting the exact original admission. No new request ID is generated automatically.

Every POST requires `Idempotency-Key: <UUID>`. Scope is authenticated device + path + key. Same canonical body returns the original status/body, including a lost response; a changed body returns 409. All admission/response identities are durable. GETs have no side effects other than presence-independent observation.

Errors: `{"error":{"code":"revision_conflict","message":"…","retryable":false}}`. Statuses: 401 authentication; 403 scope/grant; 404 absent/not owned; 409 CAS/idempotency/stale claim/active run; 410 tombstone/expiry; 422 schema; 429 capacity; 503 runtime unavailable. Never return raw provider exceptions or secrets.

## Capabilities and models

`GET /v1/capabilities` → 200:

```json
{"contractVersion":"forma-runtime-v1","runtime":{"kind":"hermes","ready":false,"revision":"349e6611a1c5d846a865368dd6c386b78edd1a54","reason":"not_configured"},"features":{"eventPolling":true,"interfaces":true,"schedules":true,"nativeToolBridge":true,"generatedCodeExecution":false,"liveGoogle":false},"tools":[],"limits":{"maxIterations":8,"maxToolCalls":12,"maxOutputTokens":4096,"maxDurationSeconds":180,"maxToolResultBytes":262144}}
```

`tools`: `{name,available,execution:"runtime"|"nativeDevice",reason?}[]`. Availability is factual, not a claim of account consent. Real Google remains disabled until its separate native policy/consent/retention gates pass.

`GET /v1/models` → 200 `{items:[{id,name,available,reason?}]}`. Explicit runtime-owned approved local-gateway catalog. Empty/unavailable catalog does not trigger direct chat or model substitution. Native selection sends the exact ID; model choice is per run/workspace.

## Runs and conversation continuity

`POST /v1/runs` → 202 `Run`:

```json
{"workspaceId":"11111111-1111-4111-8111-111111111111","message":"Make a synthetic weekly overview","modelId":"configured-model","grantRefs":[],"interfaceIds":[],"budgets":{"maxIterations":8,"maxToolCalls":12,"maxOutputTokens":4096,"maxDurationSeconds":180},"importHistory":[{"role":"user","content":"Earlier local question"},{"role":"assistant","content":"Earlier local reply"}]}
```

`message`: 1..16000 Unicode characters. `grantRefs`: `{id:UUID,generation:positive integer}[]`; `interfaceIds`: UUID array. Budgets are mandatory positive integers except tool count may be zero, capped by capabilities. `maxOutputTokens` bounds each model completion, not total run token accounting; duration/tool/iteration ceilings are independent.

`importHistory` is optional, at most 100 `{role:"user"|"assistant",content:string}` entries, at most 128 KiB JSON. Accepted **only on first workspace admission**, in the same transaction that creates its server record. No system/tool messages or trusted authority import. A later nonempty import returns 409 `history_already_initialized`; absent or empty later import is harmless. Subsequent runs use the controller's strictly validated committed Hermes message/tool history, never duplicate client history. Parent rejects system/developer/unknown roles, unknown fields, invalid Unicode, oversized text/arguments, malformed or duplicate/orphan tool exchanges, incomplete calls, changed prior history and worker-manufactured user turns before any publication or history write. Runtime history permits bounded user/assistant/tool text with function-call IDs and JSON-object arguments; no multimodal/opaque provider blocks. Known pinned timestamps and observational metadata are validated and projected out. Textual reasoning_content is bounded and retained only for compatible replay. A forbidden builtin attempt may persist only with its exact `tool_denied` result, never a successful result or expanded tool authority. The same validator runs on persisted history before replay, including older database contents; invalid legacy history fails explicitly rather than entering a prompt. First admission binds workspace to authenticated library. A tombstoned workspace cannot reappear.

`Run`:

```text
{id,workspaceId,origin:"operator"|"schedule",scheduleId?,state,reason?,modelId,
 hermesRevision,budgets,createdAt,startedAt?,finishedAt?,lastSeq,finalMessage?}
```

States: `queued|running|waiting_for_device|cancelling|succeeded|failed|cancelled|interrupted|blocked`. Terminal: succeeded/failed/cancelled/interrupted/blocked. Final text alone is not success. Only a successfully reconciled Hermes result advances committed conversation history; interrupted partial output stays visibly provisional in events. One active run per workspace and per involved connection. Initial service capacity: one worker total, bounded queued admissions.

- `GET /v1/runs?workspaceId=UUID&limit=50` → `{items:RunSummary[]}` (newest first; maximum 200).
- `GET /v1/runs/{id}?after=0&limit=200` → `RunPoll` below.
- `POST /v1/runs/{id}/cancel` body `{}` → 202 `CancelReceipt:{id,state,admitted:boolean,run?:Run}`; terminal cancellation follows actual process reconciliation/reaping. An unknown UUID produces admitted:false/state:cancelled and a durable pre-admission tombstone; late admission and GET of that UUID return410 `run_cancelled_before_admission`. Known runs return admitted:true and full Run. Repeated cancel is harmless; a cached cancelling receipt still requires GET reconciliation. Rejected admissions (e.g.409 run_active) roll back without caching a successful receipt; cancel the original UUID to prohibit future late admission.
- `POST /v1/workspaces/{id}/delete` body `{}` → 200 `{id,deletedAt}`. Tombstone fences queued/current/late work and pauses owned schedules. Shared interfaces survive. Native host also fences local late events before removing its local workspace.

`RunPoll`: `{run:Run,events:Event[],nextAfter:integer,hasMore:boolean,toolRequests:ToolRequest[]}`. `Event`: `{runId,seq,at,type,payload}`. Query returns `seq>after` ascending. `nextAfter` is the last **returned** sequence (or supplied cursor), not `run.lastSeq` on a truncated page. Commit events before delivery; readers do not consume them. Reconnect starts from last applied sequence and deduplicates `(runId,seq)`.

| Event type | Payload |
|---|---|
| `run.state` | `{state,reason?}` |
| `assistant.delta` | `{text}`; provisional |
| `assistant.message` | `{text}`; final only on success |
| `tool.requested` | `{requestId,toolName,deviceId?}` |
| `tool.result` | `{requestId,outcome}`; no raw account data |
| `interface.proposed` | `{proposalId,interfaceId?,expectedRevision}` |
| `interface.updated` | `{interfaceId,revision}` |
| `schedule.created` | `{scheduleId,state:"paused"}` |
| `error` | `{code,message,retryable}` |

## Native read-only tool bridge and grants

`POST /v1/device-heartbeat` body `{}` → `{deviceId,onlineUntil}`; 45-second presence lease. Presence cannot create grants.

`POST /v1/grants` creates a grant from trusted native controls only:

```text
{workspaceId,connectionId,deviceId,operations:["forma_gmail_list_metadata"|"forma_calendar_list_events"],
 modelId,expiresAt,dataMode:"synthetic"}
```

→ 201 `{id,generation:1,workspaceId,connectionId,deviceId,operations,modelId,expiresAt,dataMode,revoked:false}`. V1 accepts synthetic data only; real Google grants are explicitly unavailable, not simulated live access. Device must match the controller's bound profile identity. `POST /v1/grants/{id}/revoke` body `{generation}` increments generation, cancels/fences affected runs, pauses schedules, invalidates tool leases and purges sensitive tool payloads. No grant-creation/revocation model tool.

Poll `toolRequests` includes only the authenticated credential-owning device's actionable requests:

```text
{id,runId,workspaceId,deviceId,connectionId,grantRef,toolName,args,expiresAt,state:"pending"|"claimed"}
```

`args`: `{startAt,endAt,maxItems}`; UTC window <=7 days; maxItems 1..100. No raw query, URL, account ID, body/attachment switch. Gmail INBOX only excluding spam/trash; connection resolves mailbox/calendar. Exact scope/provider mapping stays native-owned.

- `POST /v1/runs/{id}/tool-claim` `{requestId}` → `{request:ToolRequest,claimToken,claimExpiresAt}`. Exclusive 30-second lease, bounded by request/run expiry. Expired read-only claims may be reclaimed; no exactly-once claim.
- `POST /v1/runs/{id}/tool-result` `{requestId,claimToken,outcome,data?,error?}` → `{accepted:true,requestId}`. Outcome `succeeded|failed|denied|unavailable`; success requires data, otherwise `{error:{code,message}}`. Both together rejected.

`data`: `{kind:"gmailMetadata"|"calendarEvents",retrievedAt,expiresAt,startAt,endAt,truncated:boolean,partial:boolean,items:[]}`. Maximum serialized payload 256 KiB, bounded by request count/window. Gmail item: `{id,sender,subject,receivedAt,unread:boolean,sourceUrl}`; no snippet/body. Calendar item: `{id,title,startAt,endAt,allDay:boolean,sourceUrl?}`. For all-day events native must preserve date semantics or explicitly return unavailable; never invent event instants. Source links are inert, HTTPS, canonical provider-owned navigation only; native validates before opening.

Native rechecks workspace/account/device/operation, grant generation/expiry/revocation, policy and selected-endpoint egress consent before any provider call. Runtime independently rechecks request/lease/run/grant on acceptance and publication. Duplicate identical result is idempotent; conflicting/stale/late/cancelled/revoked result rejected. Google access tokens never leave Mac. Offline requests wait explicitly, then become blocked at the total run deadline. No silent fixture fallback. Tool results are untrusted data, never instructions.

## Shared interfaces: proposals, CAS, tombstones

This slice persists **synthetic or non-account content only**. Live account-derived interface source/content is blocked until a separate bounded snapshot/taint/expiry design is implemented. Shared presentation never shares account grants. Genuine authored code remains a separate gated capability; trusted blocks do not claim to implement it.

All list routes return `{items,nextCursor?:string,hasMore:boolean}`, default limit20/max50, decimal offset `cursor`. Responses are byte-bounded to384KiB; do not increase native transport caps. `InterfaceSummary` omits spec; `ProposalSummary` omits spec; `ScheduleSummary` omits prompt; `RunSummary` omits finalMessage; revision summaries omit spec. Detail routes return full DTOs: `GET /v1/interfaces/{id}`, `GET /v1/interfaces/proposals/{id}`, `GET /v1/schedules/{id}`, `GET /v1/interfaces/{id}/revisions/{revision}`. Event polling remains separate with max200 and an additional384KiB whole-response cap. Summary pagination cannot silently drop records: hasMore/nextCursor reflect the last returned item. Concurrent list changes may require a fresh listing; this is not a snapshot-sync protocol.

`InterfaceSpec`: `{schemaVersion:1,kind:"components",root:Node,datasets:Dataset[]}`. This is a novel trusted composition schema, **not** the historical chat-reply block validator. Exact nodes:

```text
{id,type:"stack",children:Node[]}
{id,type:"grid",columns:integer 1..4,children:Node[]}
{id,type:"card",title:string,children:Node[]}
{id,type:"text",text:string}
{id,type:"metric",label,datasetId,field,format:"number"|"percent"|"currency",currency?:string}
{id,type:"table",datasetId,columns:[{field,label}]}
{id,type:"chart",kind:"bar"|"line",datasetId,xField,yField,label,units}
Dataset = {id,columns:[{id,type:"text"|"number"|"timestamp"}],rows:Record<string,string|number|null>[]}
```

Strict unknown-field rejection, 64 KiB whole spec UTF-8 JSON, <=3 layout ancestor levels (stack/grid/card), <=12 content nodes, <=40 total nodes, <=12 datasets, <=100 rows per dataset, <=12 columns per dataset/table, <=2000 characters/text, finite numbers abs<=1e12. Node IDs globally unique within the spec; dataset IDs and dataset column IDs independently unique. IDs are 1..80 ASCII letters/digits/underscore/hyphen/dot, beginning with a letter or digit; reject `__proto__`, `prototype`, `constructor` as IDs/properties. Labels/card titles <=160 chars, units<=80. Rows have exactly their declared columns; nullable values allowed, otherwise match column type; interface timestamps are valid RFC3339 UTC with at most3 fractional digits. Field references resolve. Metric/yField numeric; bar xField=text, line xField=timestamp. Chart x values non-null/unique; line times strictly increase; null y is a gap, not zero. Metric dataset has at most1 row (zero rows/null is no data; no implicit aggregate). Percent metric uses fraction convention:0.5 displays50%. Currency required only for currency format, exactly three uppercase ASCII letters. No currency field in other formats. No links, actions, HTML, JS, bindings or model-provided provenance. Literal datasets receive the host badge **From conversation · not live**; real-source badges require separately verified host-derived provenance.

`Interface`: `{id,libraryId,revision,title,spec,createdAt,updatedAt}`. Revision starts at 1. Title 1..160 characters. Interfaces belong to operator library, not creating chat.

`Proposal`: `{id,workspaceId,sourceRunId?,interfaceId?,expectedRevision,title,spec,state:"pending"|"published"|"invalidated",createdAt}`. New-interface proposal has no interfaceId and expectedRevision 0. Update proposal requires interfaceId and current revision. Models may propose; operator-owned native controls publish. A schedule may publish only its pre-authorized target under its captured CAS revision and current enable/grant fence.

- `GET /v1/interfaces?workspaceId=UUID&limit=50` → `{items:InterfaceSummary[]}`.
- `GET /v1/interfaces/{id}` → `Interface`.
- `GET /v1/interfaces/proposals?workspaceId=UUID&limit=50` → `{items:ProposalSummary[]}`.
- `POST /v1/interfaces/propose` `{workspaceId,interfaceId?,expectedRevision,title,spec}` → 201 `Proposal`.
- `POST /v1/interfaces/publish` `{proposalId,expectedRevision}` → 201 `Interface`. CAS also checks proposal source run success/current grant fence; pending in-flight/cancelled/tombstoned proposal cannot publish.
- `POST /v1/interfaces/{id}/rename` `{expectedRevision,title}` → 200 `Interface`; creates immutable new revision retaining spec.
- `GET /v1/interfaces/{id}/revisions?limit=50` → `{items:[{interfaceId,revision,parentRevision?,title,spec,createdAt}]}`.
- `POST /v1/interfaces/{id}/rollback` `{expectedRevision,targetRevision}` → 201 `Interface`; creates a new revision, never rewinds identity/authorization.
- `POST /v1/interfaces/{id}/delete` `{expectedRevision}` → 200 `{id,deletedAt}`. Permanent tombstone fences later proposals/publication, pauses referencing schedules, cancels associated work. No resurrection by old ID.

## Schedules (one name; no /jobs alias)

`POST /v1/schedules` → 201 `Schedule`:

```text
{workspaceId,interfaceId,expectedInterfaceRevision,prompt,modelId,cron,timezone:"UTC",
 endAt,maxRuns,budgets,grantRefs}
```

Always **paused**; reject enabled/state/consent mutation fields. `endAt` required and in future; maxRuns 1..100; same budgets as runs. Cron: exactly five numeric fields; `*`, comma lists, ranges and positive steps supported; no names/macros/seconds. Fixed UTC; reject other zones. Standard DOM/DOW OR semantics when both are restricted.

`Schedule`: `{id,workspaceId,interfaceId,interfaceRevisionPolicy:"latestAtRunStart",version,state:"paused"|"enabled"|"ended",prompt,modelId,cron,timezone,endAt,maxRuns,runsStarted,budgets,grantRefs,createdAt,updatedAt,nextRunAt?,lastRunId?,reason?}`.

- `GET /v1/schedules?workspaceId=UUID&limit=50` → `{items:ScheduleSummary[]}`.
- `POST /v1/schedules/{id}/enable` `{expectedVersion,consent:{scheduleVersion,modelId,grantRefs}}` → `Schedule`. Trusted native control only, after scope/endpoint/cadence/retention/budget disclosure. Consent matches exact version and current grants; model can't enable. Runtime readiness and current target required.
- `POST /v1/schedules/{id}/pause` `{expectedVersion}` → `Schedule`; fences/cancels pending work.
- `POST /v1/schedules/{id}/delete` `{expectedVersion}` → `{id,deletedAt}`; tombstones and fences.

Edits use a newly created paused schedule in v1; no ambiguous partial PATCH. Enable/resume increments version and computes next future minute. Scheduler admits unique `(scheduleId,scheduledAt)` occurrences transactionally, increments runsStarted before execution (attempt count), and invokes the same Hermes pipeline. No overlapping schedule/workspace/connection work, no uncertain retry. At admission capture latest interface revision; publish only CAS against it. Conflict fails that occurrence visibly. Manual updates are never silently overwritten.

No catch-up burst: record missed slots and advance next due; restart does not auto-enable paused schedules. endAt/maxRuns halt future admissions; pause/delete/revoke/expiry fence late publication. Previously executing runs become interrupted on supervisor restart and stale claims invalidate; queued never-dispatched runs require current deadline/grant checks. No exactly-once external-effect claim.

## Worker boundary and verification

Exact model-visible allowlist: `forma_interface_list`, `forma_interface_get`, `forma_interface_propose`, `forma_schedule_create`, `forma_gmail_list_metadata`, `forma_calendar_list_events`. Every handler receives server-bound run context, never model-selected library/device/grants. No shell/web/browser/arbitrary MCP/delegation/execute_code/cronjob/grant/enable tools.

Pinned API compatibility detail: Hermes automatically defers custom toolsets behind generic tool_search/tool_describe/tool_call unless `get_tool_definitions(skip_tool_search_assembly=True)` is enforced by the adapter. Forma disables that assembly and asserts the exact direct schema set before any request. `_fire_stream_delta` can also deliver non-text metadata; forward only nonempty strings as assistant.delta, with authoritative tool events supplied by the owned handler RPC.

Use actual `run_agent.AIAgent.run_conversation`, not a plain chat proxy. Fresh HOME/HERMES_HOME and safe environment **before imports**, `HERMES_SAFE_MODE=1`, no account/profile copying, explicit local gateway/model, no fallback. Toolsets filter schemas; hard effective dispatch guard independently denies all other names. Disable context-file, soul, memory and background-review loading. Hermes callbacks are observational and can swallow callback failures: authoritative tool requests/results and terminal publication are product transactions.

One isolated worker per run, parent-enforced duration/tool/iteration/resource ceilings, hard interrupt then bounded kill/reap. Hermes run_budget_seconds is not a sufficient hard deadline. Linux retains bubblewrap with network namespaces, read-only SDK/runtime/Python and a fresh tmpfs HOME. macOS uses `/usr/bin/sandbox-exec` with deny-default SBPL: exact read-only SDK/controller/Python prefix and OS libraries, fresh per-run HOME/tmp, no host file contents, no fork, and network-outbound restricted to one exact private Unix model-relay socket. No unsandboxed fallback. Before macOS readiness, a stdlib-only probe under the exact production profile must prove forbidden host reads/source writes/Python writes/IP connections/other Unix sockets/forks and ambient secret inheritance denied, while fresh HOME writes and the designated Unix socket work. The probe also creates and reaps its own nonsecret sibling canary, verifies it is alive, and requires `kern.procargs2` access to that PID denied; it never inspects real application arguments/environment. macOS requires an **explicit `(deny process-info*)`**, with only self metadata re-allowed: measured tests established that deny-default and sysctl-read filtering alone do not block sibling argument/environment reads. Sysctl access is additionally restricted to named hardware/version/uname metadata. No raw process vectors enter receipts or protocol output. Unsupported policy/layout means unavailable, not partial isolation. Worker CPU/file/fd limits remain; Linux enforces2GiB address space, macOS parent polls proc_pid_rusage and kills above2GiB resident memory because Darwin RLIMIT_AS is not a real allocation ceiling. This is a sampled resident ceiling, not an instantaneous reservation cap. Managed macOS worker remains in the native-owned controller process group; native's hard group kill reaches it, and a pre-Hermes100ms parent-death watchdog provides defense in depth. Each platform still requires its own actual adversarial/runtime evidence; Windows containment is not implemented. Durable SQLite owns runs/events/messages/claims/proposals/revisions/schedules and current authorization. The linked SQLite observed during the actual pinned-Hermes fixture is3.46.1; Hermes itself warns about its WAL-reset corruption vulnerability. The product therefore uses **DELETE rollback journal + synchronous=FULL**, not WAL, and an exclusive process ownership lock (one writer service per database). No dependency upgrade or company-state change is implied. Never infer runtime success from these DTOs or static tests.

Development tests use synthetic/offline fixtures on miniforum in a separate test copy, not the running QA service tree. Native app-owned stdio/sandbox QA is separately authorized desktop functionality on macOS, not an unrelated local backend deployment. No live Google/Higgsfield, paid-provider activation, live recurring schedule, commit or publication follows from this contract.

## Hard bounds and operational recipe

Parent relay admits only POST `/v1/chat/completions` to the exact native-approved provider and admitted model (private gateway for QA, explicit HTTPS provider in managed product mode). No gateway key enters the worker. It counts attempted upstream requests transactionally **before egress**, including retries/auxiliary calls: per-run ceiling=maxIterations; per-library UTC-day ceiling default192, configurable downward only. Capabilities limits include `dailyModelRequests` and `maxQueuedRuns:8`. Hermes `maxOutputTokens` is per completion, not a total-token accounting claim. Any auxiliary request that does not obey the exact route/model/tool contract is denied. Relay response is capped8MB before buffering/secret screening. Screening checks raw and decoded JSON strings plus one JSON-escape layer, and reconstructs SSE content/refusal/reasoning and indexed tool/function argument fragments before forwarding; this is bounded reflection protection, not general encoded-secret DLP; this v1 therefore delivers text deltas after a gateway response has been collected, not low-latency upstream streaming. Worker frames max2.2MB, assistant delta total256KiB, final message64KiB UTF-8, native response384KiB. Human committed Hermes history is capped2MB/1000 messages with explicit failure; no silent history truncation. User text<=16000 characters; assistant<=64000 characters/64KiB; tool text<=256KiB; per assistant<=12 calls; call arguments<=64KiB UTF-8 JSON-object text. Current-turn tool/assistant counts obey admitted tool/iteration budgets; call IDs are unique within a user turn and every result matches one pending call. Canonical final transcript text equals displayed final_response; no contradictory hidden replay message. Scheduled occurrences use fresh Hermes history plus the current authorized target specification, and do **not** append cron chats to the human workspace conversation.

The Python control plane itself uses the standard library. Hermes is separately installed from the pinned tracked export with its MIT LICENSE, locked core dependencies and no account/profile files. `Config.verify()` validates the full source file set/hash manifest and pinned export receipt. No mutable upstream branch or package-version-only check substitutes for it. Managed worker readiness remains unavailable without verified bundled resources, a platform sandbox and native-approved model configuration. No model configuration is needed to start the controller or inspect stored resources.

### Remote/dev HTTP fixture configuration only

The following legacy environment names belong exclusively to explicitly provisioned remote/dev fixtures, **not** the managed desktop launch. Native product launch does not set or read these secrets/environment variables:

Explicit fixture configuration names (values provisioned privately, never committed):

```text
FORMA_RUNTIME_DB                 dedicated SQLite path
FORMA_RUNTIME_LIBRARY_ID         provisioned operator library UUID
FORMA_RUNTIME_DEVICE_ID          provisioned native executor UUID
FORMA_RUNTIME_TOKEN_SHA256       SHA256 of a NEW dedicated native/runtime bearer
FORMA_RUNTIME_MODELS             comma-separated approved gateway model IDs
FORMA_HERMES_SOURCE              dedicated pinned tracked source export
FORMA_HERMES_PYTHON              dedicated venv/bin/python (preserve venv path)
FORMA_HERMES_MANIFEST            receipts/source-manifest.json
FORMA_MODEL_GATEWAY_URL          approved private gateway URL ending /v1
FORMA_MODEL_GATEWAY_KEY          separately provisioned model-gateway key, parent only
FORMA_DAILY_MODEL_REQUESTS       optional1..192, default192
FORMA_RUNTIME_PORT               optional9473; loopback only behind explicit encrypted transport
```

No environment values are obtained from the company profile or account auth files. `source-manifest.json` may be the dedicated installer list `{path,sha256,size}[]` accompanied by `source-export.json` with `commit` and `manifest_sha256`; alternatively `{revision,files}`. Extra/missing/symlink source files fail verification. Runtime source and source/venv are mounted read-only; `/home/forma` is fresh tmpfs before all Hermes imports. Unshare all namespaces including networking; the sole external communication mount is the approved Unix relay socket. Worker has no native HTTP token and no model-gateway secret.

Authorized miniforum test recipe (paths below are the dedicated test target, not an instruction to start production):

```sh
cd /opt/forma-app-runtime-qa-20260910-a1301ae10f51d22c6/managed-tests
PYTHONPATH=runtime \
FORMA_HERMES_FIXTURE_TESTS=1 \
FORMA_SANDBOX_TEST_SOURCE=/opt/forma-app-runtime-qa-20260910-a1301ae10f51d22c6/source \
FORMA_SANDBOX_TEST_PYTHON=/opt/forma-app-runtime-qa-20260910-a1301ae10f51d22c6/venv/bin/python \
FORMA_SANDBOX_TEST_MANIFEST=/opt/forma-app-runtime-qa-20260910-a1301ae10f51d22c6/receipts/source-manifest.json \
/opt/forma-app-runtime-qa-20260910-a1301ae10f51d22c6/venv/bin/python -B -m unittest discover -s runtime/tests -v
```

Without these opt-in variables, sandbox tests skip; never label skips PASS for containment. Tests use temporary SQLite and synthetic loopback HTTP responses, not models or accounts. After separate main authorization and explicit private configuration, entrypoint is `PYTHONPATH=runtime <dedicated-python> -B -m forma_runtime.server`. No systemd unit or automatic service/start/recurrence is installed by the source. Independent review, real pinned-Hermes compatibility, authenticated backend receipts and native QA remain separate gates.
