# Product grill — decision tree and evidence

> Historical baseline. Current direction: [Product vision](../product/vision.md). Workspaces are persistent chats; memory is automatic; the app is dark and chat-first. Conflicting earlier decisions are superseded.

Date: 2026-09-09.
Status: **Rounds 1–3 answered; delegated shared understanding confirmed for the foundation only. Technical spikes, live-data access and application delivery remain gated.**

This record preserves the first frontier, its Anthropic amendment, the second frontier, the Gmail scope amendment, and round 3's founder-driven custom-UI supersession. Historical Q2/Q8 catalog-only exclusions and Q10's catalog-only demo are explicitly superseded by Q15 and [ADR 0001](../adr/0001-generated-ui-is-a-product-capability-not-host-authority.md). Genuine custom React/TypeScript UI is first-class product scope; security blockers delay execution/live-data access rather than remove the capability. [brief.md](brief.md) consolidates the current product baseline; [CONTEXT.md](../../CONTEXT.md) owns the glossary. Use **Workspace**, not the earlier provisional term Domain.

## Provenance and authority

- **F — Founder direction:** open-source, local-first Hermes workspace; durable generated dashboards over explicitly connected accounts; macOS/Windows without Electron; BYO providers/custom endpoint; future multiple devices, Orca interoperability, and optional hosting. Founder delegated reversible product decisions and said not to ask them routine product questions mid-task.
- **D — Delegated PO decision:** prioritization, exclusions, defaults, proposals, and acceptance thresholds below. These are challengeable decisions, not founder approval of sensitive acts.
- **V — Verified environmental/policy evidence:** explicitly attributed below. Primary-source policy findings were supplied by the coordinator, not independently re-fetched by the PO.
- **H — Hypothesis/proposed threshold:** customer assumptions, numeric budgets, performance goals, and expected benefits. Not measured results or provider limits.
- **B — Factual or authorization blocker:** must be resolved with evidence or actual authorized human consent. A coordinator/agent decision cannot supply account consent, permit spending, change a license, or authorize public distribution.

Initial read-only inspection found only four local agent definitions and no brief, glossary, ADRs, or Beads records. Before writing round 2, the PO read the new root glossary and foundation spike; `bd --readonly list --json` returned `[]`. These are time-of-inspection facts, not a claim that concurrent bootstrap work is absent. The assignment initially owned only the two files in `docs/product`; the coordinator subsequently requested a local backlog JSON specification as well. The issue keys below are planning references, not created Beads IDs; no actual Beads writes or issue closures are authorized by the specification.

## Design tree

- Operator/job and value slice (Q1)
  - Desktop substrate and platform evidence (Q7)
  - Fixture versus connected readiness (Q10)
  - Mail retrieval and content limits (Q9, also depends on Q3/Q6)
- Generated UI trust boundary (Q2, superseded by founder direction/Q15)
  - Trusted catalog plus genuine isolated custom React/TypeScript UI (Q8/Q15)
  - Preview/save/repair/rollback, artifact ownership and versioning (Q15)
  - Isolation mechanism, resource/egress/bridge proof (**implementation spike gate**)
- Data/provider ownership (Q3)
  - Provider-specific permitted authentication (Anthropic amendment)
  - Gmail restricted-scope compliance and model egress (Q9 amendment)
- Workspace/account/run/device scope (Q4)
  - Local product identity versus authenticated remote execution (Q11)
  - Hermes transport/runtime contract and Orca interoperability (**open**)
- Bootstrap deliverable (Q5)
  - Prototype and truthful connected pilot gates (Q10)
  - License, brand, release boundaries (Q12)
- Autonomy/consent (Q6)
  - Refresh ownership, retention, revocation, late results (Q9)
  - Concrete enforcement point (**open**, depends on Hermes findings)

Settled means the product direction is chosen. It does not mean its supporting integration exists or that technical/legal prerequisites have passed. Remote execution, connector wiring, and the complete architecture cannot be finalized while their facts remain open.

## Round 1

### Q1 — Primary operator/job and first vertical slice

**Decision (D/H):** An individual power operator managing a busy personal workflow. Job: “Show me which recent emails deserve attention, explain why, and preserve that view so I can return without rebuilding it.” First connected slice: one explicitly connected Gmail account → bounded read-only retrieval → conversation-generated triage dashboard → save → reopen after restart → explicit refresh. Source links, scope, freshness, uncertainty, and failure states are part of the slice.

**Reason:** Tests conversation becoming a durable working surface on one recognizable job. Gmail is the starting hypothesis, not established customer demand.

**Rejected/deferred:** Connector marketplace, team workflows, broad automation, chat-only summaries, and a polished mock advertised as a connector.

**Acceptance (D/H):** Complete the path without manual code/config editing; retain saved layout and last successful results across cold restart; every email-derived finding links to its source; distinguish empty/stale/revoked/error states. Proposed formative pilot: at least 3 of 5 consenting target operators return on three separate days in one week and describe a concrete triage benefit. Record baseline workflow and actual evidence; this small sample does not prove market fit.

**Prerequisites (B):** Hermes integration, compliant Gmail authorization/scope, real consent, and pilot participants. Header-only data may limit useful triage; assess rather than invent body facts. Q9 narrows initial content after provider facts.

**Provenance:** Founder durable-dashboard/account direction (F); operator segment, Gmail-first scope, and thresholds selected by PO (D/H).

### Q2 — What is generative UI allowed to do? (historical; superseded by Q15)

