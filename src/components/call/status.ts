// One source of truth for "what's happening": a single status line (live
// region) and at most one banner. Derived from the engine snapshot only.

import type { SessionSnapshot } from '../../engine/CommunicationSession';
import type { BannerTone } from '../ui/Banner';

export type StatusTone = 'idle' | 'live' | 'incoming' | 'muted' | 'problem';
export type BannerAction = 'leave' | 'type' | 'settings';

export interface DerivedStatus {
  hint: string;
  tone: StatusTone;
  banner: { tone: BannerTone; text: string; action?: BannerAction } | null;
}

const ERROR_NOTICES = new Set(['permission-denied', 'device-lost', 'capture-unavailable', 'session-full', 'session-invalid', 'resume-failed', 'server-error']);

export function connectionLabel(s: SessionSnapshot | null, peer: string): string | null {
  const phase = s?.phase ?? 'connecting';
  if (phase === 'idle' || phase === 'connecting') return 'Connecting…';
  if (phase === 'waiting-for-peer') return s?.phaseReason === 'peer-away' ? `Waiting for ${peer} to reconnect…` : 'Waiting for them to join…';
  if (phase === 'reconnecting') return 'Reconnecting…';
  if (phase === 'ended' && s?.phaseReason === 'peer-left') return `${peer} left the call`;
  if (phase === 'ended' && s?.phaseReason === 'expired') return 'Call expired';
  if (phase === 'ended') return 'Call ended';
  if (phase === 'failed') return s?.phaseReason === 'connection-lost' ? 'Disconnected' : 'Connection problem';
  return null;
}

export function deriveStatus(s: SessionSnapshot | null): DerivedStatus {
  const peer = s?.peer.name ?? 'They';
  const phase = s?.phase ?? 'connecting';
  const connected = phase === 'connected';
  const conv = s?.conversation;
  const proc = s?.processing;

  // ---- banner: the single most important problem or notice
  let banner: DerivedStatus['banner'] = null;
  if (phase === 'failed') banner = { tone: 'error', text: s?.notice?.message ?? connectionLabel(s, peer)!, action: 'leave' };
  else if (phase === 'ended') banner = { tone: 'info', text: connectionLabel(s, peer)! };
  else if (s?.notice) {
    const code = s.notice.code;
    banner = { tone: ERROR_NOTICES.has(code) ? 'error' : code === 'local-only' || code === 'info' ? 'info' : 'warning', text: s.notice.message };
  } else if (phase === 'reconnecting') banner = { tone: 'warning', text: 'Reconnecting…' };
  else if (proc?.available && proc.network === 'offline') banner = { tone: 'offline', text: 'Offline — on-device only' };
  else if (proc?.available && proc.network === 'providers-unreachable') banner = { tone: 'offline', text: 'Cloud unreachable — using this device' };
  else if (proc?.available && proc.capabilities.find((c) => c.task === 'speech-input')?.state === 'unavailable' && connected)
    banner = { tone: 'info', text: 'Speech recognition isn’t available in this mode. You can type instead.', action: 'type' };
  else if (conv?.degraded) banner = { tone: 'warning', text: 'Slow translation — phrases combined' };

  // ---- status line
  const holding = Boolean(s?.holding);
  const continuous = s?.mode === 'continuous';
  const playing = Boolean(s?.playing);
  if (s?.activity === 'stopped') return { hint: 'Microphone off', tone: 'muted', banner };
  if (!connected) return { hint: connectionLabel(s, peer) ?? '', tone: phase === 'failed' ? 'problem' : 'muted', banner };
  if (holding) {
    if (conv && conv.overlapHeld > 0) return { hint: conv.overlapHeld === 1 ? '1 message waits until you finish' : `${conv.overlapHeld} messages wait until you finish`, tone: 'live', banner };
    return { hint: s?.hearingVoice ? 'Listening…' : 'Speak now', tone: 'live', banner };
  }
  if (continuous) return { hint: s?.activity === 'paused' ? `Paused while ${peer} speaks` : 'Listening hands-free', tone: s?.activity === 'paused' ? 'incoming' : 'live', banner };
  if (playing) return { hint: `${peer} is speaking — hold to interrupt`, tone: 'incoming', banner };
  if (conv?.peerTalking) return { hint: `${peer} is talking…`, tone: 'incoming', banner };
  if ((s?.translating ?? 0) > 0) return { hint: 'Translating…', tone: 'idle', banner };
  return { hint: 'Hold to talk', tone: 'idle', banner };
}
