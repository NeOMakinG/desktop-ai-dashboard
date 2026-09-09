# Integration evidence and compatibility gates

Checked 2026-09-09. This is source/documentation research, not runtime validation. No account was connected, no service started, and no model request issued during this research.

## Hermes: verified upstream revision

Research is pinned to [fd6434b3b36592367ac5faa180b905d64e29214c](https://github.com/NousResearch/hermes-agent/tree/fd6434b3b36592367ac5faa180b905d64e29214c), not an assumed stable contract at a moving branch. The package identifies itself as version 0.21.1, Python >=3.11 and <3.14. The [MIT license](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/LICENSE) requires preserving its notice. That upstream license does not select this project's license.

### Available seams

- **HTTP:** Real REST/SSE APIs cover chat completions, responses, session CRUD/history/fork/chat, and Runs status/events/approval/steer/stop. See [API route registration](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server.py#L1494) and [Runs routes](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server_runs.py#L99).
- **ACP:** `hermes acp`, `hermes-acp`, and `python -m acp_adapter` expose JSON-RPC over stdio. The adapter supports session creation, prompting, cancellation, permissions, and session lifecycle features. The ACP dependency is pinned to 0.9.0. This is not by itself a remotely authenticated desktop control plane. See [ACP server](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/acp_adapter/server.py#L497).
- **TUI host protocol:** JSON-RPC over stdio or WebSocket supports richer desktop-host behavior, including approvals, clarification, subagents, and multiple subscribers within one gateway. Upstream recommends this for custom desktop hosts seeking full feature parity. See [programmatic integration guidance](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/website/docs/developer-guide/programmatic-integration.md#which-one-should-i-use).
- **Python embedding:** `run_agent.AIAgent` accepts explicit provider settings, toolsets, session identity, and streaming/tool/status callbacks. This is an in-process Python API, not a JavaScript SDK. See [constructor](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/run_agent.py#L233).

### Provider configuration

Custom OpenAI-compatible providers are explicitly supported through `model.provider: custom`, model selection, `model.base_url`, and `model.api_key`. Named providers support endpoint-specific credentials and headers. See [provider documentation](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/website/docs/integrations/providers.md#L662).

Do not assume `OPENAI_BASE_URL` configures a bare custom provider: the [resolver](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/hermes_cli/runtime_provider_backends.py#L116) deliberately does not use it for that case. Hermes gateway credentials and upstream model-provider credentials are separate secrets and must remain separate in the product.

### Security and lifecycle limitations

1. **Server-side authority:** The HTTP server requires a usable bearer key and reports `tool_execution: "server"` with `split_runtime: false`. A remote Hermes run does not acquire the desktop client's tools. See [capabilities](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server.py#L2231) and [startup validation](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server.py#L3778).
2. **Approval identity matters:** Runs expose `approval.request`; answers accept a choice and optional exact request ID. The product must require its own exact pending-request, run, account/device scope, and expiry match. Prefer once/deny decisions over broad persistent trust. See [approval handler](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server_runs.py#L716).
3. **MCP hints are not authorization:** Default MCP trust is `full`; untrusted gating relies partly on server-supplied `readOnlyHint`. A malicious server can mislabel a mutating tool. Independent allowlists and scoped enforcement are required. ACP also accepts MCP subprocess commands and remote URLs, which must not be forwarded directly from untrusted model input. See [MCP handler](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/tools/mcp_tool_handlers.py#L38) and [MCP configuration](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/website/docs/reference/mcp-config-reference.md#L69).
4. **Runs SSE is not a sync bus:** The handler consumes one run queue with `q.get()` and drops transport on disconnect. There is no durable replay or broadcast fanout in this handler; multiple consumers can compete. A client must not promise resumable multi-device streams from that endpoint alone. See [event handler](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server_runs.py#L661).
5. **Cancellation is not rollback:** Stop returns `stopping`, requests interruption, and attempts run-scoped cleanup. Wait for terminal status and preserve uncertainty about already-executed effects. Idempotent admission does not make SSE replayable or connector writes exactly-once. See [stop handler](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/gateway/platforms/api_server_runs.py#L801).

### Native Windows

Current upstream documents native Windows 10/11 support for CLI, TUI, gateway, MCP, and dashboard. The explicit exception is the embedded POSIX PTY `/chat` terminal pane. Shell tools use Git Bash/PortableGit; ACP has a Windows UTF-8 bootstrap. “Hermes requires WSL” would be an outdated blanket statement. See [Windows documentation](https://github.com/NousResearch/hermes-agent/blob/fd6434b3b36592367ac5faa180b905d64e29214c/website/docs/user-guide/windows-native.md#L85).

This is not a Windows packaging, cancellation, performance, or runtime PASS for this product.

## Proposed architecture seam, not an implemented adapter

For the bounded initial workflow, evaluate a product-owned control-plane adapter on the designated backend host over pinned Hermes HTTP Runs/session APIs. It should own workspace/device/session mappings, one upstream event consumer, downstream authorization, recovery semantics, and approval routing. Prefer the richer TUI host protocol if full clarification/subagent/slash-command parity is required; its authenticated hosting and restart semantics need separate proof.

An adapter can replay events it has persisted, not events lost upstream before it received them. It must reconcile authoritative run/session state after reconnect, mark unrecoverable gaps visibly, and never silently label an incomplete stream complete. No transport decision removes the need for a per-device execution agent and explicit grants.

## Orca and multiple devices

A bounded local search found the founder's Orca-managed project/worktree directories, not an identifiable Orca product source tree or documented device/session/agent SDK. This means the integration contract was not located; it does not mean Orca lacks one. No API names, tokens, pairing protocol, or reuse license are assumed.

Keep these separate:

- Access to a remotely running Hermes agent.
- A device executing its own local tools within grants.
- Multiple clients observing one run.
- Paired devices synchronizing workspace changes.
- Orca interoperability and agent communication.

The first does not prove any of the others. Orca remains a contract-discovery spike before implementation. Credentials stay on the owning device by default.

## Gmail and provider-policy gates

Google classifies both `gmail.metadata` and `gmail.readonly` as Restricted scopes. Metadata covers headers/labels, not message bodies. A date limit, inbox-only interface, or count cap is an application bound, not a narrower OAuth grant. See [Gmail scopes](https://developers.google.com/workspace/gmail/api/auth/scopes).

The scope documentation states that storing restricted-scope data on servers, or transmitting it, requires a security assessment. The applicability of verification, Limited Use requirements, exceptions, and assessment obligations must be established for the precise desktop/remote-model architecture before release. Bring-your-own OAuth credentials do not establish an exemption. In particular, sending Gmail-derived data to a model provider needs a verified policy basis as well as operator consent.

Anthropic's [credential-use policy](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use) prohibits offering Claude.ai login in a third-party application or collecting/intermediating its subscription session tokens. Direct API keys and supported cloud providers are distinct. An end user logging into an unmodified Claude Code binary does not authorize reusing those credentials in Hermes. Other providers' OAuth eligibility is unverified until researched individually.

## Compatibility spike receipts required

1. On the designated backend host, use the pinned Hermes revision, an isolated profile, synthetic inputs, and only the approved local model gateway.
2. Exercise capabilities, session creation, a streamed run, and history. Save sanitized requests, responses, and status codes.
3. Prove tool denial, exact-ID approvals, model/tool cancellation, duplicate admission, and gateway restart behavior.
4. Disconnect/reconnect and attach two clients. Demonstrate adapter recovery or explicitly surfaced gaps without competing upstream consumers.
5. On an actual Windows host, test native UTF-8, paths with spaces/non-ASCII characters, cancellation/process cleanup, and cold launch.
6. Find an authorized Orca contract and prove one read-only device/session listing before promising interoperability.

No Hermes installation was found in the Mac's PATH or checked conventional locations. Custom locations and the backend host were not exhaustively inspected. The available code graph pointed to an unrelated project and was discarded as evidence.
