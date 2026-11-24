import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { initialLabState, labReducer, MAX_EVENTS, type LabState } from './labState';
import { EnginePage } from '../pages/EnginePage';
import type { WebSocketLike } from '../lib/ceSpeech';

vi.mock('../lib/audio', () => ({ getMic: vi.fn(), setMicEnabled: vi.fn(), audioContext: vi.fn() }));

/** A real message sequence recorded from the bridge (scripted adapter, synthetic signal). */
const RECORDED = readFileSync(join(__dirname, '__fixtures__/bridge-session.jsonl'), 'utf8')
  .trim()
  .split('\n')
  .map((l) => JSON.parse(l));

/** A real bridge session with Phase 5 semantics (scripted ASR: "set a timer for twenty five minutes"). */
const RECORDED_SEMANTIC = readFileSync(join(__dirname, '__fixtures__/bridge-semantic-session.jsonl'), 'utf8')
  .trim()
  .split('\n')
  .map((l) => JSON.parse(l));

const INFO = {
  type: 'info',
  protocol: 1,
  recognizers: [{ adapter: 'ce.asr.scripted', streaming: true, partial_results: true, endpointing: true, punctuation: false, casing: false, word_timestamps: false, score_semantics: null, model: null }],
  health: { state: 'running', overall: { status: 'healthy' }, open_sessions: 0, metrics: { events_dispatched: 3, recognitions_started: 0 } },
};

function replay(msgs: unknown[], from: LabState = initialLabState): LabState {
  return msgs.reduce<LabState>((s, msg, i) => labReducer(s, { type: 'message', msg, at: i }), labReducer(from, { type: 'session-start' }));
}

describe('Engine Lab state (recorded bridge session)', () => {
  it('fills a slice for every phase from real bridge messages', () => {
    const s = replay([INFO, { type: 'ready', adapter: 'ce.asr.scripted', model: null, segmentation: 'continuous' }, ...RECORDED]);
    // Phase 1
    expect(s.health.state).toBe('running');
    expect(s.sessionState).toBe('active');
    expect(s.metrics.recognitions_started).toBeGreaterThanOrEqual(1);
    // Phase 2
    expect(s.streamState).toBe('closed');
    expect(s.format.device.sample_format).toBe('i16');
    expect(s.audio.counters.frames).toBeGreaterThan(0);
    // Phase 3
    expect(s.segments[0]).toMatchObject({ segment: 0, reason: 'stream_end' });
    expect(s.analysis.algorithm.vad).toBe('ce.vad.spectral');
    expect(s.activity).toBe(false); // ended inactive
    // Phase 4
    expect(s.revisions).toBeGreaterThanOrEqual(4);
    expect(s.finals[0]).toMatchObject({ status: 'cancelled', reason: 'reset', text: 'hello' });
    expect(s.running).toBe(false);
    // Event log: newest first, content flagged, no transcript text in summaries.
    expect(s.events[0].sequence).toBeGreaterThan(s.events[s.events.length - 1].sequence);
    const tr = s.events.find((e) => e.kind === 'transcript')!;
    expect(tr.privacy).toBe('content');
    expect(s.events.map((e) => e.summary).join(' ')).not.toContain('hello');
  });

  it('keeps the event log bounded and handles partials, conditions and errors', () => {
    let s = labReducer(initialLabState, { type: 'session-start' });
    for (let i = 0; i < MAX_EVENTS + 50; i++) {
      s = labReducer(s, { type: 'message', at: i, msg: { type: 'ce-event', sequence: i, topic: 'analysis', kind: 'condition_changed', privacy: 'operational', data: { condition: 'clipping', active: i % 2 === 0 } } });
    }
    expect(s.events).toHaveLength(MAX_EVENTS);
    expect(s.conditions).toEqual([]); // last event switched it off
    s = labReducer(s, { type: 'message', at: 0, msg: { type: 'ce-event', sequence: 1, topic: 'speech', kind: 'transcript', privacy: 'content', data: { status: 'partial', revision: 0, text: { committed: 'GOOD', provisional: 'MORNING' }, meta: { utterance: 0 } } } });
    expect([s.committed, s.provisional]).toEqual(['GOOD', 'MORNING']);
    s = labReducer(s, { type: 'message', at: 0, msg: { type: 'error', code: 'language-unsupported', message: 'x', fatal: true } });
    expect(s.error).toContain('language-unsupported');
    expect(s.running).toBe(false);
  });
});

