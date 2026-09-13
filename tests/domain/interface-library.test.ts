import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  InterfaceLibrary,
} from '../src/domain/interface-library.ts';

function fakeClock(start = 1000, step = 10): { clock: () => number; tick: () => number } {
  let now = start;
  return {
    clock: () => now,
    tick: () => (now += step),
  };
}

function makeLibrary(): { lib: InterfaceLibrary; clock: ReturnType<typeof fakeClock> } {
  const clock = fakeClock();
  return { lib: new InterfaceLibrary({ clock: clock.clock }), clock };
}

function unwrap<T>(result: { ok: boolean; value?: T; error?: unknown }): T {
  assert.equal(result.ok, true, `expected ok result, got ${JSON.stringify(result)}`);
  return (result as { ok: true; value: T }).value;
}

test('create returns stable unique id, revision 1, provenance, link, and authorization', () => {
  const { lib, clock } = makeLibrary();
  const created = unwrap(lib.create('ws-A', 'Triage', { kind: 'table', rows: 10 }));
  assert.equal(created.currentRevision, 1);
  assert.equal(created.tombstone, false);
  assert.ok(created.id.length > 0);
  const second = unwrap(lib.create('ws-A', 'Other', { kind: 'text' }));
  assert.notEqual(created.id, second.id);
  const rev1 = unwrap(lib.getRevision(created.id, 1));
  assert.equal(rev1.provenance.workspaceId, 'ws-A');
  assert.equal(rev1.provenance.parentRevision, null);
  assert.equal(rev1.provenance.timestamp, clock.clock());
  assert.deepEqual(rev1.spec, { kind: 'table', rows: 10 });
  const rec = unwrap(lib.getInterface(created.id));
  assert.deepEqual(Array.from(rec.links), ['ws-A']);
  assert.deepEqual(Array.from(rec.allowlist), ['ws-A']);
});

test('authorized workspace B can propose a revision; revision increments and provenance records B and parent', () => {
  const { lib, clock } = makeLibrary();
  const created = unwrap(lib.create('ws-A', 'Board', { v: 1 }));
  unwrap(lib.grantWorkspaceAccess(created.id, 'ws-B'));
  clock.tick();
  const rev2 = unwrap(lib.proposeRevision(created.id, 'ws-B', 1, { v: 2 }));
  assert.equal(rev2.revision, 2);
  assert.equal(rev2.provenance.workspaceId, 'ws-B');
  assert.equal(rev2.provenance.parentRevision, 1);
  assert.equal(rev2.provenance.timestamp, clock.clock());
  const rec = unwrap(lib.getInterface(created.id));
  assert.equal(rec.currentRevision, 2);
  assert.ok(rec.links.has('ws-B'));
});

test('never-authorized and revoked workspaces are rejected before any mutation', () => {
  const { lib } = makeLibrary();
  const created = unwrap(lib.create('ws-A', 'Cal', { v: 1 }));
  const before = unwrap(lib.getInterface(created.id));
  const never = lib.proposeRevision(created.id, 'ws-C', 1, { v: 2 });
  assert.equal(never.ok, false);
  assert.equal((never as { error: { kind: string } }).error.kind, 'unauthorized');
  unwrap(lib.grantWorkspaceAccess(created.id, 'ws-B'));
  unwrap(lib.revokeWorkspaceAccess(created.id, 'ws-B'));
  const revoked = lib.proposeRevision(created.id, 'ws-B', 1, { v: 2 });
  assert.equal(revoked.ok, false);
  assert.equal((revoked as { error: { kind: string } }).error.kind, 'unauthorized');
  const after = unwrap(lib.getInterface(created.id));
  assert.equal(after.currentRevision, before.currentRevision);
  assert.equal(after.revisions.size, before.revisions.size);
  assert.deepEqual(after.revisions.get(1)?.spec, before.revisions.get(1)?.spec);
});

test('expectedRevision mismatch is a conflict; newer revision stays current and later correct writer still commits', () => {
  const { lib } = makeLibrary();
  const created = unwrap(lib.create('ws-A', 'List', { v: 1 }));
  const conflict = lib.proposeRevision(created.id, 'ws-A', 1, { v: 'stale' });
  assert.equal(conflict.ok, false);
  assert.equal((conflict as { error: { kind: string } }).error.kind, 'revision-conflict');
  const rec = unwrap(lib.getInterface(created.id));
  assert.equal(rec.currentRevision, 1);
  assert.deepEqual(rec.revisions.get(1)?.spec, { v: 1 });
  const good = unwrap(lib.proposeRevision(created.id, 'ws-A', 1, { v: 2 }));
  assert.equal(good.revision, 2);
});

