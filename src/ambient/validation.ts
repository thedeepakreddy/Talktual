// Phase 8.5 — privacy-safe validation instrumentation for Live translate.
//
// Purpose: let the device owner run the real-device test matrix and export
// stage timings and reliability counts WITHOUT any speech content.
//
// Guarantees (enforced by the API shape and tested):
//   * No text, audio, language names, device labels or other identifiers: the
//     recorder's methods accept only per-session sequence ids ("u3"), enums
//     and numbers. There is no method that takes a string of user content.
//   * Opt-in: created only when the page is opened with ?validate=1 (or
//     ?debug=1). Nothing is persisted; clear() drops everything.
//   * Bounded: ring buffer of events; per-unit timing records are capped.
//   * Monotonic clock (performance.now), so timings survive wall-clock changes.
//
// Timing kinds (reported separately, never mixed):
//   measured        — both ends are app events on the same clock
//   engine-reported — the recognizer's own speech-end event (VAD) as the start;
//                     only as good as the engine's endpointing
//   human-marked    — the tester tapped "Mark end of speech" when the speaker
//                     stopped; includes human reaction time (~150–300 ms)
// End-to-end speech→audio is ONLY reported from engine-reported or
// human-marked starts. A recognizer "final" is not the end of speech.

export type ValidationEventKind =
  | 'session_start'
  | 'asr_start'
  | 'asr_stop'
  | 'asr_restart'
  | 'asr_error'
  | 'speech_start'
  | 'speech_end'
  | 'partial'
  | 'final'
  | 'commit'
  | 'translate_req'
  | 'translate_res'
  | 'queue_admit'
  | 'queue_drop'
  | 'playback_start'
  | 'playback_end'
  | 'interruption'
  | 'route_change'
  | 'cancel'
  | 'marker'
  | 'teardown';

const SAFE = /^[a-z0-9_:.-]{1,40}$/;

export interface ValidationEvent {
  t: number; // ms since recorder start (monotonic)
  k: ValidationEventKind;
  id?: string; // per-session unit sequence id only
  v?: string; // enum value (status, reason, kind)
  n?: number;
}

interface UnitTiming {
  firstPartial?: number;
  speechEnd?: number; // engine-reported
  marker?: number; // human-marked
  final?: number;
  commit?: number;
  translateReq?: number;
  translateRes?: number;
  queued?: number;
  playStart?: number;
  playEnd?: number;
  early?: boolean;
}

export interface MetricSummary {
  n: number;
  p50: number;
  p90: number;
  p95: number;
  max: number;
}

const pct = (s: number[], p: number) => s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
function summarize(values: number[]): MetricSummary | null {
  const v = values.filter((x) => Number.isFinite(x) && x >= 0);
  if (!v.length) return null;
  const s = [...v].sort((a, b) => a - b);
  return { n: s.length, p50: r(pct(s, 50)), p90: r(pct(s, 90)), p95: r(pct(s, 95)), max: r(s[s.length - 1]) };
}
const r = (x: number) => Math.round(x * 10) / 10;

export interface ValidationRecorderOptions {
  now?: () => number;
  maxEvents?: number;
  maxUnits?: number;
}

export class ValidationRecorder {
  private readonly now: () => number;
  private readonly t0: number;
  private events: ValidationEvent[] = [];
  private units = new Map<string, UnitTiming>();
  private unitIds = new Map<string, string>(); // session unit id → "uN"
  private seq = 0;
  private counters: Record<string, number> = {};
  private openUtterance: { firstPartial?: number; speechEnd?: number; marker?: number; partials: number } = { partials: 0 };
  private lastMarker: number | null = null;
  private queueDepth = 0;
  private maxQueueDepth = 0;
  private readonly maxEvents: number;
  private readonly maxUnits: number;
  private startedAt: number;

  constructor(opts: ValidationRecorderOptions = {}) {
    this.now = opts.now ?? (() => (typeof performance !== 'undefined' ? performance.now() : Date.now()));
    this.t0 = this.now();
    this.startedAt = this.t0;
    this.maxEvents = opts.maxEvents ?? 3000;
    this.maxUnits = opts.maxUnits ?? 500;
  }

  // ---- recording (ids/enums/numbers only) ----------------------------------

  private ev(k: ValidationEventKind, id?: string, v?: string, n?: number): number {
    const t = this.now() - this.t0;
    const e: ValidationEvent = { t: r(t), k };
    if (id) e.id = id;
    if (v && SAFE.test(v)) e.v = v;
    if (n !== undefined && Number.isFinite(n)) e.n = n;
    this.events.push(e);
    if (this.events.length > this.maxEvents) this.events.shift();
    this.counters[k] = (this.counters[k] ?? 0) + 1;
    if (v && SAFE.test(v)) this.counters[`${k}:${v}`] = (this.counters[`${k}:${v}`] ?? 0) + 1;
    return t;
  }

