import { ScanText } from 'lucide-react';
import type { Route } from '../app/routes';
import { Page } from '../components/shell/AppShell';
import { Button } from '../components/ui';

/**
 * Lens is an approved destination, but this version has no camera text
 * recognition. Per the specification ("implement only capabilities that
 * exist"), the page says so plainly and never opens the camera.
 */
export function LensPage({ go }: { go: (r: Route) => void }) {
  return (
    <Page title="Lens" description="Point your camera at a sign or a menu and read it in your language.">
      <div className="mt-8 border-t border-line pt-8">
        <ScanText className="h-8 w-8 text-fg-subtle" aria-hidden="true" />
        <h2 className="mt-4 text-heading font-semibold">Not available in this version</h2>
        <p className="mt-2 max-w-[32rem] text-body text-fg-muted">
          Talktual can’t read text from the camera yet, so Lens doesn’t open your camera or capture anything. For speech, use AIR to translate the room, or Connect to talk with someone.
        </p>
        <div className="mt-6 flex flex-wrap gap-3">
          <Button variant="secondary" onClick={() => go('air')}>
            Open AIR
          </Button>
          <Button variant="ghost" onClick={() => go('connect')}>
            Open Connect
          </Button>
        </div>
      </div>
    </Page>
  );
}
