# Hermes-backed workspaces and shared Interfaces

Status: priority implementation brief, amended by explicit founder correction on 2026-09-10; not a shipped-capability claim. [ADR 0003](../adr/0003-hermes-is-an-app-owned-desktop-runtime.md) supersedes earlier remote-runtime setup and Direct chat designs.

## Product outcome

Forma should feel like an intelligent personal workspace, not a generic text client. The main chat talks to a Hermes agent with the correct product context and actual capabilities. A separate **Interfaces** section in the sidebar holds persistent, named visual surfaces. Any authorized conversation can create and manipulate those interfaces.

## 1. Hermes is the application runtime

**Forma is the GUI on Hermes.** The native app automatically launches and manages its own Hermes runtime under the hood. There is one Hermes-backed chat path: no Direct chat route, route selector, or direct-provider fallback. The operator configures only supported model/provider credentials and normal model selection (including model-provider settings where needed), not a Hermes runtime URL, token, enabled toggle, Tailscale, service installation, or deployment.

The actual app-owned sidecar is part of the native application, not a standalone backend users configure. Prefer supervised stdio; private dynamically allocated app-owned loopback is an acceptable implementation seam only with authenticated ownership, ephemeral session credentials, bounded exposure, and shutdown cleanup. The native host owns startup/readiness, session recovery, bounded restart, version compatibility, and child-process termination on app quit. Missing packaging or supervision is a product blocker, not an instruction to configure an external server. Keep each app identity's runtime profile, storage, credentials, memory, and tool permissions separate from the development company's Hermes and test identities.

The adapter must carry user messages, accepted model selection, text/tool events, approvals, cancellation, completion, and reconnect state. Use a verified structured Hermes host/session protocol or actual pinned Hermes agent engine rather than treating tool-capable operation as a plain text-completion endpoint. Show understandable Starting, Ready, Recovering, or Unavailable states with safe retry; no runtime-setup form. Starting Hermes does not itself authorize a model request, account read, or schedule.

Preserve existing chats and drafts during migration. The model picker must update the Hermes agent serving the selected workspace, retain history, and avoid unexpectedly changing other workspaces. Keep Opus as the preferred available default and preserve explicit choices. Never silently use a different provider/model or a direct-chat fallback while presenting the session as Hermes-backed.

Tools execute on the host that owns the authorized data. A development agent on a build machine does not automatically have access to a user's desktop.

## 2. Give the agent the right context

Provide a host-owned context envelope containing Forma's identity and purpose, the current workspace, actual enabled capabilities, granted connections, available interface/component contracts, and relevant interface references/revisions. Do not send credentials or unrelated chat histories.

When asked what Forma does, the answer should explain this product and its currently available actions, rather than listing generic language-model abilities. Do not present planned connectors as connected. Markdown fallback should render correctly; structured capability cards and interface references should be preferred when useful.

## 3. Interfaces are shared library objects

Add a distinct sidebar section with an interface list, creation entry point, and a dedicated view. Each interface has a stable ID, name, validated content/specification, revision identity, provenance, and applicable data/action bindings.

A conversation may create, inspect, rename, revise, or remove an interface through explicit agent tools. A second conversation may continue modifying the same interface when authorized. Interfaces are not imprisoned in their originating chat.

Deleting a chat removes its chat data and links, not shared interfaces used elsewhere. Deleting an interface is a separate confirmed operation. Late events or cached results must not recreate deleted objects.

## 4. Make responses genuinely visual

Implement an application-owned, versioned UI tool/response contract and renderer: cards, lists, tables, forms, timelines, calendars, boards, charts, and other useful compositions. Model output chooses and composes supported components; it is not merely a canned layout selected by a regular expression.

Genuine custom authored UI remains part of the product direction. It must run in a separately verified contained runtime, never directly in the privileged host. The trusted-component route can provide useful real generation while custom-code containment is developed; neither route should falsely claim the other's readiness.

