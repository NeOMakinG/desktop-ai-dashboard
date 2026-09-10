import { useId, useMemo, useState } from 'react';
import { InlineError } from './components';
import { validateInterfaceSpec, type InterfaceCell, type InterfaceDataset, type InterfaceNode, type InterfaceSpec } from './interface-contracts';

function cell(value: InterfaceCell | undefined): string { return value === null || value === undefined ? 'Not supplied' : typeof value === 'number' ? new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 }).format(value) : value; }
function DataTable({ dataset, columns, label }: { dataset: InterfaceDataset; columns: { field: string; label: string }[]; label: string }) {
  const [sort, setSort] = useState<{ field: string; descending: boolean } | null>(null);
  const rows = useMemo(() => {
    if (!sort) return dataset.rows;
    return [...dataset.rows].sort((a, b) => {
      const x = a[sort.field], y = b[sort.field];
      if (x === null) return y === null ? 0 : 1; if (y === null) return -1;
      return (typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y))) * (sort.descending ? -1 : 1);
    });
  }, [dataset, sort]);
  return <div className="interface-table-scroll" tabIndex={0} role="region" aria-label={label}>
    <table className="interface-table"><caption className="sr-only">{label}. Values from conversation, not live account data.</caption><thead><tr>{columns.map(column => <th key={column.field} scope="col" aria-sort={sort?.field === column.field ? sort.descending ? 'descending' : 'ascending' : 'none'} className={dataset.columns.find(c => c.id === column.field)?.type === 'number' ? 'numeric' : ''}><button type="button" onClick={() => setSort(previous => ({ field: column.field, descending: previous?.field === column.field ? !previous.descending : false }))}>{column.label}{sort?.field === column.field ? sort.descending ? ' ↓' : ' ↑' : ''}</button></th>)}</tr></thead>
      <tbody>{rows.map((row, index) => <tr key={index}>{columns.map(column => <td key={column.field} className={dataset.columns.find(c => c.id === column.field)?.type === 'number' ? 'numeric' : ''}>{cell(row[column.field])}</td>)}</tr>)}</tbody>
    </table>{!rows.length && <p className="field-note">No rows supplied in this conversation dataset. This is not a live-source empty result.</p>}
  </div>;
}

