import { Fragment, useEffect, useRef, useState } from 'react';
import { ChevronLeft, Ear, Pause, RotateCcw, SlidersHorizontal, Trash2, Volume2, VolumeX, X } from 'lucide-react';
import { findLanguage, type Language } from '../../types';
import { cn } from '../../utils';
import { useAmbientSession, createAmbientDeps } from '../../ambient/useAmbientSession';
import type { AmbientDeps, AmbientLine, AmbientSession, AmbientSnapshot, CapState } from '../../ambient/AmbientSession';
import type { ProcessingMode } from '../../privacy/policy';
import { Banner, Button, IconButton, Row, Section, Segmented, Sheet, Switch } from '../ui';
import { LanguagePicker } from '../LanguagePicker';
import { coarseBrowser, type ValidationRecorder } from '../../ambient/validation';
import { ConsentSheet } from '../ConsentSheet';

interface AmbientScreenProps {
  /** The user's language: what they read and hear. */
  language: Language;
  onLanguageChange: (l: Language) => void;
  onExit: () => void;
  depsFactory?: () => AmbientDeps;
}

const DEFAULT_FROM = 'hu-HU';
const VOLUMES = { quiet: 0.4, medium: 0.7, full: 1 } as const;
type VolumeKey = keyof typeof VOLUMES;
const volumeKey = (v: number): VolumeKey => (v <= 0.5 ? 'quiet' : v < 0.9 ? 'medium' : 'full');
const CAP_LABEL: Record<CapState, string> = { device: 'On this device', cloud: 'Cloud', 'device+cloud': 'Device, then cloud', unavailable: 'Unavailable' };

export function AmbientScreen({ language, onLanguageChange, onExit, depsFactory = createAmbientDeps }: AmbientScreenProps) {
  const initialFrom = language.bcp47.startsWith('hu') ? 'en-US' : DEFAULT_FROM;
  const { session, snapshot: s, recorder } = useAmbientSession({ from: initialFrom, to: language.bcp47, speak: true, headphones: false, volume: VOLUMES.medium }, depsFactory);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [picker, setPicker] = useState<'from' | 'to' | null>(null);

  // Keep "You read and hear" in sync with the app language.
  useEffect(() => {
    if (session && s && s.to !== language.bcp47) session.setLanguages(s.from === language.bcp47 ? initialFrom : s.from, language.bcp47);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session, language.bcp47]);

  // A consent prompt always takes over (never stacked on settings).
  useEffect(() => {
    if (s?.consent.prompt) setSettingsOpen(false);
  }, [s?.consent.prompt]);

  const listening = s?.phase === 'listening' || s?.phase === 'starting';
  // The tab says it is listening, too (visible even when another tab is in front).
  useEffect(() => {
    const base = 'Talktual';
    document.title = listening ? `● Listening — ${base}` : base;
    return () => {
      document.title = base;
    };
  }, [listening]);

  const from = findLanguage(s?.from) ?? findLanguage(initialFrom)!;
  const to = findLanguage(s?.to) ?? language;
  const exit = () => {
    session?.stop();
    onExit();
  };

  return (
    <div className={cn('transition-[padding] duration-200', (settingsOpen || picker) && 'lg:pr-[400px]')}>
      <div className="mx-auto flex h-dvh w-full max-w-[760px] flex-col">
        <header className="safe-t flex min-h-16 items-center gap-2 border-b border-line px-2 sm:px-4">
          <IconButton label="Back" onClick={exit}>
            <ChevronLeft className="h-5 w-5" />
          </IconButton>
          <div className="min-w-0 flex-1">
            <h1 className="truncate text-heading font-semibold">Live translate</h1>
            <p className="flex items-center gap-1.5 text-label text-fg-muted" data-testid="ambient-meta">
              <span className={cn('h-2 w-2 shrink-0 rounded-full', listening ? (s?.capturing ? 'bg-danger pulse-dot' : 'bg-accent') : 'bg-fg-subtle')} aria-hidden="true" />
              <span className="truncate">
                {[phaseLabel(s), `${from.native} → ${to.native}`, s?.processing.available && s.processing.mode !== 'cloud' ? (s.processing.mode === 'local-only' ? 'On-device only' : 'Hybrid') : null]
                  .filter(Boolean)
                  .join(' · ')}
              </span>
            </p>
          </div>
          <IconButton label="Live translate settings" onClick={() => setSettingsOpen(true)}>
            <SlidersHorizontal className="h-5 w-5" />
          </IconButton>
        </header>

        <AmbientBanner s={s} session={session} onSettings={() => setSettingsOpen(true)} />

        {recorder && <TestModeStrip recorder={recorder} />}

        {s && s.lines.length === 0 && !listening && s.phase !== 'paused' ? (
          <Setup s={s} session={session} from={from} to={to} onPick={setPicker} />
        ) : (
          <Feed s={s} session={session} />
        )}

        <Dock s={s} session={session} />
      </div>

      <AmbientSettings open={settingsOpen} onClose={() => setSettingsOpen(false)} s={s} session={session} from={from} to={to} onPick={(w) => (setSettingsOpen(false), setPicker(w))} />

      <LanguagePicker
        open={picker !== null}
        title={picker === 'from' ? 'They speak' : 'You read and hear'}
        selected={picker === 'from' ? from : to}
        onSelect={(l) => {
          if (picker === 'from') session?.setLanguages(l.bcp47, s?.to ?? language.bcp47);
          else onLanguageChange(l);
        }}
        onClose={() => setPicker(null)}
      />

      <ConsentSheet
        open={Boolean(s?.consent.prompt)}
        disclosure={s?.consent.disclosure ?? null}
        granted={Boolean(s?.consent.granted)}
        allowLabel="Start listening"
        onAllow={() => session?.grantConsent()}
        onDecline={() => session?.declineConsent()}
      />
    </div>
  );
}

