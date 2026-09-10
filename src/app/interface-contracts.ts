/** Presentation data only. Provenance and authority are never accepted from this wire spec. */
export type InterfaceCell = string | number | null;
export type InterfaceColumn = { id: string; type: 'text' | 'number' | 'timestamp' };
export type InterfaceDataset = { id: string; columns: InterfaceColumn[]; rows: Record<string, InterfaceCell>[] };
export type InterfaceNode =
  | { id: string; type: 'stack'; children: InterfaceNode[] }
  | { id: string; type: 'grid'; columns: number; children: InterfaceNode[] }
  | { id: string; type: 'card'; title: string; children: InterfaceNode[] }
  | { id: string; type: 'text'; text: string }
  | { id: string; type: 'metric'; label: string; datasetId: string; field: string; format: 'number' | 'percent' | 'currency'; currency?: string }
  | { id: string; type: 'table'; datasetId: string; columns: { field: string; label: string }[] }
  | { id: string; type: 'chart'; kind: 'bar' | 'line'; datasetId: string; xField: string; yField: string; label: string; units: string };
export interface InterfaceSpec { schemaVersion: 1; kind: 'components'; root: InterfaceNode; datasets: InterfaceDataset[] }
export type InterfaceValidation = { ok: true; spec: InterfaceSpec } | { ok: false; error: string };

