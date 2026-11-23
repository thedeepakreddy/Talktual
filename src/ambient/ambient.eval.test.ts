// Phase 8 evaluation matrix (SIMULATED recognizer traces, deterministic clock).
//
// Each scenario drives AmbientSession with a recognizer EVENT TRACE that models
// a real-world condition (run-on speaker, revisions, late/missing finals,
// unstable overlap, noise fragments, failing/slow translation). It measures
// pipeline behaviour only — not recognition accuracy, audio, or devices.
// Data: hand-written Hungarian sentences (authored for this repository; no
// third-party corpus). Results: AMBIENT_EVAL_OUT=<file> writes JSON.

import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { AmbientSession, type AmbientDeps } from './AmbientSession';
import { ambientConsentKey } from './disclosure';
import { CHROME_LIKE, FakeAsr, FakeCapture, FakePlayback, memoryStorage } from '../test/engineFakes';
import type { TranslationRequest, TranslationResult } from '../engine/ports';

type Ev = { at: number; kind: 'partial' | 'final'; text: string } | { at: number; kind: 'end' };
interface Scenario {
  name: string;
  condition: string;
  trace: Ev[];
  headphones?: boolean;
  translateMs?: number;
  failEvery?: number;
  /** Words that must each appear exactly once across committed lines. */
  expectWords?: string;
}

/** Interims growing word by word every `step` ms, then a final `finalDelay` ms after the last interim. */
function utter(start: number, sentence: string, step = 250, finalDelay = 300, finalText = sentence): Ev[] {
  const w = sentence.split(' ');
  const evs: Ev[] = [];
  for (let n = 1; n <= w.length; n++) evs.push({ at: start + n * step, kind: 'partial', text: w.slice(0, n).join(' ') });
  evs.push({ at: start + w.length * step + finalDelay, kind: 'final', text: finalText });
  return evs;
}
const S1 = 'Elnézést hol van a legközelebbi metróállomás';
const S2 = 'Egyenesen kell menni aztán jobbra a második sarkon';
const RUN_ON = 'holnap reggel nyolc órakor találkozunk a pályaudvar előtt és utána elmegyünk együtt a nagy piacra mert ott minden friss és olcsó';

const SCENARIOS: Scenario[] = [
  { name: 'clean', condition: 'Two phrases with pauses and timely finals', trace: [...utter(0, S1), ...utter(4000, S2)], expectWords: `${S1} ${S2}` },
  { name: 'run-on', condition: 'One speaker, 22 words without a pause (final only at the end)', trace: utter(0, RUN_ON, 280, 400), expectWords: RUN_ON },
  {
    name: 'revision',
    condition: 'Recognizer rewrites an early word after it was stable',
    trace: [
      ...utter(0, 'a vonat tíz perc múlva indul a második vágányról ma este', 260).slice(0, -1),
      { at: 3600, kind: 'final', text: 'a busz tíz perc múlva indul a második vágányról ma este' },
    ],
  },
  { name: 'late-final', condition: 'Final arrives 1.8 s after the last interim (slow cloud recognizer)', trace: [...utter(0, S1, 250, 1800), ...utter(5000, S2, 250, 1800)], expectWords: `${S1} ${S2}` },
  {
    name: 'missing-final',
    condition: 'Engine ends without a final; the adapter promotes the last interim (reliability fix) as a final',
    trace: [...utter(0, S1, 250, 0).slice(0, -1), { at: 2100, kind: 'final', text: S1 }, ...utter(4000, S2)],
    expectWords: `${S1} ${S2}`,
  },
  {
    name: 'overlap-unstable',
    condition: 'Two people talking at once: the hypothesis keeps changing (no stable prefix)',
    trace: [
      ...Array.from({ length: 16 }, (_, i) => ({ at: i * 200, kind: 'partial' as const, text: i % 2 ? 'igen de én azt mondtam hogy holnap' : 'igen de te azt mondtad hogy ma' })),
      { at: 3600, kind: 'final', text: 'igen de én azt mondtam hogy holnap' },
    ],
  },
  {
    name: 'noise',
    condition: 'Café noise: filler fragments and a duplicate re-emitted after a recognizer restart',
    trace: [{ at: 100, kind: 'final', text: 'öö' }, { at: 400, kind: 'final', text: 'hm' }, ...utter(800, S1), { at: 3400, kind: 'final', text: S1 }],
    expectWords: S1,
  },
  { name: 'translation-failures', condition: 'Every 2nd translation fails', trace: [0, 1, 2, 3, 4, 5].flatMap((i) => utter(i * 3000, `${i}. mondat itt van most`)), failEvery: 2 },
  { name: 'slow-translation', condition: 'Translation takes 4 s per phrase while a new phrase arrives every second (overload)', trace: Array.from({ length: 14 }, (_, i) => utter(i * 1000, `${i}. gyors mondat`, 120, 100)).flat(), translateMs: 4000 },
  { name: 'speaker-output', condition: 'No headphones: half-duplex; phrases keep coming', trace: [...utter(0, S1), ...utter(3000, S2)], headphones: false, expectWords: `${S1} ${S2}` },
];

