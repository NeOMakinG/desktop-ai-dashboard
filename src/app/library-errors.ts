/** UI-owned validation errors have safe text; raw transport/provider errors never reach the canvas. */
export class LibraryValidationError extends Error {}
export function libraryError(failure: unknown): string {
  if (failure instanceof LibraryValidationError) return failure.message;
  const code = failure && typeof failure === 'object' && 'code' in failure ? String(failure.code).toLowerCase() : '';
  if (code.includes('conflict')) return 'A newer revision or schedule version exists. Reload, compare, and explicitly retry.';
  if (code.includes('auth') || code.includes('credential') || code.includes('keychain')) return 'Runtime authentication needs attention. Check its dedicated settings.';
  if (code.includes('schema') || code.includes('protocol') || code.includes('invalid')) return 'The runtime rejected an unsupported or invalid request. No success was acknowledged.';
  if (code.includes('limit') || code.includes('too_large')) return 'The runtime response exceeds this client’s bounded page or payload limit. Previous records remain visible.';
  if (code.includes('deleted') || code.includes('tombstone') || code.includes('not_found')) return 'This record is no longer available. Reload its library.';
  if (code.includes('grant') || code.includes('scope')) return 'Current scope or egress authorization is missing. Review grants; model content cannot authorize access.';
  if (code.includes('browser')) return 'The native runtime is unavailable in browser evaluation.';
  if (code.includes('not_ready') || code.includes('disabled')) return 'The Hermes runtime is unavailable. Check its dedicated settings; no Direct fallback was used.';
  return 'The runtime did not acknowledge that action. Its outcome may be unknown; reload authoritative state before trying again.';
}
