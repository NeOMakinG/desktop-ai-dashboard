---
status: accepted
---

# Local ownership does not imply zero egress

Saved dashboards and their cached data belong to the operator's workspace; a hosted product account is not a prerequisite. Device credentials remain with their owning device by default. Model processing and remote execution are nevertheless explicit disclosures to separately identified destinations, so the product must obtain consent for the destination and categories of data before transmitting them.

This preserves an open-source, bring-your-own-provider product without presenting remote model processing as entirely local. We reject a mandatory hosted custody model for the initial release and reject silently copying credentials between paired devices. SaaS billing, cloud custody, synchronized workspaces, and broader tenancy remain separate product decisions rather than hidden consequences of choosing a remote Hermes backend.

Consent is necessary but does not override provider terms, Gmail data-use restrictions, connector scopes, or device authorization. An endpoint's OpenAI-compatible shape is not evidence that its authentication, tool behavior, cost, or data handling meets this product's contract.
