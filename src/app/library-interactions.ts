import { isInterfaceTimestamp } from './interface-contracts.ts';

export function isEditableTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null;
  return !!element && (['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName) || element.isContentEditable || !!element.closest?.('[contenteditable]:not([contenteditable="false"]), [role="textbox"]'));
}

/** Global navigation must never take a keystroke from a field or modal. */
export function globalShortcutBlocked(event: KeyboardEvent, scope: Pick<Document, 'activeElement' | 'querySelector'> = document, modalOpen = false): boolean {
  return event.defaultPrevented || event.isComposing || modalOpen || !!scope.querySelector('dialog[open], [role="dialog"][aria-modal="true"]')
    || [event.target, scope.activeElement, ...(event.composedPath?.() ?? [])].some(isEditableTarget);
}

/** One request epoch per view. Mutations fence previously issued list/detail reads. */
export class LibraryRequestFence {
  private sequence = 0;
  issue(): number { return ++this.sequence; }
  snapshot(): number { return this.sequence; }
  invalidate(): void { ++this.sequence; }
  current(sequence: number): boolean { return sequence === this.sequence; }
}
export function nextLibraryRow(key: string, current: number, count: number, editing: boolean, modified = false): number | null {
  if (editing || modified || count < 1 || !['j', 'k', 'ArrowDown', 'ArrowUp'].includes(key)) return null;
  return Math.max(0, Math.min(count - 1, current + (key === 'j' || key === 'ArrowDown' ? 1 : -1)));
}
export function validateScheduleBounds(cron: string, endAt: string, maxRuns: number, now = Date.now()): string | null {
  const fields = cron.trim().split(/\s+/); const ranges = [[0, 59], [0, 23], [1, 31], [1, 12], [0, 7]];
  const invalid = () => 'Use five numeric UTC cron fields: minute hour day-of-month month day-of-week. Only *, lists, ranges and positive steps within each field range are supported.';
  if (cron.length > 100 || fields.length !== 5) return invalid();
  for (let i = 0; i < fields.length; i++) {
    for (const item of fields[i].split(',')) {
      const match = /^(\*|\d+(?:-\d+)?)(?:\/(\d+))?$/.exec(item); if (!match) return invalid();
      const [min, max] = ranges[i]; const step = match[2] ? Number(match[2]) : 1;
      if (!Number.isSafeInteger(step) || step <= 0 || step > max - min + 1) return invalid();
      if (match[1] === '*') continue;
      const ends = match[1].split('-').map(Number);
      if (ends.some(value => !Number.isSafeInteger(value) || value < min || value > max) || (ends.length > 1 && ends[0] > ends[1])) return invalid();
    }
  }
  if (!Number.isInteger(maxRuns) || maxRuns < 1 || maxRuns > 100) return 'Maximum attempts must be an integer from 1 to 100.';
  if (!isInterfaceTimestamp(endAt) || Date.parse(endAt) <= now) return 'Enter a future absolute UTC deadline, such as 2026-09-17T18:00:00Z.';
  return null;
}