describe('Engine Lab state (Phase 5, recorded bridge session)', () => {
  it('tracks observations by id through new → updated → finalized and retraction', () => {
    const s = replay(RECORDED_SEMANTIC);
    const duration = s.observations.find((o) => o.label === 'duration')!;
    expect(duration).toMatchObject({ status: 'final', epistemic: 'observation', mention: 'twenty five minutes', value: '1500 s', change: 'finalized' });
    // The provisional "twenty" number was retracted when the duration appeared.
    expect(s.observations.find((o) => o.label === 'number')?.status).toBe('retracted');
    const act = s.observations.find((o) => o.kind === 'act')!;
    expect(act.label).toBe('act: request');
    expect(act.epistemic).toBe('hypothesis');
    // One line per observation id (latest version).
    expect(new Set(s.observations.map((o) => o.id)).size).toBe(s.observations.length);
    expect(s.semantic.capabilities.adapter).toBe('ce.semantic.rules');
    // The event log flags observations as content and never repeats their text.
    const obs = s.events.filter((e) => e.topic === 'semantic' && e.kind === 'observation');
    expect(obs.length).toBeGreaterThan(0);
    expect(obs.every((e) => e.privacy === 'content')).toBe(true);
    expect(s.events.map((e) => e.summary).join(' ')).not.toMatch(/twenty|minutes/i);
    expect(s.events.find((e) => e.kind === 'context')?.privacy).toBe('operational');
  });
});

class FakeSocket implements WebSocketLike {
  binaryType = 'blob';
  readyState = 0;
  sent: (string | ArrayBufferLike)[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  send(d: string | ArrayBufferLike) {
    this.sent.push(d);
  }
  close() {
    this.readyState = 3;
  }
  push(m: unknown) {
    this.onmessage?.({ data: JSON.stringify(m) });
  }
}

describe('Engine Lab page', () => {
  it('connects, runs a push-to-talk test and shows all four phases', async () => {
    const sock = new FakeSocket();
    const capture = { sampleRate: 48000, stop: vi.fn() };
    render(<EnginePage language={{ code: 'en', bcp47: 'en-US' }} deps={{ openSocket: () => sock, startCapture: async () => capture }} />);
    expect(screen.getByRole('heading', { name: 'Engine Lab' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    act(() => {
      sock.readyState = 1;
      sock.onopen?.();
    });
    expect(JSON.parse(sock.sent[0] as string)).toEqual({ type: 'info' });
    act(() => sock.push(INFO));
    expect(screen.getByText(/Connected · 1 recognizer/)).toBeTruthy();

    await act(async () => {
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Hold to talk' }));
    });
    const start = JSON.parse(sock.sent[1] as string);
    expect(start).toMatchObject({ type: 'start', lang: 'en-US', sampleRate: 48000, segmentation: 'continuous', diagnostics: true, analysis: true, semantics: true });
    act(() => {
      sock.push({ type: 'ready', adapter: 'ce.asr.scripted', model: null, segmentation: 'continuous' });
      RECORDED.forEach((m) => sock.push(m));
    });
    for (const title of [/Phase 1/, /Phase 2/, /Phase 3/, /Phase 4/, /CE event log/]) {
      expect(screen.getByRole('heading', { name: title })).toBeTruthy();
    }
    expect(screen.getByText('ce.vad.spectral')).toBeTruthy();
    expect(within(screen.getByRole('list', { name: 'Final utterances' })).getByText(/cancelled \(reset\)/)).toBeTruthy();
    expect(screen.getAllByText(/16000 Hz/).length).toBeGreaterThan(0);
    expect(screen.getByText(/healthy/)).toBeTruthy();
    expect(screen.getAllByText('transcript').length).toBeGreaterThan(0);
  });

  it('shows Phase 5 observations and sends confirmations and context resets', async () => {
    const sock = new FakeSocket();
    const capture = { sampleRate: 16000, stop: vi.fn() };
    render(<EnginePage language={{ code: 'en', bcp47: 'en-US' }} deps={{ openSocket: () => sock, startCapture: async () => capture }} />);
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    act(() => {
      sock.readyState = 1;
      sock.onopen?.();
      sock.push(INFO);
    });
    await act(async () => {
      fireEvent.pointerDown(screen.getByRole('button', { name: 'Hold to talk' }));
    });
    act(() => {
      sock.push({ type: 'ready', adapter: 'ce.asr.scripted', model: null, segmentation: 'continuous' });
      // Everything up to (not including) the end, so the session is still running.
      RECORDED_SEMANTIC.filter((m) => m.type !== 'end').forEach((m) => sock.push(m));
    });
    expect(screen.getByRole('heading', { name: /Phase 5/ })).toBeTruthy();
    const list = screen.getByRole('list', { name: 'Semantic observations' });
    expect(within(list).getByText(/twenty five minutes/)).toBeTruthy();
    expect(within(list).getByText(/1500 s/)).toBeTruthy();
    fireEvent.click(within(list).getByRole('button', { name: 'Confirm duration' }));
    const sent = sock.sent.filter((m): m is string => typeof m === 'string').map((m) => JSON.parse(m));
    const confirm = sent.find((m) => m.type === 'confirm');
    expect(confirm.observation).toMatch(/^u0\.entity\./);
    fireEvent.click(screen.getByRole('button', { name: 'Clear context' }));
    expect(JSON.parse(sock.sent[sock.sent.length - 1] as string)).toEqual({ type: 'reset-context' });
  });

  it('explains when the bridge cannot be reached', () => {
    const sock = new FakeSocket();
    render(<EnginePage language={{ code: 'es', bcp47: 'es-ES' }} deps={{ openSocket: () => sock }} />);
    expect(screen.getByText(/English only/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    act(() => sock.onerror?.());
    expect(screen.getByRole('alert').textContent).toMatch(/not reachable/);
  });
});
