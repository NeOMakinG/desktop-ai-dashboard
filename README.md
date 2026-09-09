# Forma

A calm, open-source desktop app for conversations that remember. Each workspace is a chat: continue it later, keep useful context together, and remove it when you no longer need it.

**Early alpha.** The current app has a charcoal, chat-first interface, short onboarding, automatic local persistence, workspace history, and a native OpenAI-compatible provider connection. It is not yet a complete personal-account agent.

## What works today

- Large chat canvas with useful starting prompts and a compact workspace history.
- Automatic draft and message persistence; no manual “save to memory.”
- Workspace creation, renaming, and confirmed deletion with cancellation and stale-write protection.
- Three-step onboarding and progressively disclosed AI settings.
- Native SQLite storage and endpoint-bound OS credential storage, with no plaintext key fallback.
- A searchable model picker in chat, with runtime discovery and remembered selections.
- Automatic initial model selection prefers the newest available Opus; explicit choices are preserved.
- Explicit provider checks and nonstreaming text replies through the native host.
- Browser evaluation with local chat persistence, but no native credentials or provider calls.
- Tauri desktop shell using the OS webview, not Electron.

Native state and transport logic have unit and synthetic HTTP integration tests. A mock credential store is used in the transport tests; they do not certify OS keyring behavior, every provider, or every platform.

## Run from source

Install Node.js 22.12+ and pnpm. Native development also requires Rust and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm dev
```

Open `http://127.0.0.1:1420/` for browser evaluation. Browser AI configuration is intentionally disabled; use the desktop app for provider connectivity.

```sh
pnpm desktop
```

Stop an existing frontend dev server first: the native development command starts its own server on port 1420.

On macOS, build a standalone unsigned development app with:

```sh
pnpm desktop:build
open 'src-tauri/target/debug/bundle/macos/Forma.app'
```

This is a debug build, not a signed or notarized customer release. Windows native QA and distribution packaging remain open work.

## AI and data

Choose your own compatible provider in Settings. Keys stay in the native credential store and are not returned to the renderer or included in chat history. Provider changes invalidate checks and pending requests. The app does not make model calls on launch or switch silently to another provider.

Sending a message transmits that workspace's relevant conversation to the provider you selected. “Stored on your device” does not mean “no model-provider egress.” There are no connected mailbox, calendar, browser, or filesystem tools in the current chat route.

Google/Apple account access, a persistent Forma-owned browser, explicitly attached browsers, Hermes tools, generated custom interfaces, MCP, paired desktop hosts, and mobile clients are in the [product vision](docs/product/vision.md). They must not be inferred from the alpha's provider connection. See the [domain glossary](CONTEXT.md) for the current meaning of workspace, connection, and device.

## Design and media

The default interface is cozy charcoal with restrained warm accents, system typography, and reduced-motion support. The requested ambient media source is Higgsfield. No Higgsfield assets have been generated yet; the media manifest deliberately uses null URLs and the app renders built-in illustrated fallbacks without broken requests or false provenance.

The earlier visual prototype remains available at `/prototype` in development only. Its [guide](docs/poc.md) is historical; the default application now uses automatic persistent chats rather than session-only snapshots.

## Checks

```sh
pnpm typecheck
pnpm build
node --experimental-transform-types --input-type=module -e "import('./src/app/store.regression.ts').then(async m => console.log(await m.runStoreRegressionChecks()))"
CARGO_BUILD_JOBS=2 cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

The external provider-fixture test is opt-in and requires a controlled synthetic endpoint via `FORMA_TEST_PROVIDER_URL`. Normal native tests stay offline. Browser, native-window, OS credential-store, real-provider, and Windows results are separate evidence categories; one does not prove the others.

## License

[MIT](LICENSE). Third-party dependencies retain their own licenses. Future generated media must include its actual provenance and applicable usage terms.
