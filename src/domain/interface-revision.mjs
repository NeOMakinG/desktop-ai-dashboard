function requireNonemptyString(value, field) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${field} must be a nonempty string`);
  }
  return value.trim();
}

function requireRevision(value, field = "revision") {
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new TypeError(`${field} must be a positive safe integer`);
  }
  return value;
}

function validateEntity(entity) {
  if (entity === null || typeof entity !== "object") {
    throw new TypeError("interface must be a record or tombstone");
  }
  if (entity.kind !== "interface" && entity.kind !== "tombstone") {
    throw new TypeError("interface kind is invalid");
  }
  requireNonemptyString(entity.id, "id");
  requireNonemptyString(entity.title, "title");
  requireRevision(entity.revision);
  return entity;
}

export function createInterfaceRecord(id, title, revision = 1) {
  const validId = requireNonemptyString(id, "id");
  const validTitle = requireNonemptyString(title, "title");
  if (revision !== 1) {
    throw new RangeError("a new interface must start at revision 1");
  }
  return Object.freeze({
    kind: "interface",
    id: validId,
    title: validTitle,
    revision: 1,
  });
}

export function updateInterfaceRecord(current, expectedRevision, nextTitle) {
  validateEntity(current);
  if (current.kind === "tombstone") {
    throw new Error("a deleted interface cannot be updated");
  }
  requireRevision(expectedRevision, "expectedRevision");
  if (current.revision !== expectedRevision) {
    throw new Error("interface revision does not match expectedRevision");
  }
  return Object.freeze({
    kind: "interface",
    id: current.id,
    title: requireNonemptyString(nextTitle, "title"),
    revision: current.revision + 1,
  });
}

export function deleteInterfaceRecord(current, expectedRevision) {
  validateEntity(current);
  if (current.kind === "tombstone") {
    throw new Error("interface is already deleted");
  }
  requireRevision(expectedRevision, "expectedRevision");
  if (current.revision !== expectedRevision) {
    throw new Error("interface revision does not match expectedRevision");
  }
  return Object.freeze({
    kind: "tombstone",
    id: current.id,
    title: current.title,
    revision: current.revision + 1,
  });
}
