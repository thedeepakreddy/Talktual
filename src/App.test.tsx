import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { FakeWebSocket } from './test/fakes';

vi.mock('@yudiel/react-qr-scanner', () => ({ Scanner: () => null }));
vi.mock('./lib/audio', () => ({ unlockAudio: vi.fn(async () => {}) }));
vi.mock('./lib/translate', async (orig) => ({ ...(await orig<object>()), prewarm: vi.fn() }));

const { default: App } = await import('./App');

beforeEach(() => {
  FakeWebSocket.instances = [];
  vi.stubGlobal('WebSocket', FakeWebSocket);
});
afterEach(() => window.history.replaceState(null, '', '/'));

describe('App entry flows', () => {
  it('an invite link opens Join with the code, asking only for name and language (F2)', () => {
    window.history.replaceState(null, '', '/?join=abc123');
    render(<App />);
    expect(screen.getByText(/invited to call/i).textContent).toContain('ABC123');
    expect(screen.getByLabelText('Your name')).toBeTruthy();
    expect(screen.getByRole('button', { name: /You speak and hear/ })).toBeTruthy();
    // No code entry needed on this path.
    expect(screen.queryByLabelText('Call code')).toBeNull();
  });

  it('invite join without a name shows an inline error instead of a dead button (F11)', () => {
    window.history.replaceState(null, '', '/?join=abc123');
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: 'Join' }));
    expect(FakeWebSocket.instances).toHaveLength(0);
    const name = screen.getByLabelText('Your name');
    expect(name.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(name);
  });

  it('home: Start without a name explains why and focuses the field; with a name it opens the invite screen', () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: 'Start a call' }));
    expect(screen.getByText(/add your name/i)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Your name'), { target: { value: 'Ana' } });
    fireEvent.click(screen.getByRole('button', { name: 'Start a call' }));
    expect(screen.getByRole('heading', { name: 'Invite someone' })).toBeTruthy();
  });

  it('home: Join with a code goes to code entry; Back returns home', () => {
    render(<App />);
    fireEvent.change(screen.getByLabelText('Your name'), { target: { value: 'Ana' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join with a code' }));
    expect(screen.getByLabelText('Call code')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(screen.getByRole('heading', { name: 'Talk across languages' })).toBeTruthy();
  });
});
