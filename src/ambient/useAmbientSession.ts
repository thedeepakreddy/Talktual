// UI seam for ambient mode: React reads the snapshot and calls commands; all
// behaviour lives in AmbientSession.

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { createDefaultDeps } from '../engine/defaultDeps';
import { AmbientSession, type AmbientConfig, type AmbientDeps, type AmbientSnapshot } from './AmbientSession';

const noopSubscribe = () => () => {};

export function createAmbientDeps(): AmbientDeps {
  const d = createDefaultDeps();
  return {
    createAsr: d.createAsr,
    translate: d.translate,
    playback: d.playback,
    capture: d.capture,
    browser: d.browser,
    storage: d.storage,
    getServerCapabilities: d.getServerCapabilities,
    clearCaches: d.clearCaches,
    onUserGesture: d.onUserGesture,
    processing: d.processing,
  };
}

export function useAmbientSession(config: AmbientConfig, depsFactory: () => AmbientDeps = createAmbientDeps) {
  const [session, setSession] = useState<AmbientSession | null>(null);

  useEffect(() => {
    const s = new AmbientSession(config, depsFactory());
    setSession(s);
    const w = window as unknown as { __talktualAmbientDiagnostics?: () => unknown };
    try {
      if (new URLSearchParams(window.location.search).get('debug') === '1') w.__talktualAmbientDiagnostics = () => s.diagnostics();
    } catch {
      /* no window */
    }
    return () => {
      s.dispose();
      delete w.__talktualAmbientDiagnostics;
      setSession(null);
    };
    // One session per screen visit; languages change through commands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const subscribe = useMemo(() => (session ? (fn: () => void) => session.subscribe(fn) : noopSubscribe), [session]);
  const getSnapshot = useMemo(() => (session ? () => session.getSnapshot() : () => null), [session]);
  const snapshot: AmbientSnapshot | null = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return { session, snapshot };
}