**Supersession:** The original PO catalog-only recommendation below is preserved as history, not current product scope. Its ban on all generated executable UI/JavaScript does not apply to isolated custom presentation code. No-privileged-host execution, explicit grants and external-content distrust remain binding. Current two-mode direction and acceptance are in Q15.

**Original decision (D):** Model output is untrusted versioned declarative data. Only a host-owned component/data-binding/action catalog may render or execute application actions. No arbitrary JavaScript, executable expressions, HTML/CSS, shell access, network fetches, or permission creation. Referencing an action is not implementing or authorizing it.

**Reason:** Enables useful visual variation while keeping a reviewable trust boundary. A sandbox alone does not make arbitrary generated applications safe.

**Rejected/deferred:** Model-authored executable UI, model-selected unrestricted URLs, rendering that implicitly accesses accounts, and external content that grants privileges.

**Acceptance (D):** Validate before render/save; reject unknown types/versions/actions and out-of-scope bindings; malformed or adversarial output cannot execute code, expose another account, or expand grants. Retain last-valid dashboard on invalid regeneration. Components have keyboard access and accessible names. Q8 supplies the initial bounded catalog.

**Prerequisites (B):** Renderer schema, host enforcement boundary, versioning contract, Hermes generation/streaming shape, and adversarial tests.

**Provenance:** Untrusted-UI and explicit-grant boundaries inherited from role; catalog/recovery scope selected by PO. No human permission is implied.

### Q3 — Local-first/hosted ownership and provider policy

**Decision (D):** Local operator-owned dashboard definitions, run history, and account-data cache; credentials device-local by default; no mandatory product cloud account. First model configuration is a deliberate BYO key/custom OpenAI-compatible endpoint. Support must follow each provider's terms; a subscription is not an interchangeable API credential. SaaS later.

**Reason:** Clear custody and useful offline reopening without premature hosted storage, billing, and sync responsibilities.

**Rejected/deferred:** Hosted-first mandatory storage, undocumented credential reuse, hidden model egress, and an assumption that all “OpenAI-compatible” endpoints implement the required contract.

**Acceptance (D):** Reopen saved surfaces offline with clear freshness; disclose and gain consent for endpoint and data categories before model egress; no credentials in prompts/specifications/logs/exports; revoke connections and delete cache; portable secret-free export. Development uses only the permitted local gateway, never agent-activated paid providers. BYO product support is not spending approval.

**Prerequisites (B):** OS secret storage, provider compatibility, retention/deletion design, applicable data-use terms, and lawful OAuth support for any provider offered. Gmail connector OAuth is distinct from model-provider OAuth.

**Provenance:** Local-first/BYO/hosted-later from founder (F); implementation priorities from PO (D).

#### Q3 amendment — Anthropic subscription OAuth blocked

**Verified constraint (V, coordinator-supplied):** [Anthropic authentication and credential use](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use) prohibits third-party developers from offering Claude.ai login in their own applications or collecting, storing, or intermediating Claude.ai session tokens. Signing into an unmodified Claude Code binary is distinguished; it does not authorize Hermes to reuse subscription tokens.

**Decision (D):** Direct Anthropic subscription OAuth is **unsupported/blocked absent separate explicit authorization from Anthropic**, not just a backlog implementation choice. Use lawful API-key or supported cloud-provider authentication, subject to terms. Other providers require their own verification.

**Acceptance:** No Claude.ai login, subscription-token import/proxy/storage, credential harvesting, or suggestion that a Claude subscription supplies product API access. Negative authentication-path review covers all these exclusions. No paid route is activated by this decision.

**Supersedes:** R1's generic “OAuth pending verification” classification for direct Anthropic subscription OAuth only. It does not block lawful API integration and does not change the Gmail connector authorization question.

### Q4 — What does multi-device mean now and later?

**Decision (D):** Model Workspace, Connection/connected account, Run, and Device scopes from day one. Start with one device-local client. One explicitly configured remote Hermes executor is the first intended remote capability, not device pairing or synchronization. Credentials remain with the executor device by default.

**Reason:** Avoid anonymous/global identities that would require a later rewrite without implementing a distributed product early.

**Rejected/deferred:** Enterprise tenancy, automatic remote trust, credential replication, device discovery/full pairing, simultaneous multi-client editing, automatic sync, and Orca adapter in the initial slice.

**Acceptance (D):** Every run identifies workspace, applicable connection, executor device, and dashboard revision; explicit authenticated endpoint and revocable grants for remote operation; disconnects preserve the last successful dashboard; imported/model content cannot enroll devices. Describe remote execution separately from multi-device sync.

**Prerequisites (B):** Hermes authenticated transport, grant enforcement, cancellation/reconnect, remote credential boundary, and Orca contract. Q11 makes the current remote execution blocker explicit; domain identity alone is not capability support.

**Provenance:** Multi-device/Orca ambition from founder (F); narrow initial topology from PO (D). Root glossary supersedes “Domain” terminology (V).

### Q5 — First bootstrap deliverable

**Decision (D):** Agents plus documented product/domain/architecture decisions and a dependency-ordered Beads backlog first; smallest runnable vertical slice only after its material facts/design are settled. Do not design the entire roadmap before testing value.

**Reason:** There was no existing product implementation to refine. Premature integration code hides unresolved trust/contract questions; exhaustive roadmap planning also delays learning.

**Rejected/deferred:** Coding first and treating scaffolded agents as an implemented product; declaring runtime work complete from static checks.