interface Result {
  scenario: string;
  condition: string;
  units: number;
  early: number;
  revised: number;
  translated: number;
  untranslated: number;
  skipped: number;
  spoken: number;
  outOfOrderSpeech: number;
  duplicateWords: number;
  missingWords: number;
  firstTextAfterSpeechStartMs: number | null;
  commitLagP50: number | null;
  textLagP50: number | null;
  speechLagMax: number | null;
}

const results: Result[] = [];
const norm = (s: string) => s.toLocaleLowerCase().replace(/[^\p{L}\p{N} ]/gu, '').split(/\s+/).filter(Boolean);

async function run(sc: Scenario): Promise<Result> {
  FakeAsr.all = [];
  const storage = memoryStorage();
  storage.setItem(ambientConsentKey('browser-speech'), 'granted');
  const playback = new FakePlayback();
  let n = 0;
  const translate = (req: TranslationRequest): Promise<TranslationResult> =>
    new Promise((res) => {
      const k = ++n;
      setTimeout(() => {
        if (sc.failEvery && k % sc.failEvery === 0) res({ segmentId: req.segmentId, status: 'failed', text: req.text, error: { code: 'http', message: 'x' }, attempts: 1, latencyMs: 1 });
        else res({ segmentId: req.segmentId, status: 'translated', text: `EN<${req.text}>`, attempts: 1, latencyMs: sc.translateMs ?? 120 });
      }, sc.translateMs ?? 120);
    });
  const deps: AmbientDeps = {
    createAsr: (lang, mode, cb) => new FakeAsr('webspeech', lang, mode, cb),
    translate,
    playback,
    capture: new FakeCapture(),
    browser: CHROME_LIKE,
    storage,
    getServerCapabilities: async () => ({ tts: false, stt: false, translator: 'mymemory', liveCaptions: false }),
    visibility: null,
    audioSession: null,
  };
  const s = new AmbientSession({ from: 'hu-HU', to: 'en-US', headphones: sc.headphones ?? true }, deps);
  s.start();
  await vi.advanceTimersByTimeAsync(0);
  const t0 = Date.now();
  // Playback finishes 1.2 s after it starts (models a short spoken phrase).
  playback.onStatus((_id, st) => {
    if (st === 'started') setTimeout(() => playback.finish(), 1200);
  });
  for (const ev of sc.trace) {
    const wait = t0 + ev.at - Date.now();
    if (wait > 0) await vi.advanceTimersByTimeAsync(wait);
    const asr = FakeAsr.all.filter((a) => a.active).at(-1);
    if (!asr) continue; // capture held (half-duplex): the room is not heard — counted as missing words
    if (ev.kind === 'end') continue;
    asr.say(ev.text, ev.kind === 'final');
  }
  await vi.advanceTimersByTimeAsync(15_000);
  const snap = s.getSnapshot();
  const lines = snap.lines;
  const order = lines.map((l) => l.id);
  const spokenIds = playback.played.map((p) => p.segmentId);
  let outOfOrder = 0;
  for (let i = 1; i < spokenIds.length; i++) if (order.indexOf(spokenIds[i]) < order.indexOf(spokenIds[i - 1])) outOfOrder++;
  const committed = norm(lines.map((l) => l.original).join(' '));
  let dup = 0;
  let missing = 0;
  if (sc.expectWords) {
    const want = norm(sc.expectWords);
    const counts = new Map<string, number>();
    committed.forEach((w) => counts.set(w, (counts.get(w) ?? 0) + 1));
    const wantCounts = new Map<string, number>();
    want.forEach((w) => wantCounts.set(w, (wantCounts.get(w) ?? 0) + 1));
    for (const [w, c] of wantCounts) {
      const got = counts.get(w) ?? 0;
      if (got > c) dup += got - c;
      if (got < c) missing += c - got;
    }
  }
  const d = s.diagnostics();
  const firstLine = lines[0];
  const r: Result = {
    scenario: sc.name,
    condition: sc.condition,
    units: lines.length,
    early: lines.filter((l) => l.early).length,
    revised: lines.filter((l) => l.revised).length,
    translated: lines.filter((l) => l.state === 'translated').length,
    untranslated: lines.filter((l) => l.state === 'untranslated').length,
    skipped: lines.filter((l) => l.state === 'skipped' || l.speech === 'skipped').length,
    spoken: playback.played.length,
    outOfOrderSpeech: outOfOrder,
    duplicateWords: dup,
    missingWords: missing,
    firstTextAfterSpeechStartMs: firstLine ? firstLine.heardAt - t0 + (d.metrics.amb_text_ms?.p50 ?? 0) : null,
    commitLagP50: d.metrics.amb_commit_ms?.p50 ?? null,
    textLagP50: d.metrics.amb_text_ms?.p50 ?? null,
    speechLagMax: d.metrics.amb_e2e_speech_ms?.max ?? null,
  };
  s.dispose();
  return r;
}

