import { useEffect, useRef, useState } from 'react';
import { ArrowRight, AudioLines, ChevronLeft, Ear, Pause, Square, RotateCcw, SlidersHorizontal, Trash2, Volume2, VolumeX, X } from 'lucide-react';
import { findLanguage, type Language } from '../../types';
import { cn } from '../../utils';
import { useAmbientSession, createAmbientDeps } from '../../ambient/useAmbientSession';
import type { AmbientDeps, AmbientLine, AmbientSession, AmbientSnapshot, CapState } from '../../ambient/AmbientSession';
import type { ProcessingMode } from '../../privacy/policy';
import { Banner, Button, IconButton, Row, Section, Segmented, Sheet, Switch } from '../ui';
import { LanguagePicker } from '../LanguagePicker';
import { coarseBrowser, type ValidationRecorder } from '../../ambient/validation';
import { ConsentSheet } from '../ConsentSheet';
import { ActivityTrace } from './ActivityTrace';
import { getVoicePrefs } from '../../app/prefs';

interface AmbientScreenProps {
  /** The user's language: what they read and hear. */
  language: Language;
  onLanguageChange: (l: Language) => void;
  /** What the room speaks (chosen on Home or here). */
  from: Language;
  onFromChange: (l: Language) => void;
  /** Start listening as soon as the session is ready (came from Home's Start listening). */
  autostart?: boolean;
  onExit: () => void;
  depsFactory?: () => AmbientDeps;
}

const VOLUMES = { quiet: 0.4, medium: 0.7, full: 1 } as const;
type VolumeKey = keyof typeof VOLUMES;
const volumeKey = (v: number): VolumeKey => (v <= 0.5 ? 'quiet' : v < 0.9 ? 'medium' : 'full');
const CAP_LABEL: Record<CapState, string> = { device: 'On this device', cloud: 'Cloud', 'device+cloud': 'Device, then cloud', unavailable: 'Unavailable' };

