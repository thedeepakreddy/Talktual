// Engine Lab: test every Communication Engine phase (1–5) from the browser.
// Talks to the local Talktual ↔ CE bridge; the microphone is used only between
// Start/hold and Stop/release, after the user has read what happens to it.

import { useMemo, useState, type ReactNode } from 'react';
import { Page } from '../components/shell/AppShell';
import { Banner, Button, Section, Segmented, Switch } from '../components/ui';
import { CE_BRIDGE_URL } from '../lib/config';
import { useEngineLab, type LabDeps, type LabMode } from '../engineLab/useEngineLab';
import type { LabState } from '../engineLab/labState';

/* eslint-disable @typescript-eslint/no-explicit-any */
type Json = any;

function Stat({ label, value, hint }: { label: string; value: ReactNode; hint?: string }) {
  return (
    <div className="min-w-0 py-1.5" title={hint}>
      <dt className="text-label text-fg-subtle">{label}</dt>
      <dd className="truncate text-body font-[560] tabular-nums text-fg">{value ?? '—'}</dd>
    </div>
  );
}

function Grid({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-2 gap-x-4 py-3 sm:grid-cols-3">{children}</dl>;
}

function Chip({ on, children }: { on?: boolean; children: ReactNode }) {
  return (
    <span className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-label ${on ? 'bg-[color-mix(in_oklab,var(--accent)_22%,transparent)] text-fg' : 'bg-[var(--glass-top)] text-fg-muted'}`}>{children}</span>
  );
}

const fmtMs = (v: number) => (v < 10 ? v.toFixed(2) : String(Math.round(v)));
const ms = (p: Json) => (p && p.count ? `${fmtMs(p.p50)} / ${fmtMs(p.p95)} ms` : '—');
const yes = (b: boolean | undefined) => (b ? 'yes' : 'no');
const db = (v: number | null | undefined) => (typeof v === 'number' ? `${v.toFixed(1)} dB` : '—');

function lang(obs: Json): string {
  const d = obs?.decision;
  if (!d) return '—';
  return d.decision === 'identified' ? `${d.language} (${obs.source?.source ?? ''}, evidence ${obs.evidence})` : `abstained: ${d.reason}`;
}

function Phase1({ s }: { s: LabState }) {
  const h = s.health;
  const m = s.metrics ?? {};
  return (
    <Section title="Phase 1 · Runtime, sessions, health">
      <Grid>
        <Stat label="Runtime" value={h?.state} />
        <Stat label="Overall health" value={h?.overall ? String(h.overall.status ?? '') + (h.overall.reason ? ` (${h.overall.reason})` : '') : null} />
        <Stat label="Open sessions" value={h?.open_sessions} />
        <Stat label="This session" value={s.sessionState} />
        <Stat label="Events dispatched" value={m.events_dispatched} />
        <Stat label="Commands accepted / rejected" value={m.commands_accepted != null ? `${m.commands_accepted} / ${m.commands_rejected}` : null} />
        <Stat label="Audio streams opened" value={m.audio_streams_opened} />
        <Stat label="Analyses started / failed" value={m.analyses_started != null ? `${m.analyses_started} / ${m.analysis_failures}` : null} />
        <Stat label="Recognitions started / failed" value={m.recognitions_started != null ? `${m.recognitions_started} / ${m.recognition_failures}` : null} />
      </Grid>
    </Section>
  );
}

function Phase2({ s }: { s: LabState }) {
  const c = s.audio?.counters ?? {};
  const f = s.format ?? s.audio?.info?.format;
  return (
    <Section title="Phase 2 · Audio capture">
      <Grid>
        <Stat label="Stream state" value={s.streamState ?? s.audio?.state} />
        <Stat label="Device format" value={f?.device ? `${f.device.sample_rate} Hz · ${f.device.channels} ch · ${f.device.sample_format}` : null} />
        <Stat label="Delivered" value={f?.delivered ? `${f.delivered.sample_rate} Hz · ${f.delivered.sample_format}` : null} />
        <Stat label="Frames / blocks" value={c.frames != null ? `${c.frames} / ${c.blocks}` : null} />
        <Stat label="Ring overflow frames" value={c.ring_overflow_frames} hint="Audio lost because the capture ring was full" />
        <Stat label="Discontinuities" value={c.discontinuities} />
        <Stat label="Max ring fill" value={c.max_ring_fill_permille != null ? `${(c.max_ring_fill_permille / 10).toFixed(1)} %` : null} />
        <Stat label="Queue-dropped blocks" value={c.queue_dropped_blocks} />
        <Stat label="Device" value={s.audio?.info ? 'Talktual browser mic (network)' : null} />
      </Grid>
    </Section>
  );
}

function Phase3({ s }: { s: LabState }) {
  const a = s.analysis;
  const q = a?.quality;
  return (
    <Section title="Phase 3 · Audio intelligence">
      <div className="flex items-center gap-3 pt-3">
        <span
          aria-hidden="true"
          className={`h-3.5 w-3.5 rounded-full ${s.activity ? 'bg-[var(--accent)] shadow-[0_0_12px_var(--accent)]' : 'bg-[var(--glass-stroke)]'}`}
        />
        <span className="text-body font-[560]" role="status" aria-live="polite">
          {a ? (s.activity ? 'Voice activity' : 'No activity') : 'Analysis not running'}
        </span>
      </div>
      <Grid>
        <Stat label="Detector" value={a?.algorithm?.vad} />
        <Stat label="Mode" value={s.degraded ?? a?.mode} />
        <Stat label="Hops analysed" value={a?.counters?.hops} />
        <Stat label="Level" value={q ? `${q.level_dbfs.toFixed(1)} dBFS` : null} />
        <Stat label="SNR (estimate)" value={db(q?.snr_db)} />
        <Stat label="Noise floor" value={q?.noise_floor_dbfs != null ? `${q.noise_floor_dbfs.toFixed(1)} dBFS` : null} />
        <Stat label="Usability" value={q ? `${q.usability}${q.reasons?.length ? ` (${q.reasons.join(', ')})` : ''}` : null} />
        <Stat label="Segments closed" value={a?.counters?.segments} />
        <Stat label="Clipping" value={q ? `${(q.clipping_ratio * 100).toFixed(2)} %` : null} />
      </Grid>
      <div className="flex flex-wrap gap-1.5 pb-3">
        {(a?.capabilities?.conditions ?? []).map((c: string) => (
          <Chip key={c} on={s.conditions.includes(c)}>
            {c.replace(/_/g, ' ')}
          </Chip>
        ))}
      </div>
      {s.segments.length > 0 && (
        <ul className="py-2 text-label text-fg-muted" aria-label="Recent segments">
          {s.segments.slice(0, 6).map((g) => (
            <li key={`${g.segment}-${g.durationMs}`}>
              Segment {g.segment}: {g.durationMs} ms, closed by {g.reason.replace(/_/g, ' ')}
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function Phase4({ s }: { s: LabState }) {
  const r = s.recognition;
  const c = r?.counters ?? {};
  const caps = r?.capabilities?.recognizer ?? s.info?.recognizers?.[0];
  return (
    <Section title="Phase 4 · Speech recognition & language">
      <div className="py-3">
        <p className="t-eyebrow pb-1.5 text-fg-subtle">Live transcript (model output)</p>
        <p className="min-h-[3.5rem] rounded-[18px] bg-[var(--glass-top)] px-4 py-3 text-body-lg" aria-live="polite">
          <span className="font-[600] text-fg">{s.committed}</span> <span className="text-fg-muted">{s.provisional}</span>
          {!s.committed && !s.provisional && <span className="text-fg-subtle">{s.running ? 'Listening…' : 'Start a test to see partial and committed text.'}</span>}
        </p>
      </div>
      {s.finals.length > 0 && (
        <ol className="space-y-2 py-3" aria-label="Final utterances">
          {s.finals.slice(0, 8).map((f) => (
            <li key={`${f.utterance}-${f.revisions}-${f.text}`} className="text-body">
              <span className={f.status === 'final' ? 'text-fg' : 'text-fg-subtle line-through'}>{f.text || '(empty)'}</span>
              <span className="block text-label text-fg-muted">
                #{f.utterance} · {f.status} ({f.reason.replace(/_/g, ' ')}) · {f.revisions} revisions · {f.durationMs} ms audio · language {f.language ?? '—'}
                {f.amended ? ' · amended committed text' : ''}
              </span>
            </li>
          ))}
        </ol>
      )}
      <Grid>
        <Stat label="Recognizer" value={s.ready?.adapter ?? caps?.adapter} />
        <Stat label="Model" value={s.ready?.model ?? caps?.model?.id ?? null} />
        <Stat label="Delimitation" value={r?.hints ?? s.ready?.segmentation} />
        <Stat label="Utterances / finals" value={c.utterances != null ? `${c.utterances} / ${c.finals}` : null} />
        <Stat label="Revisions" value={c.revisions ?? s.revisions} />
        <Stat label="First partial p50 / p95" value={ms(c.first_partial_ms)} hint="Audio time from utterance start" />
        <Stat label="Final delay p50 / p95" value={ms(c.final_delay_ms)} hint="Audio time after the end of speech" />
        <Stat label="Processing per block" value={ms(c.processing_ms)} />
        <Stat label="Model load" value={c.load_ms != null ? `${c.load_ms} ms` : null} />
        <Stat label="Mode" value={r?.mode} />
        <Stat label="Dropped / discontinuities" value={c.tap_dropped_blocks != null ? `${c.tap_dropped_blocks} / ${c.discontinuities}` : null} />
        <Stat label="Stream language" value={lang(s.streamLanguage ?? r?.stream_language)} />
      </Grid>
      {caps && (
        <div className="flex flex-wrap gap-1.5 pb-3" aria-label="Recognizer capabilities">
          <Chip on={caps.streaming}>streaming</Chip>
          <Chip on={caps.partial_results}>partials</Chip>
          <Chip on={caps.endpointing}>endpointing</Chip>
          <Chip on={caps.punctuation}>punctuation: {yes(caps.punctuation)}</Chip>
          <Chip on={caps.casing}>casing: {yes(caps.casing)}</Chip>
          <Chip on={caps.word_timestamps}>word timings: {yes(caps.word_timestamps)}</Chip>
          <Chip on={Boolean(caps.score_semantics)}>scores: {caps.score_semantics ?? 'none'}</Chip>
          <Chip>code-switching: {yes(r?.capabilities?.code_switching)}</Chip>
        </div>
      )}
    </Section>
  );
}

function Phase5({ s, onConfirm, onResetContext }: { s: LabState; onConfirm: (id: string) => void; onResetContext: () => void }) {
  const st = s.semantic;
  const c = st?.counters ?? {};
  const current = s.observations.filter((o) => o.status !== 'retracted');
  const retracted = s.observations.filter((o) => o.status === 'retracted');
  return (
    <Section title="Phase 5 · Semantic & contextual interpretation">
      <p className="py-3 text-label text-fg-muted">
        Interpretations are hypotheses from deterministic English rules running in the bridge, linked to the transcript revision they came from. They are not facts until you confirm one. Context lives only for this test session.
      </p>
      {s.observations.length === 0 ? (
        <p className="pb-3 text-body text-fg-subtle">{st ? 'No observations yet. Try “set a timer for twenty five minutes” or “the total is forty dollars … can you split it”.' : 'Turn on “Interpret meaning” and start a test.'}</p>
      ) : (
        <ol className="space-y-2 py-3" aria-label="Semantic observations">
          {[...current, ...retracted].slice(0, 20).map((o) => (
            <li key={o.id} className="flex items-start justify-between gap-3 text-body">
              <span className={o.status === 'retracted' ? 'text-fg-subtle line-through' : 'text-fg'}>
                <span className="font-[600]">{o.label}</span>
                {o.mention && <> · “{o.mention}”</>}
                {o.value && <> · {o.value}</>}
                <span className="block text-label text-fg-muted">
                  #{o.utterance} rev {o.revision} · {o.status} · {o.epistemic.replace(/_/g, ' ')} · certainty {o.certainty} · v{o.version} ({o.change})
                </span>
              </span>
              {o.kind === 'entity' && o.status !== 'retracted' && o.epistemic !== 'user_confirmed_fact' && (
                <Button size="sm" variant="ghost" onClick={() => onConfirm(o.id)} aria-label={`Confirm ${o.label}`}>
                  Confirm
                </Button>
              )}
            </li>
          ))}
        </ol>
      )}
      <Grid>
        <Stat label="Adapter" value={st ? `${st.capabilities?.adapter} ${st.capabilities?.version}` : null} />
        <Stat label="Runs" value={st?.capabilities?.locality?.replace(/_/g, ' ')} hint="Where transcript text is processed" />
        <Stat label="Context items" value={st?.context ? `${st.context.items} (${st.context.bytes} B)` : s.contextItems || null} />
        <Stat label="Inputs / processed / coalesced" value={c.inputs != null ? `${c.inputs} / ${c.processed} / ${c.coalesced}` : null} />
        <Stat label="New / updated / retracted" value={c.observations_new != null ? `${c.observations_new} / ${c.updated} / ${c.retracted}` : null} />
        <Stat label="Stale rejected" value={c.stale_rejected} />
        <Stat label="Processing per revision" value={ms(c.processing_ms)} />
        <Stat label="First observation" value={ms(c.first_observation_ms)} hint="Audio time from utterance start" />
        <Stat label="Config" value={st?.config_fingerprint} />
      </Grid>
      <div className="flex flex-wrap gap-2 pb-4">
        <Button size="md" disabled={!s.running || !st} onClick={onResetContext}>
          Clear context
        </Button>
      </div>
    </Section>
  );
}

const TOPICS = ['all', 'session', 'audio', 'analysis', 'speech', 'semantic'] as const;
type Topic = (typeof TOPICS)[number];

function EventLog({ s }: { s: LabState }) {
  const [topic, setTopic] = useState<Topic>('all');
  const shown = useMemo(
    () => s.events.filter((e) => topic === 'all' || (topic === 'session' ? e.topic.startsWith('session') : e.topic === topic)),
    [s.events, topic]
  );
  return (
    <Section title="CE event log (this session, newest first)">
      <div className="py-3">
        <Segmented<Topic> label="Event topic" value={topic} onChange={setTopic} options={TOPICS.map((t) => ({ value: t, label: t[0].toUpperCase() + t.slice(1) }))} />
      </div>
      <ol className="max-h-[22rem] overflow-y-auto py-2 font-mono text-[0.78rem] leading-5" aria-label="Events">
        {shown.length === 0 && <li className="text-fg-subtle">No events yet.</li>}
        {shown.map((e) => (
          <li key={e.sequence} className="flex gap-2 text-fg-muted">
            <span className="w-10 shrink-0 text-right tabular-nums text-fg-subtle">{e.sequence}</span>
            <span className="w-20 shrink-0 text-fg">{e.topic}</span>
            <span className="w-40 shrink-0">{e.kind}</span>
            <span className={`shrink-0 ${e.privacy === 'content' ? 'text-accent-text' : ''}`}>[{e.privacy}]</span>
            <span className="min-w-0 truncate">{e.summary}</span>
          </li>
        ))}
      </ol>
    </Section>
  );
}

/** The Engine Lab page. `deps` lets tests inject a fake bridge and microphone. */
export function EnginePage({ language, deps }: { language: { code: string; bcp47: string }; deps?: LabDeps }) {
  const lab = useEngineLab(deps);
  const s = lab.state;
  const [url, setUrl] = useState(CE_BRIDGE_URL);
  const [mode, setMode] = useState<LabMode>('push-to-talk');
  const [analysis, setAnalysis] = useState(true);
  const [semantics, setSemantics] = useState(true);
  const connected = s.connection === 'connected';
  const english = language.code.toLowerCase().startsWith('en');
  const startOpts = { mode, analysis: mode === 'hands-free' || analysis, lang: english ? language.bcp47 : 'en-US', semantics };

  return (
    <Page wide eyebrow="Communication Engine" title="Engine Lab" description="Test every engine phase on this computer: runtime, audio capture, audio intelligence, speech recognition and language, and semantic interpretation.">
      <Section title="Bridge">
        <div className="flex flex-col gap-3 py-3 sm:flex-row sm:items-end">
          <label className="min-w-0 flex-1">
            <span className="text-label text-fg-subtle">Bridge address</span>
            <input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              className="mt-1 h-11 w-full rounded-full bg-[var(--glass-top)] px-4 text-body text-fg outline-none focus:ring-2 focus:ring-[var(--accent)]"
              spellCheck={false}
              aria-label="Bridge address"
            />
          </label>
          {connected ? (
            <Button size="sm" onClick={lab.disconnect}>
              Disconnect
            </Button>
          ) : (
            <Button size="sm" variant="primary" onClick={() => lab.connect(url)} disabled={s.connection === 'connecting'}>
              {s.connection === 'connecting' ? 'Connecting…' : 'Connect'}
            </Button>
          )}
        </div>
        <p className="pb-3 text-label text-fg-muted" role="status">
          {connected ? `Connected · ${s.info?.recognizers?.length ?? 0} recognizer(s) registered` : s.connection === 'connecting' ? 'Connecting…' : 'Not connected'}
        </p>
      </Section>

      {s.error && (
        <div className="pt-5">
          <Banner tone="error">{s.error}</Banner>
        </div>
      )}
      {!english && (
        <div className="pt-5">
          <Banner>The speech model is English only, so the lab tests in English (en-US) whatever your Talktual language is.</Banner>
        </div>
      )}

      <Section title="Test">
        <div className="py-3">
          <Segmented<LabMode>
            label="How to listen"
            value={mode}
            onChange={setMode}
            options={[
              { value: 'push-to-talk', label: 'Push to talk' },
              { value: 'hands-free', label: 'Hands-free' },
            ]}
          />
        </div>
        <Switch
          label="Run audio intelligence (Phase 3)"
          description={mode === 'hands-free' ? 'Always on in hands-free mode: it finds where phrases start and end.' : 'Activity, segments, quality and conditions while you talk.'}
          checked={mode === 'hands-free' || analysis}
          disabled={mode === 'hands-free' || s.running}
          onChange={setAnalysis}
        />
        <Switch
          label="Interpret meaning (Phase 5)"
          description="Numbers, times, dates, amounts, references like “it” and the kind of utterance, on the bridge with on-device rules."
          checked={semantics}
          disabled={s.running}
          onChange={setSemantics}
        />
        <p className="py-3 text-label text-fg-muted">
          Your microphone is used only while you hold the button (or between Start and Stop). Audio goes to the bridge on this computer, which transcribes it with an on-device English model. Nothing is stored unless the bridge was started with <code>--log-finals</code>.
        </p>
        <div className="flex flex-wrap gap-2 pb-4">
          {mode === 'push-to-talk' ? (
            <Button
              variant="primary"
              disabled={!connected}
              onPointerDown={() => void lab.start(startOpts)}
              onPointerUp={lab.stop}
              onPointerLeave={() => s.running && lab.stop()}
              onKeyDown={(e) => (e.key === ' ' || e.key === 'Enter') && !e.repeat && void lab.start(startOpts)}
              onKeyUp={(e) => (e.key === ' ' || e.key === 'Enter') && lab.stop()}
            >
              {s.running ? 'Listening… release to finish' : 'Hold to talk'}
            </Button>
          ) : s.running ? (
            <Button variant="primary" onClick={lab.stop}>
              Stop listening
            </Button>
          ) : (
            <Button variant="primary" disabled={!connected} onClick={() => void lab.start(startOpts)}>
              Start listening
            </Button>
          )}
          <Button size="md" disabled={!s.running} onClick={lab.reset}>
            Reset recognizer
          </Button>
          <Button size="md" variant="danger" disabled={!s.running} onClick={lab.abort}>
            Abort
          </Button>
          <Button size="md" variant="ghost" disabled={!connected} onClick={lab.refreshHealth}>
            Refresh health
          </Button>
        </div>
        {s.warnings.length > 0 && <p className="pb-3 text-label text-fg-muted">Warnings: {s.warnings.join(', ')}</p>}
      </Section>

      <Phase5 s={s} onConfirm={lab.confirm} onResetContext={lab.resetContext} />
      <Phase4 s={s} />
      <Phase3 s={s} />
      <Phase2 s={s} />
      <Phase1 s={s} />
      <EventLog s={s} />
    </Page>
  );
}