describe('Phase 8 evaluation matrix (simulated traces)', () => {
  beforeAll(() => {
    vi.useFakeTimers();
  });
  afterAll(() => {
    vi.useRealTimers();
    const out = process.env.AMBIENT_EVAL_OUT;
    if (out) {
      mkdirSync(dirname(out), { recursive: true });
      writeFileSync(out, JSON.stringify({ simulated: true, note: 'Deterministic recognizer event traces; pipeline behaviour only (no audio, no recognizer accuracy, no devices).', results }, null, 2) + '\n');
    }
  });

  for (const sc of SCENARIOS) {
    it(`${sc.name}: ${sc.condition}`, async () => {
      const r = await run(sc);
      results.push(r);
      // Invariants that must hold in every condition:
      expect(r.outOfOrderSpeech).toBe(0);
      expect(r.duplicateWords).toBe(0);
      if (sc.headphones !== false) expect(r.missingWords).toBe(0);
      // Condition-specific expectations:
      if (sc.name === 'run-on') expect(r.early).toBeGreaterThanOrEqual(2);
      if (sc.name === 'overlap-unstable') expect(r.early).toBe(0);
      if (sc.name === 'revision') expect(r.revised).toBeGreaterThanOrEqual(1);
      if (sc.name === 'noise') expect(r.units).toBe(1);
      if (sc.name === 'translation-failures') expect(r.untranslated).toBe(3);
      // Speech never starts more than the lag bound after the phrase was heard (+ the playback queue's own notAfter margin).
      if (r.speechLagMax !== null) expect(r.speechLagMax).toBeLessThanOrEqual(9000 + 4000);
      if (sc.name === 'slow-translation') {
        expect(r.skipped).toBeGreaterThan(0); // overload: the excess is shown (or skipped), not spoken late
        expect(r.spoken).toBeLessThan(r.units);
      }
    });
  }
});
