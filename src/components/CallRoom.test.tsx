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
const feed = () => screen.getByRole('log', { name: 'Conversation' });
const openSettings = () => {
  fireEvent.click(screen.getByRole('button', { name: 'Call settings' }));
  return screen.getByRole('dialog', { name: 'Call settings' });
};
const closeSettings = () => fireEvent.click(screen.getByRole('button', { name: 'Close settings' }));

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
    expect(screen.getByText('Speak now')).toBeTruthy();
    expect(ptt().getAttribute('aria-pressed')).toBe('true');
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
    openSettings();
    await act(async () => {
      fireEvent.click(screen.getByRole('radio', { name: 'Hands-free' }));
      await settle();
    });
    closeSettings();
    expect(screen.getByText('Hands-free: listening continuously')).toBeTruthy();
    expect(f.asr()!.active).toBe(true);
    // The talk control becomes "Stop hands-free"; holding is not possible.
    expect(screen.queryByRole('button', { name: 'Push to talk' })).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
      await settle();
    });
    expect(screen.queryByRole('button', { name: 'Stop hands-free' })).toBeNull();
    expect(screen.queryByText('Hands-free: listening continuously')).toBeNull();
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
    fireEvent.click(screen.getByRole('button', { name: 'End call' }));
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
      expect(screen.getAllByText(/Luis left the call/i).length).toBeGreaterThan(0);
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

  it('incoming speech appears in the conversation with the original underneath', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    await act(async () => {
      f.call.receive({ t: 'segment', id: 's1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
      await settle();
    });
    expect(screen.getByText('hello')).toBeTruthy();
    expect(feed().textContent).toContain('Luis');
    expect(feed().textContent).toContain('hola');
  });
});

describe('Phase 4 conversation affordances', () => {
  async function connectCapable(f: ReturnType<typeof renderRoom>) {
    await act(async () => {
      f.call.status('waiting-for-peer');
      f.call.status('connected');
      f.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis', caps: ['conv1'] });
      await settle();
    });
  }
  const seg = { sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 } as const;

  it('provisional is visibly a draft; final replaces it; interrupted and uncertain lines are labelled', async () => {
    const f = renderRoom({ consent: true, deps: { conversation: true } });
    await connectCapable(f);
    await act(async () => {
      f.call.receive({ t: 'floor', turn: 'T1', open: true });
      f.call.receive({ t: 'segment', id: 'a', original: 'quiero reservar una', translated: 'I want to book a', state: 'provisional', rev: 0, turn: 'T1', ...seg });
      await settle();
    });
    expect(screen.getByText(/Luis is talking/)).toBeTruthy();
    expect(screen.getByText(/I want to book a/)).toBeTruthy(); // live provisional caption
    expect(feed().textContent).toContain('Still speaking — may change');
    await act(async () => {
      f.call.receive({ t: 'segment', id: 'a', original: 'quiero reservar una mesa', translated: 'I want to book a table', state: 'final', rev: 0, turn: 'T1', uncertain: true, ...seg });
      await settle();
    });
    expect(feed().textContent).not.toContain('Still speaking');
    expect(feed().textContent).toContain('I want to book a table');
    expect(feed().textContent).toContain('Unclear audio');
    // We cut in while Luis still holds the floor → the rest of that turn is labelled, not played.
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
      fireEvent.pointerUp(ptt());
      await settle();
      f.call.receive({ t: 'segment', id: 'b', original: 'y luego', translated: 'and then', state: 'final', turn: 'T1', ...seg });
      await settle();
    });
    expect(feed().textContent).toContain('Interrupted — not played');
    expect(f.playback.played.map((p) => p.segmentId)).not.toContain('b');
  });

  it('speech that arrives while we talk shows a waiting chip and plays after release', async () => {
    const f = renderRoom({ consent: true, deps: { conversation: true } });
    await connectCapable(f);
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
      f.call.receive({ t: 'segment', id: 'c', original: 'hola', translated: 'hello', turn: 'T2', ...seg });
      await settle();
    });
    expect(screen.getByText(/1 message waits until you finish/)).toBeTruthy();
    expect(f.playback.played).toHaveLength(0);
    await act(async () => {
      fireEvent.pointerUp(ptt());
      await settle();
    });
    expect(f.playback.played.map((p) => p.segmentId)).toEqual(['c']);
    expect(screen.queryByText(/waits until you finish/)).toBeNull();
  });
});

