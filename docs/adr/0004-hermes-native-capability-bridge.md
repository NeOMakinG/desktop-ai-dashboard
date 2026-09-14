---
status: proposed
---

# ADR 0004: Hermes native capability adoption and the Forma control bridge

Date: 2026-09-14. Authority: founder strategic directive 2026-09-13 plus founder addendum 2026-09-13 (multi-tab headful browser as primary product requirement). This ADR is a **spike document only**: it implements nothing, changes no source, config, or dependency, and authorizes no build, launch, account access, or publication. Status is `proposed`; nothing here is a shipped-capability claim. ADRs [0001](0001-generated-ui-is-a-product-capability-not-host-authority.md), [0002](0002-local-ownership-does-not-imply-zero-egress.md), and [0003](0003-hermes-is-an-app-owned-desktop-runtime.md) remain binding and are **not amended** by this document.

## Context

Forma already bundles Hermes as its app-owned desktop runtime (ADR 0003), pinned via `scripts/managed-hermes` and executed through `runtime/forma_runtime`. Upstream Hermes (hermes-agent.nousresearch.com, MIT) documents a broad native capability surface: browser automation (tools `browser_navigate/snapshot/click/type/scroll/press/back/get_images/vision/console/cdp/dialog`; backends including local CDP attach, packaged Chromium via agent-browser, Camofox, Lightpanda; hybrid routing; consent-gated real-profile browsing; headed mode), cron automations, persistent FTS5 memory, a skills system with autonomous skill creation and self-improvement, an MCP client, subagents with programmatic tool calling, 20+ messaging platforms, web search, vision, voice/TTS, personality via SOUL.md, context files, and a security/approval layer.

The strategic question is how much of our custom machinery should be replaced by, or hybridized with, native Hermes capabilities, and what Forma control surface should be exposed back to Hermes so the agent can drive the app (browser window, Interfaces, cron proposals, app state, its own skills/memory) while Forma remains the security boundary.

**Evidence caveat (binding for this whole ADR):** this spike was performed as a documentation-only review of Forma-side source documentation available in this job (`docs/dev/host-approval-contracts.md`, `docs/product/agent-interfaces.md`, ADRs 0001–0003). No upstream docs were fetched and no runtime was inspected within this job; every claim about upstream Hermes below is attributed to the upstream documentation index (`/docs/llms.txt`, `/docs/llms-full.txt`) or the task directive, and every claim about the pinned build is marked **UNVERIFIED** unless it derives from a Forma-side file actually read here. Nothing below claims review, build, launch, runtime QA, screenshots, or capability verification that was performed; none was performed in this job.

## Bundled-runtime inventory

The pinned Hermes runtime Forma ships is managed by `scripts/managed-hermes` (pinning, packaging, supervision) and executed via `runtime/forma_runtime` (ADR 0003). **UNVERIFIED in this job:** the exact upstream version/revision string of the pin was not read here — no fetch of `scripts/managed-hermes` contents occurred. Resolving the pin and diffing it against upstream is the first gate of the Phase 2 backlog job A (see the companion backlog) and a precondition before any SUPPORTED mark below may be asserted.

Per-capability marks (source anchor in parentheses; "directive" = founder task description of 2026-09-13, which itself cites upstream docs):

