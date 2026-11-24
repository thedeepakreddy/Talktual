import { History } from '../components/icons';
import type { Route } from '../app/routes';
import { Page } from '../components/shell/AppShell';
import { Button, Glyph } from '../components/ui';

/**
 * Rewind shows retained sessions. Talktual retains none (Phase 6/8 privacy
 * policy: text lives on screen only until you clear it or leave), so there is
 * nothing to search, and the page says exactly that instead of an empty
 * search box or invented history.
 */
export function RewindPage({ go }: { go: (r: Route) => void }) {
  return (
    <Page eyebrow="Your history" title="Rewind" description="Past conversations you chose to keep.">
      <div className="glass-strong rounded-[32px] p-6 sm:p-8">
        <Glyph icon={History} size="lg" />
        <h2 className="t-title mt-6 text-[1.5rem] leading-[1.9rem]">Nothing is kept</h2>
        <p className="mt-2.5 max-w-[32rem] text-body-lg text-fg-muted">
          Talktual doesn’t save conversations. Text from AIR and calls stays on screen only until you clear it or leave, and no audio is ever recorded. Saving conversations would need your explicit choice first, and this version doesn’t offer it.
        </p>
        <dl className="glass-clear mt-7 max-w-[34rem] rounded-[24px] px-4 text-body [&>*+*]:border-t [&>*+*]:border-[var(--glass-stroke)]">
          {[
            ['Audio', 'Never recorded'],
            ['Text on screen', 'Until you clear it or leave'],
            ['Saved conversations', 'None'],
          ].map(([k, v]) => (
            <div key={k} className="flex min-h-[52px] items-center justify-between gap-4 py-2.5">
              <dt className="font-[540]">{k}</dt>
              <dd className="text-fg-muted">{v}</dd>
            </div>
          ))}
        </dl>
        <Button variant="tertiary" className="mt-7" onClick={() => go('settings')}>
          Privacy settings
        </Button>
      </div>
    </Page>
  );
}
