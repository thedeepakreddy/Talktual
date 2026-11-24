import { useEffect, useState } from 'react';
import type { CommunicationSession, SessionSnapshot } from '../../engine/CommunicationSession';
import type { ProcessingMode } from '../../privacy/policy';
import type { Language } from '../../types';
import { Button, Row, Section, Segmented, Sheet, Switch } from '../ui';
import { cn } from '../../utils';

type Page = 'main' | 'runs' | 'memory';

interface SettingsSheetProps {
  open: boolean;
  onClose: () => void;
  session: CommunicationSession | null;
  snapshot: SessionSnapshot | null;
  roomCode: string;
  language: Language;
  onChangeLanguage: () => void;
  onShowDisclosure: () => void;
}

const MODE_HINT: Record<ProcessingMode, string> = {
  cloud: 'Speech and text go to the services listed under “Where things run”. This device is used when they can’t be reached.',
  hybrid: 'This device handles what it can; cloud services only for the rest.',
  'local-only': 'Nothing is sent to recognition, translation or voice services. Only what this device can do works.',
};

const TASK_LABEL: Record<string, string> = {
  'speech-input': 'Speech recognition',
  translation: 'Translation',
  captions: 'Live captions',
  'speech-output': 'Spoken replies',
  'typed-input': 'Typing',
  assistance: 'Conversation assistance',
};
const STATE_LABEL: Record<string, string> = { device: 'On this device', cloud: 'Cloud', 'device+cloud': 'Device, then cloud', unavailable: 'Unavailable' };
const KIND_LABEL: Record<string, string> = { person: 'Person', place: 'Place', name: 'Name', date: 'Date', time: 'Time', number: 'Number', term: 'Pinned' };

