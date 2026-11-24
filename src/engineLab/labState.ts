// Engine Lab state: a pure reducer over messages from the Talktual ↔ CE bridge
// (protocol v1 + diagnostics). Each Communication Engine phase gets its own
// slice so the page can show what every layer is doing:
//   Phase 1 — runtime, sessions, health, metrics, events
//   Phase 2 — the capture stream (state, negotiated format, counters)
//   Phase 3 — audio intelligence (activity, segments, endpoints, quality, conditions)
//   Phase 4 — recognition (partials, commits, finals) and language observations

/* eslint-disable @typescript-eslint/no-explicit-any */
type Json = any;

export interface LabEvent {
  sequence: number;
  topic: string;
  kind: string;
  privacy: string;
  at: number;
  summary: string;
}

export interface FinalLine {
  utterance: number;
  text: string;
  reason: string;
  language: string | null;
  durationMs: number;
  revisions: number;
  amended: boolean;
  status: 'final' | 'cancelled';
}

export interface SegmentLine {
  segment: number;
  durationMs: number;
  reason: string;
}

export interface LabState {
  connection: 'idle' | 'connecting' | 'connected' | 'error';
  error: string | null;
  // Phase 1
  info: { recognizers: Json[]; health: Json | null } | null;
  health: Json | null;
  metrics: Json | null;
  sessionState: string | null;
  events: LabEvent[];
  // Phase 2
  streamState: string | null;
  format: Json | null;
  audio: Json | null;
  // Phase 3
  analysis: Json | null;
  activity: boolean;
  segments: SegmentLine[];
  endpoints: { tier: number; silenceMs: number; confidence: string }[];
  conditions: string[];
  degraded: string | null;
  // Phase 4
  running: boolean;
  ready: { adapter: string; model: string | null; segmentation: string } | null;
  committed: string;
  provisional: string;
  revisions: number;
  finals: FinalLine[];
  streamLanguage: Json | null;
  recognition: Json | null;
  warnings: string[];
}

export const MAX_EVENTS = 200;
const MAX_LINES = 50;

export const initialLabState: LabState = {
  connection: 'idle',
  error: null,
  info: null,
  health: null,
  metrics: null,
  sessionState: null,
  events: [],
  streamState: null,
  format: null,
  audio: null,
  analysis: null,
  activity: false,
  segments: [],
  endpoints: [],
  conditions: [],
  degraded: null,
  running: false,
  ready: null,
  committed: '',
  provisional: '',
  revisions: 0,
  finals: [],
  streamLanguage: null,
  recognition: null,
  warnings: [],
};

export type LabAction =
  | { type: 'connecting' }
  | { type: 'connected' }
  | { type: 'disconnected'; error?: string }
  | { type: 'session-start' }
  | { type: 'message'; msg: Json; at: number };

function decision(obs: Json): string {
  const d = obs?.decision;
  if (!d) return '—';
  return d.decision === 'identified' ? d.language : `abstained (${d.reason})`;
}

/** One-line, content-free summary for the event log (no transcript text). */
export function summarize(topic: string, kind: string, d: Json): string {
  switch (kind) {
    case 'session_state_changed':
      return `${d.from} → ${d.to}`;
    case 'stream_state_changed':
      return `${d.from ?? 'new'} → ${d.to}`;
    case 'stream_format_resolved':
      return `${d.format?.device?.sample_rate} Hz ${d.format?.device?.sample_format} → ${d.format?.delivered?.sample_rate} Hz ${d.format?.delivered?.sample_format}`;
    case 'activity_changed':
      return `${d.state}${d.score != null ? ` (score ${Number(d.score).toFixed(2)})` : ''}, decided after ${d.decision_latency_ms} ms`;
    case 'segment_started':
      return `segment ${d.segment}${d.continuation ? ' (continuation)' : ''}`;
    case 'segment_ended':
      return `segment ${d.segment}: ${d.duration_ms} ms, ${d.reason}`;
    case 'endpoint_candidate':
      return `tier ${d.tier} (${d.confidence}) after ${d.silence_ms} ms silence`;
    case 'silence_window':
      return `${d.duration_ms} ms at ${Number(d.mean_level_dbfs).toFixed(0)} dBFS`;
    case 'quality_changed':
      return `${d.metrics?.usability}${d.metrics?.reasons?.length ? ` (${d.metrics.reasons.join(', ')})` : ''}`;
    case 'condition_changed':
      return `${d.condition} ${d.active ? 'on' : 'off'}`;
    case 'degraded_mode_changed':
      return `${d.from} → ${d.to}`;
    case 'policy_changed':
      return `${String(d.decision).replace(/_/g, ' ')} ${d.active ? 'applied' : 'reverted'}`;
    case 'model_ready':
      return `loaded in ${d.load_ms} ms${d.reused ? ' (shared model)' : ''}`;
    case 'utterance_started':
      return `utterance ${d.meta?.utterance}`;
    case 'transcript':
      return `utterance ${d.meta?.utterance} rev ${d.revision} ${d.status}${d.final_reason ? ` (${d.final_reason})` : ''}`;
    case 'language_observed':
      return `stream language: ${decision(d.observation)}`;
    case 'reset':
    case 'hints_changed':
    case 'mode_changed':
      return d.reason ?? d.hints ?? `${d.from} → ${d.to}`;
    case 'discontinuity':
      return `${d.context?.context ?? 'gap'}, resumes at frame ${d.resume_frame}`;
    case 'queue_pressure':
      return `backlog ${d.backlog}, dropped ${d.dropped_total}`;
    case 'failed':
      return d.error?.code ?? 'failed';
    case 'stopped':
      return d.reason ?? '';
    default:
      return topic === kind ? '' : topic;
  }
}

