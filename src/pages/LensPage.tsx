import { ArrowRight, ScanText } from '../components/icons';
import type { Route } from '../app/routes';
import { Page } from '../components/shell/AppShell';
import { Button, Glyph } from '../components/ui';

/**
 * Lens is an approved destination, but this version has no camera text
 * recognition. Per the specification ("implement only capabilities that
 * exist"), the page says so plainly and never opens the camera.
 */
export function LensPage({ go }: { go: (r: Route) => void }) {
  return (
    <Page eyebrow="Camera translation" title="Lens" description="Point your camera at a sign or a menu and read it in your language.">
      <div className="glass-strong rounded-[32px] p-6 sm:p-8">
        <Glyph icon={ScanText} size="lg" />
        <h2 className="t-title mt-6 text-[1.5rem] leading-[1.9rem]">Not available in this version</h2>
        <p className="mt-2.5 max-w-[32rem] text-body-lg text-fg-muted">
          Talktual can’t read text from the camera yet, so Lens doesn’t open your camera or capture anything. For speech, use AIR to translate the room, or Connect to talk with someone.
        </p>
        <div className="mt-7 flex flex-wrap gap-3">
          <Button variant="primary" chip={<ArrowRight />} onClick={() => go('air')}>
            Open AIR
          </Button>
          <Button variant="tertiary" onClick={() => go('connect')}>
            Open Connect
          </Button>
        </div>
      </div>
    </Page>
  );
}