  private sid(unitId: string): string {
    let s = this.unitIds.get(unitId);
    if (!s) {
      s = `u${++this.seq}`;
      this.unitIds.set(unitId, s);
      if (this.unitIds.size > this.maxUnits) {
        const oldest = this.unitIds.keys().next().value as string;
        const gone = this.unitIds.get(oldest)!;
        this.unitIds.delete(oldest);
        this.units.delete(gone);
      }
    }
    return s;
  }

  private unit(unitId: string): UnitTiming {
    const s = this.sid(unitId);
    let u = this.units.get(s);
    if (!u) {
      u = {};
      this.units.set(s, u);
    }
    return u;
  }

  sessionStart(): void {
    this.ev('session_start');
  }
  asrStart(): void {
    this.ev('asr_start');
  }
  asrStop(reason: string): void {
    this.ev('asr_stop', undefined, reason);
    // An utterance that never produced a final is "missing final" evidence.
    if (this.openUtterance.partials > 0) this.counters.utterances_without_final = (this.counters.utterances_without_final ?? 0) + 1;
    this.openUtterance = { partials: 0 };
  }
  asrRestart(reason: string): void {
    this.ev('asr_restart', undefined, reason);
  }
  asrError(code: string): void {
    this.ev('asr_error', undefined, code);
  }
  speech(active: boolean): void {
    const t = this.ev(active ? 'speech_start' : 'speech_end');
    if (!active) this.openUtterance.speechEnd = t;
  }
  partial(): void {
    const t = this.now() - this.t0;
    if (this.openUtterance.partials === 0) {
      this.openUtterance.firstPartial = t;
      this.ev('partial'); // first interim of an utterance only (counts below)
    }
    this.openUtterance.partials++;
    this.counters.partials_total = (this.counters.partials_total ?? 0) + 1;
  }
  final(): void {
    this.ev('final', undefined, undefined, this.openUtterance.partials);
    if (this.openUtterance.partials === 0) this.counters.finals_without_partials = (this.counters.finals_without_partials ?? 0) + 1;
    this.openUtterance = { partials: 0 };
  }
  /** The tester tapped "Mark end of speech". Applies to the next committed unit. */
  marker(): void {
    this.lastMarker = this.ev('marker');
  }
  commit(unitId: string, kind: 'early' | 'final'): void {
    const t = this.ev('commit', this.sid(unitId), kind);
    const u = this.unit(unitId);
    u.commit = t;
    u.early = kind === 'early';
    // Attribute utterance-level timestamps to the unit that closes the utterance.
    const last = this.events.slice(-40).reverse();
    const fin = last.find((e) => e.k === 'final');
    if (kind === 'final' && fin) u.final = fin.t;
    const sp = [...this.events].reverse().find((e) => e.k === 'speech_end' && e.t <= t && t - e.t < 15000);
    if (kind === 'final' && sp) u.speechEnd = sp.t;
    if (this.lastMarker !== null && t >= this.lastMarker && t - this.lastMarker < 15000) {
      u.marker = this.lastMarker;
      this.lastMarker = null;
    }
  }
  translateReq(unitId: string): void {
    this.unit(unitId).translateReq = this.ev('translate_req', this.sid(unitId));
  }
  translateRes(unitId: string, status: string, location: string): void {
    this.unit(unitId).translateRes = this.ev('translate_res', this.sid(unitId), `${status}:${location}`);
  }
  queueAdmit(unitId: string, depth: number): void {
    this.unit(unitId).queued = this.ev('queue_admit', this.sid(unitId), undefined, depth);
    this.queueDepth = depth;
    this.maxQueueDepth = Math.max(this.maxQueueDepth, depth);
  }
  queueDrop(unitId: string, reason: string): void {
    this.ev('queue_drop', this.sid(unitId), reason);
  }
  playback(unitId: string, status: string): void {
    const u = this.unit(unitId);
    if (status === 'started') u.playStart = this.ev('playback_start', this.sid(unitId));
    else {
      const t = this.ev('playback_end', this.sid(unitId), status);
      if (status === 'ended') u.playEnd = t;
    }
  }
  interruption(reason: string): void {
    this.ev('interruption', undefined, reason);
  }
  routeChange(inputs: number, outputs: number): void {
    // Counts of device KINDS only — never labels or ids.
    this.ev('route_change', undefined, `in${Math.min(inputs, 9)}_out${Math.min(outputs, 9)}`);
  }
  /** Safety mechanisms that fired (enum only). */
  note(what: 'echo_dropped' | 'half_duplex_hold' | 'speech_deferred'): void {
    this.counters[what] = (this.counters[what] ?? 0) + 1;
  }
  cancel(n: number): void {
    this.ev('cancel', undefined, undefined, n);
  }
  teardown(): void {
    this.ev('teardown');
  }

