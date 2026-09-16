import { test } from "node:test";
import assert from "node:assert/strict";

import {
  emptyPinState,
  isPinned,
  togglePin,
  pin,
  dropPin,
  pinnedOrdering,
  orderWorkspaceHistory,
  type WorkspacePinState,
  type WorkspaceSummary,
} from "../src/domain/workspace-pin.ts";

const ws = (id: string, lastActivityAt: number): WorkspaceSummary => ({
  id,
  lastActivityAt,
});

const ids = (list: readonly WorkspaceSummary[]) => list.map((w) => w.id);

// Baseline: newest to oldest, as the sidebar orders today.
const history: WorkspaceSummary[] = [
  ws("ws-newest", 1000),
  ws("ws-daily", 700),
  ws("ws-mid", 400),
  ws("ws-oldest", 100),
];

const knownIds = history.map((w) => w.id);

test("togglePin turns a pin on and off", () => {
  const s0 = emptyPinState();
  assert.equal(isPinned(s0, "ws-daily"), false);

  const on = togglePin(s0, "ws-daily", knownIds);
  assert.equal(on.ok, true);
  if (on.ok) {
    assert.equal(on.value.pinned, true);
    assert.equal(isPinned(on.value.state, "ws-daily"), true);
    // original state untouched (pure)
    assert.equal(isPinned(s0, "ws-daily"), false);
  }

  const off = on.ok ? togglePin(on.value.state, "ws-daily", knownIds) : null;
  assert.ok(off && off.ok);
  if (off && off.ok) {
    assert.equal(off.value.pinned, false);
    assert.equal(isPinned(off.value.state, "ws-daily"), false);
  }
});

test("togglePin returns typed errors, never throws", () => {
  const s0 = emptyPinState();

  const unknown = togglePin(s0, "ws-ghost", knownIds);
  assert.equal(unknown.ok, false);
  if (!unknown.ok) {
    assert.equal(unknown.error.kind, "workspace_not_found");
    assert.equal(unknown.error.workspaceId, "ws-ghost");
  }

  const invalid = togglePin(s0, "", knownIds);
  assert.equal(invalid.ok, false);
  if (!invalid.ok) {
    assert.equal(invalid.error.kind, "invalid_workspace_id");
  }

  // No exception escapes for any input shape.
  assert.doesNotThrow(() => togglePin(s0, "ws-newest", knownIds));
});

test("mixed ordering: pinned above unpinned; recency within each group", () => {
  // Pin an older workspace: it must rise above all unpinned ones.
  const t = togglePin(emptyPinState(), "ws-mid", knownIds);
  assert.ok(t.ok);
  const state: WorkspacePinState = t.ok ? t.value.state : emptyPinState();

  const ordered = orderWorkspaceHistory(history, state);
  assert.deepEqual(ids(ordered), [
    "ws-mid", // pinned, only one
    "ws-newest", // unpinned by recency, exactly as today
    "ws-daily",
    "ws-oldest",
  ]);

  // Multiple pinned: among pinned, most recent activity first.
  const t2 = togglePin(state, "ws-oldest", knownIds);
  assert.ok(t2.ok);
  const state2: WorkspacePinState = t2.ok ? t2.value.state : state;
  const ordered2 = orderWorkspaceHistory(history, state2);
  assert.deepEqual(ids(ordered2), [
    "ws-mid", // pinned, activity 400 > 100
    "ws-oldest", // pinned, activity 100
    "ws-newest",
    "ws-daily",
  ]);
});

test("deleting a workspace cascades pin removal; no orphan, no resurrection", () => {
  const t = togglePin(emptyPinState(), "ws-daily", knownIds);
  assert.ok(t.ok);
  const pinnedState: WorkspacePinState = t.ok ? t.value.state : emptyPinState();
  assert.equal(isPinned(pinnedState, "ws-daily"), true);

  const afterDelete = dropPin(pinnedState, "ws-daily");
  assert.equal(isPinned(afterDelete, "ws-daily"), false);
  // No orphan entry in the record.
  assert.equal(Object.prototype.hasOwnProperty.call(afterDelete.pins, "ws-daily"), false);
  // Ordering cannot resurrect the deleted workspace: without the pin,
  // remaining history orders purely by recency.
  const remaining = history.filter((w) => w.id !== "ws-daily");
  assert.deepEqual(ids(orderWorkspaceHistory(remaining, afterDelete)), [
    "ws-newest",
    "ws-mid",
    "ws-oldest",
  ]);

  // Idempotent on never-pinned / already-dropped ids.
  assert.deepEqual(dropPin(afterDelete, "ws-daily"), afterDelete);
  assert.deepEqual(dropPin(emptyPinState(), "ws-never"), emptyPinState());
});