**Acceptance (D):** Decisions have provenance, exclusions, criteria and blockers; glossary only in root CONTEXT.md, product specs in docs/product, consequential trade-offs in docs/adr; issues have role owners, dependencies and observable evidence. Delivery requires SPIKE → IMPLEMENT → independent multi-lane REVIEW → runtime QA; unavailable lanes are not passed. Runtime receipts include actual app cold-launch/screenshots and backend request/response/status where applicable. Backends run on the configured runtime host; native QA on its actual OS. Draft PR/publication is authorization-gated. Do not commit local agent/ops files or secrets.

**Prerequisites (B):** Integration/design findings, Beads availability, review/runtime infrastructure. Beads exists but had no issues at this PO read; issue creation is coordinator-owned in this assignment. No commit/push/publication consent is inferred.

**Provenance:** Bootstrap agents/Beads from founder and foundation plan (F); bounded deliverable and acceptance from PO (D).

### Q6 — What may autonomy perform?

**Decision (D):** No account access until actual operator consent establishes a scoped, revocable grant. Then only bounded reads and local dashboard changes in the requested workflow. Initial runs are operator-triggered. Scheduling requires a separate grant and is outside MVP. Consequential connector actions are absent, not merely hidden behind an approval button.

**Reason:** Read-only privilege is neither blanket permission to read every message nor permission to transmit data to arbitrary endpoints. Confidence and external-content instructions cannot authorize actions.

**Rejected/deferred:** Send/delete/archive/mark-read/label changes, purchases, shell access, browser cookie/profile harvesting, credential discovery, automatic remote trust, and agent-granted access. Future consequential actions require action-specific approval distinct from a capability grant.

**Acceptance (D):** Deny absent/expired/revoked/wrong-scope grants; injection cannot widen retrieval, change endpoint, or invoke forbidden actions; content-minimized audits record run/account/capability/scope/device/outcome without secrets. Show revocation and data-deletion behavior. Runtime receipts demonstrate zero Gmail mutations. Q9 strengthens revocation to discard in-flight results and purge sensitive cached derivatives.

**Prerequisites (B):** Concrete enforcement location, least-privilege connector behavior despite any ambient Hermes privileges, revocation/cancellation contract, audit retention and tests.

**Provenance:** Explicit consent and safety boundaries inherited; read-only/manual-run scope is PO decision, not account authorization.

## Round 2

### Q7 — Desktop technology and platform proof

**Decision (D):** Tauri v2 + React/TypeScript. This is the selected product delivery direction pending engineering feasibility, not a claim that tooling or a build currently works. Retain macOS and Windows as target platforms; no Electron.

**Reason:** One constrained component renderer and interaction implementation can serve both OSes without dual native-app staffing. Native dual apps cost more before the workflow is validated.

**Rejected/deferred:** Electron; separate native macOS and Windows implementations for the first slice. Do not treat a browser preview or one OS as proof of both desktop targets.

**Acceptance (H):** On each named OS/hardware configuration, cold launch to interactive p95 <= 3 seconds over 20 launches; local filter/sort p95 <= 100 ms over 100 interactions on the 100-message fixture; idle process-tree memory <= 250 MiB after 60 seconds quiescent, with WebView accounting documented. Measure network/model time separately. Record build/OS/hardware, cold-launch steps and screenshots; keyboard navigation and accessible status names work. These are proposed budgets, not observed benchmarks; revise transparently with evidence if infeasible. No Windows receipt means Windows remains unverified and any preview is labeled accordingly.

**Prerequisites (B):** Tauri/WebView capability and CSP design, Rust/frontend packaging, OS credential storage, actual Windows QA environment, and Hermes interface. Signing/distribution are not authorized.

**Provenance:** Platforms/no Electron from founder (F); Tauri stack and numeric budgets from PO (D/H).

### Q8 — Initial dashboard catalog and persistence (custom-UI scope amended by Q15)

**Supersession:** The catalog bounds below still define trusted catalog mode. They do not limit authored React/TypeScript UI to preset rearrangement, and the historical blanket code/CSS/bespoke-component exclusion no longer defines the product. Q15 adds isolated custom mode, ownership, compiler/runtime/design-system versioning, repair and rollback. Arbitrary dependencies and privileged code remain excluded.

**Original catalog decision (D/H):** Bounded stack/grid, metric, message list, table, and plain text. Initial proposed caps: depth 3, 12 content components, 4 grid columns. Bound rows to the snapshot with host-owned pagination/scrolling. No charts are needed to prove Gmail value.

Host owns binding resolution and action execution. Available actions: local filter/sort, save, explicit refresh under a valid grant, and operator-triggered source navigation using host-validated Gmail URLs. No model-selected network destinations. Rendering is side-effect free.

Persist dashboard revision identity/schema separately from data snapshot identity/source/retrieval time. Publish validated state atomically; never overwrite last-valid state with a partial or malformed result. Local SQLite is the intended connected-pilot store, contingent on the storage ADR; an atomic file-backed fixture prototype is permitted only with its limitations explicit. Account-derived text and summaries belong to expiring snapshots, not permanent layout fields; retention cleanup must also cover conversation history and generated labels that contain sensitive content.

**Reason:** Enough structure to test durable utility without a general application builder or chart system. Revision/snapshot separation preserves layout when data expires or a refresh fails.

**Rejected/deferred:** Arbitrary CSS/HTML/code, unbounded trees, bespoke generated components, chart catalog, direct model-authored SQL/bindings, and hidden sensitive content in retained layout strings.

