# Personal agent workspace

An operator turns conversations about explicitly connected accounts into durable dashboards. Devices perform work within grants that the operator controls.

## Language

**Operator**:
The person using the workspace and authorizing access to their accounts and devices.
_Avoid_: Customer, tenant, admin

**Workspace**:
One persistent chat, including its messages, remembered context, and references to interfaces it creates or uses. An interface is not owned exclusively by the chat that created it.
_Avoid_: Organization, project, separate conversation container

**Library**:
The operator's collection of workspaces, shared interfaces, and account connections.
_Avoid_: Workspace, tenant, organization

**Connected account**:
An external service identity, such as one Gmail mailbox, that the operator has deliberately made available to the workspace.
_Avoid_: User, provider account, credential

**Connector**:
An integration that offers a defined set of operations for an external service.
_Avoid_: Account, plugin, toolset

**Connection**:
An authorized association between the operator's library, a connected account, a connector, and the device holding its access credentials; a workspace uses only the connections granted to it.
_Avoid_: Login, connector, session

**Model provider**:
The operator-selected destination that processes model requests, which may be local or external.
_Avoid_: Connected account, agent, subscription

**Conversation**:
The exchange of messages within a workspace, not a separately managed container.
_Avoid_: Run, session, separate chat object

**Interface**:
A named, persistent visual surface in the library that presents information and permitted actions. Any authorized workspace can create, inspect, or update it.
_Avoid_: Chat message, conversation-owned dashboard, generated application

**Dashboard**:
An interface primarily used to monitor or explore information.
_Avoid_: All interfaces, chat, workspace

**Interface revision**:
An identifiable version of an interface's presentation, information bindings, and action definitions.
_Avoid_: Data snapshot, chat turn, run

**Data snapshot**:
The last successfully retrieved information underlying an interface, together with its source and freshness.
_Avoid_: Interface revision, live data

**Run**:
One bounded attempt to fulfill an operator request within a workspace, identifying its executor device and any connections or interface revisions involved.
_Avoid_: Conversation, session, job

**Device**:
A separately identified machine that can display a workspace or execute granted work.
_Avoid_: Account, backend, agent

**Desktop host**:
A computer that provides its granted accounts, information, and tools to the operator's connected clients.
_Avoid_: Cloud account, remote browser, mobile device

**Mobile client**:
The phone application through which the operator uses workspaces and authorized information or actions supplied by a paired desktop host.
_Avoid_: Credential copy, independent account mirror, remote desktop stream

**Capability grant**:
An operator's explicit, revocable authorization for defined operations within an account, device, and data scope.
_Avoid_: Trust, login, model instruction

**Approval**:
An operator's authorization of a specific proposed consequential action, distinct from permission to access a connection.
_Avoid_: Capability grant, confirmation message, agent decision

**Monitor**:
A saved request to refresh a dashboard under an explicitly authorized schedule and capability grant.
_Avoid_: Live dashboard, background agent, conversation
