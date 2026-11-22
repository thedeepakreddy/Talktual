// Phase 6: processing-mode control and per-session privacy explanation.
// Shows what actually runs where, from the engine's capability matrix — not
// from marketing copy.

import { motion, AnimatePresence } from 'motion/react';
import { X } from 'lucide-react';
import { cn } from '../utils';
import type { CommunicationSession, SessionSnapshot } from '../engine/CommunicationSession';
import type { ProcessingMode } from '../privacy/policy';

interface Props {
  open: boolean;
  onClose: () => void;
  session: CommunicationSession | null;
  snapshot: SessionSnapshot | null;
}

const MODES: Array<{ id: ProcessingMode; label: string; hint: string }> = [
  { id: 'cloud', label: 'Cloud-assisted', hint: 'Best quality. Speech and text go to cloud services (named below). On-device is used when they can’t be reached.' },
  { id: 'hybrid', label: 'Hybrid', hint: 'On this device where it can; cloud services only for what the device can’t do.' },
  { id: 'local-only', label: 'On-device only', hint: 'Nothing is sent to recognition, translation or voice services. Only what this device can do works; the rest is switched off.' },
];

export const TASK_LABEL: Record<string, string> = {
  'speech-input': 'Speech recognition',
  translation: 'Translation',
  captions: 'Live captions',
  'speech-output': 'Spoken replies',
  'typed-input': 'Typing',
  assistance: 'Conversation assistance',
};
const STATE_LABEL: Record<string, string> = { device: 'On this device', cloud: 'Cloud', 'device+cloud': 'Device, then cloud', unavailable: 'Unavailable' };

export function PrivacyPanel({ open, onClose, session, snapshot }: Props) {
  const p = snapshot?.processing;
  return (
    <AnimatePresence>
      {open && p?.available && (
        <>
          <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose} className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[2px]" />
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="Privacy and processing"
            initial={{ y: '100%' }}
            animate={{ y: 0 }}
            exit={{ y: '100%' }}
            transition={{ type: 'spring', damping: 32, stiffness: 340 }}
            className="fixed inset-x-0 bottom-0 z-50 mx-auto flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-t-[28px] border border-white/10 bg-[#141418]/95 backdrop-blur-2xl"
          >
            <div className="flex items-center justify-between px-5 pt-5 pb-2">
              <h2 className="text-[19px] font-semibold tracking-[-0.01em]">Privacy and processing</h2>
              <button onClick={onClose} aria-label="Close privacy" className="flex h-8 w-8 items-center justify-center rounded-full bg-white/8 text-[var(--text-2)] hover:bg-white/14">
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="sheet-scroll flex-1 overflow-y-auto px-5 pb-6">
              <fieldset className="space-y-2" aria-label="Processing mode">
                {MODES.map((m) => (
                  <label key={m.id} className={cn('flex cursor-pointer gap-3 rounded-xl border px-3.5 py-2.5', p.mode === m.id ? 'border-[var(--blue)]/60 bg-[var(--blue)]/10' : 'border-white/10 bg-white/[0.04]')}>
                    <input type="radio" name="processing-mode" className="mt-1 accent-[var(--blue)]" checked={p.mode === m.id} onChange={() => session?.setProcessingMode(m.id)} aria-label={m.label} />
                    <span>
                      <span className="block text-[15px] text-[var(--text-1)]">{m.label}</span>
                      <span className="block text-[12px] leading-snug text-[var(--text-3)]">{m.hint}</span>
                    </span>
                  </label>
                ))}
              </fieldset>

              {p.network !== 'online' && (
                <p role="status" className="mt-3 rounded-xl border border-[var(--amber)]/25 bg-[var(--amber)]/10 px-3 py-2 text-[13px] text-[#ffcf80]">
                  {p.network === 'offline' ? 'You’re offline: only what runs on this device is available.' : 'Cloud services aren’t reachable right now: using this device where possible.'}
                </p>
              )}

              <h3 className="mt-5 text-[15px] font-semibold">Right now</h3>
              <ul className="mt-2 space-y-1.5" aria-label="Where each function runs">
                {p.capabilities.map((c) => (
                  <li key={c.task} className="flex items-start justify-between gap-3 rounded-xl bg-white/[0.05] px-3 py-2">
                    <span className="text-[14px]">{TASK_LABEL[c.task] ?? c.task}</span>
                    <span className="text-right text-[12px] leading-snug">
                      <span className={cn('block font-medium', c.state === 'unavailable' ? 'text-[#ffcf80]' : c.state === 'cloud' ? 'text-[var(--text-2)]' : 'text-[var(--green,#7ee2a8)]')}>{STATE_LABEL[c.state]}</span>
                      <span className="block text-[var(--text-3)]">{c.detail}</span>
                    </span>
                  </li>
                ))}
              </ul>

              <h3 className="mt-5 text-[15px] font-semibold">What leaves this device</h3>
              <p className="mt-1 text-[13px] leading-snug text-[var(--text-3)]">
                {p.mode === 'local-only'
                  ? 'Your words go only to the other person (they’re asked to keep them on their device too). Talktual’s server is used only to connect the call. No recognition, translation or voice service receives anything.'
                  : 'Anything marked “Cloud” above is sent to that service while you use it. Your words also go to the other person. Talktual’s server connects the call. Nothing is stored after the call.'}
                {p.peerPrivate ? ' The other person is in On-device mode: their words are kept on this device too.' : ''}
              </p>

              <h3 className="mt-5 text-[15px] font-semibold">On-device engines</h3>
              <ul className="mt-2 space-y-1 text-[12px] text-[var(--text-3)]" aria-label="On-device engines">
                {p.runtimes.map((r) => (
                  <li key={r.id}>
                    <span className="text-[var(--text-2)]">{r.model.name}</span> — {r.readiness === 'ready' ? 'ready' : r.readiness}
                    {r.languages.length ? ` · ${r.languages.join(', ')}` : ''}
                    {r.constraints[0] ? ` · ${r.constraints[0]}` : ''}
                  </li>
                ))}
              </ul>

              <button
                onClick={() => session?.clearPrivateData()}
                className="mt-5 w-full rounded-xl border border-white/10 bg-white/[0.06] px-3 py-2.5 text-[14px] font-medium text-[var(--text-1)] hover:bg-white/[0.1]"
              >
                Clear this call’s data
              </button>
              <p className="mt-1.5 text-[11px] text-[var(--text-3)]">Stops pending speech, cancels pending work, clears caches and what assistance remembers. The transcript stays until you leave.</p>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
