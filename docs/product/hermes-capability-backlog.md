# Hermes capability backlog (Phase 2)

Companion to [ADR 0004](../adr/0004-hermes-native-capability-bridge.md) (`status: proposed`). This is a decision-and-backlog list only: nothing below is implemented, and no review, build, launch, runtime QA, screenshot, or capability verification has been performed. Every job below owes its own evidence. Dependency-ordered, highest leverage first; the browser job is ranked first because the founder addendum (2026-09-13) makes a real multi-tab headful browser with persistent per-account profiles the primary product requirement, and every other adoption (Interfaces revision, cron proposals, app-state reads, self-improvement) presumes the agent can see and drive the workspace's real browser.

## Decisions distilled from ADR 0004

1. Hybrid adoption: Hermes native browser (local CDP attach to Forma-owned patchright headful Chromium) replaces automation driving; Scrapling retained for fetch-only paths.
2. Forma's `cron_proposals.py` approval lifecycle is kept; native cron may at most execute approved schedules.
3. Connector-first account access; browser real-profile flows are additional, consent-gated.
4. Adopt memory, skills, subagents, MCP client behind host gates.
5. Bridge = local authenticated MCP server consumed by Hermes' MCP client.
6. Camofox = assessed alternative pending a verified non-Docker path; cloud stealth backends = future paid opt-in only, excluded from this build.

## Gate A (blocking prerequisite, first job)

**A. Pinned-build capability verification.** Scope: read the actual Hermes pin in `scripts/managed-hermes` and `runtime/forma_runtime`; fetch upstream `/docs/llms.txt` and relevant `/docs/llms-full.txt` sections; convert every UNVERIFIED row in ADR 0004 to SUPPORTED / NOT-IN-PINNED-BUILD with file or doc anchors. Acceptance evidence owed: updated ADR matrix plus a written verification note; no build required.

## Phase 2 implement jobs

**1. Forma-owned headful multi-tab browser driven by Hermes over local CDP.** Scope: Forma launches and renders a patchright-patched headful Chromium window with multi-tab management, persistent per-account profiles (cookies/sessions survive restarts), and per-account profile switching; Hermes' native browser toolset attaches via local CDP when automating; the human uses the same window directly otherwise; human-like pacing (typing cadence, dwell, scroll) lives in the agent layer. Required features: multi-tab management, per-account profile switching, honest automation status surfaced in the Forma UI (always visible when the agent is driving). Egress via the existing relay; consent-gated real-profile targets; no cloud stealth backends. Evidence owed: build, launch, exercise (human browse + agent-driven tab automation with status shown), screenshot on mini-mac.

**2. Forma control bridge (MCP server) — browser and app-state tools.** Scope: local authenticated app-owned MCP endpoint exposing `browser.*` tab tools (delegating to job 1's window), `app.state.read` (bounded, redacted context envelope), with the routine/consequential classification and approval gates from ADR 0004. Depends on job 1 and Gate A (MCP client confirmed in pinned build). Evidence owed: build, launch, exercise a tool round-trip with a denied ungranted call, screenshot on mini-mac.

**3. Bridge — Interface create/revise tools.** Scope: `interface.create` / `interface.revise` / `interface.delete` against the existing runtime Interface contracts (stable IDs, revisions, provenance, expected-revision conflict handling, deletion cancelling pending mutations). Preserves ADR 0001: invalid generation leaves the last working revision usable. Depends on job 2. Evidence owed: build, launch, exercise concurrent-edit conflict and deletion permanence, screenshot on mini-mac.

**4. Bridge — cron proposal tool.** Scope: `cron.propose` mapped onto `runtime/forma_runtime/cron_proposals.py` unchanged semantics: proposal is side-effect-free, `ApprovalLedger` host approval required, ADR 0003 schedule rules (start disabled, restart cannot renew authorization). Evidence owed: build, launch, exercise approve/deny/missed-slot handling, screenshot on mini-mac.

**5. Adopt Hermes memory (FTS5) behind workspace scoping.** Scope: expose persistent memory per app identity via the bridge (`memory.read`/`memory.write`, redacted); no credentials into memory; remembered content treated as untrusted data. Depends on jobs 2 and Gate A. Evidence owed: build, launch, exercise cross-session recall and redaction, screenshot on mini-mac.

**6. Adopt skills + self-improvement behind operator approval.** Scope: `skills.create`/`skills.revise` as consequential tools; host-owned skill storage; skills cannot mint grants or approvals. Depends on jobs 2 and 5. Evidence owed: build, launch, exercise an approved and a denied skill write, screenshot on mini-mac.

**7. Adopt subagents + programmatic tool calling with per-invocation grant checks.** Depends on jobs 2 and 6. Evidence owed: build, launch, exercise a subagent tool call with a revoked grant rejected, screenshot on mini-mac.

**8. Hybridize cron execution / connector-browser flows (optional, last).** Scope: only if jobs 1–7 land cleanly; connector-first policy unchanged. Evidence owed: build, launch, exercise, screenshot on mini-mac.

Each implement job preserves all host-approval invariants (approval ledger, grants, expiry/revocation, redaction) and must not regress unrelated functionality.

## Orca integration — NOT-IMPLEMENT-YET (discovery-first)

Founder wants an Orca integration so Hermes can pilot the founder's Orca subagents. This is recorded as a backlog note only; **do not implement**. Prerequisite: discovery of the Orca control surface (interface, auth model, safety boundary) comes after the ADR 0004 spike and before any design work. No Orca code, config, or contact is authorized by this note.

## Out of scope this pass

- Messaging platforms and voice/TTS: out of scope and remain disabled pending separate authorization.
- Media-related toolsets: remain disabled pending separate authorization; the absolutely frozen media provider is not named, contacted, inspected, or scheduled for access anywhere in this slice, and its absolute freeze is unaffected by anything here.
- No paid providers, subscriptions, or paid hosted gateways in any proposed path; no pushes outside the publisher.