  clear(): void {
    this.events = [];
    this.units.clear();
    this.unitIds.clear();
    this.counters = {};
    this.openUtterance = { partials: 0 };
    this.lastMarker = null;
    this.queueDepth = 0;
    this.maxQueueDepth = 0;
    this.startedAt = this.now();
  }

  // ---- export -----------------------------------------------------------------

  /** Content-free summary for the owner to export (JSON). */
  summary(env: { browser?: string; mobile?: boolean } = {}) {
    const us = [...this.units.values()];
    const d = (a: keyof UnitTiming, b: keyof UnitTiming, filter: (u: UnitTiming) => boolean = () => true) =>
      summarize(us.filter((u) => filter(u) && typeof u[a] === 'number' && typeof u[b] === 'number').map((u) => (u[b] as number) - (u[a] as number)));
    const durationMin = (this.now() - this.startedAt) / 60000;
    const c = this.counters;
    const rate = (n: number | undefined) => (durationMin > 0 ? r((n ?? 0) / durationMin) : null);
    return {
      schema: 'talktual.ambient.validation.v1',
      content: 'none — ids, enums and timings only',
      env: { browser: env.browser ?? 'unknown', mobile: env.mobile ?? null },
      durationMin: r(durationMin),
      measured: {
        first_partial_to_commit_ms: d('firstPartial', 'commit'),
        recognizer_final_to_commit_ms: d('final', 'commit'),
        translate_ms: d('translateReq', 'translateRes'),
        commit_to_text_ms: d('commit', 'translateRes'),
        queue_wait_ms: d('queued', 'playStart'),
        translated_to_audio_start_ms: d('translateRes', 'playStart'),
        playback_duration_ms: d('playStart', 'playEnd'),
        recognizer_final_to_audio_start_ms: d('final', 'playStart'),
      },
      engineReported: {
        note: 'start = recognizer speech-end (VAD) event; quality depends on the engine’s endpointing',
        speech_end_to_final_ms: d('speechEnd', 'final'),
        speech_end_to_text_ms: d('speechEnd', 'translateRes'),
        speech_end_to_audio_start_ms: d('speechEnd', 'playStart'),
      },
      humanMarked: {
        note: 'start = tester tapped “Mark end of speech”; includes human reaction time (~150–300 ms)',
        mark_to_text_ms: d('marker', 'translateRes'),
        mark_to_audio_start_ms: d('marker', 'playStart'),
      },
      reliability: {
        units: us.length,
        early_units: us.filter((u) => u.early).length,
        asr_restarts: c.asr_restart ?? 0,
        asr_restarts_per_min: rate(c.asr_restart),
        asr_errors: c.asr_error ?? 0,
        utterances_without_final: c.utterances_without_final ?? 0,
        finals_without_partials: c.finals_without_partials ?? 0,
        translate_failed: Object.entries(c).filter(([k]) => k.startsWith('translate_res:failed') || k.startsWith('translate_res:untranslated')).reduce((n, [, v]) => n + v, 0),
        queue_drops: c.queue_drop ?? 0,
        playback_failed: c['playback_end:failed'] ?? 0,
        playback_dropped: c['playback_end:dropped'] ?? 0,
        max_queue_depth: this.maxQueueDepth,
        interruptions: Object.fromEntries(Object.entries(c).filter(([k]) => k.startsWith('interruption:')).map(([k, v]) => [k.slice(13), v])),
        route_changes: c.route_change ?? 0,
        cancels: c.cancel ?? 0,
        echo_dropped: c.echo_dropped ?? 0,
        half_duplex_holds: c.half_duplex_hold ?? 0,
      },
      counters: { ...c },
      events: this.events.map((e) => ({ ...e })),
    };
  }
}

/** Coarse browser family for test context (no UA string, no version beyond family). */
export function coarseBrowser(ua: string): { browser: string; mobile: boolean } {
  const mobile = /iPhone|iPad|Android|Mobile/i.test(ua);
  const browser = /Edg\//.test(ua) ? 'edge' : /Firefox\//.test(ua) ? 'firefox' : /CriOS|Chrome\//.test(ua) ? 'chrome' : /Safari\//.test(ua) ? 'safari' : 'other';
  return { browser, mobile };
}

/** Opt-in: only when the page was opened with ?validate=1 or ?debug=1. */
export function validationEnabled(search: string): boolean {
  try {
    const p = new URLSearchParams(search);
    return p.get('validate') === '1' || p.get('debug') === '1';
  } catch {
    return false;
  }
}