**Acceptance (D/H):** Unknown schema/action/binding and limit violations are rejected; invalid regeneration and interrupted save retain last-valid revision/snapshot; cold restart restores layout; expired/purged data shows an explicit empty/stale state without fabricated results; version incompatibility is recoverable without silent loss. Persistence/security evidence must match the actual store used, not claim SQLite from a fixture file test.

**Prerequisites (B):** Renderer/storage ADRs, atomic publication and migration contract, secure retention cleanup, designer's interaction/accessibility specification, and injection tests.

**Provenance:** Supplied component-system direction from founder (F); catalog, bounds, storage preference and recovery behavior from PO (D/H).

### Q9 — Retrieval, retention, refresh, and revocation

**Decision (D/H):** First pilot uses one Gmail connection, INBOX only, rolling seven days, newest 100 messages maximum, excluding spam/trash. These are product bounds, not provider limits. Show requested scope, retrieval time, returned count, truncation, and partial status. Empty and failed are distinct; do not silently replace a complete last-valid snapshot with incomplete data.

**Content decision, amended after verified scope facts:** Initial baseline is minimum needed message identity/link, sender, subject, received time, and relevant state/labels from verified metadata operations. Original “metadata/snippets by default” recommendation is narrowed: snippets are not enabled until their exact API scope/response contract, permitted use and explicit disclosure/consent are established. A snippet is body-derived sensitive content, not harmless metadata. No bodies, raw MIME, attachments or remote images. Older/larger/wider/body retrieval is a separate bounded-grant/design gate outside the first pilot, never an automatic fallback for low model confidence.

**Refresh:** Explicit request, including first retrieval; one active retrieval/generation pipeline per connection; duplicate refresh starts no additional pipeline. No schedules, startup reads, reconnect reads or model-initiated polling. Cancellation is best effort at transport level but must prevent publication after invalidation.

**Retention:** Keep last successful bounded snapshot, with in-flight staging only. Snapshot and email-derived summaries/conversation content expire seven days after retrieval; viewing does not extend freshness/retention. Purge staging after completion/failure/cancellation. Check expiry before display, model input, export, and run publication; purge at next launch before access and while open. Do not promise deletion while the app is not running or forensic erasure of storage/backups. Retain content-free run audit metadata 30 days; content-free saved definitions remain until operator deletion. Secrets use device-local OS-supported storage.

**Revocation:** Immediately invalidate local capability, stop further calls, cancel in flight, purge associated cached account content and sensitive derivatives, and reject any late fetch/generation result carrying the old authorization even if transport reports success. Preserve only content-free definitions/audits. Check current authorization again before egress and commit. Local revocation works even if external token-revocation fails; surface the failure and do not reuse the credential. Already transmitted provider data cannot be recalled by local revocation; disclose that limit. Endpoint/data-scope changes need new consent, not inherited approval.

**Reason:** Predictable bounded work, understandable freshness and privacy, no invisible monitoring. Keeps revocation meaningful across asynchronous steps.

**Rejected/deferred:** Full-mailbox retrieval, body-by-default, indefinite raw/derived content history, scheduled reads, per-dashboard concurrent duplicate reads, and treating a canceled request as permission to retain late results.

**Acceptance (D/H):** Boundary fixtures cover zero/100/>100 messages, seven-day edge, excluded folders, truncated pages and failure mid-fetch. Verify the actual fetch strategy does not collect an unbounded mailbox to emulate the UI bound. Expiry tests cover restart and continued use. Revoke separately during fetch, model processing and pre-save; assert no new egress/calls/publication, cache is purged, and late results remain rejected even after reauthorization. No Gmail mutation. Wider scope and missing consent fail closed. Do not claim any of these runtime checks passed yet.

#### Q9 amendment — Gmail metadata is also restricted

