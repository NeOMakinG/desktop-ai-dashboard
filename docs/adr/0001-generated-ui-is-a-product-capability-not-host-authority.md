---
status: accepted
---

# Generated UI is a product capability, not host authority

The founder explicitly wants genuine LLM-authored interfaces guided by a design system, not only predefined dashboard arrangements. We therefore support two product paths: a validated, trusted-component specification and custom generated UI in an isolated runtime. The earlier catalog-only recommendation is superseded. Neither path gives model output account permissions or native authority.

Custom UI can use generated presentation code and the supplied design primitives, but it must never run in the privileged desktop host. It receives only an approved, bounded data projection through a host-validated bridge. Credentials, connector tokens, native IPC, ambient network access, arbitrary dependencies, and device enrollment are unavailable to it. Permissions are enforced outside both the generated code and the model.

## Trade-off

A catalog-only product would be easier to secure and test but would remove a capability the founder considers central. Running generated React directly in the application would maximize flexibility while turning untrusted output into privileged application code. We accept the complexity of a separate custom runtime rather than either restriction or privilege collapse.

## Gate before live data

The exact isolation mechanism remains a spike, not a solved implementation. An iframe sandbox or content-security policy alone is not sufficient evidence: the spike must test origin/bridge spoofing, all egress channels, native-command access, dependency escape, CPU/memory exhaustion, cancellation, and host recovery on macOS and Windows. Data already disclosed to a renderer cannot be retroactively undisclosed; revocation must prevent further delivery and terminate its runtime.

Until those checks pass, custom-UI experiments use labeled synthetic fixtures only. The product flow includes preview, explicit save, immutable revision identity, repair after errors, and rollback to the last working revision. Failed generation, validation, compilation, or execution must not replace that revision.
