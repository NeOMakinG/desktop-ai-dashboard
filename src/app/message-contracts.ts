// Trusted chat blocks, not generated code or a Hermes tool/runtime contract.
export const MAX_COMPONENT_BYTES = 100_000;
export const MAX_COMPONENT_STRING_BYTES = 4_000;
export const MAX_COMPONENT_ITEMS = 40;
export const MAX_COMPONENT_DEPTH = 8;

export interface MessageAction { label: string; href: string }
export interface MessageListItem { title: string; detail: string; icon: 'envelope' | 'calendar' | 'globe' | 'sparkle' }
export interface MessageKvRow { label: string; value: string }
export type MessageBlock =
  | { type: 'markdown'; text: string }
  | { type: 'card'; title: string; body: string; actions: MessageAction[] }
  | { type: 'list'; items: MessageListItem[] }
  | { type: 'kv'; rows: MessageKvRow[] }
  | { type: 'callout'; tone: 'info' | 'warn' | 'success'; text: string };
export interface ComponentMessage { kind: 'components'; blocks: MessageBlock[] }

const encoder = new TextEncoder();
function scalarString(value: string): boolean {
  for (const scalar of value) {
    const code = scalar.codePointAt(0)!;
    if (code >= 0xd800 && code <= 0xdfff) return false;
  }
  return true;
}
function text(value: unknown): value is string {
  return typeof value === 'string' && value.length <= MAX_COMPONENT_STRING_BYTES
    && !value.includes('\0') && scalarString(value) && encoder.encode(value).length <= MAX_COMPONENT_STRING_BYTES;
}
function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
function keys(value: Record<string, unknown>, required: string[], optional: string[] = []): boolean {
  return required.every(key => Object.hasOwn(value, key))
    && Object.keys(value).every(key => required.includes(key) || optional.includes(key));
}
function array(value: unknown): value is unknown[] {
  return Array.isArray(value) && value.length <= MAX_COMPONENT_ITEMS;
}

