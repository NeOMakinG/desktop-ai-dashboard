# Forma design contract

This file is binding for every new surface, human- or agent-built. A feature that violates it fails review, regardless of who wrote it. Read it before writing any UI code; reuse existing components before inventing new ones.

## Tokens are the only source of style

All colors come from `src/app/tokens.css` variables. No hex literals in component CSS. The app is dark charcoal with one warm accent used sparingly (primary action, selection, focus) — never as decoration.

## Spacing

Use only the scale: 2, 4, 8, 12, 16, 24, 32, 48, 64px. Nothing between, nothing outside.

- Stacked cards or list rows in a column: 8–12px gap. **Never zero.** If two rounded boxes touch, that is a bug.
- Sidebar items: 40px min-height, 8px padding, 8px radius, 8px gap between groups.
- Section padding: 24–32px. Page gutters: 32px desktop, 24px narrow.

## Borders and layering

**One bordered layer per control.** An input inside a bordered container must be borderless and transparent (`border:0; background:transparent`) — a field inside a field is always wrong. Row separators are either gaps between separate rounded rows or single hairlines, never both.

Global `app.css` styles every raw `input` with its own border and background. When embedding an input inside a composed control, override with a **two-class selector** (e.g. `.browser-page .browser-address input`), because `app.css` loads last and wins equal-specificity ties.

## Radii

Controls and inputs: 8px. List rows: 8px standalone, up to 13px inside spacious pages. Cards/modals: 16–20px. Pills only for status chips. Pick from these; do not invent new radii.

## Reuse before creating

Existing primitives — `Modal`, `IconButton`, `InlineError`, `SafeImage`, `.button primary/secondary/danger`, `.account-row`, `.text-button`, `.field-note` — cover most needs. New CSS files must wire into the same wrappers (e.g. give list wrappers `display:flex; flex-direction:column; gap`) instead of restyling rows from scratch.

## States and honesty

Every control works or is visibly disabled with a reason. Loading uses skeletons/status text, not spinners over the whole page. No fake "Connected" states; availability copy must match actual capability. Reduced motion collapses animation. Focus rings are never suppressed.

## Enforcement

1. Every UI task's acceptance criteria must cite this file; reviewers check spacing/border/radius violations as **blocking** findings.
2. Before delivery, capture 1440×950 and 1100×800 screenshots and check: no touching rounded boxes, no double-bordered fields, no off-scale spacing, no accent overuse.
3. Remote workers receive this file in their source context (it lives in `docs/`, which the company snapshot includes) and their prompts must forbid new hex colors and off-scale spacing.
4. When a violation ships, fix the shared stylesheet or primitive so the class of bug dies — not just the single instance.
