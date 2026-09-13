# Host-approval contracts in the Forma runtime

Status: developer documentation. This document describes the **model-proposes / host-approves** pattern implemented by two runtime modules, referenced here by relative path only:

- `runtime/forma_runtime/cron_proposals.py` — chat-originated cron proposals
- `runtime/forma_runtime/composio_actions.py` — Composio connector action requests

Every semantic claim below is intended to trace to those two modules. Where a behavior is not implemented, this document says so explicitly rather than describing an aspiration as shipped.

## Why host approval is required

The model (a Hermes agent turn) is untrusted output from the host's perspective. It can *propose* consequential work, but it can never *authorize* it. Two categories of consequential work are covered here:

1. **Scheduling.** A cron proposal originates from chat; nothing is scheduled until the host records an explicit operator approval in the approval ledger.
2. **Connector execution.** A Composio action request originates from chat; no connector action executes until the host finds a currently valid grant and produces a receipt.

Approval in this document follows the glossary in `CONTEXT.md`: an **Approval** is the operator's authorization of a specific proposed consequential action, and is distinct from a **Capability grant**, which is the operator's explicit, revocable authorization for defined operations within an account, device, and data scope. The model holds neither.

## Cron proposal lifecycle (`runtime/forma_runtime/cron_proposals.py`)

1. The model emits a `CronProposal` as structured chat output. A proposal is a *request*; producing one has no scheduling side effect.
2. The host validates the proposal's shape (including its cron expression and any action dictionary, which the module defensively copies) and presents it to the operator.
3. The operator either approves or declines. Approval is recorded in the `ApprovalLedger` by the host, not by the model.
4. Only a ledger entry (or an equivalent host-side approval check) authorizes the host to schedule anything.
5. Declined or unapproved proposals never reach a scheduler; only approved proposals are enumerated as schedulable, while denied and merely-proposed ones are excluded.

The `ApprovalLedger` is the durable record of which proposals the operator has approved. It is host-owned state: the model can query whether an approval exists but cannot create, forge, or mutate ledger entries. The ledger enforces explicit decisions only: recording a decision for an unknown proposal, recording two decisions for the same proposal, or registering a duplicate proposal id is rejected by the module (these rejections are raised as standard `ValueError`s by the ledger and proposal constructors rather than dedicated error classes; the module is the source of truth for exact messages).

## Composio action lifecycle (`runtime/forma_runtime/composio_actions.py`)

1. The model emits an `ActionRequest` describing a connector action it wants performed.
2. The host validates the request against the module's typed validation errors. Invalid requests fail with the typed validation error classes defined in this module; they never fall through to execution.
3. The host checks for a current `ActionGrant` covering the requested action.
4. If a valid grant exists, the host executes the connector action through its transport abstraction and produces an `ActionReceipt` recording the outcome (receipt status is itself validated).
5. If no valid grant exists — absent, expired, or revoked — the request is rejected before any connector call is made.

## Data shapes

From `runtime/forma_runtime/cron_proposals.py`:

- `CronProposal` — a proposed schedule originating from chat. Validated fields include the cron expression (five fields; numeric ranges and steps only — name tokens and macros such as `@daily` are rejected) and an action dictionary that is defensively copied on construction.
- `ApprovalLedger` — the host-owned record of operator approvals for proposals. Accepts exactly one explicit approve/deny decision per known, non-duplicate proposal id.

From `runtime/forma_runtime/composio_actions.py`:

- `ActionRequest` — a proposed connector action originating from chat.
- `ActionGrant` — the host-held authorization that must be valid for an action to execute. Carries an expiry and can be revoked via the module's revoke helper.
- `ActionReceipt` — the record the host produces after executing an approved, granted action; its status value is validated.
- Typed validation errors raised by this module:
  - `UnknownConnectorError` — the requested connector is not one the module knows.
  - `ActionNotGrantedError` — no grant covers the requested action.
  - `GrantExpiredError` — a matching grant exists but its expiry has passed.
  - `GrantRevokedError` — a matching grant exists but has been revoked.

This document intentionally does not duplicate field-by-field definitions, because the modules are the source of truth and drift between this page and the code would be worse than omission.

## Grant expiry and revocation

An `ActionGrant` is valid only for a bounded window and scope:

- **Expiry.** Grants carry an expiry; a request evaluated after expiry raises `GrantExpiredError` and is rejected before execution. The boundary check is exclusive at the expiry instant: a request evaluated exactly at the expiry timestamp is treated as expired, and one evaluated before it is not.
- **Revocation.** The module provides a revoke helper that invalidates the grant so subsequent action requests raise `GrantRevokedError`. Enforcement is at request-evaluation time by the host: the model cannot bypass it, and no connector call proceeds against a revoked grant.

These semantics are the ones actually implemented in `runtime/forma_runtime/composio_actions.py`; anything not present in that module is not a behavior of this system.

## Payload redaction

Action payloads and receipts may contain operator-sensitive content. The host redacts payload content — masking sensitive keys case-insensitively, including values nested inside dictionaries — before it is logged, echoed into chat, or otherwise persisted, as implemented in `runtime/forma_runtime/composio_actions.py`. The purpose is that secrets and credential-bearing values never appear in transcripts, ledgers, or receipts. If a redaction boundary is not implemented for a given surface, that surface must be treated as unredacted and not relied upon.

## Deliberately out of model reach

The model can **never**:

- **Auto-approve.** No code path lets model output create an `ApprovalLedger` entry or mint an `ActionGrant`. Approval and grant creation are host/operator actions only.
- **Hold credentials.** Connector tokens, API keys, and account credentials live with the host and are never handed to the model.
- **Reach the network directly.** All connector and network access happens through host-mediated execution under a valid grant; the model has no ambient network access.
- **Use OS scheduling.** The model cannot register OS-level timers, launchd/cron entries, or background tasks. Scheduling happens only through the host's approval-gated cron path in `runtime/forma_runtime/cron_proposals.py`.

## Invariants

- Proposal or request creation never has a side effect beyond producing structured data for the host to validate.
- Every schedule or connector execution is downstream of a host-recorded operator approval and, for actions, a currently valid grant.
- Every executed action yields a receipt; every rejected request yields a typed error or explicit rejection, not silence.