// Conservative, shared URL subset. Validation is not navigation authorization.
export function validMessageHref(href: string): boolean {
  if (!text(href) || !href.startsWith('https://')) return false;
  for (const scalar of href) {
    const code = scalar.codePointAt(0)!;
    if (code <= 0x20 || (code >= 0x7f && code <= 0xa0) || code === 0x1680
      || (code >= 0x2000 && code <= 0x200f) || (code >= 0x2028 && code <= 0x202f)
      || (code >= 0x205f && code <= 0x206f) || code === 0x3000 || code === 0xfeff || scalar === '\\') return false;
  }
  for (let i = 0; i < href.length; i++) {
    if (href[i] === '%') {
      if (!/^[0-9a-fA-F]{2}$/.test(href.slice(i + 1, i + 3))) return false;
      const byte = parseInt(href.slice(i + 1, i + 3), 16);
      if (byte <= 0x20 || byte === 0x7f || byte === 0x5c) return false;
      i += 2;
    }
  }
  const authority = href.slice(8).split(/[/?#]/, 1)[0];
  if (!authority || /[^a-zA-Z0-9.:-]/.test(authority)) return false;
  const parts = authority.split(':');
  if (parts.length > 2) return false;
  const [host, port] = parts;
  if (port !== undefined && (!/^[0-9]{1,5}$/.test(port) || Number(port) < 1 || Number(port) > 65535)) return false;
  if (host.length > 253) return false;
  const labels = host.split('.');
  if (labels.some(label => !/^[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(label))) return false;
  const last = labels[labels.length - 1];
  // Avoid WHATWG's noncanonical numeric/hex/octal IPv4 interpretations.
  if (/^[0-9]+$/.test(last) || /^0x[0-9a-f]+$/i.test(last)) {
    if (labels.length !== 4 || labels.some(label => !/^(0|[1-9][0-9]{0,2})$/.test(label) || Number(label) > 255)) return false;
  }
  try {
    const url = new URL(href);
    return url.protocol === 'https:' && url.hostname.length > 0 && !url.username && !url.password;
  } catch { return false; }
}

// Bound parser work before JSON.parse; inspect overwritten duplicate strings too.
// No field accepts JSON numbers. Reject every numeric token, even overwritten
// ones, rather than inherit differing JS/serde_json float-overflow rounding.
function boundedJson(raw: string): unknown {
  if (raw.length > MAX_COMPONENT_BYTES || !scalarString(raw) || encoder.encode(raw).length > MAX_COMPONENT_BYTES) throw new Error('size/scalars');
  let depth = 0;
  for (let i = 0; i < raw.length; i++) {
    const char = raw[i];
    if (char === '"') {
      const start = i++;
      for (; i < raw.length && raw[i] !== '"'; i++) { if (raw[i] === '\\') i++; }
      const decoded: unknown = JSON.parse(raw.slice(start, i + 1));
      if (typeof decoded !== 'string' || !scalarString(decoded)) throw new Error('scalar');
    } else {
      if (char >= '0' && char <= '9') throw new Error('number');
      if (char === '{' || char === '[') { if (++depth > MAX_COMPONENT_DEPTH) throw new Error('depth'); }
      if (char === '}' || char === ']') depth--;
    }
  }
  // No JS trim: only JSON whitespace is supported on both sides of the bridge.
  return JSON.parse(raw);
}

export function validateComponentMessage(raw: string): ComponentMessage | null {
  try {
    const root = boundedJson(raw);
    if (!object(root) || !keys(root, ['kind', 'blocks']) || root.kind !== 'components' || !array(root.blocks)) return null;
    const blocks: MessageBlock[] = [];
    for (const value of root.blocks) {
      if (!object(value)) return null;
      switch (value.type) {
        case 'markdown':
          if (!keys(value, ['type', 'text']) || !text(value.text)) return null;
          blocks.push({ type: 'markdown', text: value.text }); break;
        case 'card': {
          if (!keys(value, ['type', 'title', 'body'], ['actions']) || !text(value.title) || !text(value.body)) return null;
          const actions = Object.hasOwn(value, 'actions') ? value.actions : [];
          if (!array(actions)) return null;
          const result: MessageAction[] = [];
          for (const action of actions) {
            if (!object(action) || !keys(action, ['label', 'href']) || !text(action.label) || !text(action.href) || !validMessageHref(action.href)) return null;
            result.push({ label: action.label, href: action.href });
          }
          blocks.push({ type: 'card', title: value.title, body: value.body, actions: result }); break;
        }
        case 'list': {
          if (!keys(value, ['type', 'items']) || !array(value.items)) return null;
          const items: MessageListItem[] = [];
          for (const item of value.items) {
            if (!object(item) || !keys(item, ['title', 'detail', 'icon']) || !text(item.title) || !text(item.detail)
              || !['envelope', 'calendar', 'globe', 'sparkle'].includes(item.icon as string)) return null;
            items.push({ title: item.title, detail: item.detail, icon: item.icon as MessageListItem['icon'] });
          }
          blocks.push({ type: 'list', items }); break;
        }
        case 'kv': {
          if (!keys(value, ['type', 'rows']) || !array(value.rows)) return null;
          const rows: MessageKvRow[] = [];
          for (const row of value.rows) {
            if (!object(row) || !keys(row, ['label', 'value']) || !text(row.label) || !text(row.value)) return null;
            rows.push({ label: row.label, value: row.value });
          }
          blocks.push({ type: 'kv', rows }); break;
        }
        case 'callout':
          if (!keys(value, ['type', 'tone', 'text']) || !text(value.text) || !['info', 'warn', 'success'].includes(value.tone as string)) return null;
          blocks.push({ type: 'callout', tone: value.tone as 'info' | 'warn' | 'success', text: value.text }); break;
        default: return null;
      }
    }
    return { kind: 'components', blocks };
  } catch { return null; }
}

export function completedComponentMessage(message: { role: string; status: string; content: string }): ComponentMessage | null {
  return message.role === 'assistant' && message.status === 'complete' ? validateComponentMessage(message.content) : null;
}
