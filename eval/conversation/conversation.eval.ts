// Runs every scenario and writes metrics JSON.
//   EVAL_LABEL=baseline npx vitest run --config vitest.eval.config.ts
// Results: eval/conversation/results/<label>.json (+ printed table).

import { it, vi, afterAll } from 'vitest';
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { SCENARIOS, runScenario, normText } from './scenarios';

const LABEL = process.env.EVAL_LABEL || 'run';
const OUT = path.join(__dirname, 'results');
const norm = normText;
const pct = (a: number[], p: number) => (a.length ? [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor((p / 100) * a.length))] : null);

const results: Record<string, unknown>[] = [];

for (const s of SCENARIOS) {
  it(s.id, async () => {
    vi.useFakeTimers({ now: new Date('2026-11-21T00:00:00Z') });
    try {
      const { sim, marks } = await runScenario(s);
      const listener = sim[s.listener];
      const speaker = s.listener === 'B' ? sim.A : sim.B;
      const log = listener.playback.log;
      const spoken = log.filter((r) => r.startedAt !== undefined);
      const spokenTexts = spoken.map((r) => norm(r.text));
      const ideal = s.ideal.map(norm);
      const stale = (s.stale ?? []).map(norm);

      const idealHeard = ideal.filter((t) => spokenTexts.includes(t)).length;
      const staleSpoken = spokenTexts.filter((t) => stale.includes(t)).length;
      const counts = new Map<string, number>();
      spokenTexts.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1));
      const duplicates = [...counts.values()].reduce((a, c) => a + Math.max(0, c - 1), 0);
      // Fragments: spoken units that are a strict part of an ideal unit (premature split).
      const fragments = spokenTexts.filter((t) => !ideal.includes(t) && !stale.includes(t) && ideal.some((i) => i.includes(t) && i !== t)).length;
      // Merged: one spoken unit carrying 2+ ideal units (merge-under-pressure).
      const isMerged = (t: string) => !ideal.includes(t) && ideal.filter((i) => t.includes(i)).length >= 2;
      const merged = spokenTexts.filter(isMerged).length;
      const unexpected = spokenTexts.filter((t) => !ideal.includes(t) && !stale.includes(t) && !isMerged(t) && !ideal.some((i) => i.includes(t))).length;
      // Covered: ideal content heard exactly or inside a merged unit.
      const idealCovered = ideal.filter((i) => spokenTexts.some((t) => t === i || (isMerged(t) && t.includes(i)))).length;

      const firstStartAfter = (t: number) => {
        const r = spoken.find((x) => x.startedAt! - sim.t0 >= t);
        return r ? r.startedAt! - sim.t0 - t : null;
      };
      const firstCancelAfter = (t: number) => {
        const c = log.filter((x) => x.cancelledAt !== undefined && x.cancelledAt - sim.t0 >= t);
        return c.length ? Math.min(...c.map((x) => x.cancelledAt! - sim.t0)) - t : null;
      };
      const queueWaits = spoken.map((r) => r.startedAt! - r.queuedAt);

      // Outbound protocol facts (sender side)
      const segs = speaker.sent.filter((m) => m.t === 'segment') as Array<Record<string, unknown>>;
      const provisional = segs.filter((x) => x.state === 'provisional');
      const finals = segs.filter((x) => x.state === undefined || x.state === 'final');
      const provIdx = segs.findIndex((x) => x.state === 'provisional');
      const firstProvisionalSentAt = provIdx >= 0 ? speaker.sentAt[speaker.sent.indexOf(segs[provIdx] as never)] : null;
      const firstSaid = sim.said.size ? Math.min(...sim.said.values()) : null;
      // Lag: recognizer finalized a phrase → listener starts hearing it (per spoken unit).
      // Per finalized phrase: a merged unit contributes one lag PER phrase it carries
      // (so merging can't hide the wait of its earliest phrase).
      const lags = spoken.flatMap((r) => {
        const t = norm(r.text);
        const exact = sim.said.get(t);
        const times = exact !== undefined ? [exact] : [...sim.said.entries()].filter(([k]) => t.includes(k)).map(([, v]) => v);
        return times.map((saidAt) => r.startedAt! - sim.t0 - saidAt);
      });

      const diag = speaker.session.getDiagnostics();
      const listenerDiag = listener.session.getDiagnostics();
      const diagDump = JSON.stringify([diag, listenerDiag]);
      // Leak check on word PAIRS (single words collide with status names like "translated").
      const bigrams = new Set(
        s.ideal.concat(s.stale ?? []).flatMap((t) => {
          const w = norm(t).split(' ');
          return w.slice(1).map((x, i) => `${w[i]} ${x}`);
        })
      );
      const leakedWords = [...bigrams].filter((b) => diagDump.toLowerCase().includes(b));

      const row = {
        scenario: s.id,
        matrixRow: s.matrixRow,
        idealUnits: ideal.length,
        spokenUnits: spokenTexts.length,
        idealHeard,
        idealCovered,
        merged,
        fragments,
        unexpected,
        staleSpoken,
        duplicates,
        droppedAsStale: log.filter((r) => r.dropped).length,
        finalsSent: finals.length,
        provisionalSent: provisional.length,
        translationCalls: sim.translations.calls,
        translationsAborted: sim.translations.aborted,
        maxPlaybackQueue: listener.playback.maxDepth,
        maxQueueWaitMs: queueWaits.length ? Math.max(...queueWaits) : 0,
        timeToFirstSpeechAfterEndMs: marks.speechEnd !== undefined ? firstStartAfter(marks.speechEnd) : marks.speechEnd1 !== undefined ? firstStartAfter(marks.speechEnd1) : null,
        interruptionStopMs: marks.interrupt !== undefined ? firstCancelAfter(marks.interrupt) : null,
        playedDuringOverlap: marks.overlapStart !== undefined ? spoken.filter((r) => r.startedAt! - sim.t0 >= marks.overlapStart && r.startedAt! - sim.t0 < marks.bReleased).length : null,
        workAfterEnd: marks.end !== undefined ? log.filter((r) => r.queuedAt - sim.t0 > marks.end).length : null,
        diagnosticsTextLeaks: leakedWords.length,
        spoken: spokenTexts,
        marks,
        timeToFirstProvisionalMs: firstProvisionalSentAt !== null && firstSaid !== null ? firstProvisionalSentAt - firstSaid : null,
        lagMedianMs: pct(lags, 50),
        lagMaxMs: lags.length ? Math.max(...lags) : null,
      };
      results.push(row);
      sim.A.session.dispose();
      sim.B.session.dispose();
    } finally {
      vi.useRealTimers();
    }
  });
}

afterAll(() => {
  mkdirSync(OUT, { recursive: true });
  writeFileSync(path.join(OUT, `${LABEL}.json`), JSON.stringify({ label: LABEL, generatedAt: new Date().toISOString(), node: process.version, results }, null, 1));
  const cols = ['scenario', 'lagMedianMs', 'lagMaxMs', 'timeToFirstProvisionalMs', 'idealUnits', 'spokenUnits', 'idealHeard', 'idealCovered', 'merged', 'fragments', 'staleSpoken', 'duplicates', 'unexpected', 'finalsSent', 'provisionalSent', 'maxPlaybackQueue', 'maxQueueWaitMs', 'timeToFirstSpeechAfterEndMs', 'interruptionStopMs', 'playedDuringOverlap', 'workAfterEnd', 'diagnosticsTextLeaks'];
  console.log('\n' + cols.join('\t'));
  for (const r of results) console.log(cols.map((c) => String((r as Record<string, unknown>)[c] ?? '—')).join('\t'));
});