const MAX_BYTES = 65_536;
const forbidden = new Set(['__proto__', 'prototype', 'constructor']);
function fail(reason: string): never { throw new Error(reason); }
function object(value: unknown, keys?: string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail('Expected an object.');
  const result = value as Record<string, unknown>;
  if (Object.keys(result).some(key => forbidden.has(key) || (keys && !keys.includes(key)))) fail('Unsupported field in interface.');
  return result;
}
function text(value: unknown, max = 2000, empty = true): string {
  if (typeof value !== 'string' || Array.from(value).length > max || (!empty && !value.trim()) || /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/u.test(value)) fail('Invalid or oversized text.');
  for (const character of value) { const point = character.codePointAt(0)!; if (point >= 0xd800 && point <= 0xdfff) fail('Malformed Unicode text.'); }
  return value;
}
function id(value: unknown): string {
  const result = text(value, 80, false);
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(result) || forbidden.has(result)) fail('Invalid field or node ID.');
  return result;
}
function array(value: unknown, max: number): unknown[] {
  if (!Array.isArray(value) || value.length > max) fail('Interface exceeds its item limit.');
  return value;
}
function integer(value: unknown, min: number, max: number): number {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < min || value > max) fail('Invalid bounded integer.');
  return value;
}
export function isInterfaceTimestamp(value: unknown): value is string {
  return typeof value === 'string' && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?Z$/.test(value) && Number.isFinite(Date.parse(value)) && new Date(value).toISOString().replace('.000Z', 'Z') === value.replace(/\.(\d{1,2})Z$/, (_, digits: string) => `.${digits.padEnd(3, '0')}Z`).replace('.000Z', 'Z');
}
export function validateInterfaceSpec(input: unknown): InterfaceValidation {
  try {
    // Bounded preflight avoids recursive JSON serialization of attacker-controlled depth/cycles.
    const seen = new Set<object>(); const pending: { value: unknown; depth: number }[] = [{ value: input, depth: 0 }]; let entries = 0;
    while (pending.length) {
      const { value, depth } = pending.pop()!;
      if (++entries > 6000 || depth > 16) fail('Interface exceeds its structural limit.');
      if (value && typeof value === 'object') {
        if (seen.has(value)) fail('Repeated or cyclic objects are unsupported.'); seen.add(value);
        const values = Object.values(value); if (values.length > 100) fail('Interface exceeds its item limit.');
        for (const child of values) pending.push({ value: child, depth: depth + 1 });
      } else if (typeof value === 'string' && value.length > MAX_BYTES) fail('Interface exceeds its text limit.');
    }
    if (new TextEncoder().encode(JSON.stringify(input)).length > MAX_BYTES) fail('Interface exceeds 64 KiB.');
    const spec = object(input, ['schemaVersion', 'kind', 'root', 'datasets']);
    if (spec.schemaVersion !== 1 || spec.kind !== 'components') fail('Unsupported interface version or mode. Custom code is unavailable until isolation is verified.');
    const datasetIds = new Set<string>();
    const datasets: InterfaceDataset[] = array(spec.datasets, 12).map(raw => {
      const data = object(raw, ['id', 'columns', 'rows']); const dataId = id(data.id);
      if (datasetIds.has(dataId)) fail('Duplicate dataset ID.'); datasetIds.add(dataId);
      const fields = new Set<string>();
      const columns = array(data.columns, 12).map(rawColumn => {
        const column = object(rawColumn, ['id', 'type']); const field = id(column.id);
        if (fields.has(field)) fail('Duplicate column ID.'); fields.add(field);
        if (!['text', 'number', 'timestamp'].includes(column.type as string)) fail('Unsupported column type.');
        return { id: field, type: column.type as InterfaceColumn['type'] };
      });
      if (!columns.length) fail('A dataset needs typed columns.');
      const rows = array(data.rows, 100).map(rawRow => {
        const row = object(rawRow, [...fields]); const clean: Record<string, InterfaceCell> = {};
        for (const column of columns) {
          const value = row[column.id];
          if (value === null) clean[column.id] = null;
          else if (column.type === 'number') {
            if (typeof value !== 'number' || !Number.isFinite(value) || Math.abs(value) > 1e12) fail('Numbers must be finite and bounded by 1e12.');
            clean[column.id] = value;
          } else {
            clean[column.id] = text(value);
            if (column.type === 'timestamp' && !isInterfaceTimestamp(value)) fail('Timestamps must be valid UTC instants.');
          }
        }
        return clean;
      });
      return { id: dataId, columns, rows };
    });
    let nodes = 0; let content = 0; const nodeIds = new Set<string>();
    function node(raw: unknown, layoutDepth: number): InterfaceNode {
      const n = object(raw); const nodeId = id(n.id);
      if (++nodes > 40 || nodeIds.has(nodeId)) fail('Too many nodes or duplicate node ID.'); nodeIds.add(nodeId);
      const common = ['id', 'type'];
      if (n.type === 'stack' || n.type === 'grid' || n.type === 'card') {
        if (layoutDepth >= 3) fail('At most three layout levels are supported.');
        object(n, [...common, 'children', ...(n.type === 'grid' ? ['columns'] : n.type === 'card' ? ['title'] : [])]);
        const children = array(n.children, 40).map(child => node(child, layoutDepth + 1));
        if (n.type === 'grid') return { id: nodeId, type: 'grid', columns: integer(n.columns, 1, 4), children };
        if (n.type === 'card') return { id: nodeId, type: 'card', title: text(n.title, 160), children };
        return { id: nodeId, type: 'stack', children };
      }
      if (++content > 12) fail('At most twelve content blocks are supported.');
      if (n.type === 'text') { object(n, [...common, 'text']); return { id: nodeId, type: 'text', text: text(n.text) }; }
      if (!['metric', 'table', 'chart'].includes(n.type as string)) fail('Unsupported component. Scripts, HTML, styles and actions are not allowed.');
      const datasetId = id(n.datasetId); const dataset = datasets.find(data => data.id === datasetId);
      if (!dataset) fail('Unknown dataset reference.');
      function field(value: unknown, requiredType?: InterfaceColumn['type']): string {
        const fieldId = id(value); const column = dataset!.columns.find(c => c.id === fieldId);
        if (!column || (requiredType && column.type !== requiredType)) fail('Missing or incorrectly typed data field.');
        return fieldId;
      }
      if (n.type === 'metric') {
        object(n, [...common, 'label', 'datasetId', 'field', 'format', 'currency']);
        if (!['number', 'percent', 'currency'].includes(n.format as string)) fail('Unsupported number format.');
        if (dataset.rows.length > 1) fail('A metric needs a single explicitly supplied value, not an implicit aggregate.');
        if (n.format === 'currency' ? typeof n.currency !== 'string' || !/^[A-Z]{3}$/.test(n.currency) : n.currency !== undefined) fail('Currency must be an explicit three-letter code for currency format only.');
        return { id: nodeId, type: 'metric', datasetId, field: field(n.field, 'number'), label: text(n.label, 160), format: n.format as 'number' | 'percent' | 'currency', ...(n.currency ? { currency: n.currency as string } : {}) };
      }
      if (n.type === 'table') {
        object(n, [...common, 'datasetId', 'columns']); const used = new Set<string>();
        const columns = array(n.columns, 12).map(rawColumn => { const column = object(rawColumn, ['field', 'label']); const f = field(column.field); if (used.has(f)) fail('Duplicate table field.'); used.add(f); return { field: f, label: text(column.label, 160) }; });
        if (!columns.length) fail('A table needs columns.');
        return { id: nodeId, type: 'table', datasetId, columns };
      }
      object(n, [...common, 'kind', 'datasetId', 'xField', 'yField', 'label', 'units']);
      if (n.kind !== 'bar' && n.kind !== 'line') fail('Only single-series bar and line charts are supported.');
      const xField = field(n.xField, n.kind === 'line' ? 'timestamp' : 'text'); const yField = field(n.yField, 'number');
      const xs = new Set<InterfaceCell>(); let previous = -Infinity;
      for (const row of dataset.rows) {
        if (row[xField] === null || xs.has(row[xField])) fail('Chart categories/times must be present and unique.'); xs.add(row[xField]);
        if (n.kind === 'line') { const time = Date.parse(row[xField] as string); if (time <= previous) fail('Line timestamps must increase.'); previous = time; }
      }
      return { id: nodeId, type: 'chart', kind: n.kind, datasetId, xField, yField, label: text(n.label, 160), units: text(n.units, 80) };
    }
    return { ok: true, spec: { schemaVersion: 1, kind: 'components', root: node(spec.root, 0), datasets } };
  } catch (error) { return { ok: false, error: error instanceof Error ? error.message : 'Invalid interface specification.' }; }
}

export function emptyInterfaceSpec(): InterfaceSpec { return { schemaVersion: 1, kind: 'components', root: { id: 'root', type: 'stack', children: [] }, datasets: [] }; }