| Upstream capability | Mark | Anchor / note |
|---|---|---|
| Browser toolset (navigate/snapshot/click/type/scroll/press/back/get_images/vision/console/cdp/dialog) | UNVERIFIED in pinned build; described upstream (directive) | Not observed in `runtime/forma_runtime` in this job |
| Browser backends: local CDP attach | UNVERIFIED in pinned build (directive) | Candidate backend for the founder browser requirement; packaging must be confirmed |
| Browser backends: agent-browser packaged Chromium | UNVERIFIED in pinned build (directive) | |
| Browser backends: Camofox (anti-detect Firefox, persistent sessions, tab adoption, VNC view, Docker) | UNVERIFIED in pinned build (directive) | Docker/VNC shape assessed below for desktop-bundle fit |
| Browser backends: Lightpanda; hybrid routing; consent-gated real-profile browsing; headed mode | UNVERIFIED in pinned build (directive) | |
| Cron automations | UNVERIFIED in pinned build (directive) | Forma side has `runtime/forma_runtime/cron_proposals.py` (approval-gated proposals; see host-approval-contracts.md) |
| Persistent cross-session memory (FTS5) | UNVERIFIED in pinned build (directive) | |
| Skills system, autonomous skill creation, self-improvement | UNVERIFIED in pinned build (directive) | |
| MCP client | UNVERIFIED in pinned build (directive) | Relevant to bridge transport choice |
| Subagents + programmatic tool calling | UNVERIFIED in pinned build (directive) | |
| Messaging platforms (20+) | Out of scope this pass | Directive; recorded as disabled pending separate authorization |
| Voice/TTS | Out of scope this pass | Directive |
| Web search / vision | UNVERIFIED in pinned build (directive) | Media-related vision generation stays disabled pending separate authorization |
| Context files / SOUL.md personality | UNVERIFIED in pinned build (directive) | Maps to Forma's host-owned context envelope (agent-interfaces.md §2) |
| Security / approval layer | UNVERIFIED in pinned build (directive) | Forma's own host-approval contracts (host-approval-contracts.md) remain controlling regardless |

Anything marked UNVERIFIED must be confirmed by direct inspection of the pinned build before Phase 2 jobs rely on it; upstream doc sentences are **not** evidence of pinned-build behavior.

## Capability-adoption matrix

### REPLACE-CANDIDATES

**1. Custom Scrapling engine vs Hermes native browser tools — recommendation: HYBRID (replace automation driving with Hermes browser over local CDP attach to a Forma-owned headful Chromium; keep Scrapling for non-interactive fetch/extract paths).**
Reason: the founder addendum makes a real headful multi-tab browser with persistent per-account profiles the primary product requirement; Hermes' native browser toolset driven over CDP attach against a Forma-owned window serves R04's interactive-browser goal better than our fetch-oriented engine. What must be preserved if replaced: host approval ledger semantics for consequential browsing actions, capability grants with expiry/revocation, payload redaction before anything is logged or echoed to chat, and egress through the existing relay. This is a Phase 2 gate, not a settled outcome.

**2. `runtime/forma_runtime/cron_proposals.py` vs Hermes native cron — recommendation: KEEP Forma's proposal layer, HYBRID at most for execution.**
Reason: our module implements the model-proposes/host-approves lifecycle (validated proposals, `ApprovalLedger`, one explicit decision per proposal id) exactly as documented in host-approval-contracts.md; native Hermes cron has no observed equivalent of our operator-approval ledger, grant generations, UTC cadence disclosure, or finite attempt limits required by ADR 0003. Any adoption must preserve: approval ledger, denied/duplicate proposal exclusion, expiry/revocation, app-owned durable storage of schedule definitions and run records, and the ADR 0003 rule that schedules start disabled and restart cannot renew authorization.

**3. Per-tool Google/Composio reads vs Hermes real-profile browser flows — recommendation: HYBRID, connector-first.**
Reason: verified API connectors (supported Google authorization, scoped grants, typed receipts) remain the compliant path for mail/calendar; browser real-profile flows are an additional, consent-gated surface for services without verified contracts, never a bypass of service authentication (vision.md browser rules). Preserved invariants: connection/operation grants, `ActionGrant` expiry and revocation, redaction of payload/receipt content, and the rule that a browser session is not an API credential and cannot fabricate a connected state.

### ADOPT (each contingent on pinned-build verification)

