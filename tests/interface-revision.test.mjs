import test from "node:test";
import assert from "node:assert/strict";

import {
  createInterfaceRecord,
  deleteInterfaceRecord,
  updateInterfaceRecord,
} from "../src/domain/interface-revision.mjs";

test("creation preserves caller identity and starts at revision 1", () => {
  const record = createInterfaceRecord("inbox", "Inbox triage");

  assert.deepEqual(record, {
    kind: "interface",
    id: "inbox",
    title: "Inbox triage",
    revision: 1,
  });
  assert.equal(Object.isFrozen(record), true);
});

test("creation rejects invalid id, title, and initial revision", () => {
  assert.throws(() => createInterfaceRecord("", "Title"), /id/);
  assert.throws(() => createInterfaceRecord("  ", "Title"), /id/);
  assert.throws(() => createInterfaceRecord("id", ""), /title/);
  assert.throws(() => createInterfaceRecord("id", " \n "), /title/);
  assert.throws(() => createInterfaceRecord("id", "Title", 0), /revision 1/);
  assert.throws(() => createInterfaceRecord("id", "Title", 2), /revision 1/);
});

test("matching update returns a new immutable revision", () => {
  const original = createInterfaceRecord("calendar", "Calendar");
  const before = { ...original };
  const updated = updateInterfaceRecord(original, 1, "Week plan");

  assert.notStrictEqual(updated, original);
  assert.deepEqual(original, before);
  assert.deepEqual(updated, {
    kind: "interface",
    id: "calendar",
    title: "Week plan",
    revision: 2,
  });
  assert.equal(Object.isFrozen(updated), true);
});

test("stale and invalid revisions are rejected without mutation", () => {
  const original = createInterfaceRecord("tasks", "Tasks");
  const before = { ...original };

  assert.throws(
    () => updateInterfaceRecord(original, 2, "Changed"),
    /does not match/,
  );
  assert.throws(() => updateInterfaceRecord(original, 0, "Changed"), /positive/);
  assert.throws(() => updateInterfaceRecord(original, 1.5, "Changed"), /positive/);
  assert.deepEqual(original, before);
});

test("deletion creates a tombstone that rejects late updates", () => {
  const record = createInterfaceRecord("board", "Project board");
  const tombstone = deleteInterfaceRecord(record, 1);

  assert.deepEqual(tombstone, {
    kind: "tombstone",
    id: "board",
    title: "Project board",
    revision: 2,
  });
  assert.equal(Object.isFrozen(tombstone), true);
  assert.throws(
    () => updateInterfaceRecord(tombstone, 2, "Resurrected"),
    /deleted interface/,
  );
});

test("mutating one interface leaves unrelated records unchanged", () => {
  const first = createInterfaceRecord("first", "First");
  const unrelated = createInterfaceRecord("second", "Second");
  const unrelatedBefore = { ...unrelated };

  updateInterfaceRecord(first, 1, "Updated first");
  deleteInterfaceRecord(first, 1);

  assert.deepEqual(unrelated, unrelatedBefore);
  assert.equal(unrelated.revision, 1);
});
