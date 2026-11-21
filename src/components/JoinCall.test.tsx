import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import { FakeWebSocket } from '../test/fakes';
import { SUPPORTED_LANGUAGES } from '../types';

vi.mock('@yudiel/react-qr-scanner', () => ({ Scanner: () => null }));

const { JoinCall, JOIN_TIMEOUT_MS } = await import('./JoinCall');
const lang = SUPPORTED_LANGUAGES[0];

beforeEach(() => {
  FakeWebSocket.instances = [];
  vi.stubGlobal('WebSocket', FakeWebSocket);
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

function renderJoin(initialCode = 'ABC123') {
  const onJoined = vi.fn();
  const onBack = vi.fn();
  const utils = render(<JoinCall language={lang} onBack={onBack} onJoined={onJoined} initialCode={initialCode} />);
  return { ...utils, onJoined, onBack };
}

const joinButton = () => screen.getByRole('button', { name: /^(join|connecting…)$/i });

describe('JoinCall', () => {
  it('joins the lobby and enters the call on ready', () => {
    const { onJoined } = renderJoin();
    fireEvent.click(joinButton());
    const ws = FakeWebSocket.instances[0];
    act(() => ws.open());
    expect(ws.sent[0]).toEqual({ type: 'join', sessionId: 'ABC123_signal' });
    act(() => ws.receive({ type: 'ready' }));
    expect(onJoined).toHaveBeenCalledWith('ABC123', lang);
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
  });

  it('DEF-C04: a code nobody is hosting times out with an error instead of spinning forever', () => {
    const { onJoined } = renderJoin('ZZZZZZ');
    fireEvent.click(joinButton());
    const ws = FakeWebSocket.instances[0];
    act(() => ws.open());
    expect(joinButton()).toHaveProperty('disabled', true);
    act(() => {
      vi.advanceTimersByTime(JOIN_TIMEOUT_MS + 1);
    });
    expect(screen.getByRole('alert').textContent).toMatch(/no one is waiting/i);
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
    expect(joinButton()).toHaveProperty('disabled', false);
    // A late 'ready' on the abandoned socket must not navigate.
    act(() => ws.receive({ type: 'ready' }));
    expect(onJoined).not.toHaveBeenCalled();
  });

  it('DEF-C04: leaving the screen mid-join closes the lobby socket and never navigates', () => {
    const { onJoined, unmount } = renderJoin();
    fireEvent.click(joinButton());
    const ws = FakeWebSocket.instances[0];
    act(() => ws.open());
    unmount();
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
    ws.receive({ type: 'ready' });
    vi.advanceTimersByTime(JOIN_TIMEOUT_MS * 2);
    expect(onJoined).not.toHaveBeenCalled();
  });

  it('shows "full" only for session_full, and a distinct message for other errors', () => {
    renderJoin();
    fireEvent.click(joinButton());
    act(() => FakeWebSocket.instances[0].open());
    act(() => FakeWebSocket.instances[0].receive({ type: 'error', code: 'session_full' }));
    expect(screen.getByRole('alert').textContent).toMatch(/already full/i);

    fireEvent.click(joinButton());
    act(() => FakeWebSocket.instances[1].open());
    act(() => FakeWebSocket.instances[1].receive({ type: 'error', code: 'invalid_session' }));
    expect(screen.getByRole('alert').textContent).toMatch(/isn’t valid/i);
  });

  it('server unreachable shows an error and re-enables Join', () => {
    renderJoin();
    fireEvent.click(joinButton());
    act(() => FakeWebSocket.instances[0].onerror!());
    expect(screen.getByRole('alert').textContent).toMatch(/could not reach the server/i);
    expect(joinButton()).toHaveProperty('disabled', false);
  });

  it('the code input is labelled for assistive tech (not aria-hidden) and normalizes input', () => {
    renderJoin('');
    fireEvent.click(screen.getByRole('button', { name: /enter code/i }));
    const input = screen.getByLabelText('Channel code') as HTMLInputElement;
    expect(input.getAttribute('aria-hidden')).toBeNull();
    fireEvent.change(input, { target: { value: 'ab-1' } });
    expect(input.value).toBe('AB1');
  });

  it('sanitizes a junk ?join= value', () => {
    renderJoin('<img src=x>');
    fireEvent.click(screen.getByRole('button', { name: /enter code/i }));
    expect((screen.getByLabelText('Channel code') as HTMLInputElement).value).toBe('IMGSRC');
  });
});