describe('Phase 5 assistance UI', () => {
  async function connectAci(f: ReturnType<typeof renderRoom>) {
    await act(async () => {
      f.call.status('waiting-for-peer');
      f.call.status('connected');
      f.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis', caps: ['aci1'] });
      await settle();
    });
  }
  const seg = { sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 } as const;

  it('notes and tags appear under the line; settings show, correct and reset context; off switch', async () => {
    const f = renderRoom({ consent: true, deps: { intelligence: true } });
    await connectAci(f);
    await act(async () => {
      f.call.receive({ t: 'segment', id: 'a', original: 'Nos vemos en la Plaza Mayor', translated: 'See you at Plaza Mayor', ...seg });
      f.call.receive({ t: 'segment', id: 'b', original: '¿Llegas allí a las cinco?', translated: 'Do you get there at five?', ...seg });
      await settle();
    });
    expect(feed().textContent).toContain('“allí” → Plaza Mayor (mentioned earlier)');
    expect(feed().textContent).toContain('Question');

    const sheet = openSettings();
    expect(sheet.textContent).toMatch(/this call only/);
    fireEvent.click(screen.getByRole('button', { name: /What it remembers/ }));
    const memory = screen.getByRole('dialog', { name: 'What assistance remembers' });
    expect(memory.textContent).toContain('Plaza Mayor');
    fireEvent.click(screen.getByRole('button', { name: 'Forget Plaza Mayor' }));
    expect(screen.getByRole('dialog', { name: 'What assistance remembers' }).textContent).not.toContain('Plaza Mayor');
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.click(screen.getByRole('switch', { name: 'Concise speech' }));
    expect(screen.getByRole('switch', { name: 'Concise speech' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('switch', { name: 'Assistance' }));
    expect(screen.getByRole('switch', { name: 'Assistance' }).getAttribute('aria-checked')).toBe('false');
    // Off hides the sub-options instead of leaving dead controls.
    expect(screen.queryByRole('switch', { name: 'Concise speech' })).toBeNull();
    closeSettings();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('a suggestion is a dismissible note, not a popup', async () => {
    const f = renderRoom({ consent: true, deps: { intelligence: true } });
    await connectAci(f);
    await act(async () => {
      fireEvent.pointerDown(ptt());
      await settle();
      f.asr()!.say('Meet me at the bank');
      fireEvent.pointerUp(ptt());
      await settle(40);
    });
    const note = screen.getByRole('note', { name: 'Suggestion' });
    expect(note.textContent).toMatch(/“bank” can mean/);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss suggestion' }));
    expect(screen.queryByRole('note', { name: 'Suggestion' })).toBeNull();
  });

  it('hidden when the feature is off (default engine deps)', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    openSettings();
    expect(screen.queryByRole('switch', { name: 'Assistance' })).toBeNull();
  });
});

describe('Phase 6 privacy and processing UI', () => {
  const network = { onLine: () => true, subscribe: () => () => {} };
  async function connectPriv(f: ReturnType<typeof renderRoom>) {
    await act(async () => {
      f.call.status('waiting-for-peer');
      f.call.status('connected');
      f.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis', caps: ['priv1'] });
      await settle();
    });
  }

  it('privacy sheet shows where things run; On-device mode turns speech off, typing on; typed phrase is translated on this device', async () => {
    const f = renderRoom({ consent: true, deps: { processing: { network } } });
    await connectPriv(f);
    openSettings();
    fireEvent.click(screen.getByRole('button', { name: /Where things run/ }));
    let sheet = screen.getByRole('dialog', { name: 'Where things run' });
    expect(sheet.textContent).toContain('Speech recognition');
    expect(sheet.textContent).toMatch(/Talktual offline phrasebook/);
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('radio', { name: 'On-device' }));
      await settle();
    });
    fireEvent.click(screen.getByRole('button', { name: /Where things run/ }));
    sheet = screen.getByRole('dialog', { name: 'Where things run' });
    expect(sheet.textContent).toMatch(/not available on this device in On-device mode/);
    expect(sheet.textContent).toMatch(/No recognition, translation or voice service receives anything/);
    closeSettings();
    expect(screen.getByTestId('call-meta').textContent).toContain('On-device only');
    const box = screen.getByRole('textbox', { name: 'Type a message' });
    await act(async () => {
      fireEvent.change(box, { target: { value: 'where is the train station' } });
      fireEvent.submit(box.closest('form')!);
      await settle(40);
    });
    expect(f.translate).not.toHaveBeenCalled();
    expect(f.call.segments().at(-1)).toMatchObject({ translated: '¿Dónde está la estación de tren?', priv: 'device' });
    expect(feed().textContent).toMatch(/Typed · Translated on this device/);
  });

  it('hidden when the feature is off', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    expect(screen.queryByRole('textbox', { name: 'Type a message' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Type' })).toBeNull();
    openSettings();
    expect(screen.queryByRole('radiogroup', { name: 'Processing mode' })).toBeNull();
  });
});

describe('VoiceMirror controls (Phase 3)', () => {
  it('hidden when the feature is off (default build)', async () => {
    const f = renderRoom({ consent: true });
    await connect(f);
    openSettings();
    expect(screen.queryByRole('switch', { name: 'Voice match' })).toBeNull();
  });

  it('flag on: toggle opens the VoiceMirror disclosure; accepting turns it on; toggling again turns it off', async () => {
    const { FakeVoiceTap } = await import('../test/engineFakes');
    const f = renderRoom({ consent: true, deps: { voiceTap: new FakeVoiceTap() } });
    await connect(f);
    openSettings();
    const toggle = () => screen.getByRole('switch', { name: 'Voice match' });
    expect(toggle().getAttribute('aria-checked')).toBe('false');
    fireEvent.click(toggle());
    const dialog = await screen.findByRole('dialog', { name: /share your voice style/i });
    expect(dialog.textContent).toMatch(/does not copy or clone your voice/);
    expect(dialog.textContent).toMatch(/Nothing is stored/);
    // Settings closed so the disclosure is the only dialog.
    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Share voice style' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    openSettings();
    expect(toggle().getAttribute('aria-checked')).toBe('true');
    fireEvent.click(toggle());
    expect(toggle().getAttribute('aria-checked')).toBe('false');
  });

  it('shows when the other person’s voice style is applied', async () => {
    const { FakeVoiceTap } = await import('../test/engineFakes');
    const f = renderRoom({ consent: true, deps: { voiceTap: new FakeVoiceTap() } });
    await connect(f);
    await act(async () => f.call.receive({ t: 'voice', descriptor: { v: 1, pitchStep: 20, pitchRange: 'normal', tempoPct: 100 } }));
    expect(screen.getByTestId('call-meta').textContent).toContain('Voice matched');
    await act(async () => f.call.receive({ t: 'voice', revoked: true }));
    expect(screen.getByTestId('call-meta').textContent).not.toContain('Voice matched');
  });
});
