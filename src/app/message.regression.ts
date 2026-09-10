import { strict as assert } from 'node:assert';
import { readFile } from 'node:fs/promises';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { transformWithOxc } from 'vite';
import { completedComponentMessage, validateComponentMessage } from './message-contracts.ts';
import type { ChatMessage } from './contracts.ts';

interface Fixture { name: string; valid: boolean; input?: unknown; href?: string; repeat?: { path: string; unit: unknown; count: number }; padTo?: number }
function fixtureRaw(fixture: Fixture): string {
  const input = fixture.href === undefined ? structuredClone(fixture.input) : {
    kind: 'components', blocks: [{ type: 'card', title: 't', body: 'b', actions: [{ label: 'link', href: fixture.href }] }],
  };
  if (fixture.repeat) {
    const { path, unit, count } = fixture.repeat;
    const parts = path.slice(1).split('/');
    let target = input as Record<string, unknown>;
    for (const key of parts.slice(0, -1)) target = target[key] as Record<string, unknown>;
    target[parts[parts.length - 1]] = typeof unit === 'string' ? unit.repeat(count) : Array.from({ length: count }, () => structuredClone(unit));
  }
  const raw = typeof input === 'string' ? input : JSON.stringify(input);
  return fixture.padTo ? raw + ' '.repeat(Math.max(0, fixture.padTo - new TextEncoder().encode(raw).length)) : raw;
}
const message = (content: string): ChatMessage => ({ id: 'synthetic-message', role: 'assistant', status: 'complete', content, createdAt: '2026-09-10T00:00:00Z', requestId: null });

// Node's type stripping does not compile TSX. Use Vite's existing transform
// in memory on first-party source only (no model-code evaluation/build/server).
async function renderer() {
  const source = await readFile(new URL('./message.tsx', import.meta.url), 'utf8');
  const output = (await transformWithOxc(source, 'message.tsx', { jsx: { runtime: 'automatic' } })).code
    .replace(/import ['"]\.\/message\.css['"];?\n/g, '')
    .replace(/from (['"])(.*?)\1/g, (_match, _quote, specifier: string) => {
      const resolved = specifier.startsWith('.') ? new URL(`${specifier}.ts`, import.meta.url).href : import.meta.resolve(specifier);
      return `from ${JSON.stringify(resolved)}`;
    });
  return import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`) as Promise<typeof import('./message.tsx')>;
}

export async function runMessageRegressionChecks() {
  const fixtures: Fixture[] = JSON.parse(await readFile(new URL('./message-conformance.json', import.meta.url), 'utf8'));
  for (const fixture of fixtures) {
    const raw = fixtureRaw(fixture);
    const result = validateComponentMessage(raw);
    assert.equal(result !== null, fixture.valid, fixture.name);
    if (result) assert.deepEqual(validateComponentMessage(JSON.stringify(result)), result, `${fixture.name}: normalized roundtrip`);
  }
  const valid = fixtureRaw(fixtures[0]);
  assert.ok(completedComponentMessage(message(valid)));
  for (const state of [{ role: 'user', status: 'complete' }, { role: 'assistant', status: 'pending' }, { role: 'assistant', status: 'cancelled' }, { role: 'assistant', status: 'error' }]) {
    assert.equal(completedComponentMessage({ ...message(valid), ...state }), null);
  }
  assert.equal(validateComponentMessage(`{"kind":"components","blocks":[{"type":"markdown","text":"${String.fromCharCode(0xd800)}"}]}`), null);
  const original = message(valid);
  completedComponentMessage(original);
  assert.equal(original.content, valid, 'presentation must not mutate stored content');

  const { AssistantMessageContent, RawMessage, MessageRenderBoundary, InertMarkdown } = await renderer();
  const render = (content: string, role: ChatMessage['role'] = 'assistant', status: ChatMessage['status'] = 'complete') => renderToStaticMarkup(createElement(AssistantMessageContent, { message: { ...message(content), role, status } }));
  const html = render(valid);
  for (const semantic of ['<section', '<ul', '<li', '<dl', '<dt', '<dd', '<aside', '<strong', '<em', '<code', '<pre']) assert.ok(html.includes(semantic), `missing ${semantic}`);
  for (const forbidden of ['<a ', '<img', '<script', '<iframe', ' href=', ' src=', 'tabindex=', 'role="alert"', 'role="status"', 'aria-live=']) assert.ok(!html.includes(forbidden), `active/authority DOM: ${forbidden}`);
  for (const label of ['Links are not available in chat yet.', 'Assistant reports success', 'Caution', 'Note', 'Link']) assert.ok(html.includes(label), label);
  assert.ok(html.includes('&lt;b&gt;Literal body&lt;/b&gt;'));
  assert.ok(html.includes('Plain **detail**'), 'details must not become markdown');
  for (const fixture of fixtures.filter(fixture => !fixture.valid)) {
    const raw = fixtureRaw(fixture);
    assert.equal(render(raw), renderToStaticMarkup(createElement(RawMessage, { raw })), `${fixture.name}: original escaped fallback`);
  }
  for (const [role, status] of [['user', 'complete'], ['assistant', 'pending'], ['assistant', 'cancelled'], ['assistant', 'error']] as const) {
    assert.equal(render(valid, role, status), renderToStaticMarkup(createElement(RawMessage, { raw: valid })), `${role}/${status} must stay literal`);
  }
  assert.ok(render('{"kind":"components","blocks":[]}').includes('This reply has no displayable content.'));
  assert.ok(render('{"kind":"components","blocks":[{"type":"markdown","text":" "},{"type":"card","title":"","body":""}]}').includes('This reply has no displayable content.'));
  const empty = render(fixtureRaw(fixtures[2]));
  assert.ok(empty.includes('No items in this block.'));
  assert.ok(empty.includes('No details in this block.'));
  const inert = renderToStaticMarkup(createElement(InertMarkdown, { text: '- first\n- second\n\n1. one\n2. two\n\n![image](https://example.com/x) <img src=x>\n```\n<script>no()</script>\n```' }));
  assert.ok(inert.includes('<ul>') && inert.includes('<ol>') && !inert.includes('<img') && !inert.includes('<script'));
  // This exercises the boundary's recovery contract, not a mounted runtime throw.
  const boundary = new MessageRenderBoundary({ raw: '<broken>\n', children: createElement('span', null, 'blocks') });
  boundary.state = { ...boundary.state, ...MessageRenderBoundary.getDerivedStateFromError() };
  assert.equal(renderToStaticMarkup(boundary.render()), renderToStaticMarkup(createElement(RawMessage, { raw: '<broken>\n' })));
  assert.deepEqual(MessageRenderBoundary.getDerivedStateFromProps({ raw: 'new' }, boundary.state), { failed: false, raw: 'new' });

  return [
    `${fixtures.length} shared component conformance fixtures pass`,
    'completed assistant only; scalar limits and source preservation pass',
    'all five renderer types, semantic labels and unavailable links pass static DOM checks',
    'invalid replies fall back unchanged as escaped text; user/pending unaffected',
    'empty states and inert markdown lists/code pass static DOM checks',
    'per-message boundary fallback/reset contract passes (mounted runtime QA separate)',
  ];
}
