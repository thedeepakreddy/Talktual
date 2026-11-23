// Policy-routed translation, shared by the two-person call session and the
// Phase 8 ambient session so both enforce the same Phase 6 rules:
// on-device and/or cloud steps in the order the policy allows; a cloud step
// re-checks permission at execution time and is aborted if permission is
// revoked mid-flight (then the next allowed step runs). Bounded: each step at
// most once; no loops. The translator's input never leaks as output: on
// failure the ORIGINAL text is returned with status 'failed'.

import type { TranslationRequest, TranslationResult, Translator } from '../engine/ports';
import type { RuntimeRegistry } from '../local/registry';
import { anySignal, type ProcessingPolicy, type RouteReason } from './policy';

export interface RoutingHooks {
  record?(type: string, fields: Record<string, string | number | boolean>): void;
  count?(name: string): void;
  networkFailure?(): void;
  networkSuccess?(): void;
}

export type RoutedLocation = 'device' | 'cloud' | 'none';

export async function routedTranslate(
  policy: ProcessingPolicy,
  reg: RuntimeRegistry,
  translate: Translator,
  req: TranslationRequest,
  opts: { cloudText?: string; peerRequestedDevice?: boolean } = {},
  hooks: RoutingHooks = {}
): Promise<{ result: TranslationResult; location: RoutedLocation }> {
  const plan = policy.plan('translate', { deviceAvailable: reg.canTranslate(req.from, req.to), cloudAvailable: true, peerRequestedDevice: opts.peerRequestedDevice });
  const cancelled = (): { result: TranslationResult; location: 'none' } => ({ result: { segmentId: req.segmentId, status: 'cancelled', text: req.text, attempts: 0, latencyMs: 0 }, location: 'none' });
  let last: TranslationResult = { segmentId: req.segmentId, status: 'failed', text: req.text, error: { code: 'unavailable', message: plan.reason }, attempts: 0, latencyMs: 0 };
  let why: RouteReason | 'not-covered' = plan.reason;
  for (const step of plan.steps) {
    if (req.signal?.aborted) return cancelled();
    if (step === 'device') {
      const o = await reg.translate({ text: req.text, from: req.from, to: req.to, signal: req.signal });
      if (o.ok) {
        policy.noteDevice('translate', plan.reason);
        hooks.record?.('route', { kind: 'translate', status: 'device', reason: plan.reason });
        return { result: { segmentId: req.segmentId, status: 'translated', text: o.text, attempts: 1, latencyMs: Math.round(o.ms) }, location: 'device' };
      }
      const miss = 'reason' in o ? o.reason : 'error';
      if (miss === 'cancelled') return cancelled();
      why = 'not-covered';
      hooks.count?.(`local_${miss}`);
      continue;
    }
    let gate: { signal: AbortSignal; end: () => void };
    try {
      gate = policy.beginRemote('translate', { peerRequestedDevice: opts.peerRequestedDevice });
    } catch {
      continue; // permission changed since planning
    }
    try {
      const r = await translate({ ...req, text: opts.cloudText ?? req.text, signal: anySignal(req.signal, gate.signal) });
      if (r.status === 'cancelled') {
        if (req.signal?.aborted) return cancelled();
        hooks.count?.('remote_revoked');
        continue; // revoked mid-flight → next allowed step (never resumes remotely)
      }
      if (r.status === 'failed') {
        if (r.error?.code === 'network' || r.error?.code === 'timeout') hooks.networkFailure?.();
        last = { ...r, text: req.text }; // never leak the MT input
        continue;
      }
      hooks.networkSuccess?.();
      hooks.record?.('route', { kind: 'translate', status: 'cloud', reason: plan.reason });
      return { result: r, location: 'cloud' };
    } finally {
      gate.end();
    }
  }
  hooks.record?.('route', { kind: 'translate', status: 'none', reason: why });
  if (last.error?.code === 'unavailable') last = { ...last, error: { code: 'unavailable', message: why } };
  return { result: last, location: 'none' };
}
