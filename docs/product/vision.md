# Forma product vision

Updated 2026-09-09 from the founder's latest direction. This document supersedes conflicting early prototype decisions; it is a mission and roadmap, not a claim that every capability exists.

## One simple personal workspace

Forma should help a person manage their digital life with minimal setup or technical knowledge. It should bring useful information and permitted actions from their accounts, applications, browsers, files, and devices into one calm interface.

A **workspace is one persistent chat**. It contains its messages, remembered context, and references to the interfaces it creates or uses. Workspaces appear in a history sidebar, save automatically, and can be deleted. There is no separate conversation container and no manual “Save to memory” action. Deleting a workspace must prevent late requests or autosaves from restoring it. Shared interfaces have their own lifecycle and are not deleted merely because a referring chat is removed.

The main surface is a large, uncluttered chat. Empty chats show a few useful examples with attractive imagery and icons; selecting an example prepares a prompt, never silently sends it. Working views show useful information, not repeated sample badges, implementation disclaimers, or ornamental text. Connection and error details appear where a person can act on them. Model selection is available directly from chat. Automatic initial selection prefers an available Opus model, while explicit operator choices remain remembered rather than being silently replaced.

The visual direction is cozy charcoal and dark grey, with warm restrained accents, thoughtful motion, and imagery/video produced through Higgsfield when authorized access is available. The desktop client remains native through Tauri rather than Electron.

## Setup should do the work

Onboarding should take at most a few short steps and ask ordinary questions such as which accounts to connect. Advanced model-provider configuration is progressively disclosed rather than imposed on everyone. The open-source product supports bring-your-own providers and lawful authentication; an optional hosted subscription can simplify this later.

Supported account authentication, permissions, credential renewal, indexing, and routine synchronization should be handled under the hood. The operator still completes required login, multi-factor authentication, OAuth consent, or OS permissions. A browser session is not automatically an API credential, and the app must not fabricate successful connections or hide unsupported integrations.

A real empty workspace is preferable to fake personal data. An optional evaluation mode can be disclosed once on entry and inspected in Settings, without placing disclaimers on every row.

## Browsers and account access

The preferred direction is a Forma-owned browser profile with persistent sessions, separated from the person's everyday browser. Explicit attachment to an existing browser is an additional option through a supported, narrowly granted connection.

A Chromium companion and a fully embedded browser require a packaging and security decision. A separate profile is not, by itself, a security sandbox or protection against a compromised OS. The control channel must be private and authenticated; an unauthenticated debugging port is not acceptable for personal-account access. Do not harvest cookie databases, copy existing profiles, bypass service authentication, or expose credentials to a model.

Google mail/calendar access requires supported Google authorization. Apple Calendar needs its native permission path. Safari is a browser, not a separate mail or calendar account provider. The connector catalog should grow through verified contracts rather than treating all services as interchangeable browser pages.

## Hermes is the application agent

The desktop chat should talk to a Hermes-backed workspace session, not a parallel generic model client. Hermes owns the reasoning/tool loop and receives Forma's product identity, actual capability state, workspace context, and UI contracts. Forma owns the interface, user-visible storage/projections, approvals, and generated-interface rendering. A thin adapter handles lifecycle, cancellation, events, and reconnection.

The model picker must configure the Hermes agent serving the selected workspace, preserve its history, and show the accepted model rather than changing an unrelated direct-provider route. The development team's Hermes on its build host is a different runtime; its existence does not imply that a user's desktop chat is connected to Hermes. Desktop-local tools must execute on the user's explicitly authorized host.

## Generated interfaces are central

A separate **Interfaces** section in the sidebar lists named, persistent visual surfaces. These belong to the operator's library, not exclusively to their originating conversation. Any authorized workspace can create, inspect, revise, or remove an interface through the agent. Interfaces can present data, forms, actions, lists, calendars, boards, charts, and other useful arrangements rather than being restricted to text answers or generic dashboards.

Updates need revision/provenance tracking and conflict protection so concurrent chats or devices do not silently overwrite one another. Deleting a shared interface is distinct from deleting a chat. Actions remain host-validated capabilities; an interface or model cannot grant itself authority.

Forma should generate genuinely useful custom interfaces from conversation, guided by a coherent design system and reusable primitives. The product includes a trusted component route and a future isolated custom-code route; it is not restricted to preset dashboard rearrangement.

Generated code cannot acquire account, native, filesystem, or network authority merely by being generated. Data access, actions, resource limits, revocation, and recovery belong to independently enforced host boundaries. Interface and chat state should save automatically, with recoverable revisions where appropriate.

## Multiple devices and mobile

A future mobile app is a first-class client of a paired desktop host. A person should be able to continue workspaces from their phone and use the authorized information and tools available on their computer, without separately reconnecting every account on the phone.

Credentials remain on the owning desktop by default. Pairing is explicit and revocable; each client receives scoped access rather than copied account sessions. Offline or sleeping hosts must be represented honestly, and synchronization must handle reconnects, concurrent clients, duplicate requests, and stale data. Do not imply that data remains live while its host is unreachable.

The roadmap includes desktop server/client architecture, an authenticated MCP surface, remote execution and synchronization, iOS/Android clients, and Orca interoperability for communicating with the operator's agents and machines.

## Continuous development mission

The founder wants a persistent, continuously working product/development/review/QA organization, coordinated by GLM and using available approved specialist models. Product questions go to the product-owner role. The team should progress through a dependency-ordered backlog, create as many useful verified connectors as practical, test end to end, and build a marketing site using the landing-page design discipline.

“Feature complete” must be decomposed into measurable milestones and acceptance tests. A running loop is not evidence of completed features. Work blocked by credentials, service policies, hardware, or human-owned approvals remains visibly blocked while the team advances other ready work. Human merge and sensitive-action boundaries remain in force unless explicitly changed.

The public repository contains product source and sanitized documentation. Private environments, account data, credentials, local issue databases, orchestration configuration, and operational rules stay outside it.