function Chart({ node, dataset }: { node: Extract<InterfaceNode, { type: 'chart' }>; dataset: InterfaceDataset }) {
  const titleId = useId(); const tipId = useId(); const [active, setActive] = useState<number | null>(null);
  const values = dataset.rows.map(row => row[node.yField] as number | null);
  const finite = values.filter((value): value is number => value !== null);
  const low = Math.min(0, ...finite), high = Math.max(0, ...finite);
  const span = high - low || 1;
  const width = 640, height = 256, left = 64, right = 616, top = 24, bottom = 208;
  const y = (value: number) => bottom - (value - low) / span * (bottom - top);
  const times = dataset.rows.map(row => node.kind === 'line' ? Date.parse(String(row[node.xField])) : 0);
  const timeSpan = (times.at(-1) ?? 0) - (times[0] ?? 0);
  const x = (index: number) => node.kind === 'line' ? timeSpan ? left + (times[index] - times[0]) / timeSpan * (right - left) : (left + right) / 2 : left + (index + .5) / Math.max(1, values.length) * (right - left);
  const activeRow = active === null ? undefined : dataset.rows[active];
  let path = ''; let penDown = false;
  values.forEach((value, index) => { if (value === null) { penDown = false; return; } path += `${penDown ? 'L' : 'M'}${x(index)},${y(value)} `; penDown = true; });
  const label = (index: number) => `${cell(dataset.rows[index][node.xField])}: ${cell(values[index])}${node.units ? ` ${node.units}` : ''}`;
  return <section className="interface-chart" aria-labelledby={titleId}>
    <h3 id={titleId}>{node.label}</h3><p className="field-note">{node.units || 'Values'} · Single series · Missing values remain gaps</p>
    {!dataset.rows.length || !finite.length ? <p className="field-note">No numeric points supplied. No zero values have been invented.</p> : <>
      <svg viewBox={`0 0 ${width} ${height}`} aria-labelledby={titleId} className="interface-plot" onPointerLeave={() => setActive(null)} onPointerMove={event => {
        if (node.kind !== 'line') return;
        const rect = event.currentTarget.getBoundingClientRect(); const pos = (event.clientX - rect.left) / rect.width * width;
        let nearest = 0; for (let i = 1; i < values.length; i++) if (Math.abs(x(i) - pos) < Math.abs(x(nearest) - pos)) nearest = i;
        setActive(nearest);
      }}>
        {[low, (low + high) / 2, high].filter((value, index, all) => all.indexOf(value) === index).map(value => <g key={value}><line className="chart-axis" x1={left} x2={right} y1={y(value)} y2={y(value)} /><text className="chart-tick" x={left - 8} y={y(value) + 4} textAnchor="end">{new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value)}</text></g>)}
        <line className="chart-baseline" x1={left} x2={right} y1={y(0)} y2={y(0)} />
        {node.kind === 'line' && <path className="chart-line" d={path} />}
        {node.kind === 'line' && active !== null && <line className="chart-crosshair" x1={x(active)} x2={x(active)} y1={top} y2={bottom} />}
        {values.map((value, index) => {
          const cx = x(index); const cy = y(value ?? 0); const barWidth = Math.min(24, (right - left) / values.length - 2); const zero = y(0); const barHeight = Math.abs(cy - zero); const radius = Math.min(4, barHeight / 2, barWidth / 2); const a = cx - barWidth / 2; const b = cx + barWidth / 2;
          const barPath = value !== null && value >= 0 ? `M${a},${zero}V${cy + radius}Q${a},${cy} ${a + radius},${cy}H${b - radius}Q${b},${cy} ${b},${cy + radius}V${zero}Z` : `M${a},${zero}V${cy - radius}Q${a},${cy} ${a + radius},${cy}H${b - radius}Q${b},${cy} ${b},${cy - radius}V${zero}Z`;
          return <g key={index}>
            {value !== null && (node.kind === 'bar' ? <path className="chart-bar" d={barPath} /> : <circle className="chart-point" cx={cx} cy={cy} r={4} />)}
            <rect className="chart-hit" x={cx - 12} y={node.kind === 'line' ? top : Math.min(cy, zero) - 8} width={24} height={node.kind === 'line' ? bottom - top : Math.max(24, barHeight + 16)} tabIndex={0} role="img" aria-label={label(index)} aria-describedby={active === index ? tipId : undefined} onFocus={() => setActive(index)} onBlur={() => setActive(null)} onPointerEnter={() => setActive(index)} />
          </g>;
        })}
        <text className="chart-tick" x={left} y={240}>{node.kind === 'line' ? String(dataset.rows[0][node.xField]).slice(0, 10) : 'Categories in supplied order'}</text>
        {node.kind === 'line' && <text className="chart-tick" x={right} y={240} textAnchor="end">{String(dataset.rows.at(-1)![node.xField]).slice(0, 10)}</text>}
      </svg>
      <div id={tipId} className="chart-tooltip" role="status">{activeRow ? <><strong>{cell(activeRow[node.yField])} {node.units}</strong><span>{cell(activeRow[node.xField])}</span></> : <span>Hover or focus a point for its exact value. All values are available below.</span>}</div>
    </>}
    <details className="chart-data"><summary>View data table · {dataset.rows.length} rows</summary><DataTable dataset={dataset} columns={[{ field: node.xField, label: node.kind === 'line' ? 'Time (UTC)' : 'Category' }, { field: node.yField, label: `${node.label}${node.units ? ` (${node.units})` : ''}` }]} label={`${node.label} data`} /></details>
  </section>;
}

function RenderNode({ node, spec }: { node: InterfaceNode; spec: InterfaceSpec }) {
  if (node.type === 'stack' || node.type === 'grid' || node.type === 'card') return <div className={`interface-${node.type}${node.type === 'grid' ? ` columns-${node.columns}` : ''}`}>
    {node.type === 'card' && <h3>{node.title}</h3>}{node.children.map(child => <RenderNode key={child.id} node={child} spec={spec} />)}
    {!node.children.length && <p className="field-note">Empty composition. Revise in a conversation to add content.</p>}
  </div>;
  if (node.type === 'text') return <p className="interface-text">{node.text}</p>;
  const dataset = spec.datasets.find(data => data.id === node.datasetId)!;
  if (node.type === 'table') return <DataTable dataset={dataset} columns={node.columns} label="Interface data" />;
  if (node.type === 'chart') return <Chart node={node} dataset={dataset} />;
  const value = dataset.rows[0]?.[node.field];
  let formatted = cell(value);
  if (typeof value === 'number') {
    try { formatted = new Intl.NumberFormat(undefined, { style: node.format === 'number' ? 'decimal' : node.format, ...(node.currency ? { currency: node.currency } : {}), maximumFractionDigits: 2 }).format(value); } catch { formatted = cell(value); }
  }
  return <section className="interface-metric"><h3>{node.label}</h3><strong>{formatted}</strong><span className="field-note">From conversation · not live</span></section>;
}

export function InterfaceRenderer({ spec }: { spec: unknown }) {
  const validated = useMemo(() => validateInterfaceSpec(spec), [spec]);
  if (!validated.ok) return <InlineError>{validated.error} The saved artifact has not been changed.</InlineError>;
  return <div className="trusted-interface"><div className="interface-source" role="note"><strong>From conversation · not live</strong><span>Literal model/operator data. No account retrieval or freshness is asserted. Trusted components v1; custom code isolation is not available.</span></div><RenderNode node={validated.spec.root} spec={validated.spec} /></div>;
}
