import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { FakeWebSocket } from './test/fakes';

vi.mock('@yudiel/react-qr-scanner', () => ({ Scanner: () => null }));
vi.mock('./lib/audio', () => ({ unlockAudio: vi.fn(async () => {}), micPermission: vi.fn(async () => 'prompt'), captureAdapter: { permission: async () => 'prompt', release: () => {}, onDeviceLost: () => () => {} } }));
vi.mock('./lib/translate', async (orig) => ({ ...(await orig<object>()), prewarm: vi.fn() }));

const { default: App } = await import('./App');

beforeEach(() => {
  FakeWebSocket.instances = [];
  vi.stubGlobal('WebSocket', FakeWebSocket);
});
afterEach(() => {
  window.history.replaceState(null, '', '/');
  window.location.hash = '';
});

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

  it('Connect: Start without a name explains why and focuses the field; with a name it opens the invite screen', () => {
    window.history.replaceState(null, '', '/#/connect');
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: 'Start a call' }));
    expect(screen.getByText(/add your name/i)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Your name'), { target: { value: 'Ana' } });
    fireEvent.click(screen.getByRole('button', { name: 'Start a call' }));
    expect(screen.getByRole('heading', { name: 'Invite someone' })).toBeTruthy();
    // An active call flow hides the navigation (no accidental tab switch).
    expect(screen.queryByRole('navigation', { name: 'Main' })).toBeNull();
  });

  it('Connect: Join with a code goes to code entry; Back returns to Connect', () => {
    window.history.replaceState(null, '', '/#/connect');
    render(<App />);
    fireEvent.change(screen.getByLabelText('Your name'), { target: { value: 'Ana' } });
    fireEvent.click(screen.getByRole('button', { name: 'Join with a code' }));
    expect(screen.getByLabelText('Call code')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(screen.getByRole('heading', { name: 'Connect' })).toBeTruthy();
  });

  it('Home: AIR is the primary action; every destination is reachable from navigation', () => {
    window.history.replaceState(null, '', '/');
    render(<App />);
    expect(screen.getByRole('heading', { name: 'Translate the room around you' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Start listening' })).toBeTruthy();
    const nav = screen.getAllByRole('navigation', { name: 'Main' });
    for (const label of ['Home', 'Connect', 'AIR', 'Lens', 'Rewind']) expect(nav.some((n) => n.textContent?.includes(label))).toBe(true);
  });

  it('Lens and Rewind state plainly what is not available — no camera, no fake history', () => {
    window.history.replaceState(null, '', '/#/lens');
    const { unmount } = render(<App />);
    expect(screen.getByRole('heading', { name: 'Not available in this version' })).toBeTruthy();
    unmount();
    window.history.replaceState(null, '', '/#/rewind');
    render(<App />);
    expect(screen.getByRole('heading', { name: 'Nothing is kept' })).toBeTruthy();
    expect(screen.queryByRole('searchbox')).toBeNull();
  });

  it('Settings: theme preference applies to the document and is remembered', () => {
    window.history.replaceState(null, '', '/#/settings');
    render(<App />);
    fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(localStorage.getItem('talktual.theme.v1')).toBe('light');
    fireEvent.click(screen.getByRole('radio', { name: 'System' }));
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    expect(localStorage.getItem('talktual.theme.v1')).toBeNull();
  });
});
