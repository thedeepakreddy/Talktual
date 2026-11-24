import type { Disclosure } from '../engine/privacy';
import { Button, Sheet } from './ui';

interface ConsentSheetProps {
  open: boolean;
  disclosure: Disclosure | null;
  /** Already granted: the sheet is informational (re-opened from settings). */
  granted: boolean;
  onAllow: () => void;
  onDecline: () => void;
  /** Label for the accept button (default "Allow microphone"). */
  allowLabel?: string;
}

/**
 * Processing disclosure shown before the microphone is used for the first time
 * on this device (and on demand). Nothing is captured until "Allow microphone".
 * The disclosure text comes from the engine and is shown verbatim.
 */
export function ConsentSheet({ open, disclosure, granted, onAllow, onDecline, allowLabel = 'Allow microphone' }: ConsentSheetProps) {
  return (
    <Sheet
      open={open && Boolean(disclosure)}
      onClose={onDecline}
      title={granted ? 'How your voice is processed' : disclosure?.title ?? ''}
      closeLabel="Close"
      footer={
        granted ? (
          <Button block onClick={onDecline}>
            Done
          </Button>
        ) : (
          <div className="grid grid-cols-2 gap-3">
            <Button onClick={onDecline}>Not now</Button>
            <Button variant="primary" onClick={onAllow} data-autofocus>
              {allowLabel}
            </Button>
          </div>
        )
      }
    >
      <ul className="space-y-3 pb-1">
        {disclosure?.points.map((p) => (
          <li key={p} className="flex gap-3 text-body text-fg-muted">
            <span className="orb-core mt-[7px] h-2 w-2 shrink-0 rounded-full" aria-hidden="true" />
            <span>{p}</span>
          </li>
        ))}
      </ul>
    </Sheet>
  );
}
