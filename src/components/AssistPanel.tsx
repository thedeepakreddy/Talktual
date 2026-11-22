// Phase 5: conversation-assistance controls and the inspectable, correctable
// context summary. Rendered as a bottom sheet (same pattern as the transcript);
// nothing here pops up on its own.

import { motion, AnimatePresence } from 'motion/react';
import { X } from 'lucide-react';
import { cn } from '../utils';
import type { CommunicationSession, SessionSnapshot } from '../engine/CommunicationSession';

interface Props {
  open: boolean;
  onClose: () => void;
  session: CommunicationSession | null;
  snapshot: SessionSnapshot | null;
}

function Toggle({ label, hint, on, onChange, disabled }: { label: string; hint?: string; on: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <label className={cn('flex items-start justify-between gap-4 py-2.5', disabled && 'opacity-40')}>
      <span>
        <span className="block text-[15px] text-[var(--text-1)]">{label}</span>
        {hint && <span className="mt-0.5 block text-[12px] leading-snug text-[var(--text-3)]">{hint}</span>}
      </span>
      <input type="checkbox" role="switch" aria-label={label} className="mt-1 h-5 w-5 shrink-0 accent-[var(--blue)]" checked={on} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

const KIND_LABEL: Record<string, string> = { person: 'Person', place: 'Place', name: 'Name', date: 'Date', time: 'Time', number: 'Number', term: 'Pinned' };

export function AssistPanel({ open, onClose, session, snapshot }: Props) {
  const intel = snapshot?.intelligence;
  const prefs = intel?.prefs;
  const ctx = intel?.context;
  const enabled = Boolean(prefs?.enabled);
  return (
    <AnimatePresence>
      {open && intel?.available && prefs && (
        <>
          <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose} className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[2px]" />
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="Conversation assistance"
            initial={{ y: '100%' }}
            animate={{ y: 0 }}
            exit={{ y: '100%' }}
            transition={{ type: 'spring', damping: 32, stiffness: 340 }}
            className="fixed inset-x-0 bottom-0 z-50 mx-auto flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-t-[28px] border border-white/10 bg-[#141418]/95 backdrop-blur-2xl"
          >
            <div className="flex items-center justify-between px-5 pt-5 pb-2">
              <h2 className="text-[19px] font-semibold tracking-[-0.01em]">Conversation assistance</h2>
              <button onClick={onClose} aria-label="Close assistance" className="flex h-8 w-8 items-center justify-center rounded-full bg-white/8 text-[var(--text-2)] hover:bg-white/14">
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="sheet-scroll flex-1 overflow-y-auto px-5 pb-6">
              <p className="text-[13px] leading-snug text-[var(--text-3)]">
                Runs on this device. It remembers names, places and topics from <strong>this call only</strong> to keep translations consistent and to add notes.
                Nothing extra is sent to the translator, and it’s all cleared when the call ends.
              </p>
              <div className="mt-3 divide-y divide-white/8">
                <Toggle label="Assistance" hint="Off = plain translation, context cleared now" on={enabled} onChange={(v) => session?.setIntelligenceEnabled(v)} />
                <Toggle
                  label="Concise speech"
                  hint="Skip filler words (“you know”, “o sea”) when speaking their words to me. The transcript keeps everything."
                  on={prefs.mode === 'concise'}
                  disabled={!enabled}
                  onChange={(v) => session?.setIntelligencePrefs({ mode: v ? 'concise' : 'faithful' })}
                />
                <Toggle
                  label="Context-assisted translation"
                  hint="Keep names as spoken; read words like “bank” the way the conversation makes clear. Always labelled."
                  on={prefs.assist}
                  disabled={!enabled}
                  onChange={(v) => session?.setIntelligencePrefs({ assist: v })}
                />
                <Toggle label="Notes" hint="Ambiguous words, references, idioms and corrections, under each line" on={prefs.notes} disabled={!enabled} onChange={(v) => session?.setIntelligencePrefs({ notes: v })} />
                <Toggle label="Suggestions" hint="Occasional tips, e.g. when they may not have understood" on={prefs.suggestions} disabled={!enabled} onChange={(v) => session?.setIntelligencePrefs({ suggestions: v })} />
                {snapshot?.conversation.enabled && (
                  <Toggle
                    label="Send drafts while I pause"
                    hint="A draft translation of an unfinished sentence is shown (not spoken) to them"
                    on={snapshot.conversation.drafts}
                    onChange={(v) => session?.setDraftsEnabled(v)}
                  />
                )}
              </div>

              <div className="mt-5 flex items-center justify-between">
                <h3 className="text-[15px] font-semibold">What it remembers</h3>
                <button
                  onClick={() => session?.resetIntelligenceContext()}
                  disabled={!enabled}
                  className="rounded-full border border-white/10 bg-white/[0.06] px-3 py-1 text-[12px] font-medium text-[var(--text-2)] hover:bg-white/[0.1] disabled:opacity-40"
                >
                  Reset context
                </button>
              </div>
              {ctx && ctx.entities.length > 0 ? (
                <ul className="mt-2 space-y-1.5" aria-label="Remembered items">
                  {ctx.entities.map((e) => (
                    <li key={e.id} className="flex items-center justify-between gap-3 rounded-xl bg-white/[0.05] px-3 py-2">
                      <span className="min-w-0">
                        <span className={cn('text-[14px]', e.superseded && 'line-through opacity-60')}>{e.text}</span>
                        <span className="ml-2 text-[11px] text-[var(--text-3)]">
                          {KIND_LABEL[e.kind] ?? e.kind}
                          {e.source === 'inferred' ? ' · guessed' : ''}
                          {e.superseded ? ' · corrected' : ''}
                        </span>
                      </span>
                      <button onClick={() => session?.removeContextItem(e.id)} aria-label={`Forget ${e.text}`} className="text-[12px] text-[var(--text-3)] hover:text-[var(--text-1)]">
                        Forget
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="mt-2 text-[13px] text-[var(--text-3)]">Nothing yet.</p>
              )}
              {ctx && (
                <p className="mt-3 text-[11px] text-[var(--text-3)]">
                  {ctx.turns}/{ctx.budget.turns} recent lines · {ctx.entities.length}/{ctx.budget.entities} items
                  {ctx.topic.length ? ` · topic words: ${ctx.topic.slice(0, 5).join(', ')}` : ''}
                </p>
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
