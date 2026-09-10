# Forma

A calm, open-source desktop app for conversations that remember. Each workspace is a chat: continue it later, keep useful context together, and remove it when you no longer need it.

**Early alpha.** Forma is a desktop interface for Hermes. The app bundles and manages its agent runtime automatically; there is no Direct chat mode, separate runtime server to configure, or runtime token to supply. Configure your model/provider credentials in Settings, then work through conversations, shared Interfaces, and bounded schedules. Live personal-account integrations remain gated.

## What works today

- Large chat canvas with useful starting prompts and a compact workspace history.
- Automatic draft and message persistence; no manual “save to memory.”
- Workspace creation, renaming, and confirmed deletion with cancellation and stale-write protection.
- Three-step onboarding and progressively disclosed AI settings.
- Native SQLite storage and endpoint-bound OS credential storage, with no plaintext key fallback.
- A searchable model picker in chat, with runtime discovery and remembered selections.
- Automatic initial model selection prefers the newest available Opus; explicit choices are preserved.
- App-owned Hermes startup, private process communication, explicit model/provider checks, and workspace-scoped model selection.
- Bounded trusted assistant blocks: inert markdown, cards, lists, key/value rows, and callouts, with raw-text fallback. Card links are noninteractive.
- Shared, versioned Interfaces with model-proposed layouts, metrics, tables, and charts; explicit publication, conflict checks, rollback, and confirmed deletion.
- Durable, bounded UTC schedules, initially paused and enabled only by an operator. Closing the window can keep approved work running in the tray; explicit Quit stops the owned runtime. Work cannot run while the app is stopped or the machine is asleep/powered off.
- Generated JavaScript/React execution is not enabled. Trusted compositions do not imply an unrestricted custom-code runtime.
- An isolated, persistent Forma-owned WebKit browser on macOS 14 or later, separate from the privileged app and normal browser profiles.
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

Stop an existing frontend dev server first: the native development command starts its own server on port 1420. Native dev/build hooks automatically prepare pinned Python, Hermes, and core dependencies as bundled resources. The first preparation needs network access and additional disk space; subsequent builds verify and reuse the cache. Users of a packaged app do not install Python, uv, or Hermes separately.

On macOS, build a standalone unsigned development app with:

```sh
pnpm desktop:build
open 'src-tauri/target/debug/bundle/macos/Forma.app'
```

This is a debug build, not a signed or notarized customer release. The currently prepared managed-runtime bundle targets Apple Silicon macOS. Other architectures, Windows native QA, and distribution packaging remain open gates; unsupported builds fail explicitly instead of falling back to Direct chat.

## AI and data

Choose your own compatible provider in Settings. Keys stay in the native credential store and are not returned to the renderer or included in chat history. Provider changes invalidate checks and pending requests. The app does not make model calls on launch or switch silently to another provider.

Hermes runs under the app's ownership. Sending a message transmits that workspace's relevant conversation to the model provider you selected. “Stored on your device” does not mean “no model-provider egress.” The bounded Forma toolset can propose shared Interfaces and paused schedules; models cannot grant account access, publish arbitrary code, or enable schedules themselves. Live mailbox/calendar reads, browser automation, and unrestricted filesystem tools are not enabled.

The owned browser currently uses WebKit. Managed system Chromium is intentionally unavailable until network blocking can be enforced before startup traffic; Chromium navigation and Google-login compatibility are not verified.

Google OAuth plumbing requires a build-time `FORMA_GOOGLE_CLIENT_ID`. Without a registered client, sign-in is visibly disabled. Real sign-in, the applicable Google policy/compliance gates, and connected-account runtime QA remain unverified. Local disconnection does not revoke access at Google. Neither browser sign-in nor OAuth plumbing gives the model Gmail, Calendar, or browser tools.

Apple account access, explicitly attached browsers, additional Hermes toolsets, genuinely authored custom-code interfaces, MCP, paired desktop hosts, and mobile clients remain in the [product vision](docs/product/vision.md). They must not be inferred from the bounded Forma toolset or trusted component rendering. See the [domain glossary](CONTEXT.md) for the current meaning of workspace, connection, and device.

## Design and media

The default interface is cozy charcoal with restrained warm accents, system typography, and reduced-motion support. The requested ambient media source is Higgsfield. No Higgsfield assets have been generated yet; the media manifest deliberately uses null URLs and the app renders built-in illustrated fallbacks without broken requests or false provenance.

The earlier visual prototype remains available at `/prototype` in development only. Its [guide](docs/poc.md) is historical; the default application now uses automatic persistent chats rather than session-only snapshots.

## Checks

```sh
pnpm typecheck
pnpm build
npm test
CARGO_BUILD_JOBS=2 cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Native unit tests stay offline; the model-provider utilities no longer implement a separate direct-chat path. Managed-runtime tests and protocol details are documented in [the runtime contract](docs/runtime/protocol.md). Source builds require prepared managed assets; Tauri dev/build hooks perform that preparation automatically. Browser, native-window, OS credential-store, actual Hermes/model execution, and other-platform results are separate evidence categories; one does not prove the others.

## License

[MIT](LICENSE). Third-party dependencies retain their own licenses. Future generated media must include its actual provenance and applicable usage terms.
