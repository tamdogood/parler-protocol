import { SessionsFeature } from "@/components/sessions-feature";
import { SessionViewer } from "@/components/session-viewer";

/**
 * The "session hub" — the Hub page's Sessions tab. Combines the sessions/handoff explainer with the
 * read-only watch viewer in one place, so understanding the feature and using it live aren't two
 * separate destinations anymore.
 */
export function SessionHub() {
  return (
    <>
      <SessionsFeature showViewerCta={false} />
      <div className="border-t border-graphite-rail">
        <SessionViewer />
      </div>
    </>
  );
}
