// UI seam for ambient mode: React reads the snapshot and calls commands; all
// behaviour lives in AmbientSession.

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { createDefaultDeps } from '../engine/defaultDeps';
import { AmbientSession, type AmbientConfig, type AmbientDeps, type AmbientSnapshot, type DeviceSource } from './AmbientSession';
import { ValidationRecorder, coarseBrowser, validationEnabled } from './validation';

/** Counts of audio input/output devices — labels and ids are never read. */
function browserDevices(): DeviceSource | null {
  const md = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
  if (!md?.enumerateDevices || !md.addEventListener) return null;
  return {
    counts: async () => {
      const list = await md.enumerateDevices();
      return { inputs: list.filter((d) => d.kind === 'audioinput').length, outputs: list.filter((d) => d.kind === 'audiooutput').length };
    },
    subscribe: (fn) => {
      md.addEventListener('devicechange', fn);
      return () => md.removeEventListener('devicechange', fn);
    },
  };
}

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
    devices: browserDevices(),
    // Opt-in only (?validate=1 or ?debug=1); never persisted.
    recorder: typeof window !== 'undefined' && validationEnabled(window.location.search) ? new ValidationRecorder() : null,
  };
}

export function useAmbientSession(config: AmbientConfig, depsFactory: () => AmbientDeps = createAmbientDeps) {
  const [session, setSession] = useState<AmbientSession | null>(null);
  const [recorder, setRecorder] = useState<ValidationRecorder | null>(null);

  useEffect(() => {
    const deps = depsFactory();
    const s = new AmbientSession(config, deps);
    setSession(s);
    const rec = deps.recorder ?? null;
    setRecorder(rec);
    const w = window as unknown as { __talktualAmbientDiagnostics?: () => unknown; __talktualValidation?: unknown };
    if (rec) {
      const env = coarseBrowser(navigator.userAgent);
      w.__talktualValidation = { summary: () => rec.summary(env), mark: () => rec.marker(), clear: () => rec.clear() };
    }
    try {
      if (new URLSearchParams(window.location.search).get('debug') === '1') w.__talktualAmbientDiagnostics = () => s.diagnostics();
    } catch {
      /* no window */
    }
    return () => {
      s.dispose();
      delete w.__talktualAmbientDiagnostics;
      delete w.__talktualValidation;
      setSession(null);
      setRecorder(null);
    };
    // One session per screen visit; languages change through commands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const subscribe = useMemo(() => (session ? (fn: () => void) => session.subscribe(fn) : noopSubscribe), [session]);
  const getSnapshot = useMemo(() => (session ? () => session.getSnapshot() : () => null), [session]);
  const snapshot: AmbientSnapshot | null = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return { session, snapshot, recorder };
}
