import { test } from "node:test";
import assert from "node:assert/strict";

import {
  createCatalog,
  addConnector,
  resolve,
  lookup,
  connectedSet,
  serializeEntry,
  serializeManifest,
  type ReadinessComponents
} from "../src/contracts/connector-catalog.ts";
import { REFERENCE_MAIL_MANIFEST, isSecretLikeKey } from "../src/contracts/connector-manifest.ts";

const all: ReadinessComponents = { browserSession: true, apiAuthorization: true, osPermission: true };
const opts = { fixtureEnabled: true, hostSdkVersion: "22.1.0" };

// Independent pinned pattern, cross-checking the shared deny-list for drift.
const SECRET_RE = /credential|token|secret|password|passphrase|api[-_]?key|apikey|client[-_]?secret|refresh[-_]?key|auth[-_]?header|bearer/i;

function assertNoCredentialKeys(json: string): void {
  const walk = (node: unknown, path: string): void => {
    if (node === null || typeof node !== "object") return;
    if (Array.isArray(node)) {
      node.forEach((v, i) => walk(v, `${path}[${i}]`));
      return;
    }
    for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
      assert.equal(SECRET_RE.test(k), false, `credential-like key "${k}" at ${path}`);
      assert.equal(isSecretLikeKey(k), false, `shared deny-list drift on "${k}" at ${path}`);
      walk(v, `${path}.${k}`);
    }
  };
  walk(JSON.parse(json), "$");
}

test("connector is connected only when fixture is enabled and all components are satisfied", () => {
  const cat = createCatalog();
  assert.equal(addConnector(cat, REFERENCE_MAIL_MANIFEST, opts).ok, true);
  const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, all, opts);
  assert.equal(entry.connected, true);
  assert.deepEqual(connectedSet(cat, all, opts), [REFERENCE_MAIL_MANIFEST.id]);
});

test("readiness components are evaluated independently with per-component reason codes", () => {
  const cat = createCatalog();
  addConnector(cat, REFERENCE_MAIL_MANIFEST, opts);
  const cases: Array<[ReadinessComponents, string]> = [
    [{ ...all, browserSession: false }, "BROWSER_SESSION_REQUIRED"],
    [{ ...all, apiAuthorization: false }, "API_AUTHORIZATION_REQUIRED"],
    [{ ...all, osPermission: false }, "OS_PERMISSION_REQUIRED"]
  ];
  for (const [readiness, reason] of cases) {
    const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, readiness, opts);
    assert.equal(entry.connected, false);
    assert.equal(entry.reasonCode, reason);
    assert.equal(entry.unsatisfiedComponents.length, 1);
  }
  assert.deepEqual(connectedSet(cat, { browserSession: false, apiAuthorization: true, osPermission: true }, opts), []);
});

test("manifest self-claims cannot force a connected state", () => {
  const cat = createCatalog();
  // Fully self-satisfied manifest (terms accepted, valid, in range) with nothing else ready.
  addConnector(cat, REFERENCE_MAIL_MANIFEST, opts);
  const none: ReadinessComponents = { browserSession: false, apiAuthorization: false, osPermission: false };
  const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, none, opts);
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "BROWSER_SESSION_REQUIRED");
  assert.deepEqual(entry.unsatisfiedComponents, ["browserSession", "apiAuthorization", "osPermission"]);
});

test("fixture disablement keeps connectors out of the connected set", () => {
  const cat = createCatalog();
  addConnector(cat, REFERENCE_MAIL_MANIFEST, opts);
  const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, all, { fixtureEnabled: false, hostSdkVersion: "22.1.0" });
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "FIXTURE_DISABLED");
  assert.deepEqual(connectedSet(cat, all, { fixtureEnabled: false }), []);
});

test("invalid manifests never appear connected or available", () => {
  const cat = createCatalog();
  const bad = { ...REFERENCE_MAIL_MANIFEST, id: "fixture-bad", version: "nope" };
  const added = addConnector(cat, bad, opts);
  assert.equal(added.ok, false);
  assert.equal(added.reasonCode, "MANIFEST_INVALID");
  const entry = resolve(cat, "fixture-bad", all, opts);
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "MANIFEST_INVALID");
  assert.ok(entry.manifestErrors && entry.manifestErrors.some((e) => e.field === "version"));
  assert.deepEqual(connectedSet(cat, all, opts), []);
});

test("duplicate ids are rejected and surface an actionable reason", () => {
  const cat = createCatalog();
  assert.equal(addConnector(cat, REFERENCE_MAIL_MANIFEST, opts).ok, true);
  const second = addConnector(cat, REFERENCE_MAIL_MANIFEST, opts);
  assert.equal(second.ok, false);
  assert.equal(second.reasonCode, "DUPLICATE_ID");
  const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, all, opts);
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "DUPLICATE_ID");
});

test("version gate is enforced at resolve time against the host SDK version", () => {
  const cat = createCatalog();
  // Added under an in-range host SDK.
  assert.equal(addConnector(cat, REFERENCE_MAIL_MANIFEST, opts).ok, true);
  // Resolved later under an out-of-range host SDK: real, reachable gate.
  const entry = resolve(cat, REFERENCE_MAIL_MANIFEST.id, all, {
    fixtureEnabled: true,
    hostSdkVersion: "99.0.0"
  });
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "SDK_OUT_OF_RANGE");
});

test("lookup of an unknown id returns the actionable setup seam, not silence", () => {
  const cat = createCatalog();
  const entry = lookup(cat, "fixture-ghost", all, opts);
  assert.equal(entry.connected, false);
  assert.equal(entry.reasonCode, "UNKNOWN_CONNECTOR");
});

test("serialized manifests and catalog entries contain no credential-like fields", () => {
  const cat = createCatalog();
  addConnector(cat, REFERENCE_MAIL_MANIFEST, opts);
  const manifestJson = serializeManifest({
    ...REFERENCE_MAIL_MANIFEST,
    metadata: { apiToken: "should-be-stripped", password: "x", note: "keep" }
  });
  assertNoCredentialKeys(manifestJson);
  assert.ok(manifestJson.includes("keep"));
  assert.equal(manifestJson.includes("should-be-stripped"), false);

  const entryJson = serializeEntry({
    id: "x",
    connected: false,
    reasonCode: "API_AUTHORIZATION_REQUIRED",
    unsatisfiedComponents: ["apiAuthorization"]
  });
  assertNoCredentialKeys(entryJson);
  assert.ok(entryJson.includes("API_AUTHORIZATION_REQUIRED"));
});
