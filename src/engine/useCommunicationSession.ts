// The UI seam: React reads the session snapshot and calls commands. All
// realtime behaviour lives in CommunicationSession, not in components.

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { CommunicationSession, SessionConfig, SessionDeps, SessionSnapshot } from './CommunicationSession';
import { createDefaultDeps } from './defaultDeps';

const noopSubscribe = () => () => {};

function debugEnabled(): boolean {
  try {
    return new URLSearchParams(window.location.search).get('debug') === '1' || window.localStorage.getItem('talktualDebug') === '1';
  } catch {
    return false;
  }
}

export function useCommunicationSession(
  config: SessionConfig,
  depsFactory: () => SessionDeps = createDefaultDeps
): { session: CommunicationSession | null; snapshot: SessionSnapshot | null } {
  const [session, setSession] = useState<CommunicationSession | null>(null);

  // One session per room. Created in an effect (not during render) so React
  // StrictMode's mount/unmount/mount gets a fresh, fully disposed session.
  useEffect(() => {
    const s = new CommunicationSession(config, depsFactory());
    setSession(s);
    s.start();
    const w = window as unknown as { __talktualDiagnostics?: () => unknown };
    if (debugEnabled()) w.__talktualDiagnostics = () => s.getDiagnostics();
    return () => {
      s.dispose();
      if (w.__talktualDiagnostics) delete w.__talktualDiagnostics;
      setSession(null);
    };
    // Only the room identifies a session; language/name changes are commands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config.roomCode]);

  useEffect(() => {
    session?.setLanguage(config.lang);
  }, [session, config.lang]);

  const subscribe = useMemo(() => session?.subscribe ?? noopSubscribe, [session]);
  const getSnapshot = useMemo(() => session?.getSnapshot ?? (() => null), [session]);
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return { session, snapshot };
}
