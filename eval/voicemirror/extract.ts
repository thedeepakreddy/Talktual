// CLI used by the evaluation harness: runs the PRODUCT analyzer/descriptor code
// (src/voicemirror) over WAV files and prints JSON. Node: --experimental-strip-types.
//   node --experimental-strip-types eval/voicemirror/extract.ts a.wav b.wav ...
import { readFileSync } from 'node:fs';
import { VoiceAnalyzer } from '../../src/voicemirror/analyzer.ts';
import { toDescriptor } from '../../src/voicemirror/descriptor.ts';

function readWav(path: string): { rate: number; samples: Float32Array } {
  const b = readFileSync(path);
  if (b.toString('ascii', 0, 4) !== 'RIFF' || b.toString('ascii', 8, 12) !== 'WAVE') throw new Error(`not a WAV: ${path}`);
  let off = 12;
  let rate = 0;
  let bits = 0;
  let channels = 1;
  while (off + 8 <= b.length) {
    const id = b.toString('ascii', off, off + 4);
    let size = b.readUInt32LE(off + 4);
    const body = off + 8;
    if (id === 'fmt ') {
      channels = b.readUInt16LE(body + 2);
      rate = b.readUInt32LE(body + 4);
      bits = b.readUInt16LE(body + 14);
    } else if (id === 'data') {
      if (bits !== 16) throw new Error('only PCM16 supported');
      if (size === 0 || body + size > b.length) size = b.length - body; // espeak streams write size 0
      const n = Math.floor(size / 2 / channels);
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) out[i] = b.readInt16LE(body + i * 2 * channels) / 32768;
      return { rate, samples: out };
    }
    off = body + size + (size % 2);
  }
  throw new Error('no data chunk');
}

const results = [];
for (const path of process.argv.slice(2)) {
  const { rate, samples } = readWav(path);
  const a = new VoiceAnalyzer(rate);
  const t0 = performance.now();
  // Feed in 20 ms chunks, as a live audio callback would.
  const chunk = Math.round(rate * 0.02);
  for (let i = 0; i < samples.length; i += chunk) a.push(samples.subarray(i, i + chunk));
  const ms = performance.now() - t0;
  const profile = a.profile();
  results.push({ path, rate, seconds: samples.length / rate, analyzeMs: ms, profile, descriptor: toDescriptor(profile) });
}
console.log(JSON.stringify(results));
