import { describe, it, expect, vi, beforeEach } from 'vitest';
import { deferred } from '../test/fakes';

function fakeStream() {
  const track = { readyState: 'live', enabled: true, stopped: false, stop() { this.stopped = true; this.readyState = 'ended'; } };
  return { track, stream: { getAudioTracks: () => [track], getTracks: () => [track] } as unknown as MediaStream };
}

let gum: ReturnType<typeof vi.fn>;

beforeEach(() => {
  gum = vi.fn();
  Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia: gum }, configurable: true });
});

async function load() {
  vi.resetModules();
  return import('./audio');
}

describe('microphone lifecycle', () => {
  it('acquires once per call and reuses the live stream', async () => {
    const { stream } = fakeStream();
    gum.mockResolvedValue(stream);
    const audio = await load();
    const [a, b] = await Promise.all([audio.getMic(), audio.getMic()]);
    expect(a).toBe(stream);
    expect(b).toBe(stream);
    expect(await audio.getMic()).toBe(stream);
    expect(gum).toHaveBeenCalledTimes(1);
  });

  it('releaseMic stops every track', async () => {
    const { stream, track } = fakeStream();
    gum.mockResolvedValue(stream);
    const audio = await load();
    await audio.getMic();
    audio.releaseMic();
    expect(track.stopped).toBe(true);
  });

  it('setMicEnabled gates the tracks without reacquiring', async () => {
    const { stream, track } = fakeStream();
    gum.mockResolvedValue(stream);
    const audio = await load();
    await audio.getMic();
    audio.setMicEnabled(false);
    expect(track.enabled).toBe(false);
    audio.setMicEnabled(true);
    expect(track.enabled).toBe(true);
  });

  it('DEF-C03: a permission prompt answered after leaving the call does not leave the mic on', async () => {
    const pending = deferred<MediaStream>();
    gum.mockReturnValueOnce(pending.promise);
    const audio = await load();
    const request = audio.getMic();
    audio.releaseMic(); // user left the call while the prompt was open
    const { stream, track } = fakeStream();
    pending.resolve(stream); // ...then tapped "Allow"
    await expect(request).rejects.toMatchObject({ name: 'AbortError' });
    expect(track.stopped).toBe(true);

    // The next call gets a fresh stream.
    const next = fakeStream();
    gum.mockResolvedValueOnce(next.stream);
    expect(await audio.getMic()).toBe(next.stream);
  });
});
