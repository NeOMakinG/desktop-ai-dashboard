import { Component, Fragment, useMemo, type ReactNode } from 'react';
import { CalendarBlank, EnvelopeSimple, Globe, Sparkle } from '@phosphor-icons/react';
import { completedComponentMessage, type MessageBlock } from './message-contracts';
import type { ChatMessage } from './contracts';
import './message.css';

const icons = { envelope: EnvelopeSimple, calendar: CalendarBlank, globe: Globe, sparkle: Sparkle };
const tones = { info: 'Note', warn: 'Caution', success: 'Assistant reports success' };
const present = (text: string) => text.trim().length > 0;

export function RawMessage({ raw }: { raw: string }) {
  return <div className="message-text message-raw">{raw}</div>;
}

export class MessageRenderBoundary extends Component<{ raw: string; children: ReactNode }, { failed: boolean; raw: string }> {
  state = { failed: false, raw: this.props.raw };
  static getDerivedStateFromError() { return { failed: true }; }
  static getDerivedStateFromProps(props: { raw: string }, state: { raw: string }) {
    return props.raw === state.raw ? null : { failed: false, raw: props.raw };
  }
  render() { return this.state.failed ? <RawMessage raw={this.props.raw} /> : this.props.children; }
}

// Single nonrecursive pass. Unsupported syntax stays escaped text; no link,
// image, HTML, highlighting, executable code or generated React props path.
function inline(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(`[^`\n]+`|\*\*[^*\n]+\*\*|\*[^*\n]+\*)/g;
  let offset = 0;
  for (const match of text.matchAll(pattern)) {
    nodes.push(text.slice(offset, match.index));
    const token = match[0];
    nodes.push(token.startsWith('`') ? <code key={match.index}>{token.slice(1, -1)}</code>
      : token.startsWith('**') ? <strong key={match.index}>{token.slice(2, -2)}</strong>
        : <em key={match.index}>{token.slice(1, -1)}</em>);
    offset = match.index! + token.length;
  }
  nodes.push(text.slice(offset));
  return nodes;
}

export function InertMarkdown({ text }: { text: string }) {
  const lines = text.split('\n');
  const nodes: ReactNode[] = [];
  let index = 0;
  while (index < lines.length) {
    const start = index;
    if (/^```[^`]*$/.test(lines[index])) {
      const end = lines.findIndex((line, position) => position > start && line === '```');
      if (end !== -1) {
        nodes.push(<pre key={start} dir="auto"><code>{lines.slice(start + 1, end).join('\n')}</code></pre>);
        index = end + 1; continue;
      }
    }
    const list = /^(?:([-*]) |([0-9]+)\. )(.+)$/.exec(lines[index]);
    if (list) {
      const ordered = !!list[2];
      const items: ReactNode[] = [];
      while (index < lines.length) {
        const item = /^(?:([-*]) |([0-9]+)\. )(.+)$/.exec(lines[index]);
        if (!item || !!item[2] !== ordered) break;
        items.push(<li key={index} dir="auto">{inline(item[3])}</li>); index++;
      }
      nodes.push(ordered ? <ol key={start}>{items}</ol> : <ul key={start}>{items}</ul>); continue;
    }
    if (!lines[index].trim()) { index++; continue; }
    const paragraph: string[] = [lines[index++]];
    while (index < lines.length && lines[index].trim() && !/^```|^(?:[-*] |[0-9]+\. )/.test(lines[index])) paragraph.push(lines[index++]);
    nodes.push(<p key={start} dir="auto">{inline(paragraph.join('\n'))}</p>);
  }
  return <div className="message-markdown">{nodes}</div>;
}

function EmptyBlock({ children }: { children: ReactNode }) {
  return <p className="message-block-empty">{children}</p>;
}

function BlockView({ block }: { block: MessageBlock }) {
  switch (block.type) {
    case 'markdown': return present(block.text) ? <InertMarkdown text={block.text} /> : null;
    case 'card':
      if (!present(block.title) && !present(block.body) && !block.actions.length) return null;
      return <section className="message-card" aria-label={present(block.title) ? block.title : undefined}>
        {present(block.title) && <p className="message-block-title" dir="auto">{block.title}</p>}
        {present(block.body) && <p className="message-block-prose" dir="auto">{block.body}</p>}
        {block.actions.length > 0 && <div className="message-actions">
          {block.actions.map((action, index) => <div className="message-action" key={index}>
            <span dir="auto">{present(action.label) ? action.label : 'Link'}</span><span className="message-action-url" dir="auto">{action.href}</span>
          </div>)}
          <p className="message-block-empty">Links are not available in chat yet.</p>
        </div>}
      </section>;
    case 'list': {
      const items = block.items.filter(item => present(item.title) || present(item.detail));
      return items.length ? <ul className="message-list">{items.map((item, index) => {
        const Icon = icons[item.icon];
        return <li key={index}><Icon size={16} aria-hidden="true" /><div>
          {present(item.title) && <p className="message-block-title" dir="auto">{item.title}</p>}
          {present(item.detail) && <p className="message-block-detail" dir="auto">{item.detail}</p>}
        </div></li>;
      })}</ul> : <EmptyBlock>No items in this block.</EmptyBlock>;
    }
    case 'kv': {
      const rows = block.rows.filter(row => present(row.label) || present(row.value));
      return rows.length ? <dl className="message-kv">{rows.map((row, index) => <div key={index}>
        <dt dir="auto">{row.label}</dt><dd dir="auto">{row.value}</dd>
      </div>)}</dl> : <EmptyBlock>No details in this block.</EmptyBlock>;
    }
    case 'callout': return <aside className={`message-callout message-callout-${block.tone}`}>
      <p className="message-callout-label">{tones[block.tone]}</p>
      {present(block.text) && <p className="message-block-prose" dir="auto">{block.text}</p>}
    </aside>;
  }
}

function displayable(block: MessageBlock): boolean {
  if (block.type === 'markdown') return present(block.text);
  if (block.type === 'card') return present(block.title) || present(block.body) || block.actions.length > 0;
  return true; // Empty list/kv and every callout carry host-owned explanatory text.
}

function RenderedMessage({ message }: { message: ChatMessage }) {
  const components = useMemo(() => completedComponentMessage(message), [message.role, message.status, message.content]);
  if (!components) return <RawMessage raw={message.content} />;
  const blocks = components.blocks.filter(displayable);
  return <div className="message-blocks">{blocks.length
    ? blocks.map((block, index) => <Fragment key={index}><BlockView block={block} /></Fragment>)
    : <EmptyBlock>This reply has no displayable content.</EmptyBlock>}
  </div>;
}

export function AssistantMessageContent({ message }: { message: ChatMessage }) {
  return <MessageRenderBoundary raw={message.content}><RenderedMessage message={message} /></MessageRenderBoundary>;
}