export function AmbientScreen({ language, onLanguageChange, from: fromProp, onFromChange, autostart, onExit, depsFactory = createAmbientDeps }: AmbientScreenProps) {
  const [voice] = useState(() => getVoicePrefs());
  const { session, snapshot: s, recorder } = useAmbientSession({ from: fromProp.bcp47, to: language.bcp47, speak: voice.speak, headphones: false, volume: voice.volume }, depsFactory);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [picker, setPicker] = useState<'from' | 'to' | null>(null);

  // Languages are app state (shared with Home); the session follows them.
  useEffect(() => {
    if (session && s && (s.to !== language.bcp47 || s.from !== fromProp.bcp47)) session.setLanguages(fromProp.bcp47, language.bcp47);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session, language.bcp47, fromProp.bcp47]);

  // Came from Home's "Start listening": start once (consent is still asked first if needed).
  const autostarted = useRef(false);
  useEffect(() => {
    if (!autostart || autostarted.current || !session || s?.phase !== 'idle') return;
    autostarted.current = true;
    session.start();
  }, [autostart, session, s?.phase]);

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

  const from = findLanguage(s?.from) ?? fromProp;
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
            <h1 className="truncate text-heading font-semibold">AIR</h1>
            <p className="flex items-center gap-1.5 text-label text-fg-muted" data-testid="ambient-meta">
              <span className={cn('h-2 w-2 shrink-0 rounded-full', listening ? (s?.capturing ? 'bg-danger' : 'bg-accent') : s?.phase === 'paused' ? 'bg-warning' : 'bg-fg-subtle')} aria-hidden="true" />
              <span className="truncate">
                {[phaseLabel(s), s?.processing.available && s.processing.mode !== 'cloud' ? (s.processing.mode === 'local-only' ? 'On-device only' : 'Hybrid') : null]
                  .filter(Boolean)
                  .join(' · ')}
              </span>
            </p>
          </div>
          <IconButton label="AIR settings" onClick={() => setSettingsOpen(true)}>
            <SlidersHorizontal className="h-5 w-5" />
          </IconButton>
        </header>

        <LanguagePair from={from} to={to} onPick={setPicker} />

        <AmbientBanner s={s} session={session} onSettings={() => setSettingsOpen(true)} />

        {recorder && <TestModeStrip recorder={recorder} />}

        {s && s.lines.length === 0 && !listening && s.phase !== 'paused' ? (
          <Setup s={s} session={session} to={to} />
        ) : (
          <Feed s={s} session={session} />
        )}

        <div className="px-4 pt-3 sm:px-6">
          <ActivityTrace capturing={Boolean(s?.capturing)} hearing={Boolean(s?.hearing)} partial={s?.partial ?? ''} settled={!listening} />
        </div>

        <Dock s={s} session={session} />
      </div>

      <AmbientSettings open={settingsOpen} onClose={() => setSettingsOpen(false)} s={s} session={session} from={from} to={to} onPick={(w) => (setSettingsOpen(false), setPicker(w))} />

      <LanguagePicker
        open={picker !== null}
        title={picker === 'from' ? 'They speak' : 'You read and hear'}
        selected={picker === 'from' ? from : to}
        onSelect={(l) => {
          if (picker === 'from') onFromChange(l);
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

/** They speak → You hear, borderless and tappable (the concept-B channel strip without the box). */
function LanguagePair({ from, to, onPick }: { from: Language; to: Language; onPick: (w: 'from' | 'to') => void }) {
  return (
    <div className="grid grid-cols-[1fr_auto_1fr] items-center border-b border-line px-4 sm:px-6" role="group" aria-label="Language pair">
      <button type="button" onClick={() => onPick('from')} className="min-w-0 py-2.5 text-left" aria-label={`They speak: ${from.native}. Change`}>
        <span className="block text-meta text-fg-subtle">They speak</span>
        <span className="block truncate text-body font-semibold">{from.native}</span>
      </button>
      <ArrowRight className="mx-3 h-4 w-4 text-fg-subtle" aria-hidden="true" />
      <button type="button" onClick={() => onPick('to')} className="min-w-0 py-2.5 text-right" aria-label={`You hear: ${to.native}. Change`}>
        <span className="block text-meta text-fg-subtle">You hear</span>
        <span className="block truncate text-body font-semibold">{to.native}</span>
      </button>
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
    case 'stopped':
      return 'Stopped';
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

function Setup({ s, session, to }: { s: AmbientSnapshot; session: AmbientSession | null; to: Language }) {
  return (
    <div className="scroll-area flex-1 overflow-y-auto px-4 pb-4 pt-6 sm:px-6">
      <p className="max-w-[32rem] text-body text-fg-muted">Hear what people around you are saying, translated into {to.name}. It listens only while you want it to.</p>
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
        <ol className="flex min-h-full flex-col justify-end py-4">
          {lines.map((l, i) => (
            // "Play again" only on the latest translated line: one replay control, where it's wanted.
            <Line key={l.id} line={l} latest={i === lines.length - 1} playing={s?.playingId === l.id} canReplay={Boolean(s?.speak) && l.id === lastTranslated} onReplay={() => session?.replay(l.id)} />
          ))}
          {s?.partial && (
            <li className="border-t border-line py-3" aria-label="Hearing">
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

function Line({ line: l, latest, playing, canReplay, onReplay }: { line: AmbientLine; latest: boolean; playing: boolean; canReplay: boolean; onReplay: () => void }) {
  const markers = [
    l.uncertain ? 'Unclear audio — may be inaccurate' : null,
    l.revised ? 'Corrected by the recognizer' : null,
    l.location === 'device' ? 'Translated on this device' : null,
    l.note ?? null,
  ].filter(Boolean);
  const translated = l.state === 'translated' && l.translation;
  return (
    <li className="flex items-start gap-2 border-t border-line py-3.5 first:border-t-0">
      <div className={cn('min-w-0 flex-1', l.state === 'skipped' && 'opacity-60')}>
        {playing && (
          <p className="mb-1 flex items-center gap-1.5 text-meta font-semibold text-accent-text">
            <Volume2 className="h-3.5 w-3.5" aria-hidden="true" />
            Playing
          </p>
        )}
        {translated ? (
          <p className={cn('text-fg', latest ? 'text-translation font-medium tracking-[-0.005em]' : 'text-body-lg')}>{l.translation}</p>
        ) : (
          <p data-original className={cn(latest ? 'text-translation' : 'text-body-lg', l.state === 'translating' ? 'italic text-fg-muted' : 'text-fg')}>
            {l.original}
          </p>
        )}
        {translated ? (
          <p data-original className="mt-1 text-label text-fg-muted">
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
  const paused = s?.phase === 'paused' || s?.phase === 'stalled';
  const live = listening || paused;
  const hint = !s
    ? ''
    : s.phase === 'listening'
      ? s.heldForSpeech
        ? 'Listening resumes after the translation'
        : s.hearing
          ? 'Hearing speech'
          : 'Listening to the room'
      : s.phase === 'starting'
        ? 'Starting…'
        : s.phase === 'paused'
          ? 'Paused — not listening'
          : s.phase === 'stopped'
            ? 'Stopped. Nothing is listening.'
            : s.phase === 'stalled' || s.phase === 'unavailable'
              ? ''
              : 'Not listening yet';
  const canStart = !(s?.phase === 'unavailable' && s.notice?.code !== 'permission-denied');
  return (
    <div className="safe-b px-4 pt-3 sm:px-6">
      <p aria-live="polite" className={cn('mb-3 min-h-5 text-center text-label font-medium', listening ? 'text-accent-text' : paused ? 'text-warning' : 'text-fg-muted')}>
        {hint}
      </p>
      <div className="mx-auto grid max-w-[420px] grid-cols-[1fr_auto_1fr] items-center gap-3">
        <div className="flex justify-start">
          <IconButton label="Speak translations" aria-pressed={Boolean(s?.speak)} tone={s?.speak ? 'default' : 'active'} onClick={() => session?.setSpeak(!s?.speak)}>
            {s?.speak ? <Volume2 className="h-5 w-5" /> : <VolumeX className="h-5 w-5" />}
          </IconButton>
        </div>
        {!canStart ? (
          // Nothing to start until something changes (browser, settings): no dead button.
          <span className="h-12 w-[164px]" aria-hidden="true" />
        ) : listening ? (
          <Button variant="primary" className="w-[164px]" onClick={() => session?.pause()} aria-label="Pause listening" icon={s?.hearing ? <AudioLines className="h-5 w-5" /> : <Pause className="h-5 w-5" />}>
            Pause
          </Button>
        ) : (
          <Button
            variant="primary"
            className="w-[164px]"
            disabled={!session}
            onClick={() => (paused ? session?.resume() : session?.start())}
            aria-label={paused ? 'Resume listening' : 'Start listening'}
            icon={<Ear className="h-5 w-5" />}
          >
            {paused ? 'Resume' : s?.phase === 'stopped' ? 'Start again' : 'Start'}
          </Button>
        )}
        <div className="flex justify-end">
          {live ? (
            <IconButton label="Stop listening" onClick={() => session?.stop()}>
              <Square className="h-[18px] w-[18px]" />
            </IconButton>
          ) : Boolean(s?.lines.length) ? (
            <IconButton label="Clear translations" onClick={() => session?.clear()}>
              <Trash2 className="h-5 w-5" />
            </IconButton>
          ) : null}
        </div>
      </div>
    </div>
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
    <Sheet open={open} onClose={onClose} title="AIR settings" closeLabel="Close settings">
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