**Verified constraint (V, coordinator-supplied):** [Gmail API scopes](https://developers.google.com/workspace/gmail/api/auth/scopes) classifies both `gmail.metadata` and `gmail.readonly` as **Restricted**. Metadata covers headers/labels, not bodies; readonly is needed for bodies. Application-level inbox/date/count limits do not narrow the OAuth grant. The page states that storing restricted-scope data on servers or transmitting it requires security assessment; it establishes no desktop/BYO exemption.

**Gate (B):** Document OAuth verification, Limited Use, and security-assessment applicability for the exact deployment and data flow, particularly model-provider transmission of snippets/bodies or other restricted-scope data. BYO OAuth and local-first branding do not waive rules. Establish an evidenced compliant path before enabling real-account access/egress; operator consent alone does not satisfy provider requirements. Verify metadata-only bounded listing/search behavior and snippet scope from actual API contracts. If required behavior is unsupported, keep the connected pilot blocked and revisit with evidence rather than silently request broader scopes.

**Provenance:** Numeric scope/retention and manual-run defaults are PO decisions (D/H), not Google constraints; scope classifications and assessment statement are coordinator-verified policy facts (V). Legal applicability and exact integration remain unresolved, not assumed exempt.

### Q10 — Fixture prototype versus connected pilot readiness (authored demo expanded by Q15)

**Supersession:** Simulation/playback can exercise interaction tests but no longer satisfies the first authored-UI demonstration. Q15 requires actual model-authored React/TypeScript generation and revision with synthetic data through the permitted gateway, contained execution, preview/save/reopen/rollback. This fixture path does not depend on Gmail compliance or account access; only the real-data path does.

**Original staged decision (D):** A clearly labeled synthetic fixture interaction slice precedes real Gmail. It may simulate model output/retrieval, but not pretend those are integrated. It uses a real desktop renderer, validation and save/reopen path. The connected pilot is a separate gated milestone and must use actual authorized services.

**Reason:** Learn interaction/recovery and exercise contracts without unauthorized account access, paid routing, or pretending unknown Hermes/Gmail facts are solved.

**Rejected/deferred:** Fake connected badges, screenshots promoted as proof of real Gmail, fixture success marked as integration QA, and a prototype branch that bypasses later consent/security gates.

**Acceptance (D):** Fixture labeling visible in setup, dashboard, refresh state, and screenshot/export context; no real connection/credential request or accidental connector/provider call in fixture mode. Demonstrate synthetic conversation → catalog dashboard → save/restart/reopen → refresh plus invalid/stale/revoked/error scenarios. Distinguish simulated revocation UI from real enforcement evidence. Connected-pilot completion additionally requires verified Hermes/provider contract, Google compliance and operator consent, grant/egress/retention tests, zero-mutation runtime receipts, independent review, and per-OS native QA. Report absent credentials, blocked policy gates and missing review lanes as blockers.

**Prerequisites (B):** Fixture-specific catalog/interaction/persistence design can settle separately; the entire Hermes/Orca design branch is still open. Foundation completion is explicit decisions/backlog/evidence or precise blockers, not a claim that an app exists.

**Provenance:** Bootstrap-first from founder/coordinator direction (F); staged readiness and truthful labeling from PO (D).

### Q11 — Product account, workspace identity, and remote execution

**Decision (D):** Device-local workspace, stable workspace/device identifiers; no hosted product login, team account, sync or pairing in MVP. Identity is not trust. Preserve connection/executor scope from day one, without inventing a distributed authorization protocol.

Remote Hermes is the first intended remote execution capability, but **blocked until authenticated transport and per-request authorization evidence exists**. No unauthenticated “temporary” mode, implicit trust from a private network, automatic remote enrollment, or forwarding local credentials. No claimed Orca adapter support before research.

**Reason:** Separates operator ownership and attribution from an unnecessary SaaS identity layer. A reachable endpoint is not a trusted executor.

**Rejected/deferred:** Cloud account prerequisite, device discovery, trust-on-import, transparent sync, multi-client conflict handling, and treating Tailscale/network reachability alone as the full application authorization contract.

**Acceptance (D):** Offline local fixture works without product login; runs retain workspace/connection/device/revision association. Remote controls clearly show unavailable/blocked until verified. Later remote tests must cover endpoint identity mismatch, absent/revoked/wrong-scope grants, disconnect/reconnect and late responses, with no credential transfer or automatic duplicate execution. Do not claim these future tests have passed.

**Prerequisites (B):** Hermes transport/auth/session/run events, replay/idempotency/cancellation behavior, effective tool restrictions and secret custody; Orca contract independently. Q4's planned remote capability remains conditional, not an implemented MVP promise.

**Provenance:** Local-first/multi-device ambition from founder (F); no product account and remote gate selected by PO (D).

### Q12 — License, release, and brand

**Decision (D):** Use descriptive provisional working title “Personal agent workspace”; no public brand launch. Open-source remains the direction. **Apache-2.0 is a proposal only**, pending Hermes/Orca/dependency compatibility and explicit authorized license selection before distribution. Do not add a LICENSE file or assert the project is already licensed.

**Reason:** Avoid irreversible public/legal positioning before dependency obligations and authority are settled. A proposal makes the next decision concrete without fabricating approval.

**Rejected/deferred:** Unreviewed license selection, assuming Hermes can be embedded under any license, trademark claims, public repository/release, package signing, purchases and paid endpoints.

**Acceptance (D):** Product docs mark title and license as provisional; dependency/license evidence precedes an explicit selection; public distribution remains blocked until authorized license/brand/release checks exist. Native unsigned internal QA is labeled as such and does not imply installation/distribution readiness. Signing, paid routes, commit/push and publication are not authorized by these product decisions.

**Prerequisites (B):** Upstream license/dependency notices, integration/distribution mode, authorized legal/license selection, brand clearance for any final name, and signing/distribution ownership. No need to ask founder during this fact-finding round; preserve the authorization gate for later.

**Provenance:** Open-source direction from founder (F); descriptive title and Apache-2.0 proposal from PO (D), not an actual license change or human approval.

## Round 3

### New evidence and founder-directed supersession

**Founder direction, relayed by coordinator (F):** “Generated LLM UIs feels interesting man, could be a buzzing aspect, let's do it, don't exclude it, if the agent can generate itself based on guideline thats cool”. This is a direction to support actual model-authored custom UI, not only catalog arrangement. It supersedes the PO's narrower Q2/Q8 exclusions; it is not permission for real-account access or privileged generated-code execution. The updated product-owner role and accepted [ADR 0001](../adr/0001-generated-ui-is-a-product-capability-not-host-authority.md) align with this direction.

**Research evidence, coordinator-supplied (V):** Hermes pinned to `fd6434b3b36592367ac5faa180b905d64e29214c`, MIT v0.21.1, Python >=3.11,<3.14. HTTP Runs/session APIs, ACP stdio, and TUI JSON-RPC/WebSocket exist. Upstream recommends TUI for full desktop parity. HTTP bearer access is powerful and tools execute on the server (`split_runtime=false`). Runs SSE has one consumptive queue, disconnect drops transport, and no replay/fanout is provided. MCP defaults to full trust; `readOnlyHint` is not a security boundary. Custom provider `base_url`/`api_key` are supported. Native Windows is documented, not runtime-tested, except the embedded POSIX PTY chat pane. Bounded local research found no Orca source/API and no local Mac Hermes installation; do not generalize that into nonexistence. Beads is initialized in embedded local mode with zero issues; no service is needed.

These are supplied source/code findings, not this PO's runtime verification. Exact HTTP compatibility, restricted tools and security behavior still need executable proof.

### Q13 — MVP transport versus full desktop parity

**Decision (D):** Select an **owned the configured runtime host control-plane adapter over Hermes HTTP Runs/session APIs as the proposed MVP seam**, conditional on a compatibility/security spike. Preserve the bounded Gmail/generated-dashboard workflow rather than promise all upstream desktop/TUI features. Do not finalize an adapter contract solely from API existence.

**Reason:** An owned seam can enforce product identity, grants, redaction and recovery without coupling the desktop to full TUI parity. Upstream TUI recommendation remains acknowledged; HTTP suitability must be demonstrated for this narrower job.

**Rejected/deferred:** Full parity in MVP; passing Hermes's powerful bearer directly to desktop custom code; browser/desktop sessions treated as raw Hermes sessions; treating ACP as remote device authentication; using hints/prompts as tool authorization. If server-side restrictions cannot be enforced, remain blocked or revisit the seam rather than silently grant all tools.

**Acceptance (D):** On the configured runtime host, using only synthetic data/permitted gateway, demonstrate authenticated create/run/status/cancel/error flow and session ownership; record request/response/status with secrets redacted. Negative cases reject absent credentials, wrong workspace/run scope and disallowed tools at the effective execution boundary. Verify custom endpoint/model request behavior and no provider fallback to an unauthorized route. Document omissions versus TUI and compatible pinned upstream version. No live account, paid route or credential forwarding is authorized.

**Prerequisites (B):** Adapter compatibility spike; actual upstream/configurable tool enforcement; powerful-bearer secret custody; product-side authenticated transport and device authorization; runtime resource check. Product data/account permission remains separate from network reachability.

**Provenance:** HTTP seam is PO-selected research recommendation (D), not an upstream guarantee. Backend placement/safety comes from founder infrastructure constraints. Pin/API findings are coordinator-supplied (V).

### Q14 — Event, approval and cancellation identity

**Decision (D):** Scope product events/actions to explicit workspace, connection, executor device, run, dashboard revision, applicable snapshot and grant generation. Use stable event IDs, monotonic per-run sequence and correlation identity; scope any future approval to its specific proposed action and current run/grant. A session ID is not a grant. Consequential actions stay unavailable in MVP.

One owned upstream SSE consumer per run feeds the adapter's durable event/status record. Multiple product observers must not compete on the consumptive upstream queue. Reconnect reconciles status before new work; if status cannot be established, show unknown/interrupted, never manufacture replay or success. Idempotency and durable replay are product semantics to prove, not asserted upstream behavior.

Cancellation invalidates local delivery/publication, requests upstream cancellation and reconciles actual terminal status. It is not rollback of work already performed server-side. Revoked/old-generation results stay rejected after reauthorization. Do not retry ambiguous execution as a fresh run automatically.

**Reason:** Prevents cross-scope leakage, duplicate work and false completion under disconnect/retry. Correctness now supports later device observers without claiming sync exists.

**Rejected/deferred:** Multiple direct SSE subscribers, implicit fanout/replay, optimistic success on disconnect, cancellation-as-undo, reusable blanket approval tokens, and automatic replay of unknown work.

**Acceptance (D):** Synthetic tests cover disconnect before/after run creation, lost create response, duplicate events/clicks, adapter restart, one consumer enforcement, revocation during generation and late terminal events. Demonstrate either reconciled actual status or explicit unknown/interrupted; no duplicate run without deliberate new request. Persist sanitized event metadata without prolonging account-content retention. Future observers consume only adapter-owned scoped records. Only proven upstream cancellation may be labeled remotely canceled; local stop alone is reported distinctly.

**Prerequisites (B):** Pinned API status/cancel semantics; adapter event persistence/recovery design; authenticated connection/device identity; content retention and grant-generation tests.

**Provenance:** Consumptive/nonreplay SSE facts are coordinator research (V); event protocol, fail-closed recovery and identity are PO decisions (D), pending implementation proof.

### Q15 — First authored-UI demo, safety gate, ownership and repair

**Decision (F/D):** Support two first-class paths: trusted declarative catalog mode and **genuine generated React/TypeScript presentation code** guided by design tokens/components. Custom code can create new layouts/presentation logic, not just rearrange presets. The historical blanket ban on all generated JSX/JavaScript/CSS/bespoke presentation is superseded. Uncontained execution and host authority remain forbidden.

First authored-UI demonstration: synthetic mail only; actual permitted-gateway generation → isolated preview → operator-requested conversational revision → explicit save → cold restart/reopen → rollback. A prerecorded artifact is allowed for playback/negative tests but does not satisfy genuine generation/revision acceptance. Clearly label synthetic inputs and any simulated integration steps. This path is independent of Gmail compliance, account consent, remote Hermes readiness and Orca; no hidden dependency should postpone it behind those gates.

**Containment:** Generated code never runs in the privileged desktop host. Use a pinned host-supplied dependency bundle/design primitives; no arbitrary package installation, remote scripts/assets, credentials, native IPC, ambient network, connector authority or device enrollment. The isolation mechanism remains open for a dedicated spike. Test process/origin boundaries, protocol/bridge spoofing and replay, all egress classes, compiler/dependency escape, CPU/memory exhaustion, watchdog termination and host recovery on each target OS. Iframe/CSP labels alone are not proof. Even synthetic code needs baseline resource/host containment before preview; no data secrets does not mean no host risk.

**Bridge:** Host validates identity/origin/session, message schema/size/rate/correlation, current grant and approved bounded data projection. Generated code cannot create trustworthy user gestures. Refresh/navigation that affect the outside world require host-owned operator controls/confirmation; do not authorize them based solely on a custom message saying “clicked”. Catalog/custom rendering alone never accesses Gmail. Live data is blocked until adversarial runtime proof and independent review; Gmail compliance/consent still apply separately. Revocation stops delivery, rejects queued messages, terminates the runtime and purges sensitive derivatives; data already delivered cannot be retroactively undisclosed.

**Ownership/versioning:** The operator owns source/specification, preview and saved artifacts inside the workspace. Saved revisions are immutable and include mode, source/spec hash, parent revision and schema/compiler/runtime/design-system versions. Save/repair/rollback controls live in the trusted host. A repair creates a new preview; it does not silently change the saved dashboard or escalate permissions. Compile/runtime failure preserves last-working state and an understandable error. Rollback restores presentation, not expired snapshots/revoked grants. Incompatible runtime versions preserve the artifact for repair/export rather than executing blindly. Source/bundles containing connection-derived content are sensitive and expire/purge with the connection; prefer schema/synthetic inputs for code generation and never assume source is content-free.

**Acceptance (D/H):** Demonstrate actual generated custom layout/presentation logic beyond catalog arrangement; capture synthetic generation/revision receipts without secrets; pass preview/save/restart/repair/rollback scenarios. Inject invalid source, compile failure, runtime throw, infinite loop, allocation exhaustion, bridge spoof/replay and egress attempts; host remains recoverable and last-working revision intact. Spike must set finite enforceable resource budgets; proposed host-responsive termination target <=2 seconds after watchdog/limit violation. Native macOS/Windows receipts required for respective claims. No fixture success counts as live-data bridge proof. Live custom UI additionally passes scoped projection, current-grant recheck, expiry/revocation termination and independent review before any real data delivery.

**Rejected/deferred:** Removing custom UI because it is hard to sandbox; catalog-only presented as satisfying the founder's new direction; arbitrary dependencies; privileged execution; declaring safety from static checks or browser sandbox flags alone; silent self-repair of saved code. Charts are not necessary for this first Gmail demo, but custom presentation is not confined to the initial catalog.

**Prerequisites (B):** Designer's authored-UI/token guidelines; isolated runtime/resource/bridge spike and OS evidence; pinned compilation/dependency contract; revision storage/repair rules; permitted synthetic model route. For live only: verified grants, secure storage, Gmail compliance and operator account/egress consent.

**Provenance:** Genuine custom UI is founder direction relayed by coordinator and recorded in ADR 0001 (F/V). Two-mode staging, interaction/repair defaults and thresholds are delegated PO choices (D/H). None is account authorization or a claim that isolation is solved.

### Q16 — May foundation discovery close with explicit spike gates?

**Decision (D):** Yes. **I confirm delegated shared understanding for the foundation only:** product scope, staged delivery, authority boundaries, measurable criteria and a dependency backlog are sufficiently agreed to record the foundation plan. Technical spikes are explicit unpassed gates, not silently settled facts. This does not declare the app implemented, the entire future architecture finalized, human consent obtained, or REVIEW/QA passed.

The ready independent path is design/catalog/custom-runtime investigation → native shell/revision foundation → synthetic authored-UI preview/revise/save/rollback → independent review and native QA. Gmail compliance and Hermes/remote adapter research proceed separately and block real integration, not the core fixture capability. Orca remains an evidence-gated roadmap branch, not a prerequisite for the first fixture or Gmail slice.

**Acceptance (D):** Docs explicitly supersede Q2/Q8, distinguish founder/PO/policy/runtime evidence, and match ADR 0001 and glossary. Backlog specification has 15–22 open epic/task definitions with owner, priority, observable acceptance and acyclic dependencies; no invented Beads IDs or closed work. A coordinator may then create actual Beads issues. Missing technical/legal/native-QA evidence remains visible. Foundation document validation is not application runtime QA.

**Remaining implementation gates (B):** (1) custom runtime choice and enforced resource/origin/bridge/egress isolation on both OSes; (2) artifact/compiler/design-system versioning and repair/storage contract; (3) scoped grants/retention/secret custody including late results and tainted code; (4) HTTP adapter compatibility/tool restrictions and event reconciliation; (5) lawful Gmail scope/verification/Limited Use/assessment path and actual consent; (6) native packaging/performance/accessibility evidence; (7) independent review and runtime QA; (8) upstream/Orca compatibility and authorized license/distribution decisions for release. Signing, spending, account access, commit/push and publication remain unauthorized.

**Provenance:** Foundation-only confirmation is delegated PO approval of the plan, not founder approval of sensitive acts. The coordinator authorized this document/backlog-spec work; no actual Beads mutations or operational services are needed here.

## Testable next issues

These are **proposed issue references, not Beads IDs**. The requested local backlog JSON uses the same keys and includes full acceptance criteria. The coordinator materializes actual issues and prerequisites idempotently. No issue is represented as closed. `Depends on` means prerequisites, not implementation claims; external consent/security gates in acceptance also remain binding.

| Ref | Suggested owner | Work / completion evidence | Depends on |
| --- | --- | --- | --- |
| P01 | Code | Pinned Hermes HTTP compatibility/tool-enforcement spike on the configured runtime host; synthetic authenticated run/status/cancel/error receipts or explicit blocker. | None |
| P02 | Product owner + research | Gmail scope/search/snippet and restricted-data compliance evidence; lawful exact-flow plan or blocker, no credentials requested. | None |
| P03 | Product owner + code | Later Orca interoperability and Beads/agent-work dogfood: verify real source/API; view scoped work read-only; repo changes require explicit approvals; no auto-merge or self-modifying trusted host. | P11 |
| P04 | Code | Workspace/connection/run/device/grant and egress/revocation contract; enforce call/delivery/commit boundaries and late-result rejection. | P01, P02 |
| P05 | Designer | Design tokens/components and catalog/custom preview/revise/save/repair/rollback flows, fixture labels and accessibility state matrix. | None |
| P06 | Code | Tauri/platform/storage spike and ADR: finite resource feasibility, revision/snapshot separation, atomic recovery, intended SQLite and OS-secret-store plan. | P05 |
| P07 | Code | Trusted catalog fixture renderer with bounded validation, explicit refresh and save/reopen/error tests; no account calls. | P05, P17, P19 |
| P08 | Independent reviewers | Required multi-lane fixture/custom-runtime review; findings resolved or genuine blockers retained; no fabricated missing-lane pass. | P07, P16 |
| P09 | Code | Permitted gateway synthetic generation contract, custom endpoint/key safety and no unauthorized fallback; independent of Gmail/Hermes remote readiness. | None |
| P10 | Code | Actual bounded Gmail connection only after compliant path and operator account/egress consent; <=100/7-day/inbox, metadata baseline, zero mutations. | P02, P04, P09, P12, P18, P21 |
| P11 | Independent reviewers + QA | Live catalog/custom bridge security review and native/backend runtime receipts, including expiry/revocation/in-flight tests; PASS only for proven scope. | P10, P20 |
| P12 | Code | Owned the configured runtime host adapter: authentication, powerful-secret custody, one upstream SSE consumer, durable scoped events, status reconciliation and cancellation semantics. | P01, P04 |
| P13 | Product owner + authorized release owner | Upstream/dependency license compatibility, actual license/brand/release authorization and review/QA receipts before any distribution. No automatic publication. | P01, P11 |
| P14 | Product owner | Epic: staged authored-UI workspace delivered with fixture and connected evidence; exclusions/authorization boundaries preserved. | P18, P22 |
| P15 | Code + security review | Synthetic custom-runtime/compilation/bridge/resource/egress spike with adversarial native evidence; select mechanism or remain blocked. No Gmail dependency. | None |
| P16 | Code | Genuine synthetic React/TypeScript generation/revision through permitted gateway, isolated preview/save/reopen/repair/rollback beyond catalog rearrangement. | P05, P09, P15, P17, P19 |
| P17 | Code | Local immutable revision/snapshot/artifact persistence, atomic last-valid save, version compatibility and repair/rollback contracts on synthetic data. | P06 |
| P18 | QA | Native macOS and Windows fixture cold-launch/screenshots, performance/accessibility and adversarial recovery receipts; separate each platform verdict. | P08 |
| P19 | Code | Tauri v2 React/TypeScript host shell with trusted controls, workspace/device identity and credential-free fixture mode; no Electron. | P05, P06 |
| P20 | Code | Live custom-data bridge only after isolation proof, effective grants and explicit consent; projection/rate/origin checks, runtime revocation and tainted-artifact purge. | P10, P15, P16, P21 |
| P21 | Code | Secure connection storage and privacy lifecycle: seven-day content/30-day content-free audit, staged-result cleanup, expiry/revocation and current-grant checks. | P02, P04, P06, P17 |
| P22 | Product owner | Five-operator formative pilot with actual consent and measured return/benefit findings; report target misses honestly. | P11 |

**Independent fixture path:** P05/P06/P09/P15 → P17/P19 → P07/P16 → P08 → P18. No transitive Gmail, real-account, remote Hermes or Orca dependency. P14 is an epic roll-up, not a prerequisite for its own tasks.

## Foundation conclusion and remaining implementation frontier

Delegated foundation shared understanding is confirmed in Q16. Ready product decisions and testable open spikes are sufficient for the foundation plan; they are not evidence that the spikes or app passed. The historical catalog-only exclusion is superseded in Q2/Q8/Q15 and ADR 0001.

Next evidence-dependent choices: exact custom-runtime isolation/resource/compilation mechanism; artifact repair/version compatibility and durable store contract; effective server-side grant enforcement; HTTP adapter identity/event recovery; exact lawful Gmail retrieval/egress scope. Orca/device pairing/sync stay gated roadmap work; viewing Beads/agent activity through the app is a later dogfood milestone, not permission for autonomous repository changes. No auto-merge or self-modifying trusted host.

Do not close an implementation issue from a product decision, static test, fixture stand-in, nonexistent review lane, or unavailable native OS. Account access, license selection, paid-provider activation, signing, commit/push and publication still require their actual authorization gates.
