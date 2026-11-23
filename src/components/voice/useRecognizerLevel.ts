import { useEffect, useRef, useState } from 'react';
import { ActivityLevel } from './activityLevel';

/**
 * Feeds an ActivityLevel from real recognizer events: "speech detected" and
 * each new interim transcript. Optionally from a real microphone stream when
 * one is open (measured loudness takes over while it reports).
 */
export function useRecognizerLevel(
  capturing: boolean,
  hearing: boolean,
  partial: string,
  meter?: (onLevel: (v: number) => void) => (() => void) | null
): ActivityLevel {
  const [level] = useState(() => new ActivityLevel());
  const lastPartial = useRef(partial);

  useEffect(() => {
    if (!capturing) level.reset();
  }, [capturing, level]);

  useEffect(() => {
    level.setHearing(capturing && hearing);
  }, [capturing, hearing, level]);

  useEffect(() => {
    const prev = lastPartial.current;
    lastPartial.current = partial;
    if (!capturing || !partial || partial === prev) return;
    const added = partial.startsWith(prev) ? partial.length - prev.length : partial.length;
    level.bump(Math.max(1, added));
  }, [partial, capturing, level]);

  useEffect(() => {
    if (!capturing || !meter) return;
    const stop = meter((v) => level.setDirect(v));
    return () => stop?.();
  }, [capturing, meter, level]);

  return level;
}
