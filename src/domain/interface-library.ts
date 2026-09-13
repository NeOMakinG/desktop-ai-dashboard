// Pure, in-memory, side-effect-free Interface library domain module.
// No I/O, no persistence, no rendering, no connector or live-agent behavior.
// Erasable-types-only TypeScript (no enum/namespace/parameter properties/decorators),
// so Node's built-in type stripping can execute this file directly.

export type InterfaceSpec = Record<string, unknown>;

export interface RevisionProvenance {
  workspaceId: string;
  parentRevision: number | null;
  timestamp: number;
}

export interface InterfaceRevision {
  revision: number;
  spec: InterfaceSpec;
  provenance: RevisionProvenance;
}

export interface InterfaceRecord {
  id: string;
  name: string;
  currentRevision: number;
  revisions: Map<number, InterfaceRevision>;
  links: Set<string>;
  allowlist: Set<string>;
  tombstone: boolean;
}

export interface LibraryConfig {
  clock?: () => number;
  idFactory?: () => string;
}

export type LibraryError =
  | { kind: 'invalid-name' }
  | { kind: 'invalid-spec' }
  | { kind: 'invalid-workspace' }
  | { kind: 'unknown-interface'; id: string }
  | { kind: 'tombstoned'; id: string }
  | { kind: 'unauthorized'; id: string; workspaceId: string }
  | { kind: 'revision-conflict'; id: string; expectedRevision: number; currentRevision: number }
  | { kind: 'invalid-expected-revision'; expectedRevision: unknown };

export interface Rejection {
  ok: false;
  error: LibraryError;
}

export interface Ok<T> {
  ok: true;
  value: T;
}

export type Result<T> = Ok<T> | Rejection;

export class InterfaceLibrary {
  private clock: () => number;
  private idFactory: () => string;
  private interfaces: Map<string, InterfaceRecord> = new Map();
  private liveIds: Set<string> = new Set(); // ids ever minted; never reassigned

  constructor(config?: LibraryConfig) {
    this.clock = config?.clock ?? (() => Date.now());
    this.idFactory = config?.idFactory ?? (() => defaultMintId());
  }

  // Guard order for proposeRevision: unknown id, tombstone, authorization,
  // structural spec validity, then expectedRevision equality. No partial mutation.

  create(workspaceId: string, name: string, spec: unknown): Result<InterfaceRecord> {
    if (!isValidWorkspaceId(workspaceId)) return fail({ kind: 'invalid-workspace' });
    if (!isValidName(name)) return fail({ kind: 'invalid-name' });
    if (!isStructurallyValidSpec(spec)) return fail({ kind: 'invalid-spec' });
    const id = this.idFactory();
    if (this.liveIds.has(id)) return fail({ kind: 'unknown-interface', id }); // id factory must mint unique ids
    this.liveIds.add(id);
    const record: InterfaceRecord = {
      id,
      name,
      currentRevision: 1,
      revisions: new Map(),
      links: new Set(),
      allowlist: new Set(),
      tombstone: false,
    };
    record.revisions.set(1, {
      revision: 1,
      spec: freezeSpec(spec as InterfaceSpec),
      provenance: {
        workspaceId,
        parentRevision: null,
        timestamp: this.clock(),
      },
    });
    record.links.add(workspaceId);
    record.allowlist.add(workspaceId);
    this.interfaces.set(id, record);
    return ok(record);
  }

  proposeRevision(
    id: string,
    workspaceId: string,
    expectedRevision: unknown,
    spec: unknown,
  ): Result<InterfaceRevision> {
    if (!isValidWorkspaceId(workspaceId)) return fail({ kind: 'invalid-workspace' });
    if (typeof id !== 'string' || id.length === 0) return fail({ kind: 'unknown-interface', id: String(id) });
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    if (record.tombstone) return fail({ kind: 'tombstoned', id });
    if (!record.allowlist.has(workspaceId)) {
      return fail({ kind: 'unauthorized', id, workspaceId });
    }
    if (!isStructurallyValidSpec(spec)) return fail({ kind: 'invalid-spec' });
    if (!isValidExpectedRevision(expectedRevision)) {
      return fail({ kind: 'invalid-expected-revision', expectedRevision });
    }
    if (expectedRevision !== record.currentRevision) {
      return fail({
        kind: 'revision-conflict',
        id,
        expectedRevision: expectedRevision as number,
        currentRevision: record.currentRevision,
      });
    }
    const next = record.currentRevision + 1;
    const revisionRecord: InterfaceRevision = {
      revision: next,
      spec: freezeSpec(spec as InterfaceSpec),
      provenance: {
        workspaceId,
        parentRevision: record.currentRevision,
        timestamp: this.clock(),
      },
    };
    record.revisions.set(next, revisionRecord);
    record.currentRevision = next;
    record.links.add(workspaceId);
    return ok(revisionRecord);
  }

