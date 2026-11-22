import { describe, it, expect } from 'vitest';
import type { SessionSnapshot } from '../../engine/CommunicationSession';
import { deriveStatus, connectionLabel } from './status';

/** Minimal snapshot: only the fields deriveStatus reads. */
function snap(over: Record<string, unknown> = {}): SessionSnapshot {
  return {
    phase: 'connected',
    phaseReason: null,
    activity: 'idle',
    mode: 'push-to-talk',
    holding: false,
    hearingVoice: false,
    playing: false,
    translating: 0,
    notice: null,
    peer: { name: 'Luis', lang: 'es-ES' },
    conversation: { enabled: false, held: null, overlapHeld: 0, peerTalking: false, degraded: false, drafts: false },
    processing: { available: false },
    ...over,
  } as unknown as SessionSnapshot;
}

describe('deriveStatus', () => {
  it('idle call: one hint, no banner', () => {
    expect(deriveStatus(snap())).toEqual({ hint: 'Hold to talk', tone: 'idle', banner: null });
  });

  it('holding: live tone; overlap count takes priority over "Speak now"', () => {
    expect(deriveStatus(snap({ holding: true }))).toMatchObject({ hint: 'Speak now', tone: 'live' });
    expect(deriveStatus(snap({ holding: true, hearingVoice: true })).hint).toBe('Listening…');
    const conv = { enabled: true, held: null, overlapHeld: 2, peerTalking: false, degraded: false, drafts: false };
    expect(deriveStatus(snap({ holding: true, conversation: conv })).hint).toBe('2 messages wait until you finish');
  });

  it('incoming speech and peer talking use the incoming tone with the peer name', () => {
    expect(deriveStatus(snap({ playing: true }))).toMatchObject({ hint: 'Luis is speaking — hold to interrupt', tone: 'incoming' });
    const conv = { enabled: true, held: null, overlapHeld: 0, peerTalking: true, degraded: false, drafts: false };
    expect(deriveStatus(snap({ conversation: conv })).hint).toBe('Luis is talking…');
  });

  it('mic off wins over everything in the status line', () => {
    expect(deriveStatus(snap({ activity: 'stopped', playing: true }))).toMatchObject({ hint: 'Microphone off', tone: 'muted' });
  });

  it('hands-free: listening vs paused while the peer speaks', () => {
    expect(deriveStatus(snap({ mode: 'continuous', activity: 'listening' })).hint).toBe('Listening hands-free');
    expect(deriveStatus(snap({ mode: 'continuous', activity: 'paused' }))).toMatchObject({ hint: 'Paused while Luis speaks', tone: 'incoming' });
  });

  it('only one banner: failure beats a notice, notice beats reconnecting', () => {
    const notice = { code: 'device-lost', message: 'Your microphone was disconnected.' };
    expect(deriveStatus(snap({ phase: 'failed', phaseReason: 'connection-lost', notice })).banner).toMatchObject({ tone: 'error', action: 'leave' });
    expect(deriveStatus(snap({ phase: 'reconnecting', notice })).banner).toEqual({ tone: 'error', text: notice.message });
    expect(deriveStatus(snap({ phase: 'reconnecting' })).banner).toEqual({ tone: 'warning', text: 'Reconnecting…' });
  });

  it('processing: offline and "speech unavailable → type" banners', () => {
    const proc = (network: string, speech = 'cloud') => ({ available: true, network, mode: 'cloud', capabilities: [{ task: 'speech-input', state: speech }] });
    expect(deriveStatus(snap({ processing: proc('offline') })).banner).toMatchObject({ tone: 'offline', text: 'Offline — on-device only' });
    expect(deriveStatus(snap({ processing: proc('online', 'unavailable') })).banner).toMatchObject({ tone: 'info', action: 'type' });
  });

  it('connection labels name the peer and never leak codes', () => {
    expect(connectionLabel(snap({ phase: 'ended', phaseReason: 'peer-left' }), 'Luis')).toBe('Luis left the call');
    expect(connectionLabel(snap({ phase: 'waiting-for-peer' }), 'They')).toBe('Waiting for them to join…');
    expect(connectionLabel(snap(), 'Luis')).toBeNull();
    expect(deriveStatus(null).hint).toBe('Connecting…');
  });
});
