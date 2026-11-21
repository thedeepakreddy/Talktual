import { describe, it, expect, vi } from 'vitest';
import { VoiceAnalyzer } from './analyzer';
import { toDescriptor, isVoiceDescriptor, toRenderHints, descriptorF0Hz, MIN_CONFIDENCE } from './descriptor';
import { VoiceMirrorController, isVoiceWireMessage } from './controller';
import { FakeVoiceTap, synthVoice } from '../test/engineFakes';
import type { VoiceDescriptor } from './types';

const st = (a: number, b: number) => Math.abs(12 * Math.log2(a / b));

function analyze(x: Float32Array, rate: number) {
  const a = new VoiceAnalyzer(rate);
  for (let i = 0; i < x.length; i += 960) a.push(x.subarray(i, i + 960));
  return a;
}

describe('VoiceAnalyzer', () => {
  it.each([80, 120, 180, 240, 320])('estimates median pitch of a %i Hz voice-like signal within 0.5 semitone (16k and 48k)', (f0) => {
    for (const rate of [16000, 48000]) {
      const p = analyze(synthVoice(f0, 3, rate), rate).profile();
      expect(st(p.f0MedianHz, f0)).toBeLessThan(0.5);
    }
  });

  it('tempo estimate rises with syllable rate (monotonic)', () => {
    const rates = [3, 4, 5, 6].map((hz) => analyze(synthVoice(150, 4, 16000, hz), 16000).profile().syllableRate);
    for (let i = 1; i < rates.length; i++) expect(rates[i]).toBeGreaterThan(rates[i - 1]);
  });

  it('fails closed on silence and noise: no confident profile, no descriptor', () => {
    const silence = new Float32Array(16000 * 3);
    const noise = Float32Array.from({ length: 16000 * 3 }, () => (Math.random() - 0.5) * 0.1);
    for (const x of [silence, noise]) {
      const p = analyze(x, 16000).profile();
      expect(p.confidence).toBeLessThan(MIN_CONFIDENCE);
      expect(toDescriptor(p)).toBeNull();
    }
  });

  it('needs enough speech: 0.4 s yields no descriptor; accumulating across pushes does', () => {
    const a = new VoiceAnalyzer(16000);
    a.push(synthVoice(150, 0.4));
    expect(toDescriptor(a.profile())).toBeNull();
    for (let i = 0; i < 6; i++) a.push(synthVoice(150, 0.6)); // several short turns
    expect(toDescriptor(a.profile())).not.toBeNull();
  });

  it('memory is bounded: state size is independent of how long someone talks', () => {
    expect(VoiceAnalyzer.stateBytes(48000)).toBe(VoiceAnalyzer.stateBytes(48000));
    const a = new VoiceAnalyzer(16000);
    const before = JSON.stringify(Object.keys(a));
    for (let i = 0; i < 30; i++) a.push(synthVoice(150, 1)); // 30 s
    expect(JSON.stringify(Object.keys(a))).toBe(before);
    expect(VoiceAnalyzer.stateBytes(16000)).toBeLessThan(4096);
  });

  it('reset() forgets everything', () => {
    const a = analyze(synthVoice(150, 3), 16000);
    expect(a.profile().voicedSeconds).toBeGreaterThan(1);
    a.reset();
    expect(a.profile()).toMatchObject({ voicedSeconds: 0, f0MedianHz: 0, confidence: 0 });
  });
});