test("re-pinning an already-pinned workspace is idempotent", () => {
  const t = togglePin(emptyPinState(), "ws-daily", knownIds);
  assert.ok(t.ok);
  const once: WorkspacePinState = t.ok ? t.value.state : emptyPinState();

  // `pin` on an already-pinned workspace returns the same state object.
  const again = pin(once, "ws-daily", knownIds);
  assert.ok(again.ok);
  if (again.ok) {
    assert.equal(again.value.pinned, true);
    assert.equal(again.value.state, once); // same reference: no duplicate entry
  }

  // Toggling twice returns to unpinned, a third toggle re-pins once.
  const off = togglePin(once, "ws-daily", knownIds);
  const onAgain = off.ok ? togglePin(off.value.state, "ws-daily", knownIds) : null;
  assert.ok(onAgain && onAgain.ok);
  if (onAgain && onAgain.ok) {
    assert.deepEqual(onAgain.value.state.pins, { "ws-daily": true });
  }
});

test("deterministic tie-break on workspace id when activity is equal", () => {
  const tied: WorkspaceSummary[] = [
    ws("ws-b", 500),
    ws("ws-a", 500),
    ws("ws-c", 500),
  ];
  const tieIds = tied.map((w) => w.id);

  // Unpinned ties: ascending id regardless of input order.
  const cmp = pinnedOrdering(emptyPinState());
  assert.deepEqual(ids([...tied].sort(cmp)), ["ws-a", "ws-b", "ws-c"]);
  assert.deepEqual(
    ids(orderWorkspaceHistory([ws("ws-c", 500), ws("ws-a", 500), ws("ws-b", 500)], emptyPinState())),
    ["ws-a", "ws-b", "ws-c"],
  );

  // Pinned ties: same deterministic id order.
  const t = togglePin(emptyPinState(), "ws-b", tieIds);
  const state: WorkspacePinState = t.ok ? t.value.state : emptyPinState();
  assert.deepEqual(ids(orderWorkspaceHistory(tied, state)), [
    "ws-b",
    "ws-a",
    "ws-c",
  ]);

  // Repeated sorts of the same input are stable.
  const first = ids(orderWorkspaceHistory(tied, state));
  for (let i = 0; i < 5; i++) {
    assert.deepEqual(ids(orderWorkspaceHistory(tied, state)), first);
  }
});

test("ordering is stable when a pinned workspace's activity updates", () => {
  // Two pinned workspaces; bump the older pinned one's activity so it
  // becomes the most recent pinned. Group membership does not change.
  const s0 = emptyPinState();
  const p1 = togglePin(s0, "ws-mid", knownIds);
  const p2 = p1.ok ? togglePin(p1.value.state, "ws-oldest", knownIds) : null;
  assert.ok(p2 && p2.ok);
  const pinned: WorkspacePinState = p2 && p2.ok ? p2.value.state : s0;

  const before = ids(orderWorkspaceHistory(history, pinned));
  assert.deepEqual(before, ["ws-mid", "ws-oldest", "ws-newest", "ws-daily"]);

  // Activity update on a pinned workspace: pinned set unchanged, order
  // among pinned follows the new activity, unpinned block untouched.
  const updated: WorkspaceSummary[] = history.map((w) =>
    w.id === "ws-oldest" ? ws("ws-oldest", 9999) : w,
  );
  const after = ids(orderWorkspaceHistory(updated, pinned));
  assert.deepEqual(after, ["ws-oldest", "ws-mid", "ws-newest", "ws-daily"]);

  // Activity update on an unpinned workspace must not leapfrog a pin.
  const updated2: WorkspaceSummary[] = history.map((w) =>
    w.id === "ws-daily" ? ws("ws-daily", 99999) : w,
  );
  const after2 = ids(orderWorkspaceHistory(updated2, pinned));
  assert.deepEqual(after2, [
    "ws-mid",
    "ws-oldest",
    "ws-daily", // newest unpinned, but still below both pins
    "ws-newest",
  ]);
});

test("ordering never mutates its input array", () => {
  const t = togglePin(emptyPinState(), "ws-oldest", knownIds);
  const state: WorkspacePinState = t.ok ? t.value.state : emptyPinState();
  const input = [...history];
  const snapshot = ids(input);
  orderWorkspaceHistory(input, state);
  assert.deepEqual(ids(input), snapshot); // original order preserved
});
