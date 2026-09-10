---
status: accepted
---

# Hermes is an app-owned desktop runtime

Date: 2026-09-10. Authority: explicit founder correction, not a new PO hypothesis or a runtime PASS. Applies to `desktop-ai-dashboard-cid` and the [agent/Interfaces product contract](../product/agent-interfaces.md).

## Decision

Forma is the GUI on Hermes. The native application automatically launches, supervises, and stops its own Hermes runtime. Users configure supported model/provider credentials and select models normally; they do not deploy Hermes, install a service, configure runtime URL/token/enabled fields or Tailscale, or select a Direct chat route. Provider configuration is distinct from forbidden Hermes runtime setup. No silent direct-provider or model fallback.

Use an actual pinned Hermes agent engine or verified structured session protocol behind the native host. Prefer app-supervised stdio. Private dynamically allocated loopback is acceptable only when the native app owns the endpoint/process, authenticates the session with ephemeral private credentials, prevents unrelated local/browser access, and cleans up on quit. A fixed user-entered endpoint or an unauthenticated localhost listener is not this decision. The renderer receives bounded product events/commands, never runtime credentials or arbitrary Hermes APIs.

The app owns per-installation/profile durable sessions, Interface revisions, schedules and run records. Keep native credentials in the owning device's secure store and deliver only the minimum required through a reviewed native-to-owned-runtime boundary, never renderer readback, command-line secrets, logs, or automatic transfer to a development server. Approved model requests still egress to the operator-selected provider. Runtime startup alone makes no model/account request and grants no authority.

Native supervision covers packaging/version compatibility, startup readiness, bounded crash recovery, event/status reconciliation, and termination of owned descendants on app quit. Show Starting, Ready, Recovering, or Unavailable with actionable retry; never replace missing packaging with a runtime-configuration screen. Preserve chats, drafts, accepted per-workspace model choices, immutable Interface revisions, schedule limits and grant generations during migration. Reuse useful existing Hermes engine/interfaces/scheduling implementation; no wholesale removal of the prior unstaged work.

## Honest scheduling

Persist schedules in app-owned storage; a webview timer is not durable infrastructure. Account-backed schedules start disabled and require a separate host-owned explicit enable action with current scoped grants, model-egress consent, UTC cadence, finite attempt/deadline limits and retention disclosure. Model/tool content cannot enable them. Existing authorized enablement may resume only within the same valid scope and remaining limits; restart cannot grant, renew, expand or reset authorization.

Background means while the native app and its runtime remain alive and the device is awake. Window-close behavior must explicitly distinguish a still-running app from Quit. Quit terminates the sidecar. Sleep, app exit, shutdown and powered-off devices stop execution. Relaunch/wake reconciles interrupted work, records missed occurrences and advances to the next due time without a catch-up storm. Bounded retries, occurrence identity, leases and fenced publication prevent overlap and stale writes; no exactly-once promise. No independent daemon, login item, wake/automation permission, or off-device execution is implied or authorized.

## Supersession and unchanged gates

This supersedes cid's earlier required miniforum application server, server-owned customer storage, customer runtime connection controls, Direct chat choice, and continuous execution after app quit. Conflicting historical transport/design notes are historical only. Existing external servers, test fixtures, and deployment receipts remain useful engineering assets on miniforum; **external-server/deployment QA is NOT the customer product and cannot satisfy desktop-owned runtime acceptance**. The actual app-owned sidecar is part of the native application, not an unrelated backend relocated to the laptop.

Keep registered Google client, supported scope/compliance, explicit connection/operation grants, expiry/revocation, retention, and selected-model/data-category egress gates. Local execution does not waive them or confer OS permissions/account approval. Higgsfield's absolute freeze remains unchanged: no account, credential, generation, upload, status/balance/model/cost, authentication or retry access without explicit new user authorization. No arbitrary generated JS/HTML/native commands execute in privileged desktop or runtime hosts; custom UI remains independently containment-gated. ADRs 0001 and 0002 remain binding.

## Acceptance and evidence

An isolated native cold launch, with only ordinary model/provider configuration and no preconfigured Hermes service, must start owned Hermes and demonstrate an actual reasoning/text/tool run, workspace-scoped accepted model change, chat/draft preservation, shared Interface revision/reopen behavior, and real sidecar failure/recovery/quit cleanup. Prove session isolation and execution-time tool denial, not prompt-only restrictions. Synthetic schedule QA must exercise app-alive work, actual window-close/quit semantics, restart/wake reconciliation, missed slots, finite limits and late-result rejection. Model/tool receipts are distinct from synthetic Google data; real Google remains separately blocked until authorized and evidenced.

Independent multi-lane review and runtime QA remain required; missing lanes or platforms remain incomplete. Record actual app identity, process ownership, source hash and native screenshots, with browser/macOS/Windows verdicts separate. This documentation amendment implements nothing, runs no builds or accounts, and authorizes no commit, push, deployment or publication.
