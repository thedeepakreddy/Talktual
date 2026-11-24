// Engine Lab controller: one WebSocket to the Talktual ↔ CE bridge for the
// page's lifetime, microphone capture only between start and stop/abort,
// and the pure reducer in labState.ts for everything shown.

import { useCallback, useEffect, useReducer, useRef } from 'react';
import { CE_BRIDGE_URL, RELEASE_TAIL_MS } from '../lib/config';
import { setMicEnabled } from '../lib/audio';
import { startMicPcm, toPcm16, type PcmCapture, type StartPcmCapture, type WebSocketLike } from '../lib/ceSpeech';
import { initialLabState, labReducer, type LabState } from './labState';

export type LabMode = 'push-to-talk' | 'hands-free';

export interface LabDeps {
  openSocket?: (url: string) => WebSocketLike;
  startCapture?: StartPcmCapture;
}

export interface EngineLab {
  state: LabState;
  url: string;
  connect(url?: string): void;
  disconnect(): void;
  start(opts: { mode: LabMode; analysis: boolean; lang: string; semantics?: boolean }): Promise<void>;
  /** Phase 5: clear the semantic context. */
  resetContext(): void;
  /** Phase 5: record that the user confirmed an observation. */
  confirm(observation: string): void;
  stop(): void;
  abort(): void;
  reset(): void;
  refreshHealth(): void;
}

export function useEngineLab(deps: LabDeps = {}): EngineLab {
  const [state, dispatch] = useReducer(labReducer, initialLabState);
  const ws = useRef<WebSocketLike | null>(null);
  const capture = useRef<PcmCapture | null>(null);
  const ready = useRef(false);
  const urlRef = useRef(CE_BRIDGE_URL);
  const openSocket = deps.openSocket ?? ((u: string) => new WebSocket(u) as unknown as WebSocketLike);
  const startCapture = deps.startCapture ?? startMicPcm;

  const stopCapture = useCallback(() => {
    capture.current?.stop();
    capture.current = null;
    setMicEnabled(false);
  }, []);

  const send = useCallback((m: object) => {
    const s = ws.current;
    if (s && s.readyState === 1) s.send(JSON.stringify(m));
  }, []);

  const connect = useCallback(
    (url?: string) => {
      if (url) urlRef.current = url;
      ws.current?.close();
      dispatch({ type: 'connecting' });
      let s: WebSocketLike;
      try {
        s = openSocket(urlRef.current);
      } catch {
        dispatch({ type: 'disconnected', error: `Cannot open ${urlRef.current}` });
        return;
      }
      ws.current = s;
      s.binaryType = 'arraybuffer';
      s.onopen = () => {
        dispatch({ type: 'connected' });
        s.send(JSON.stringify({ type: 'info' }));
      };
      s.onmessage = (e) => {
        if (typeof e.data !== 'string') return;
        let msg: { type?: string };
        try {
          msg = JSON.parse(e.data);
        } catch {
          return;
        }
        if (msg.type === 'ready') ready.current = true;
        if (msg.type === 'end' || (msg.type === 'error' && (msg as { fatal?: boolean }).fatal)) {
          ready.current = false;
          stopCapture();
        }
        dispatch({ type: 'message', msg, at: Date.now() });
      };
      s.onerror = () => dispatch({ type: 'disconnected', error: `The bridge is not reachable at ${urlRef.current}. Start it (see the bridge README) and connect again.` });
      s.onclose = () => {
        if (ws.current === s) {
          ws.current = null;
          ready.current = false;
          stopCapture();
          dispatch({ type: 'disconnected' });
        }
      };
    },
    [openSocket, stopCapture]
  );

  const disconnect = useCallback(() => {
    const s = ws.current;
    ws.current = null;
    stopCapture();
    s?.close();
    dispatch({ type: 'disconnected' });
  }, [stopCapture]);

  const start = useCallback(
    async ({ mode, analysis, lang, semantics = false }: { mode: LabMode; analysis: boolean; lang: string; semantics?: boolean }) => {
      if (!ws.current || ws.current.readyState !== 1 || capture.current) return;
      dispatch({ type: 'session-start' });
      ready.current = false;
      const held: ArrayBuffer[] = [];
      let cap: PcmCapture;
      try {
        cap = await startCapture((chunk) => {
          const buf = toPcm16(chunk);
          if (ready.current) ws.current?.send(buf);
          else if (held.length < 150) held.push(buf); // ≤ ~3 s before ready
        });
      } catch (err) {
        dispatch({ type: 'message', msg: { type: 'error', code: 'microphone', message: String(err), fatal: true }, at: Date.now() });
        return;
      }
      capture.current = cap;
      setMicEnabled(true);
      send({
        type: 'start',
        lang,
        sampleRate: cap.sampleRate,
        segmentation: mode === 'hands-free' ? 'activity' : 'continuous',
        diagnostics: true,
        analysis,
        semantics,
      });
      // Flush what was captured while the bridge was getting ready.
      const flush = () => {
        if (!ready.current) {
          if (capture.current === cap) setTimeout(flush, 20);
          return;
        }
        held.splice(0).forEach((b) => ws.current?.send(b));
      };
      flush();
    },
    [send, startCapture]
  );

  const stop = useCallback(() => {
    if (!capture.current) return;
    const cap = capture.current;
    setTimeout(() => {
      if (capture.current === cap) stopCapture();
      send({ type: 'stop' });
    }, RELEASE_TAIL_MS);
  }, [send, stopCapture]);

  const abort = useCallback(() => {
    stopCapture();
    send({ type: 'abort' });
  }, [send, stopCapture]);

  const reset = useCallback(() => send({ type: 'reset' }), [send]);
  const refreshHealth = useCallback(() => send({ type: 'health' }), [send]);
  const resetContext = useCallback(() => send({ type: 'reset-context' }), [send]);
  const confirm = useCallback((observation: string) => send({ type: 'confirm', observation }), [send]);

  useEffect(() => () => {
    capture.current?.stop();
    ws.current?.close();
  }, []);

  return { state, url: urlRef.current, connect, disconnect, start, stop, abort, reset, refreshHealth, resetContext, confirm };
}
