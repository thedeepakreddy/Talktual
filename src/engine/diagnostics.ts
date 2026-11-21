// Privacy-conscious diagnostics: a bounded ring buffer of redacted events plus
// latency samples and counters.
//
// Redaction is by ALLOWLIST. A field survives only if it is a number or
// boolean, or a string under one of the keys below whose value looks like an
// enum/id (short, no spaces). Transcript text, translations, names, audio and
// secrets therefore cannot be recorded even by accident.

const STRING_KEYS = new Set([
  'from',
  'to',
  'reason',
  'code',
  'status',
  'engine',
  'provider',
  'transport',
  'mode',
  'route',
  'kind',
  'segmentId',
  'state',
  'phase',
  'activity',
  'lang',
  'error',
]);
const SAFE_STRING = /^[A-Za-z0-9_.:\-]{1,64}$/;

export type DiagFields = Record<string, unknown>;
export interface DiagEntry {
  t: number;
  type: string;
  [k: string]: unknown;
}

export function redact(fields: DiagFields): Record<string, string | number | boolean> {
  const out: Record<string, string | number | boolean> = {};
  for (const [k, v] of Object.entries(fields)) {
    if (typeof v === 'number' && Number.isFinite(v)) out[k] = v;
    else if (typeof v === 'boolean') out[k] = v;
    else if (typeof v === 'string' && STRING_KEYS.has(k) && SAFE_STRING.test(v)) out[k] = v;
  }
  return out;
}

const pct = (sorted: number[], p: number) => sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];

export class Diagnostics {
  private ring: DiagEntry[] = [];
  private samples = new Map<string, number[]>();
  private counters = new Map<string, number>();

  constructor(
    private readonly maxEvents = 300,
    private readonly maxSamples = 200,
    private readonly now: () => number = () => Date.now()
  ) {}

  record(type: string, fields: DiagFields = {}): void {
    if (!SAFE_STRING.test(type)) return;
    this.ring.push({ t: this.now(), type, ...redact(fields) });
    if (this.ring.length > this.maxEvents) this.ring.splice(0, this.ring.length - this.maxEvents);
  }

  count(name: string, by = 1): void {
    if (!SAFE_STRING.test(name)) return;
    this.counters.set(name, (this.counters.get(name) ?? 0) + by);
  }

  sample(metric: string, ms: number): void {
    if (!SAFE_STRING.test(metric) || !Number.isFinite(ms)) return;
    const arr = this.samples.get(metric) ?? [];
    arr.push(ms);
    if (arr.length > this.maxSamples) arr.shift();
    this.samples.set(metric, arr);
  }

  snapshot() {
    const metrics: Record<string, { n: number; p50: number; p95: number; max: number }> = {};
    for (const [k, arr] of this.samples) {
      if (!arr.length) continue;
      const s = [...arr].sort((a, b) => a - b);
      metrics[k] = { n: s.length, p50: pct(s, 50), p95: pct(s, 95), max: s[s.length - 1] };
    }
    return {
      events: this.ring.map((e) => ({ ...e })),
      counters: Object.fromEntries(this.counters),
      metrics,
    };
  }
}
