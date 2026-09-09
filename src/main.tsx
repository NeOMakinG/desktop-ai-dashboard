import { Component, lazy, StrictMode, Suspense, type ErrorInfo, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';

const PrototypeApp = lazy(() => import('./prototype/PrototypeApp').then(module => ({ default: module.PrototypeApp })));

class AppErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() { return { failed: true }; }

  componentDidCatch(_error: Error, _info: ErrorInfo) {
    // No chat content, credentials, or transport errors leave this device.
  }

  render() {
    if (this.state.failed) {
      return <main className="error-boundary">
        <h1>Let’s take a breath.</h1>
        <p>Forma hit a problem. Saved work has not been reset. Reopen the app to try again.</p>
        <button className="button secondary" type="button" onClick={() => window.location.reload()}>Reopen Forma</button>
      </main>;
    }
    return this.props.children;
  }
}

const showPrototype = import.meta.env.DEV && window.location.pathname === '/prototype';
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppErrorBoundary>
      {showPrototype ? <Suspense fallback={<main className="startup-screen">Opening comparison…</main>}><PrototypeApp /></Suspense> : <App />}
    </AppErrorBoundary>
  </StrictMode>,
);