function phaseLabel(s: AmbientSnapshot | null): string {
  switch (s?.phase) {
    case 'listening':
      return s.capturing ? 'Listening' : s.heldForSpeech ? 'Speaking' : 'Listening';
    case 'starting':
      return 'Starting…';
    case 'paused':
      return 'Paused';
    case 'stalled':
      return 'Stopped responding';
    case 'unavailable':
      return 'Unavailable';
    default:
      return 'Not listening';
  }
}

// ---------------------------------------------------------------------------

function AmbientBanner({ s, session, onSettings }: { s: AmbientSnapshot | null; session: AmbientSession | null; onSettings: () => void }) {
  if (!s) return null;
  const action = (label: string, fn: () => void, danger = false) => (
    <button className={cn('-my-3 -mr-2 min-h-11 px-2 text-label font-semibold', danger ? 'text-danger' : 'text-accent-text')} onClick={fn}>
      {label}
    </button>
  );
  const hinted = s.languageHint ? findLanguage(s.languageHint) : null;
  return (
    <>
      {(s.notice || hinted) && (
        <div className="space-y-2 px-4 pt-3 sm:px-6">
          {s.notice && (
            <Banner
              tone={s.notice.tone === 'error' ? 'error' : s.notice.code === 'offline' ? 'offline' : s.notice.tone}
              action={
                s.notice.code.startsWith('asr-') || s.pauseReason === 'policy'
                  ? action('Settings', onSettings)
                  : s.phase === 'stalled' || (s.phase === 'paused' && s.pauseReason !== 'offline')
                    ? action('Resume', () => session?.resume())
                    : undefined
              }
            >
              {s.notice.message}
            </Banner>
          )}
          {hinted && (
            <Banner
              tone="info"
              action={
                <span className="flex items-center">
                  {action(`Switch to ${hinted.native}`, () => session?.setLanguages(hinted.bcp47, s.to))}
                  <button className="-my-3 -mr-3 flex h-11 w-11 items-center justify-center text-fg-subtle hover:text-fg" aria-label="Dismiss" onClick={() => session?.dismissLanguageHint()}>
                    <X className="h-4 w-4" />
                  </button>
                </span>
              }
            >
              This sounds like {hinted.name}, not {findLanguage(s.from)?.name}.
            </Banner>
          )}
        </div>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------

/**
 * Phase 8.5 test mode (only with ?validate=1 or ?debug=1): the device owner marks
 * the end of each spoken test phrase and exports content-free timings.
 */
function TestModeStrip({ recorder }: { recorder: ValidationRecorder }) {
  const [marks, setMarks] = useState(0);
  const [msg, setMsg] = useState<string | null>(null);
  const exportSummary = async () => {
    const json = JSON.stringify(recorder.summary(coarseBrowser(navigator.userAgent)), null, 2);
    try {
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = `talktual-validation-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')}.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
      setMsg('Exported');
    } catch {
      try {
        await navigator.clipboard.writeText(json);
        setMsg('Copied');
      } catch {
        setMsg('Export failed');
      }
    }
  };
  return (
    <div className="mx-4 mt-3 flex flex-wrap items-center gap-2 rounded-md border border-dashed border-line-strong px-3 py-1.5 sm:mx-6" aria-label="Test mode">
      <span className="mr-auto text-label font-semibold text-fg-muted">Test mode{msg ? ` · ${msg}` : ''}</span>
      <Button
        size="sm"
        variant="secondary"
        onClick={() => {
          recorder.marker();
          setMarks((m) => m + 1);
        }}
      >
        Mark end of speech{marks ? ` (${marks})` : ''}
      </Button>
      <Button size="sm" variant="ghost" onClick={exportSummary}>
        Export
      </Button>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          recorder.clear();
          setMarks(0);
          setMsg('Cleared');
        }}
      >
        Clear
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------

function Setup({ s, session, from, to, onPick }: { s: AmbientSnapshot; session: AmbientSession | null; from: Language; to: Language; onPick: (w: 'from' | 'to') => void }) {
  return (
    <div className="scroll-area flex-1 overflow-y-auto px-4 pb-4 pt-6 sm:px-6">
      <p className="max-w-[32rem] text-body text-fg-muted">Hear what people around you are saying, translated into {to.name}. It listens only while you want it to.</p>
      <Section title="Languages">
        <Row label="They speak" value={`${from.flag} ${from.native}`} onClick={() => onPick('from')} ariaLabel={`They speak: ${from.native}. Change`} />
        <Row label="You read and hear" value={`${to.flag} ${to.native}`} onClick={() => onPick('to')} ariaLabel={`You read and hear: ${to.native}. Change`} />
      </Section>
      <Section title="Listening">
        <Switch label="Speak translations" description="Off: text only." checked={s.speak} onChange={(v) => session?.setSpeak(v)} />
        <Switch
          label="I’m listening through headphones"
          description="Keeps listening while translations play. Without headphones, listening pauses while Talktual speaks, so it doesn’t translate itself."
          checked={s.headphones}
          onChange={(v) => session?.setHeadphones(v)}
        />
      </Section>
      <p className="mt-4 text-label text-fg-muted">Uses this device’s current microphone. Hold the phone toward the people speaking.</p>
    </div>
  );
}

// ---------------------------------------------------------------------------

function Feed({ s, session }: { s: AmbientSnapshot | null; session: AmbientSession | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const lines = s?.lines ?? [];
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines, s?.partial]);
  const empty = lines.length === 0 && !s?.partial;
  const lastTranslated = [...lines].reverse().find((l) => l.state === 'translated')?.id;
  return (
    <div
      ref={ref}
      role="log"
      aria-label="Translations"
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      className="scroll-area flex-1 overflow-y-auto px-4 sm:px-6"
    >
      {empty ? (
        <div className="flex h-full min-h-48 items-center justify-center py-10">
          <p className="max-w-[18rem] text-center text-body text-fg-muted">{s?.phase === 'listening' ? 'Listening. Translations appear here as people speak.' : 'Nothing yet.'}</p>
        </div>
      ) : (
        <ol className="flex min-h-full flex-col justify-end gap-4 py-6">
          {lines.map((l) => (
            <Fragment key={l.id}>
              {/* "Play again" only on the latest translated line: one replay control, where it's wanted. */}
              <Line line={l} playing={s?.playingId === l.id} canReplay={Boolean(s?.speak) && l.id === lastTranslated} onReplay={() => session?.replay(l.id)} />
            </Fragment>
          ))}
          {s?.partial && (
            <li className="max-w-[85%]" aria-label="Hearing">
              <p className="text-body italic text-fg-muted">
                {s.partial}
                <span aria-hidden="true">…</span>
              </p>
            </li>
          )}
        </ol>
      )}
    </div>
  );
}

function Line({ line: l, playing, canReplay, onReplay }: { line: AmbientLine; playing: boolean; canReplay: boolean; onReplay: () => void }) {
  const markers = [
    l.uncertain ? 'Unclear audio — may be inaccurate' : null,
    l.revised ? 'Corrected by the recognizer' : null,
    l.location === 'device' ? 'Translated on this device' : null,
    l.note ?? null,
  ].filter(Boolean);
  const translated = l.state === 'translated' && l.translation;
  return (
    <li className="flex items-start gap-2">
      <div className={cn('min-w-0 flex-1 border-l-2 pl-3', playing ? 'border-incoming' : 'border-line', l.state === 'skipped' && 'opacity-60')}>
        {translated ? (
          <p className="text-body-lg text-fg">
            {playing && <Volume2 className="mr-1.5 inline h-4 w-4 align-[-2px] text-incoming" aria-label="Playing" />}
            {l.translation}
          </p>
        ) : (
          <p data-original className={cn('text-body-lg', l.state === 'translating' ? 'italic text-fg-muted' : 'text-fg')}>
            {l.original}
          </p>
        )}
        {translated ? (
          <p data-original className="mt-0.5 text-label text-fg-muted">
            {l.original}
          </p>
        ) : l.state === 'translating' ? (
          <p className="mt-0.5 text-label text-fg-muted">Translating…</p>
        ) : null}
        {markers.length > 0 && <p className="mt-1 text-meta text-fg-subtle">{markers.join(' · ')}</p>}
      </div>
      {translated && canReplay && (
        <IconButton label="Play again" onClick={onReplay} className="text-fg-subtle">
          <RotateCcw className="h-4 w-4" />
        </IconButton>
      )}
    </li>
  );
}

// ---------------------------------------------------------------------------

function Dock({ s, session }: { s: AmbientSnapshot | null; session: AmbientSession | null }) {
  const listening = s?.phase === 'listening' || s?.phase === 'starting';
  const hint = !s
    ? ''
    : s.phase === 'listening'
      ? s.heldForSpeech
        ? 'Listening resumes after the translation'
        : s.hearing
          ? 'Hearing speech…'
          : 'Listening to the room'
      : s.phase === 'starting'
        ? 'Starting…'
        : s.phase === 'paused'
          ? 'Paused — not listening'
          : s.phase === 'stalled' || s.phase === 'unavailable'
            ? ''
            : 'Press Start to listen';
  return (
    <div className="safe-b px-4 pt-2 sm:px-6">
      <p aria-live="polite" className={cn('mb-3 min-h-5 text-center text-label font-medium', listening ? 'text-accent-text' : 'text-fg-muted')}>
        {hint}
      </p>
      <div className="mx-auto grid max-w-[360px] grid-cols-[1fr_auto_1fr] items-center">
        <div className="flex justify-start">
          <IconButton label="Speak translations" aria-pressed={Boolean(s?.speak)} tone={s?.speak ? 'default' : 'active'} onClick={() => session?.setSpeak(!s?.speak)}>
            {s?.speak ? <Volume2 className="h-5 w-5" /> : <VolumeX className="h-5 w-5" />}
          </IconButton>
        </div>
        {s?.phase === 'unavailable' && s.notice?.code !== 'permission-denied' ? (
          // Nothing to start until something changes (browser, settings): no dead button.
          <span className="h-[88px] w-[88px]" aria-hidden="true" />
        ) : listening ? (
          <button
            type="button"
            onClick={() => session?.pause()}
            aria-label="Pause listening"
            className="flex h-[88px] w-[88px] flex-col items-center justify-center gap-1 rounded-full bg-accent text-on-accent transition-transform active:scale-95"
          >
            {s?.hearing ? <Bars /> : <Pause className="h-6 w-6 fill-current" />}
            <span className="text-meta font-semibold">Pause</span>
          </button>
        ) : (
          <button
            type="button"
            onClick={() => (s?.phase === 'idle' || s?.phase === 'stopped' ? session?.start() : session?.resume())}
            disabled={!session}
            aria-label={s?.phase === 'paused' || s?.phase === 'stalled' ? 'Resume listening' : 'Start listening'}
            className="flex h-[88px] w-[88px] flex-col items-center justify-center gap-1 rounded-full bg-surface text-fg shadow-[inset_0_0_0_1px_var(--line-strong)] transition-[transform,background-color] hover:bg-surface-hover active:scale-95 disabled:opacity-40"
          >
            <Ear className="h-7 w-7" />
            <span className="text-meta font-semibold">{s?.phase === 'paused' || s?.phase === 'stalled' ? 'Resume' : 'Start'}</span>
          </button>
        )}
        <div className="flex justify-end">
          {Boolean(s?.lines.length) && (
            <IconButton label="Clear translations" onClick={() => session?.clear()}>
              <Trash2 className="h-5 w-5" />
            </IconButton>
          )}
        </div>
      </div>
    </div>
  );
}

function Bars() {
  return (
    <span className="flex h-7 items-center gap-1" aria-hidden="true">
      {[0, 160, 320, 80].map((d) => (
        <span key={d} className="level-bar h-6 w-1.5 rounded-full bg-current" style={{ animationDelay: `${d}ms` }} />
      ))}
    </span>
  );
}

// ---------------------------------------------------------------------------

function AmbientSettings({
  open,
  onClose,
  s,
  session,
  from,
  to,
  onPick,
}: {
  open: boolean;
  onClose: () => void;
  s: AmbientSnapshot | null;
  session: AmbientSession | null;
  from: Language;
  to: Language;
  onPick: (w: 'from' | 'to') => void;
}) {
  return (
    <Sheet open={open} onClose={onClose} title="Live translate settings" closeLabel="Close settings">
      <Section title="Languages">
        <Row label="They speak" value={`${from.flag} ${from.native}`} onClick={() => onPick('from')} ariaLabel={`They speak: ${from.native}. Change`} />
        <Row label="You read and hear" value={`${to.flag} ${to.native}`} onClick={() => onPick('to')} ariaLabel={`You read and hear: ${to.native}. Change`} />
      </Section>
      {s && (
        <>
          <Section title="Listening">
            <Switch label="Speak translations" description="Off: text only." checked={s.speak} onChange={(v) => session?.setSpeak(v)} />
            {s.speak && (
              <div className="py-3">
                <Segmented<VolumeKey>
                  label="Translation volume"
                  value={volumeKey(s.volume)}
                  onChange={(k) => session?.setVolume(VOLUMES[k])}
                  options={[
                    { value: 'quiet', label: 'Quiet' },
                    { value: 'medium', label: 'Medium' },
                    { value: 'full', label: 'Full' },
                  ]}
                />
                <p className="mt-2 text-label text-fg-muted">Keep it low enough to still hear the room. Late translations are shown, not spoken.</p>
              </div>
            )}
            <Switch
              label="I’m listening through headphones"
              description="Keeps listening while translations play. Without headphones, listening pauses while Talktual speaks."
              checked={s.headphones}
              onChange={(v) => session?.setHeadphones(v)}
            />
          </Section>
          {s.processing.available && (
            <Section title="Processing">
              <div className="py-3">
                <Segmented<ProcessingMode>
                  label="Processing mode"
                  value={s.processing.mode}
                  onChange={(m) => session?.setProcessingMode(m)}
                  options={[
                    { value: 'cloud', label: 'Cloud' },
                    { value: 'hybrid', label: 'Hybrid' },
                    { value: 'local-only', label: 'On-device' },
                  ]}
                />
              </div>
              <ul className="divide-y divide-line" aria-label="Where each step runs">
                {(
                  [
                    ['Speech recognition', s.processing.asr],
                    ['Translation', s.processing.translate],
                    ['Spoken translation', s.processing.speech],
                  ] as const
                ).map(([label, st]) => (
                  <li key={label} className="flex min-h-11 items-center justify-between py-2">
                    <span className="text-body">{label}</span>
                    <span className={cn('text-label font-semibold', st === 'unavailable' ? 'text-accent-text' : 'text-fg-muted')}>{CAP_LABEL[st]}</span>
                  </li>
                ))}
              </ul>
            </Section>
          )}
          <Section title="Microphone">
            <p className="py-3 text-label text-fg-muted">
              Talktual uses this device’s current microphone; a web page can’t choose a different one for speech recognition. With AirPods connected, an iPhone usually switches to the AirPods microphone, which lowers quality for listening and recording. Speaker labels aren’t available: lines appear in the order heard.
            </p>
          </Section>
          <Section title="Privacy">
            <Row label="What’s captured and where it goes" onClick={() => session?.showDisclosure()} />
            <div className="py-3">
              <Button variant="ghost" size="sm" className="-ml-3 text-danger" onClick={() => session?.clear()}>
                Clear translations
              </Button>
            </div>
          </Section>
        </>
      )}
    </Sheet>
  );
}
