// Long-session stability: push a very long conversation through one session
// and check that every buffer stays bounded. Timing is printed (informational)
// and only loosely asserted — it runs on shared CI hardware.

import { describe, it, expect } from 'vitest';
import { makeSession, connected, settle, FakeAsr } from '../test/engineFakes';

describe('long session (M2.6 performance)', () => {
  it('20,000 inbound + 2,000 outbound segments: transcript, dedupe, diagnostics and listeners stay bounded', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const listenersBefore = h.session.events.listenerCount();

    const t0 = performance.now();
    for (let i = 0; i < 20_000; i++) {
      h.call.receive({ t: 'segment', id: `in-${i}`, original: 'hola', translated: `hello ${i}`, sourceLang: 'es-ES', targetLang: 'en-US', sentAt: i });
      h.playback.finish();
    }
    const inboundMs = performance.now() - t0;

    const t1 = performance.now();
    for (let i = 0; i < 200; i++) {
      h.session.pressTalk();
      await settle(3);
      for (let j = 0; j < 10; j++) FakeAsr.all.at(-1)!.say(`out ${i}-${j}`);
      h.session.releaseTalk();
      await settle(30);
    }
    // Wait for the serialized translate→send chain to drain.
    for (let guard = 0; h.session.getSnapshot().translating > 0 && guard < 10_000; guard++) await settle(20);
    const outboundMs = performance.now() - t1;

    const snap = h.session.getSnapshot();
    const diag = h.session.getDiagnostics();
    expect(snap.segments.length).toBe(500);
    expect(diag.events.length).toBeLessThanOrEqual(300);
    for (const m of Object.values(diag.metrics)) expect(m.n).toBeLessThanOrEqual(200);
    expect(h.call.segments().length).toBe(2000);
    expect(h.playback.queue.length).toBe(0);
    expect(snap.translating).toBe(0);
    expect(h.session.events.listenerCount()).toBe(listenersBefore);
    // Only one recognizer instance is reused across 200 pushes.
    expect(FakeAsr.all.length).toBe(1);

    // A resend of an id older than the dedupe window is accepted again — the
    // documented limit of the bounded dedupe (500 ids).
    h.call.receive({ t: 'segment', id: 'in-19999', original: 'x', translated: 'x', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 0 });
    expect(diag.counters.duplicate_segments_dropped ?? 0).toBe(0);

    console.log(
      `long session: inbound 20k in ${inboundMs.toFixed(0)}ms (${((inboundMs / 20000) * 1000).toFixed(1)}µs/segment), ` +
        `outbound 2k in ${outboundMs.toFixed(0)}ms; diag p50 receive→playback ${diag.metrics.receive_to_playback_ms?.p50}ms`
    );
    expect(inboundMs / 20000).toBeLessThan(5); // < 5 ms per inbound segment even on slow CI
  }, 60_000);
});