All data bindings and actions are validated by the host. A generated button cannot grant permissions or execute an arbitrary native command. Shared-interface updates use expected revisions/conflict handling; invalid output preserves the last working version. Sensitive actions retain appropriate operator approval even though routine authorized work should be low-friction.

## 5. Durable schedules follow the desktop lifecycle

Preserve useful Hermes engine, session/interface contracts, and durable scheduling work already in progress; adapt their ownership and transport rather than discard the existing unstaged implementation. Schedule definitions, bounded attempts, UTC cadence, next due time, grant/egress generation, run records, and interface revisions belong to app-owned durable storage, not webview timers or a required external service.

Schedules start disabled. A separate host-owned enable action discloses scope, model destination, UTC cadence, finite attempt/deadline limits, retention, and lifecycle, and checks current grants. Creating a draft, choosing a provider, launching the app, or a model request cannot enable recurrence. Previously enabled schedules may resume only within their unchanged, unexpired authorization and remaining limits; restart cannot grant or renew permission.

Background execution means **while the app and its owned runtime remain alive and the computer is awake**. Window closure must say whether the app remains running or quits. App quit stops its sidecar; sleep, shutdown, and powered-off devices cannot execute schedules. On relaunch/wake, reconcile interrupted runs, record missed slots and advance without catch-up bursts or duplicate publication. No daemon, launch-at-login, wake permission, OS automation permission, or account approval is implied. An unavailable runtime or credential host is blocked/skipped, not success.

## 6. Required acceptance evidence

- Cold-launch the actual isolated native app without a separately configured Hermes server: it starts and supervises its own pinned Hermes runtime. Capture startup, failure/recovery, quit cleanup, and an actual prompt producing real reasoning/text/tool events through that runtime.
- No Direct chat choice, runtime URL/token/enabled/Tailscale setup, or external-server prerequisite appears in onboarding, settings, chat, or recovery. Only normal model/provider configuration remains.
- Changing the model changes the intended Hermes session without losing history or altering unrelated sessions.
- The agent explains Forma using its product/capability context and does not claim unavailable account access.
- A novel user request produces a validated visual composition through the actual model/tool path, not prerecorded playback.
- Chat A creates an interface; chat B edits it; reopening restores it; deleting chat A leaves the shared interface intact.
- Concurrent edits cannot silently overwrite a newer interface revision; invalid generation leaves the previous revision usable.
- Interface deletion cancels applicable pending mutations and remains effective after restart/reconnect.
- Action attempts without grants fail; untrusted page/tool/model content cannot authorize itself.
- Schedule evidence covers app-alive background execution, actual window-close/quit semantics, sidecar crash/restart, wake/relaunch reconciliation, finite bounds, missed-slot handling, grant revocation and stale-result fencing. Persisted enablement never substitutes for current authorization.
- Browser/native runtime receipts identify their actual platform and source revision. Mock tests are not represented as live Hermes, connector, or custom-code isolation evidence. External-server/deployment QA is engineering evidence only, **not the customer product** and not acceptance of desktop-owned Hermes.

## 7. Delivery boundaries

Keep work incremental and testable, with independent product judgment, code review, and runtime QA. Unrelated deployed servers and backend fixtures stay on miniforum; that infrastructure rule does not turn the app-owned native sidecar into an externally configured customer backend. Preserve other workers' unstaged source changes.

Missing OAuth registration, operating-system permissions, devices, or external service authorization block the affected capability—not unrelated ready work. Read-only Gmail/Calendar still require registered clients, supported scopes/compliance, explicit account connection and scoped operation grants, and consent for the selected model endpoint/data categories before network access. Local Hermes does not waive egress consent or authorize live account schedules.

Higgsfield is explicitly prohibited until the owner authorizes it again. Do not generate, upload, inspect account/balance/model/cost state, read its credentials, or retry access. No media requirement overrides this hold. Use existing local visual primitives without pretending that unavailable media was generated.

Native production profiles are not test fixtures. Use a distinct test app identity, database path, and process verification before automated interactions. No personal credentials, private environment files, local databases, or company operating rules belong in the public repository.
