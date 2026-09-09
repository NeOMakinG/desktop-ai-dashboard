# Forma visual prototype

Forma is a provisional name for a throwaway, macOS-inspired exploration of the personal agent workspace. This POC answers a design question: does describing, refining, and keeping a personal workspace feel useful and delightful?

The prototype is **first-party UI with synthetic fixtures and scripted responses**. It does not connect to Hermes, call a model, read accounts, or execute model-authored code. Genuine custom UI generation remains the product direction; this POC explores its user experience, not its security implementation.

## Run in a browser

```sh
pnpm install
pnpm poc
```

The frontend-only Vite server binds to `127.0.0.1:1420`. It does not start an application backend or expose the prototype to the network.

Three structurally different layouts share the same demo state:

- `http://127.0.0.1:1420/prototype?variant=studio`: conversation beside a composed daily workspace.
- `http://127.0.0.1:1420/prototype?variant=focus`: spacious, conversation-led exploration.
- `http://127.0.0.1:1420/prototype?variant=canvas`: expansive workspace with optional conversation.

The floating design switcher appears in development only. Use the command palette's layout actions in the native bundled prototype.

## Run as a native desktop window

With Rust and the platform's Tauri prerequisites installed:

```sh
pnpm desktop
```

This command starts its own frontend dev server. Stop an existing `pnpm poc` or `pnpm dev` process first so port 1420 is free.

For a standalone local macOS application:

```sh
pnpm desktop:build
open 'src-tauri/target/debug/bundle/macos/Forma Prototype.app'
```

This is an unsigned debug POC, not a notarized release or customer distribution artifact. It uses Tauri's OS webview, not Electron. No signing credentials, updater, filesystem/shell plugins, provider credentials, or account permissions are configured. Windows runtime support is not implied by a macOS build.

## Explore the interaction

1. Start in Studio and use a suggested prompt, or enter your own. Unsupported prompts explicitly fall back to a scripted layout.
2. Try a quieter layout or a needs-reply filter. Cancel during a preview to retain the last completed state.
3. Save a version to memory, refine it, then restore the earlier version from history.
4. Open a message or meeting to inspect sample details and keep a local preparation note.
5. Open the command palette with Command-K or Control-K. Explore layout changes, density, saved spaces, reset, and the simulated error state.

All saves, notes, prompts, and revisions live only in the current window's memory. Reloading or closing it resets them. There is no browser localStorage, account synchronization, or durable persistence.

## Verification scope

`pnpm typecheck` checks TypeScript. `pnpm build` checks and bundles the frontend. Runtime receipts are local under `.local/receipts/`; browser and native results must be reported separately. Visual prototype success does not close the separate sandbox, live-model, connector, retention, multi-device, or release-review gates.