describe('descriptor (what is transmitted)', () => {
  const prof = { f0MedianHz: 150, f0IqrSemitones: 2.5, syllableRate: 5.9, voicedSeconds: 3, confidence: 0.9 };

  it('is coarse: 2-semitone pitch steps, three pitch-range classes, 5% tempo steps; integers/enums only', () => {
    const d = toDescriptor(prof)!;
    expect(d).toEqual({ v: 1, pitchStep: 18, pitchRange: 'normal', tempoPct: 100 });
    // Nearby voices map to the same descriptor (not a fingerprint of the exact voice).
    expect(toDescriptor({ ...prof, f0MedianHz: 154, syllableRate: 6.0 })).toEqual(d);
    expect(JSON.stringify(d).length).toBeLessThan(80);
  });

  it('clamps tempo to 80–125% and pitch to the step range', () => {
    expect(toDescriptor({ ...prof, syllableRate: 30 })!.tempoPct).toBe(125);
    expect(toDescriptor({ ...prof, syllableRate: 1 })!.tempoPct).toBe(80);
    expect(toDescriptor({ ...prof, f0MedianHz: 2000 })!.pitchStep).toBe(44);
  });

  it('low confidence → no descriptor (fallback to the default voice)', () => {
    expect(toDescriptor({ ...prof, confidence: MIN_CONFIDENCE - 0.01 })).toBeNull();
  });

  it.each([
    null,
    { v: 2, pitchStep: 18, pitchRange: 'normal', tempoPct: 100 },
    { v: 1, pitchStep: 17, pitchRange: 'normal', tempoPct: 100 },
    { v: 1, pitchStep: 46, pitchRange: 'normal', tempoPct: 100 },
    { v: 1, pitchStep: 18, pitchRange: 'wild', tempoPct: 100 },
    { v: 1, pitchStep: 18, pitchRange: 'normal', tempoPct: 103 },
    { v: 1, pitchStep: 18, pitchRange: 'normal', tempoPct: 100, embedding: [0.1, 0.2] },
  ])('validator rejects %j', (d) => {
    expect(isVoiceDescriptor(d)).toBe(false);
  });

  it('render hints are clamped to safe ranges for every possible descriptor', () => {
    for (let pitchStep = 0; pitchStep <= 44; pitchStep += 2) {
      for (let tempoPct = 80; tempoPct <= 125; tempoPct += 5) {
        const h = toRenderHints({ v: 1, pitchStep, pitchRange: 'normal', tempoPct });
        expect(h.browserPitch).toBeGreaterThanOrEqual(0.6);
        expect(h.browserPitch).toBeLessThanOrEqual(1.6);
        expect(h.browserRate).toBeGreaterThanOrEqual(0.8);
        expect(h.browserRate).toBeLessThanOrEqual(1.25);
        expect(h.ttsSpeed).toBeGreaterThanOrEqual(0.8);
        expect(h.ttsSpeed).toBeLessThanOrEqual(1.2);
      }
    }
  });

  it('register follows pitch', () => {
    const at = (hz: number) => toRenderHints({ v: 1, pitchStep: Math.round((12 * Math.log2(hz / 55)) / 2) * 2, pitchRange: 'normal', tempoPct: 100 });
    expect(at(110).register).toBe('low');
    expect(at(170).register).toBe('mid');
    expect(at(240).register).toBe('high');
    expect(descriptorF0Hz({ v: 1, pitchStep: 24, pitchRange: 'flat', tempoPct: 100 })).toBeCloseTo(220);
  });
});

