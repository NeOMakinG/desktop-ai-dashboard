# Hermes-backed workspaces and shared Interfaces

Status: priority implementation brief, not a shipped-capability claim.

## Product outcome

Forma should feel like an intelligent personal workspace, not a generic text client. The main chat talks to a Hermes agent with the correct product context and actual capabilities. A separate **Interfaces** section in the sidebar holds persistent, named visual surfaces. Any authorized conversation can create and manipulate those interfaces.

## 1. Hermes is the application runtime

Replace the desktop's parallel direct-provider chat path with an adapter to Hermes workspace sessions. Keep the development company's Hermes separate from each user's application runtime, credentials, memory, and tool permissions.

The adapter must carry user messages, accepted model selection, text/tool events, approvals, cancellation, completion, and reconnect state. Use a verified structured Hermes host/session protocol rather than treating tool-capable operation as a plain text-completion endpoint.

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

## 5. Required acceptance evidence

- A real Hermes session receives a prompt and returns a real response/tool event that Forma displays.
- Changing the model changes the intended Hermes session without losing history or altering unrelated sessions.
- The agent explains Forma using its product/capability context and does not claim unavailable account access.
- A novel user request produces a validated visual composition through the actual model/tool path, not prerecorded playback.
- Chat A creates an interface; chat B edits it; reopening restores it; deleting chat A leaves the shared interface intact.
- Concurrent edits cannot silently overwrite a newer interface revision; invalid generation leaves the previous revision usable.
- Interface deletion cancels applicable pending mutations and remains effective after restart/reconnect.
- Action attempts without grants fail; untrusted page/tool/model content cannot authorize itself.
- Browser/native runtime receipts identify their actual platform and source revision. Mock tests are not represented as live Hermes, connector, or custom-code isolation evidence.

## 6. Delivery boundaries

Keep work incremental and testable, with independent product judgment, code review, and runtime QA. Missing OAuth registration, operating-system permissions, devices, or external service authorization block the affected capability—not unrelated ready work.

Higgsfield is explicitly prohibited until the owner authorizes it again. Do not generate, upload, inspect account/balance/model/cost state, read its credentials, or retry access. No media requirement overrides this hold. Use existing local visual primitives without pretending that unavailable media was generated.

Native production profiles are not test fixtures. Use a distinct test app identity, database path, and process verification before automated interactions. No personal credentials, private environment files, local databases, or company operating rules belong in the public repository.
