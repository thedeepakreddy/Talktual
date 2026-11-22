// Conversational signal detection: categories + confidence only. Observable
// interaction cues; no emotion, personality or sensitive-attribute inference.

import { NOT_UNDERSTOOD, SIGNAL_CUES, STOP } from './lexicon';
import { norm, words } from './text';
import type { AciLang, Signal, SignalCategory } from './types';

export interface SignalContext {
  /** The previous turn was by the other speaker (for misunderstanding). */
  previousByOther: boolean;
  /** The previous turn by the other speaker was itself a clarification request (repeated clarification). */
  previousWasClarification: boolean;
  topicWords: string[];
  turnsSoFar: number;
}

export function detectSignals(text: string, lang: AciLang, ctx: SignalContext): Signal[] {
  const table = SIGNAL_CUES[lang];
  if (!table) return [];
  const t = norm(text).replace(/^¿\s*/, (m) => m); // keep ¿ for es question cue
  const out = new Map<SignalCategory, number>();
  const add = (c: SignalCategory, conf: number) => out.set(c, Math.max(out.get(c) ?? 0, conf));
  for (const [cat, cues] of Object.entries(table) as Array<[SignalCategory, NonNullable<(typeof table)[SignalCategory]>]>) {
    for (const cue of cues) if (cue.re.test(t)) add(cat, cue.confidence);
  }
  // Resolve overlaps between rule families.
  if (out.has('correction')) out.delete('refusal'); // "no, I meant Tuesday"
  if (out.has('clarification-request')) {
    out.delete('question');
    out.delete('request'); // "can you repeat" is a clarification, not a task request
  }
  if (out.has('offer') && out.get('offer')! >= 0.8) out.delete('request');
  // "Could you call a taxi?" / "Shall I carry it?" ask for action, not information:
  // keep `question` only when an interrogative word asks for information.
  const wh = lang === 'es' ? /^¿?\s*(qué|dónde|cuándo|quién|por qué|cómo|cuál|cuánto|cuánta|cuántos|cuántas)\b/ : /^(what|where|when|who|whom|whose|why|how|which)\b/;
  // A refusal declines something: it needs a preceding turn by the other person.
  if (!ctx.previousByOther) out.delete('refusal');
  if ((out.get('request') ?? 0) >= 0.75 || (out.get('offer') ?? 0) >= 0.6) {
    if (!wh.test(t)) out.delete('question');
  }
  const nu = NOT_UNDERSTOOD[lang];
  if ((nu && nu.test(t)) || (out.has('clarification-request') && ctx.previousByOther)) {
    add('misunderstanding', nu && nu.test(t) ? 0.8 : ctx.previousWasClarification ? 0.75 : 0.6);
  }
  // Topic: explicit marker wins; otherwise lexical overlap with the session topic.
  if (!out.has('topic-change') && ctx.turnsSoFar >= 2 && ctx.topicWords.length >= 3) {
    const content = words(text).filter((w) => w.length >= 4 && !STOP.has(w));
    if (content.length >= 3) {
      const overlap = content.filter((w) => ctx.topicWords.includes(w)).length;
      if (overlap === 0) add('topic-change', 0.45);
      else add('topic-continuation', Math.min(0.9, 0.5 + 0.1 * overlap));
    }
  }
  return [...out.entries()].map(([category, confidence]) => ({ category, confidence })).sort((a, b) => b.confidence - a.confidence);
}
