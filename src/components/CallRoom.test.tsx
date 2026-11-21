import { describe, it, expect, vi } from 'vitest';
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react';
import { findLanguage } from '../types';
import { makeDeps, settle, FakeAsr } from '../test/engineFakes';
import { CallRoom } from './CallRoom';

const en = findLanguage('en')!;

function renderRoom(opts: Parameters<typeof makeDeps>[0] = {}) {
  const f = makeDeps(opts);
  const onEndCall = vi.fn();
  const utils = render(
    <CallRoom roomCode="ABC123" language={en} userName="Ana" onLanguageChange={() => {}} onEndCall={onEndCall} depsFactory={() => f.deps} />
  );
  return { ...utils, ...f, onEndCall };
}

async function connect(f: ReturnType<typeof renderRoom>) {
  await act(async () => {
    f.call.status('waiting-for-peer');
    f.call.status('connected');
    f.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis' });
    await settle();
  });
}

const ptt = () => screen.getByRole('button', { name: 'Push to talk' }) as HTMLButtonElement;

describe('CallRoom view over the engine', () => {
  it('first use: shows the processing disclosure; nothing is captured until Allow', async () => {
    const f = renderRoom();
    await connect(f);
    const dialog = await screen.findByRole('dialog', { name: /before you talk/i });
    expect(dialog.textContent).toMatch(/speech-recognition service/i);
    expect(dialog.textContent).toMatch(/MyMemory/);
    expect(FakeAsr.all).toHaveLength(0);
    // Pressing while undecided re-opens the disclosure instead of capturing.
    fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
    });
    expect(FakeAsr.all).toHaveLength(0);
    fireEvent.click(await screen.findByRole('button', { name: /allow microphone/i }));
    expect(f.storage.data.get('talktual.consent.v1.browser-speech')).toBe('granted');
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
    });
    expect(FakeAsr.all).toHaveLength(1);
    expect(f.asr()!.active).toBe(true);
    expect(screen.getByText('On air')).toBeTruthy();
    await act(async () => {
      fireEvent.pointerUp(ptt());
      await settle();
    });
    expect(f.asr()!.active).toBe(false);
  });

  it('returning user (consent stored): no prompt', async () => {
    await connect(renderRoom({ consent: true }));
    expect(screen.queryByRole('dialog', { name: /before you talk/i })).toBeNull();
  });

  it('hands-free: explicit opt-in shows a persistent indicator; Stop ends capture', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /hands-free: off/i }));
      await settle();
    });
    expect(screen.getByText(/listening continuously/i)).toBeTruthy();
    expect(f.asr()!.active).toBe(true);
    expect(ptt().disabled).toBe(true);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
      await settle();
    });
    expect(screen.queryByText(/listening continuously/i)).toBeNull();
    expect(FakeAsr.all.every((a) => !a.active)).toBe(true);
  });

  it('mic off stops capture immediately and disables talking', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Turn microphone off' }));
      await settle();
    });
    expect(f.asr()?.active ?? false).toBe(false);
    expect(screen.getByText('Microphone off')).toBeTruthy();
    expect(ptt().disabled).toBe(true);
    expect(f.capture.releases).toBeGreaterThan(0);
  });

  it('device lost: notice + mic shown off; the mic button is the way back (UI recovery path)', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
    });
    await act(async () => {
      f.capture.loseDevice();
      fireEvent.pointerUp(ptt());
      await settle();
    });
    expect(screen.getByText(/microphone was disconnected/i)).toBeTruthy();
    expect(ptt().disabled).toBe(true);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Turn microphone on' }));
      await settle();
    });
    expect(ptt().disabled).toBe(false);
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
    });
    expect(f.asr()!.active).toBe(true);
  });

  it('leaving tears down the transport and returns home', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    fireEvent.click(screen.getByRole('button', { name: 'Leave channel' }));
    expect(f.onEndCall).toHaveBeenCalled();
    expect(f.call.closed).toBe(1);
    f.unmount();
    expect(f.call.closed).toBe(1);
  });

  it('peer leaving ends the call after a short notice (ported)', async () => {
    vi.useFakeTimers();
    try {
      const f = renderRoom({ consent: true });
      await act(async () => {
        f.call.status('connected');
        f.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis' });
        await vi.advanceTimersByTimeAsync(0);
      });
      await act(async () => f.call.status('peer-left'));
      expect(screen.getAllByText(/Luis left the channel/i).length).toBeGreaterThan(0);
      await act(async () => vi.advanceTimersByTime(2000));
      expect(f.onEndCall).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('reconnecting is shown, then cleared', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => f.call.status('reconnecting'));
    expect(screen.getAllByText(/reconnecting/i).length).toBeGreaterThan(0);
    await act(async () => f.call.status('connected'));
    expect(f.session).toBeDefined();
    await waitFor(() => expect(screen.queryByText(/reconnecting/i)).toBeNull());
  });

  it('peer names and captions are rendered as text, never as HTML (ported)', async () => {
    const f = renderRoom({ consent: true });
    await act(async () => {
      f.call.status('connected');
      f.call.receive({ t: 'hello', lang: 'es-ES', name: '<img src=x onerror=alert(1)>' });
      f.call.receive({ t: 'partial', text: '<b>bold</b>', lang: 'es-ES' });
      await settle();
    });
    expect(document.querySelector('img')).toBeNull();
    expect(document.querySelector('b')).toBeNull();
    expect(screen.getByText('<b>bold</b>', { exact: false })).toBeTruthy();
  });

  it('incoming speech shows as the last heard line and in the transcript', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => {
      f.call.receive({ t: 'segment', id: 's1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
      await settle();
    });
    expect(screen.getByText('hello')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Show transcript' }));
    const dialog = screen.getByRole('dialog', { name: 'Transcript' });
    expect(dialog.textContent).toContain('Luis');
    expect(dialog.textContent).toContain('hola');
  });
});

describe('VoiceMirror controls (Phase 3)', () => {
  it('hidden when the feature is off (default build)', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    expect(screen.queryByRole('button', { name: /voice match/i })).toBeNull();
  });

  it('flag on: toggle opens the VoiceMirror disclosure; accepting turns it on; toggling again turns it off', async () => {
    const { FakeVoiceTap } = await import('../test/engineFakes');
    const f = renderRoom({ consent: true, deps: { voiceTap: new FakeVoiceTap() } });
    await connect(f);
    fireEvent.click(screen.getByRole('button', { name: 'Voice match: off' }));
    const dialog = await screen.findByRole('dialog', { name: /share your voice style/i });
    expect(dialog.textContent).toMatch(/does not copy or clone your voice/);
    expect(dialog.textContent).toMatch(/Nothing is stored/);
    fireEvent.click(screen.getByRole('button', { name: 'Share voice style' }));
    expect(screen.getByRole('button', { name: 'Voice match: on' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Voice match: on' }));
    expect(screen.getByRole('button', { name: 'Voice match: off' })).toBeTruthy();
  });

  it('shows when the other person’s voice style is applied', async () => {
    const { FakeVoiceTap } = await import('../test/engineFakes');
    const f = renderRoom({ consent: true, deps: { voiceTap: new FakeVoiceTap() } });
    await connect(f);
    await act(async () => f.call.receive({ t: 'voice', descriptor: { v: 1, pitchStep: 20, pitchRange: 'normal', tempoPct: 100 } }));
    expect(screen.getByText('Their voice style matched')).toBeTruthy();
    await act(async () => f.call.receive({ t: 'voice', revoked: true }));
    expect(screen.queryByText('Their voice style matched')).toBeNull();
  });
});
