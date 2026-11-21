// Conversation scenarios (spec §6 evaluation matrix). Each scenario scripts both
// participants with explicit timings and declares ground truth:
//   ideal — what an ideal listener hears (original words, in order)
//   stale — content that must NOT be spoken (e.g. after the listener interrupted)
// Times are ms since the sim started; recognizer timing is scripted, so results
// are SIMULATED (deterministic), not measured on real speech.

import { vi } from 'vitest';
import { at, connectSim, createSim, Sim, SimOptions } from './sim';

export interface Marks {
  [name: string]: number;
}

export interface Scenario {
  id: string;
  matrixRow: string;
  listener: 'A' | 'B';
  ideal: string[];
  stale?: string[];
  opts?: SimOptions;
  run: (sim: Sim, marks: Marks) => Promise<void>;
}

const now = (sim: Sim) => Date.now() - sim.t0;

async function press(sim: Sim, who: 'A' | 'B') {
  sim[who].session.pressTalk();
  await vi.advanceTimersByTimeAsync(0);
}
function release(sim: Sim, who: 'A' | 'B') {
  sim[who].session.releaseTalk();
}
export const normText = (t: string) => t.replace(/[«»]/g, '').toLowerCase().replace(/[^\p{L}\p{N}' ]/gu, ' ').replace(/\s+/g, ' ').trim();

function say(sim: Sim, who: 'A' | 'B', text: string, final = true, confidence?: number) {
  const asr = sim[who].asr();
  if (!asr?.active) return;
  if (final && !sim.said.has(normText(text))) sim.said.set(normText(text), Date.now() - sim.t0);
  if (final) asr.cb.onFinal({ text, lang: asr.lang, startedAt: Date.now() - 500, endedAt: Date.now(), engine: 'webspeech', ...(confidence !== undefined ? { confidence } : {}) } as never);
  else asr.cb.onPartial(text);
}
async function interim(sim: Sim, who: 'A' | 'B', words: string[], startMs: number, gapMs = 180) {
  for (let i = 0; i < words.length; i++) {
    await at(sim, startMs + i * gapMs);
    say(sim, who, words.slice(0, i + 1).join(' '), false);
  }
}

export const SCENARIOS: Scenario[] = [
  {
    id: 'normal-alternating',
    matrixRow: 'Normal alternating conversation',
    listener: 'B',
    ideal: ['good morning how are you', 'shall we meet at noon'],
    async run(sim, m) {
      await press(sim, 'A');
      await interim(sim, 'A', 'good morning how are you'.split(' '), 1000);
      await at(sim, 2100);
      say(sim, 'A', 'good morning how are you');
      m.speechEnd1 = now(sim);
      await at(sim, 2600);
      await interim(sim, 'A', 'shall we meet at noon'.split(' '), 2600);
      await at(sim, 3600);
      say(sim, 'A', 'shall we meet at noon');
      m.speechEnd2 = now(sim);
      await at(sim, 3800);
      release(sim, 'A');
      await at(sim, 12000);
      // B answers; A listens (not scored here, checks the next turn isn't blocked)
      await press(sim, 'B');
      await at(sim, 12800);
      say(sim, 'B', 'sí a mediodía');
      release(sim, 'B');
      await at(sim, 20000);
    },
  },
  {
    id: 'brief-hesitation',
    matrixRow: 'Brief hesitation',
    listener: 'B',
    ideal: ['i would like to book a table for two'],
    async run(sim, m) {
      await press(sim, 'A');
      await interim(sim, 'A', 'i would like to book a'.split(' '), 1000);
      await at(sim, 2200);
      // The recognizer finalizes at a short pause in the middle of the thought.
      say(sim, 'A', 'i would like to book a');
      m.fragmentEnd = now(sim);
      await at(sim, 2800); // 600 ms hesitation
      await interim(sim, 'A', 'table for two'.split(' '), 2800);
      await at(sim, 3500);
      say(sim, 'A', 'table for two');
      m.speechEnd = now(sim);
      await at(sim, 3700);
      release(sim, 'A');
      await at(sim, 12000);
    },
  },
  {
    id: 'long-pause-hands-free',
    matrixRow: 'Long pause / completed thought',
    listener: 'B',
    ideal: ['i think that', 'the museum opens at ten'],
    async run(sim, m) {
      sim.A.session.setListeningMode('continuous');
      await vi.advanceTimersByTimeAsync(0);
      await at(sim, 1000);
      say(sim, 'A', 'i think that'); // trailing function word, then the speaker stops
      m.speechEnd1 = now(sim);
      await at(sim, 7000); // long silence: must not stay open indefinitely
      say(sim, 'A', 'the museum opens at ten');
      m.speechEnd2 = now(sim);
      await at(sim, 16000);
      sim.A.session.setListeningMode('push-to-talk');
    },
  },
  {
    id: 'rapid-speaker-change',
    matrixRow: 'Rapid speaker change',
    listener: 'A',
    ideal: ['vale nos vemos allí'],
    opts: { translateMs: 900 },
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1500);
      say(sim, 'A', 'see you at the station');
      release(sim, 'A');
      await at(sim, 1700); // B starts replying almost immediately
      await press(sim, 'B');
      await at(sim, 2600);
      say(sim, 'B', 'vale nos vemos allí');
      m.speechEnd = now(sim);
      release(sim, 'B');
      await at(sim, 12000);
    },
  },
  {
    id: 'barge-in-during-playback',
    matrixRow: 'Barge-in during playback',
    listener: 'B',
    ideal: ['first let me explain the whole plan in detail'],
    stale: ['then we take the second train to the coast', 'and after that we walk along the beach'],
    opts: { translateMs: 300 },
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1500);
      say(sim, 'A', 'first let me explain the whole plan in detail');
      await at(sim, 2300);
      say(sim, 'A', 'then we take the second train to the coast');
      await at(sim, 2700);
      // B interrupts while the first segment is playing
      m.interrupt = now(sim);
      await press(sim, 'B');
      await at(sim, 3200);
      say(sim, 'A', 'and after that we walk along the beach'); // A was still talking
      release(sim, 'A');
      await at(sim, 3800);
      say(sim, 'B', 'espera un momento');
      release(sim, 'B');
      await at(sim, 15000);
    },
  },
  {
    id: 'interrupt-during-translation',
    matrixRow: 'Interrupt during translation',
    listener: 'B',
    ideal: [],
    stale: ['this sentence is still being translated'],
    opts: { translateMs: 2500 },
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1000);
      say(sim, 'A', 'this sentence is still being translated');
      // A keeps the floor (still holding, thinking); B cuts in before the
      // translation arrives. (Revised before any Phase 4 code: a reply after
      // A has released is a speaker change, covered by rapid-speaker-change.)
      await at(sim, 1500);
      m.interrupt = now(sim);
      await press(sim, 'B');
      await at(sim, 1800);
      release(sim, 'A');
      await at(sim, 2500);
      say(sim, 'B', 'perdona');
      release(sim, 'B');
      await at(sim, 12000);
    },
  },
  {
    id: 'overlapping-voices',
    matrixRow: 'Overlapping voices',
    listener: 'B',
    ideal: ['are you coming to dinner tonight'],
    async run(sim, m) {
      // B is mid-sentence (holding) when A's segment arrives
      await press(sim, 'B');
      await interim(sim, 'B', 'creo que voy a llegar tarde porque'.split(' '), 800, 250);
      await at(sim, 1000);
      await press(sim, 'A');
      await at(sim, 1500);
      say(sim, 'A', 'are you coming to dinner tonight');
      release(sim, 'A');
      m.overlapStart = now(sim);
      await at(sim, 3500);
      say(sim, 'B', 'creo que voy a llegar tarde porque tengo trabajo');
      release(sim, 'B');
      m.bReleased = now(sim);
      await at(sim, 12000);
    },
  },
  {
    id: 'noise-and-speech-rate',
    matrixRow: 'Noise / varied speech rates',
    listener: 'B',
    ideal: ['the bus leaves in five minutes', 'we need to run now', 'take the umbrella it is raining'],
    async run(sim, m) {
      sim.A.session.setListeningMode('continuous');
      await vi.advanceTimersByTimeAsync(0);
      await at(sim, 800);
      say(sim, 'A', 'uh', true, 0.2); // background noise recognised as a filler
      await at(sim, 1500);
      say(sim, 'A', 'the bus leaves in five minutes', true, 0.92);
      await at(sim, 1900); // fast speaker: two quick phrases
      say(sim, 'A', 'we need to run now', true, 0.9);
      await at(sim, 2600);
      say(sim, 'A', 'mm', true, 0.15);
      await at(sim, 4000);
      say(sim, 'A', 'take the umbrella it is raining', true, 0.88);
      m.speechEnd = now(sim);
      await at(sim, 14000);
      sim.A.session.setListeningMode('push-to-talk');
    },
  },
  {
    id: 'correction-restart-repetition',
    matrixRow: 'Correction / restart / repetition',
    listener: 'B',
    ideal: ['i want to go to the airport', 'can you call a taxi'],
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1200);
      say(sim, 'A', 'i want to'); // abandoned start
      await at(sim, 1700);
      say(sim, 'A', 'i want to go to the airport'); // restart that supersedes it
      await at(sim, 2600);
      say(sim, 'A', 'can you can you call a taxi'); // repetition
      m.speechEnd = now(sim);
      release(sim, 'A');
      await at(sim, 12000);
    },
  },
  {
    id: 'network-loss-recovery',
    matrixRow: 'Network loss / recovery',
    listener: 'B',
    ideal: ['before the outage', 'during the outage', 'after the outage'],
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1000);
      say(sim, 'A', 'before the outage');
      await at(sim, 1500);
      sim.net.setUp(false);
      m.down = now(sim);
      await at(sim, 2500);
      say(sim, 'A', 'during the outage');
      await at(sim, 4500);
      sim.net.setUp(true);
      m.up = now(sim);
      // the transport re-sends held segments AND the sender may re-send after resume
      await at(sim, 5000);
      say(sim, 'A', 'after the outage');
      release(sim, 'A');
      await at(sim, 15000);
    },
  },
  {
    id: 'slow-provider-saturation',
    matrixRow: 'Slow provider / saturation',
    listener: 'B',
    ideal: Array.from({ length: 10 }, (_, i) => `short sentence number ${i}`),
    opts: { translateMs: 2500 },
    async run(sim, m) {
      await press(sim, 'A');
      for (let i = 0; i < 10; i++) {
        await at(sim, 1000 + i * 600);
        say(sim, 'A', `short sentence number ${i}`);
      }
      m.speechEnd = now(sim);
      release(sim, 'A');
      await at(sim, 60000);
    },
  },
  {
    id: 'cancellation-session-end',
    matrixRow: 'Cancellation / session end',
    listener: 'B',
    ideal: [],
    stale: ['this will never be heard', 'nor will this'],
    opts: { translateMs: 1500 },
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1000);
      say(sim, 'A', 'this will never be heard');
      await at(sim, 1200);
      say(sim, 'A', 'nor will this');
      m.end = now(sim);
      sim.A.session.leave(); // speaker hangs up mid-translation
      sim.B.session.leave();
      await at(sim, 10000);
    },
  },
  {
    id: 'privacy-revocation',
    matrixRow: 'Privacy / revocation',
    listener: 'B',
    ideal: ['my phone number is private'],
    async run(sim, m) {
      await press(sim, 'A');
      await at(sim, 1000);
      say(sim, 'A', 'my phone number is private');
      release(sim, 'A');
      await at(sim, 6000);
      sim.A.session.setMicEnabled(false); // revoke capture
      m.revoked = now(sim);
      await at(sim, 7000);
    },
  },
];

export async function runScenario(s: Scenario) {
  const sim = createSim(s.opts);
  const marks: Marks = {};
  await connectSim(sim);
  await s.run(sim, marks);
  return { sim, marks };
}