- **Memory (FTS5):** adopt behind Forma-owned workspace scoping; wiring must keep memory per app identity, never ship credentials into memory, and treat remembered content as untrusted data on read.
- **Skills + self-improvement:** adopt with host-owned gates — skill creation/revision is consequential and requires operator approval; skills cannot mint grants or approvals.
- **Subagents + programmatic tool calling:** adopt only with the same tool-level grant checks applied per subagent invocation; no ambient host access.
- **MCP client:** adopt as the consumer side of the Forma control bridge (below), configured to point only at Forma's local authenticated bridge.

## Browser backend comparison (founder addendum criteria)

Criteria: (a) real headful window Forma owns and renders; (b) multi-tab; (c) persistent per-account profiles surviving restarts; (d) human-usable when not automating; (e) residential-IP anti-detection posture (human+agent sharing one real profile on the user's own IP); (f) desktop-bundle fit (Tauri app-owned, pinned, no daemon).

| Criterion | Patchright headful Chromium + CDP attach | Hermes Camofox backend | Plain Chromium + CDP, agent-layer stealth pacing |
|---|---|---|---|
| Headful Forma-owned window | Yes; app launches and renders the window, Hermes attaches via local CDP | Headful in origin (Firefox), but upstream shape is Docker + VNC view — VNC-in-a-window is not a native owned window on desktop | Yes |
| Multi-tab | Yes (CDP targets) | Yes (tab adoption noted upstream) | Yes |
| Persistent per-account profiles | Yes (per-account user-data dirs) | Yes (persistent sessions upstream) | Yes |
| Human-usable when idle | Yes — it is just Chromium | Via VNC only; poor desktop fit | Yes |
| Anti-detection posture | Strong: CDP-detection patches plus real headful profile on residential IP | Strong (purpose-built anti-detect) but containerized | Moderate; plain CDP fingerprint detectable, pacing alone insufficient |
| Desktop-bundle fit | Good: single binary, app-supervised, ADR 0003-compatible | Weak-to-moderate: Docker/VNC may not suit a desktop app bundle; keep as assessed alternative, not dismissed — a future job may evaluate a locally-run Camofox if upstream supports non-Docker use (currently UNVERIFIED) | Best packaging, weakest posture |

**Recommendation: patchright-launched headful Chromium + local CDP attach, driven by Hermes' native browser toolset, with the window owned and rendered by Forma.** Cloud stealth backends (Browser Use cloud, Browserbase — residential proxies, CAPTCHA solving) are **PAID** and are documented here as a **future explicit opt-in only**, excluded from this build; no paid provider, subscription, or paid hosted gateway (including any paid upstream portal) is part of the implementation path. Human-like pacing (typing cadence, dwell, scroll) comes from the agent layer, not from fingerprint tricks. Required Phase 2 features: multi-tab management, per-account profile switching, honest in-UI automation status. Real-profile automation on the operator's own sessions carries ban and data-exposure risk and touches service terms; this ADR records it as a design target with explicit consent requirements, not as an authorization.

## Bridge architecture: the Forma control surface

**Transport choice: MCP server (local, app-owned, authenticated), consumed by Hermes' MCP client — recommended over a native toolset fork.** Rationale: MCP is upstream-supported (UNVERIFIED in pinned build — gate A confirms), keeps our pinned runtime unpatched (ADR 0003 pin discipline), is versioned independently of the Hermes pin, and matches the roadmap's authenticated MCP surface (vision.md). A native toolset would require patching the pinned build and re-reviewing every pin bump.

Boundary conditions: the bridge binds only on an authenticated private loopback endpoint owned by the native app (ADR 0003 loopback rules); all browser egress flows through the existing egress relay; no secrets cross the bridge in either direction; tool results are redacted per host-approval-contracts.md before reaching the model.

Tool surface, each classified routine (executes under existing grants without a new prompt) or consequential (requires a host-owned operator approval):

| Tool | Class | Approval gate |
|---|---|---|
| `browser.open_tab` / `browser.close_tab` / `browser.activate_tab` | Routine within an active browser session grant | Browser session capability grant (scoped, revocable, expiry) |
| `browser.navigate` / `click` / `type` / `scroll` (delegating to Hermes native browser over CDP) | Routine within grant | Same grant; per-site consent for real-profile (logged-in) targets |
| `interface.create` / `interface.revise` | Create routine; revise consequential when replacing a newer revision | Host revision/conflict contract (expected-revision checks; invalid output preserves last working version per ADR 0001 / agent-interfaces.md §4) |
| `interface.delete` | Consequential | Explicit operator confirmation, cancellation of pending mutations |
| `cron.propose` | Consequential (proposal itself side-effect-free) | `ApprovalLedger` entry in `runtime/forma_runtime/cron_proposals.py`; ADR 0003 schedule-enable rules |
| `app.state.read` (bounded product context: workspaces, interfaces, grants, automation status) | Routine | Host-owned context envelope; no credentials, no unrelated chat history |
| `skills.create` / `skills.revise` | Consequential | Operator approval per skill write; skills stored host-side, treated as untrusted data |
| `memory.write` / `memory.read` | Routine within workspace scope | Workspace scoping; redaction on read-back |

**Security invariants (restated, in effect):** Forma remains the security boundary. Host-owned approvals apply to every consequential action. No secrets reach the model or Python; model/tool content cannot self-authorize, mint grants, or create `ApprovalLedger` entries. Browser traffic egresses via the existing relay under operator consent for destination and data categories (ADR 0002). ADRs 0001, 0002, and 0003 remain binding and unamended.

## Decision list

1. Adopt, subject to pinned-build verification (backlog gate A): Hermes native browser over local CDP attach to a Forma-owned patchright headful Chromium (hybrid with Scrapling retained for fetch-only paths).
2. Keep Forma's `cron_proposals.py` approval lifecycle; at most hybridize execution.
3. Keep connector-first account access; browser real-profile flows are additional and consent-gated.
4. Adopt memory, skills, subagents, and MCP client behind the gates described above.
5. Bridge = local authenticated MCP server; tool classification and approval gates as tabulated.
6. Camofox stays an assessed alternative pending a verified non-Docker path; cloud stealth backends are future paid opt-ins only, excluded now.

## Non-goals

- No implementation in this spike: no source, runtime, config, dependency, build, or account changes; two docs files only.
- Messaging platforms and voice/TTS are out of scope this pass and remain disabled pending separate authorization.
- Media-related toolsets stay disabled pending separate authorization; the absolutely frozen media provider is not named, contacted, inspected, or scheduled for access anywhere in this slice, and its freeze is unaffected.
- No paid providers, subscriptions, or paid hosted gateways in any proposed path.
- No separate user-facing Hermes setup; the bundled runtime stays app-owned and pinned (ADR 0003).

## Evidence and gates

- Evidence used in this spike: Forma-side docs read in this job (`docs/dev/host-approval-contracts.md`, `docs/product/agent-interfaces.md`, `docs/product/vision.md`, ADRs 0001–0003, `CONTEXT.md`, `docs/design/system.md`) and the founder directive's summary of upstream docs. Upstream `/docs/llms.txt` and `/docs/llms-full.txt` were **not fetched in this job**; all upstream-derived rows are therefore UNVERIFIED and attributed to the directive/upstream docs, not asserted as Forma behavior.
- Gate A (before any Phase 2 browser job): read the actual pin in `scripts/managed-hermes`, fetch the upstream doc sections, and convert UNVERIFIED marks to SUPPORTED / NOT-IN-PINNED-BUILD with file or doc anchors. Only direct observation of the pinned build may justify a SUPPORTED mark.
- Gate B: every replace row's preserved invariants (approval ledger, grants, expiry/revocation, redaction) verified in the implementing job's tests.
- Phase 2 jobs owe mini-mac evidence: build, launch, exercise, screenshot — recorded as owed, not performed, by this ADR.