test('invalid specifications are rejected on create and propose; previous revision stays retrievable', () => {
  const { lib } = makeLibrary();
  assert.equal(lib.create('ws-A', 'Bad', null).ok, false);
  assert.equal(lib.create('ws-A', 'Bad', [1, 2]).ok, false);
  assert.equal(lib.create('ws-A', 'Bad', new Date()).ok, false);
  const created = unwrap(lib.create('ws-A', 'Good', { v: 1 }));
  const rejected = lib.proposeRevision(created.id, 'ws-A', 1, 'not an object');
  assert.equal(rejected.ok, false);
  assert.equal((rejected as { error: { kind: string } }).error.kind, 'invalid-spec');
  const rec = unwrap(lib.getInterface(created.id));
  assert.equal(rec.currentRevision, 1);
  const rev1 = unwrap(lib.getRevision(created.id, 1));
  assert.equal(rev1.provenance.workspaceId, 'ws-A');
  assert.deepEqual(rev1.spec, { v: 1 });
});

test('removeWorkspace removes only that workspace links and authorizations; everything else survives', () => {
  const { lib } = makeLibrary();
  const a = unwrap(lib.create('ws-A', 'One', { v: 1 }));
  const b = unwrap(lib.create('ws-A', 'Two', { v: 1 }));
  unwrap(lib.grantWorkspaceAccess(a.id, 'ws-B'));
  unwrap(lib.grantWorkspaceAccess(b.id, 'ws-B'));
  unwrap(lib.proposeRevision(a.id, 'ws-B', 1, { v: 2 }));
  unwrap(lib.removeWorkspace('ws-B'));
  const recA = unwrap(lib.getInterface(a.id));
  const recB = unwrap(lib.getInterface(b.id));
  assert.equal(recA.tombstone, false);
  assert.equal(recB.tombstone, false);
  assert.deepEqual(Array.from(recA.links), ['ws-A']);
  assert.deepEqual(Array.from(recB.links), ['ws-A']);
  assert.deepEqual(Array.from(recA.allowlist), ['ws-A']);
  assert.deepEqual(Array.from(recB.allowlist), ['ws-A']);
  assert.equal(recA.currentRevision, 2);
  assert.equal(recA.revisions.size, 2);
  assert.equal(recA.revisions.get(2)?.provenance.workspaceId, 'ws-B');
  const late = lib.proposeRevision(a.id, 'ws-B', 2, { v: 3 });
  assert.equal(late.ok, false);
  assert.equal((late as { error: { kind: string } }).error.kind, 'unauthorized');
  assert.equal(lib.listInterfaces().length, 2);
});

test('removeInterface installs a permanent tombstone that rejects any later proposal', () => {
  const { lib } = makeLibrary();
  const created = unwrap(lib.create('ws-A', 'Temp', { v: 1 }));
  unwrap(lib.proposeRevision(created.id, 'ws-A', 1, { v: 2 }));
  const staleDelete = lib.removeInterface(created.id, 1);
  assert.equal(staleDelete.ok, false);
  unwrap(lib.removeInterface(created.id, 2));
  const rec = unwrap(lib.getInterface(created.id));
  assert.equal(rec.tombstone, true);
  assert.equal(rec.revisions.size, 2);
  for (const expected of [2, 1]) {
    const attempt = lib.proposeRevision(created.id, 'ws-A', expected, { v: 'resurrect' });
    assert.equal(attempt.ok, false);
    assert.equal((attempt as { error: { kind: string } }).error.kind, 'tombstoned');
  }
  const after = unwrap(lib.getInterface(created.id));
  assert.equal(after.currentRevision, 2);
  assert.deepEqual(after.revisions.get(2)?.spec, { v: 2 });
});

test('stored specs are defensively copied so caller mutation cannot alter history', () => {
  const { lib } = makeLibrary();
  const spec: Record<string, unknown> = { v: 1, nested: { x: 1 } };
  const created = unwrap(lib.create('ws-A', 'Frozen', spec));
  spec.v = 'mutated';
  (spec.nested as Record<string, unknown>).x = 'mutated';
  const rev1 = unwrap(lib.getRevision(created.id, 1));
  assert.equal(rev1.spec.v, 1);
  assert.equal((rev1.spec.nested as Record<string, unknown>).x, 1);
});
