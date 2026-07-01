import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}
interface State {
  error: Error | null;
}

/**
 * Catches render/runtime errors anywhere in the tree so a single bad payload (e.g. a malformed
 * directory entry or session message from a hub) shows a recoverable fallback instead of a blank
 * white window. Logs to the console so the headless smoke run (which watches console errors) sees it.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("[renderer] uncaught error:", error, info.componentStack);
  }

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex h-screen w-screen flex-col items-center justify-center gap-4 bg-black px-8 text-center">
        <div className="drag absolute inset-x-0 top-0 h-11" />
        <h1 className="text-[18px] font-semibold text-frost">Something went wrong</h1>
        <p className="max-w-md text-[13px] leading-relaxed text-fog">
          The app hit an unexpected error and paused this view. Your local hub and its data are
          untouched — reloading usually clears it.
        </p>
        <pre className="max-w-lg overflow-x-auto rounded-[10px] border border-graphite-rail bg-void-black px-3 py-2 text-left font-mono text-[11.5px] text-steel">
          {error.message}
        </pre>
        <button
          onClick={() => {
            this.setState({ error: null });
            window.location.reload();
          }}
          className="no-drag rounded-[8px] border border-electric-blue/40 bg-electric-blue/10 px-4 py-2 text-[13px] font-medium text-frost transition-colors hover:bg-electric-blue/20"
        >
          Reload
        </button>
      </div>
    );
  }
}
