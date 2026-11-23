import { History } from 'lucide-react';
import type { Route } from '../app/routes';
import { Page } from '../components/shell/AppShell';
import { Button } from '../components/ui';

/**
 * Rewind shows retained sessions. Talktual retains none (Phase 6/8 privacy
 * policy: text lives on screen only until you clear it or leave), so there is
 * nothing to search, and the page says exactly that instead of an empty
 * search box or invented history.
 */
export function RewindPage({ go }: { go: (r: Route) => void }) {
  return (
    <Page title="Rewind" description="Past conversations you chose to keep.">
      <div className="mt-8 border-t border-line pt-8">
        <History className="h-8 w-8 text-fg-subtle" aria-hidden="true" />
        <h2 className="mt-4 text-heading font-semibold">Nothing is kept</h2>
        <p className="mt-2 max-w-[32rem] text-body text-fg-muted">
          Talktual doesn’t save conversations. Text from AIR and calls stays on screen only until you clear it or leave, and no audio is ever recorded. Saving conversations would need your explicit choice first, and this version doesn’t offer it.
        </p>
        <dl className="mt-6 max-w-[32rem] divide-y divide-line border-y border-line text-body">
          {[
            ['Audio', 'Never recorded'],
            ['Text on screen', 'Until you clear it or leave'],
            ['Saved conversations', 'None'],
          ].map(([k, v]) => (
            <div key={k} className="flex min-h-12 items-center justify-between gap-4 py-2.5">
              <dt>{k}</dt>
              <dd className="text-fg-muted">{v}</dd>
            </div>
          ))}
        </dl>
        <Button variant="ghost" className="-ml-3 mt-6" onClick={() => go('settings')}>
          Privacy settings
        </Button>
      </div>
    </Page>
  );
}