describe('VoiceMirrorController', () => {
  function make() {
    const tap = new FakeVoiceTap();
    const sent: unknown[] = [];
    const diag: string[] = [];
    const onChange = vi.fn();
    const c = new VoiceMirrorController({ tap, send: (m) => sent.push(m), diag: (s) => diag.push(s), onChange });
    return { c, tap, sent, diag, onChange };
  }
  const settle = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const desc: VoiceDescriptor = { v: 1, pitchStep: 20, pitchRange: 'normal', tempoPct: 105 };

  it('off by default: capture does not open the tap', async () => {
    const { c, tap } = make();
    c.onCaptureStart();
    await settle();
    expect(tap.opens).toBe(0);
    expect(c.holdsVoiceData()).toBe(false);
  });

  it('opted in: tap open only during capture; descriptor sent once enough speech is analysed', async () => {
    const { c, tap, sent } = make();
    c.optIn();
    c.onCaptureStart();
    await settle();
    expect(tap.open).toBe(true);
    tap.feed(synthVoice(200, 3), 16000);
    c.onCaptureStop();
    expect(tap.open).toBe(false);
    expect(sent).toHaveLength(1);
    expect(isVoiceWireMessage(sent[0])).toBe(true);
    // Unchanged voice → not re-sent on the next push.
    c.onCaptureStart();
    await settle();
    tap.feed(synthVoice(200, 1), 16000);
    c.onCaptureStop();
    expect(sent).toHaveLength(1);
  });

  it('too little speech: descriptor withheld (default voice used)', async () => {
    const { c, tap, sent, diag } = make();
    c.optIn();
    c.onCaptureStart();
    await settle();
    tap.feed(synthVoice(200, 0.3), 16000);
    c.onCaptureStop();
    expect(sent).toHaveLength(0);
    expect(diag).toContain('descriptor-withheld');
  });

  it('race: capture stops before the tap finishes opening → the tap is closed immediately', async () => {
    const { c, tap } = make();
    c.optIn();
    c.onCaptureStart();
    c.onCaptureStop(); // released before getUserMedia resolved
    await settle();
    expect(tap.opens).toBe(1);
    expect(tap.closes).toBe(1);
    expect(tap.open).toBe(false);
  });

  it('revocation: analysis stops, local data deleted, peer told to delete', async () => {
    const { c, tap, sent } = make();
    c.optIn();
    c.onCaptureStart();
    await settle();
    tap.feed(synthVoice(200, 3), 16000);
    c.onCaptureStop();
    c.onCaptureStart();
    await settle();
    c.optOut();
    expect(tap.open).toBe(false);
    expect(sent.at(-1)).toEqual({ t: 'voice', revoked: true });
    expect(c.holdsVoiceData()).toBe(false);
    expect(c.state).toEqual({ sharing: false, descriptorSent: false, peerActive: false });
  });

  it('peer descriptor: validated, used for hints; revoke deletes it; malformed is rejected', () => {
    const { c, diag } = make();
    expect(c.peerHints()).toBeUndefined();
    c.onPeerMessage({ t: 'voice', descriptor: desc });
    expect(c.peerHints()?.targetF0Hz).toBeCloseTo(descriptorF0Hz(desc));
    c.onPeerMessage({ t: 'voice', descriptor: { ...desc, pitchStep: 999 } });
    expect(diag).toContain('peer-rejected');
    expect(c.peerHints()).toBeDefined(); // a bad message doesn't wipe a good descriptor
    c.onPeerMessage({ t: 'voice', revoked: true });
    expect(c.peerHints()).toBeUndefined();
  });

  it('reconnect: re-sends the current descriptor (the outbox does not keep voice messages)', async () => {
    const { c, tap, sent } = make();
    c.onConnected();
    expect(sent).toHaveLength(0);
    c.optIn();
    c.onCaptureStart();
    await settle();
    tap.feed(synthVoice(200, 3), 16000);
    c.onCaptureStop();
    c.onConnected();
    expect(sent).toHaveLength(2);
    expect(sent[1]).toEqual(sent[0]);
  });

  it('dispose clears everything, is idempotent, and ignores later input', async () => {
    const { c, tap } = make();
    c.optIn();
    c.onPeerMessage({ t: 'voice', descriptor: desc });
    c.onCaptureStart();
    await settle();
    c.dispose();
    c.dispose();
    expect(tap.open).toBe(false);
    expect(c.holdsVoiceData()).toBe(false);
    c.onPeerMessage({ t: 'voice', descriptor: desc });
    c.optIn();
    expect(c.holdsVoiceData()).toBe(false);
    expect(c.state.sharing).toBe(false);
  });

  it('tap failure is reported and nothing is held', async () => {
    const { c, tap, diag } = make();
    tap.failNext = true;
    c.optIn();
    c.onCaptureStart();
    await settle();
    expect(diag).toContain('tap-failed');
    expect(tap.open).toBe(false);
  });
});
