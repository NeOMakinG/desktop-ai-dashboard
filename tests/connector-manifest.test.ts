import { test } from "node:test";
import assert from "node:assert/strict";

import {
  validateManifest,
  REFERENCE_MAIL_MANIFEST,
  type ConnectorManifest
} from "../src/contracts/connector-manifest.ts";

function variant(overrides: Record<string, unknown>): ConnectorManifest {
  return { ...REFERENCE_MAIL_MANIFEST, ...overrides } as ConnectorManifest;
}

function hasError(result: ReturnType<typeof validateManifest>, field: string, code: string): boolean {
  return result.errors.some((e) => e.field === field && e.code === code);
}

test("reference mail fixture manifest validates successfully", () => {
  const result = validateManifest(REFERENCE_MAIL_MANIFEST, { hostSdkVersion: "22.1.0" });
  assert.equal(result.ok, true, JSON.stringify(result.errors));
});

test("rejects missing id with a field-specific error", () => {
  const m = variant({ id: "" });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "id", "MISSING_ID"));
});

test("rejects missing version with a field-specific error", () => {
  const result = validateManifest(variant({ version: "" }));
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "version", "MISSING_VERSION"));
});

test("rejects invalid semver with a field-specific error", () => {
  const result = validateManifest(variant({ version: "1.2" }));
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "version", "INVALID_SEMVER"));
});

test("rejects host SDK version outside the compatibility range", () => {
  const result = validateManifest(REFERENCE_MAIL_MANIFEST, { hostSdkVersion: "21.9.0" });
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "sdkVersion", "SDK_OUT_OF_RANGE"));
});

test("rejects manifest sdkVersion outside the compatibility range", () => {
  const result = validateManifest(variant({ sdkVersion: "99.0.0" }));
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "sdkVersion", "SDK_OUT_OF_RANGE"));
});

test("rejects non-bounded operation kinds with a field-specific error", () => {
  const m = variant({
    operations: [
      { name: "sendMail", kind: "write", maxItems: 1 }
    ]
  });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "operations[0].kind", "NON_BOUNDED_KIND"));
});

test("rejects duplicate operation names with a field-specific error", () => {
  const m = variant({
    operations: [
      { name: "a", kind: "boundedRead", maxItems: 5 },
      { name: "a", kind: "boundedRead", maxItems: 5 }
    ]
  });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "operations", "DUPLICATE_OPERATION_NAME"));
});

test("rejects secret-like credential fields, including nested metadata", () => {
  const m = variant({ metadata: { apiToken: "abc", note: "fine" } });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "metadata.apiToken", "SECRET_LIKE_FIELD"));
});

test("rejects freshness metadata lacking units with a field-specific error", () => {
  const m = variant({ freshness: { value: 5 } });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "freshness.units", "MISSING_FRESHNESS_UNITS"));
});

test("rejects source metadata lacking an explicit kind", () => {
  const m = variant({ source: { label: "x" } });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "source.kind", "MISSING_SOURCE_KIND"));
});

test("rejects an unlabeled non-fixture manifest", () => {
  const m = variant({ isTestFixture: false });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "isTestFixture", "NOT_FIXTURE_LABELED"));
});

test("rejects a missing provider terms prerequisite", () => {
  const m = variant({ prerequisites: {} });
  const result = validateManifest(m);
  assert.equal(result.ok, false);
  assert.ok(hasError(result, "prerequisites.providerTermsAccepted", "MISSING_PROVIDER_TERMS"));
});