function ceEvent(s: LabState, m: Json, at: number): LabState {
  const d = m.data ?? {};
  const ev: LabEvent = { sequence: m.sequence, topic: m.topic, kind: m.kind, privacy: m.privacy, at, summary: summarize(m.topic, m.kind, d) };
  let next: LabState = { ...s, events: [ev, ...s.events].slice(0, MAX_EVENTS) };
  switch (m.kind) {
    case 'session_state_changed':
      next.sessionState = d.to;
      break;
    case 'stream_state_changed':
      next.streamState = d.to;
      break;
    case 'stream_format_resolved':
      next.format = d.format;
      break;
    case 'activity_changed':
      next.activity = d.state === 'active';
      break;
    case 'segment_ended':
      next.segments = [{ segment: d.segment, durationMs: d.duration_ms, reason: d.reason }, ...s.segments].slice(0, MAX_LINES);
      break;
    case 'endpoint_candidate':
      next.endpoints = [{ tier: d.tier, silenceMs: d.silence_ms, confidence: d.confidence }, ...s.endpoints].slice(0, MAX_LINES);
      break;
    case 'condition_changed':
      next.conditions = d.active ? Array.from(new Set([...s.conditions, d.condition])) : s.conditions.filter((c) => c !== d.condition);
      break;
    case 'degraded_mode_changed':
      next.degraded = d.to;
      break;
    case 'language_observed':
      next.streamLanguage = d.observation;
      break;
    case 'transcript': {
      next.revisions = s.revisions + 1;
      if (d.status !== 'partial') {
        const line: FinalLine = {
          utterance: d.meta?.utterance ?? 0,
          text: d.text?.committed ?? '',
          reason: d.final_reason ?? '',
          language: d.language ? decision(d.language) : null,
          durationMs: Math.round(((d.meta?.time_range?.[1] ?? 0) - (d.meta?.time_range?.[0] ?? 0)) / 1e6),
          revisions: (d.revision ?? 0) + 1,
          amended: Boolean(d.commit_amended),
          status: d.status === 'final' ? 'final' : 'cancelled',
        };
        next.finals = [line, ...s.finals].slice(0, MAX_LINES);
        next.committed = '';
        next.provisional = '';
      } else {
        next.committed = d.text?.committed ?? '';
        next.provisional = d.text?.provisional ?? '';
      }
      break;
    }
    default:
      break;
  }
  return next;
}

export function labReducer(s: LabState, a: LabAction): LabState {
  switch (a.type) {
    case 'connecting':
      return { ...s, connection: 'connecting', error: null };
    case 'connected':
      return { ...s, connection: 'connected' };
    case 'disconnected':
      return { ...s, connection: a.error ? 'error' : 'idle', error: a.error ?? null, running: false, activity: false };
    case 'session-start':
      return {
        ...s,
        running: true,
        ready: null,
        committed: '',
        provisional: '',
        revisions: 0,
        segments: [],
        endpoints: [],
        conditions: [],
        degraded: null,
        activity: false,
        streamState: null,
        format: null,
        audio: null,
        analysis: null,
        recognition: null,
        streamLanguage: null,
        warnings: [],
        error: null,
      };
    case 'message': {
      const m = a.msg;
      switch (m?.type) {
        case 'info':
          return { ...s, info: { recognizers: m.recognizers ?? [], health: m.health ?? null }, health: m.health ?? null, metrics: m.health?.metrics ?? s.metrics };
        case 'health':
          return { ...s, health: m.health, metrics: m.health?.metrics ?? s.metrics };
        case 'ready':
          return { ...s, ready: { adapter: m.adapter, model: m.model ?? null, segmentation: m.segmentation } };
        case 'ce-event':
          return ceEvent(s, m, a.at);
        case 'status':
          return { ...s, audio: m.audio ?? s.audio, analysis: m.analysis ?? s.analysis, recognition: m.recognition ?? s.recognition, metrics: m.metrics ?? s.metrics };
        case 'warning':
          return { ...s, warnings: [m.kind, ...s.warnings].slice(0, 10) };
        case 'error':
          return { ...s, error: `${m.code}: ${m.message}`, running: m.fatal ? false : s.running };
        case 'end':
          return { ...s, running: false, activity: false };
        default:
          return s; // utterance / partial / final / activity duplicates of ce-events
      }
    }
    default:
      return s;
  }
}