export function SettingsSheet({ open, onClose, session, snapshot: s, roomCode, language, onChangeLanguage, onShowDisclosure }: SettingsSheetProps) {
  const [page, setPage] = useState<Page>('main');
  useEffect(() => {
    if (open) setPage('main');
  }, [open]);
  // A sub-page starts at its top, not at the parent's scroll offset.
  useEffect(() => {
    document.querySelector('[role="dialog"] .scroll-area')?.scrollTo?.(0, 0);
  }, [page]);

  const proc = s?.processing;
  const intel = s?.intelligence;
  const prefs = intel?.prefs;
  const ctx = intel?.context;
  const connected = s?.phase === 'connected';
  const title = page === 'runs' ? 'Where things run' : page === 'memory' ? 'What assistance remembers' : 'Call settings';

  return (
    <Sheet open={open} onClose={onClose} title={title} closeLabel="Close settings" onBack={page === 'main' ? undefined : () => setPage('main')}>
      {page === 'main' && (
        <>
          <Section title="Talking">
            {s?.features.continuous && (
              <div className="py-3">
                <Segmented
                  label="Listening"
                  value={s.mode}
                  onChange={(m) => session?.setListeningMode(m)}
                  options={[
                    { value: 'push-to-talk', label: 'Hold to talk' },
                    { value: 'continuous', label: 'Hands-free', disabled: !connected || s.activity === 'stopped' },
                  ]}
                />
                <p className="mt-2 text-label text-fg-muted">
                  {s.mode === 'continuous' ? 'Listening continuously; it pauses while they speak.' : 'The microphone is used only while you hold the button.'}
                </p>
              </div>
            )}
            <Row label="Your language" value={`${language.flag} ${language.native}`} onClick={onChangeLanguage} ariaLabel={`Your language: ${language.native}. Change`} />
            {s?.conversation.enabled && (
              <Switch
                label="Send drafts while I pause"
                description="An unfinished sentence is shown to them, not spoken."
                checked={s.conversation.drafts}
                onChange={(v) => session?.setDraftsEnabled(v)}
              />
            )}
          </Section>

          {proc?.available && (
            <Section title="Processing">
              <div className="py-3">
                <Segmented<ProcessingMode>
                  label="Processing mode"
                  value={proc.mode}
                  onChange={(m) => session?.setProcessingMode(m)}
                  options={[
                    { value: 'cloud', label: 'Cloud' },
                    { value: 'hybrid', label: 'Hybrid' },
                    { value: 'local-only', label: 'On-device' },
                  ]}
                />
                <p className="mt-2 text-label text-fg-muted">{MODE_HINT[proc.mode]}</p>
              </div>
              <Row label="Where things run" value={proc.degraded ? 'Some unavailable' : undefined} onClick={() => setPage('runs')} />
              <div className="py-3">
                <Button variant="ghost" size="sm" className="-ml-3 text-danger" onClick={() => session?.clearPrivateData()}>
                  Clear this call’s data
                </Button>
              </div>
            </Section>
          )}

          {intel?.available && prefs && (
            <Section title="Assistance">
              <Switch label="Assistance" description="Runs on this device; remembers this call only." checked={prefs.enabled} onChange={(v) => session?.setIntelligenceEnabled(v)} />
              {prefs.enabled && (
                <>
                  <Switch label="Concise speech" description="Skip filler words when their speech is read to you." checked={prefs.mode === 'concise'} onChange={(v) => session?.setIntelligencePrefs({ mode: v ? 'concise' : 'faithful' })} />
                  <Switch label="Context-assisted translation" description="Keep names as spoken; use the conversation to pick word senses." checked={prefs.assist} onChange={(v) => session?.setIntelligencePrefs({ assist: v })} />
                  <Switch label="Notes" description="Ambiguous words, references, idioms and corrections." checked={prefs.notes} onChange={(v) => session?.setIntelligencePrefs({ notes: v })} />
                  <Switch label="Suggestions" checked={prefs.suggestions} onChange={(v) => session?.setIntelligencePrefs({ suggestions: v })} />
                  <Row label="What it remembers" value={ctx ? String(ctx.entities.length) : undefined} onClick={() => setPage('memory')} />
                </>
              )}
            </Section>
          )}

          {s?.voice.available && (
            <Section title="Voice">
              <Switch
                label="Voice match"
                description="Their replies use a voice closer to yours. Off unless you allow it."
                checked={s.voice.sharing}
                disabled={!connected}
                onChange={() => session?.toggleVoiceMirror()}
              />
              {s.voice.peerActive && <p className="pb-3 text-label text-fg-muted">Their voice style matched</p>}
            </Section>
          )}

          <Section title="This call">
            <div className="flex min-h-12 items-center justify-between py-2.5">
              <span className="text-body-lg font-[520]">Call code</span>
              <span className="font-mono tracking-[0.12em] text-fg-muted">{roomCode}</span>
            </div>
            <div className="flex min-h-12 items-center justify-between py-2.5">
              <span className="text-body-lg font-[520]">Connection</span>
              <span className="text-fg-muted">{s?.transport === 'p2p' ? 'Direct' : s?.transport === 'relay' ? 'Relayed' : '—'}</span>
            </div>
            <Row label="How your voice is processed" onClick={onShowDisclosure} />
          </Section>
        </>
      )}

      {page === 'runs' && proc && (
        <div className="space-y-6 pt-1">
          <ul className="[&>*+*]:border-t [&>*+*]:border-[var(--glass-stroke)]" aria-label="Where each function runs">
            {proc.capabilities.map((c) => (
              <li key={c.task} className="flex items-start justify-between gap-4 py-3">
                <span className="text-body-lg font-[520]">{TASK_LABEL[c.task] ?? c.task}</span>
                <span className="text-right">
                  <span className={cn('block text-label font-semibold', c.state === 'unavailable' ? 'text-accent-text' : 'text-fg')}>{STATE_LABEL[c.state]}</span>
                  <span className="block text-label text-fg-muted">{c.detail}</span>
                </span>
              </li>
            ))}
          </ul>
          <div>
            <h3 className="text-label font-semibold text-fg-muted">What leaves this device</h3>
            <p className="mt-1.5 text-body text-fg-muted">
              {proc.mode === 'local-only'
                ? 'Your words go only to the other person (they’re asked to keep them on their device too). Talktual’s server is used only to connect the call. No recognition, translation or voice service receives anything.'
                : 'Anything marked “Cloud” is sent to that service while you use it. Your words also go to the other person. Talktual’s server connects the call. Nothing is stored after the call.'}
              {proc.peerPrivate ? ' The other person is in on-device mode: their words stay on this device too.' : ''}
            </p>
          </div>
          <div>
            <h3 className="text-label font-semibold text-fg-muted">On-device engines</h3>
            <ul className="mt-1.5 space-y-1 text-label text-fg-muted" aria-label="On-device engines">
              {proc.runtimes.map((r) => (
                <li key={r.id}>
                  <span className="text-fg">{r.model.name}</span> — {r.readiness === 'ready' ? 'ready' : r.readiness}
                  {r.languages.length ? ` · ${r.languages.join(', ')}` : ''}
                  {r.constraints[0] ? ` · ${r.constraints[0]}` : ''}
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}

      {page === 'memory' && ctx && (
        <div className="pt-1">
          <p className="text-label text-fg-muted">Names, places and topics from this call only. Cleared when the call ends.</p>
          {ctx.entities.length ? (
            <ul className="mt-3 [&>*+*]:border-t [&>*+*]:border-[var(--glass-stroke)]" aria-label="Remembered items">
              {ctx.entities.map((e) => (
                <li key={e.id} className="flex min-h-12 items-center justify-between gap-3 py-2">
                  <span className="min-w-0">
                    <span className={cn('text-body', e.superseded && 'line-through opacity-60')}>{e.text}</span>
                    <span className="ml-2 text-label text-fg-subtle">
                      {KIND_LABEL[e.kind] ?? e.kind}
                      {e.source === 'inferred' ? ' · guessed' : ''}
                      {e.superseded ? ' · corrected' : ''}
                    </span>
                  </span>
                  <Button variant="ghost" size="sm" aria-label={`Forget ${e.text}`} onClick={() => session?.removeContextItem(e.id)}>
                    Forget
                  </Button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-6 text-body text-fg-muted">Nothing yet.</p>
          )}
          <Button variant="secondary" size="sm" className="mt-6" disabled={!ctx.entities.length && !ctx.turns} onClick={() => session?.resetIntelligenceContext()}>
            Reset context
          </Button>
        </div>
      )}
    </Sheet>
  );
}