  removeInterface(id: string, expectedRevision: unknown): Result<true> {
    if (typeof id !== 'string' || id.length === 0) return fail({ kind: 'unknown-interface', id: String(id) });
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    if (!isValidExpectedRevision(expectedRevision)) {
      return fail({ kind: 'invalid-expected-revision', expectedRevision });
    }
    if (expectedRevision !== record.currentRevision) {
      return fail({
        kind: 'revision-conflict',
        id,
        expectedRevision: expectedRevision as number,
        currentRevision: record.currentRevision,
      });
    }
    record.tombstone = true; // permanent; history is preserved
    return ok(true);
  }

  // Removes only that workspace's links and per-interface authorizations everywhere.
  // Never deletes interfaces, revisions, or provenance.
  removeWorkspace(workspaceId: string): Result<true> {
    if (!isValidWorkspaceId(workspaceId)) return fail({ kind: 'invalid-workspace' });
    for (const record of this.interfaces.values()) {
      record.links.delete(workspaceId);
      record.allowlist.delete(workspaceId);
    }
    return ok(true);
  }

  grantWorkspaceAccess(id: string, workspaceId: string): Result<true> {
    if (!isValidWorkspaceId(workspaceId)) return fail({ kind: 'invalid-workspace' });
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    if (record.tombstone) return fail({ kind: 'tombstoned', id });
    record.allowlist.add(workspaceId);
    return ok(true);
  }

  revokeWorkspaceAccess(id: string, workspaceId: string): Result<true> {
    if (!isValidWorkspaceId(workspaceId)) return fail({ kind: 'invalid-workspace' });
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    record.allowlist.delete(workspaceId);
    return ok(true);
  }

  getInterface(id: string): Result<InterfaceRecord> {
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    return ok(record);
  }

  getRevision(id: string, revision: number): Result<InterfaceRevision> {
    const record = this.interfaces.get(id);
    if (record === undefined) return fail({ kind: 'unknown-interface', id });
    const rev = record.revisions.get(revision);
    if (rev === undefined) {
      return fail({ kind: 'revision-conflict', id, expectedRevision: revision, currentRevision: record.currentRevision });
    }
    return ok(rev);
  }

  listInterfaces(): InterfaceRecord[] {
    return Array.from(this.interfaces.values());
  }
}

function isValidName(name: unknown): name is string {
  return typeof name === 'string' && name.trim().length > 0;
}

function isValidWorkspaceId(id: unknown): id is string {
  return typeof id === 'string' && id.trim().length > 0;
}

function isValidExpectedRevision(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0;
}

// Structural validation only: must be a non-array plain object. Deeper schema
// validation is deliberately left open on parent HI03.
function isStructurallyValidSpec(spec: unknown): spec is InterfaceSpec {
  if (typeof spec !== 'object' || spec === null) return false;
  if (Array.isArray(spec)) return false;
  const proto = Object.getPrototypeOf(spec);
  return proto === Object.prototype || proto === null;
}

function freezeSpec(spec: InterfaceSpec): InterfaceSpec {
  const copy: InterfaceSpec = {};
  for (const key of Object.keys(spec)) {
    copy[key] = deepFreezeValue(spec[key]);
  }
  return Object.freeze(copy) as InterfaceSpec;
}

function deepFreezeValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return Object.freeze(value.map(deepFreezeValue));
  }
  if (typeof value === 'object' && value !== null) {
    const proto = Object.getPrototypeOf(value);
    if (proto === Object.prototype || proto === null) {
      const copy: Record<string, unknown> = {};
      for (const key of Object.keys(value as Record<string, unknown>)) {
        copy[key] = deepFreezeValue((value as Record<string, unknown>)[key]);
      }
      return Object.freeze(copy);
    }
  }
  return value;
}

let mintCounter = 0;
function defaultMintId(): string {
  mintCounter += 1;
  return `ifc_${mintCounter.toString(36).padStart(8, '0')}_${Math.random().toString(36).slice(2, 10)}`;
}

function ok<T>(value: T): Ok<T> {
  return { ok: true, value };
}

function fail(error: LibraryError): Rejection {
  return { ok: false, error };
}
