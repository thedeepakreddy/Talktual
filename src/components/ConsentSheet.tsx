import { motion, AnimatePresence } from 'motion/react';
import { ShieldCheck } from 'lucide-react';
import type { Disclosure } from '../engine/privacy';

interface ConsentSheetProps {
  open: boolean;
  disclosure: Disclosure | null;
  /** Already granted: the sheet is informational (re-opened from the call screen). */
  granted: boolean;
  onAllow: () => void;
  onDecline: () => void;
}

/**
 * Remote-processing disclosure shown before the microphone is used for the
 * first time on this device (and on demand). Nothing is captured until the
 * user chooses "Allow microphone".
 */
export function ConsentSheet({ open, disclosure, granted, onAllow, onDecline }: ConsentSheetProps) {
  return (
    <AnimatePresence>
      {open && disclosure && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[2px]"
            onClick={onDecline}
          />
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-labelledby="consent-title"
            initial={{ y: '100%' }}
            animate={{ y: 0 }}
            exit={{ y: '100%' }}
            transition={{ type: 'spring', damping: 32, stiffness: 340 }}
            className="fixed inset-x-0 bottom-0 z-50 mx-auto w-full max-w-md rounded-t-[28px] border border-white/10 bg-[#141418]/95 px-5 pb-6 pt-5 backdrop-blur-2xl"
          >
            <div className="mb-3 flex items-center gap-2.5">
              <ShieldCheck className="h-5 w-5 text-[var(--green)]" aria-hidden="true" />
              <h2 id="consent-title" className="text-[19px] font-semibold tracking-[-0.01em] text-[var(--text-1)]">
                {granted ? 'How your voice is processed' : disclosure.title}
              </h2>
            </div>
            <ul className="space-y-2.5">
              {disclosure.points.map((p) => (
                <li key={p} className="text-[14px] leading-snug text-[var(--text-2)]">
                  {p}
                </li>
              ))}
            </ul>
            <div className="safe-b mt-5 flex gap-3">
              {granted ? (
                <button
                  onClick={onDecline}
                  className="flex-1 rounded-2xl bg-white/[0.1] py-3.5 text-[15px] font-semibold text-[var(--text-1)]"
                >
                  Close
                </button>
              ) : (
                <>
                  <button
                    onClick={onDecline}
                    className="flex-1 rounded-2xl border border-white/12 bg-white/[0.05] py-3.5 text-[15px] font-semibold text-[var(--text-1)]"
                  >
                    Not now
                  </button>
                  <button
                    onClick={onAllow}
                    className="flex-1 rounded-2xl bg-[var(--blue)] py-3.5 text-[15px] font-semibold text-white"
                  >
                    Allow microphone
                  </button>
                </>
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
