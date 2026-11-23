import { useEffect, useRef } from 'react';
import { cn } from '../../utils';
import type { ActivityLevel } from './activityLevel';

/**
 * Sujit Waveform: a 3D voice waveform. Seven sine curves share one envelope;
 * each lies in its own plane around the time axis, so together they form a
 * slowly turning bundle. Light strokes glow on the dark theme and draw as ink
 * on the light theme.
 *
 * Honest by construction: height comes only from `level` (real recognizer
 * events or a real microphone stream, see ActivityLevel). With no level the
 * curves collapse onto one calm line, and the animation loop stops when the
 * wave has nothing to show.
 */
const CURVES = [
  { w: 2.0, o: 1.0, s: 1.0, off: 0.0 },
  { w: 1.3, o: 0.62, s: 0.86, off: 0.9 },
  { w: 1.1, o: 0.46, s: 0.72, off: 1.9 },
  { w: 1.0, o: 0.34, s: 0.95, off: 2.7 },
  { w: 1.0, o: 0.26, s: 0.62, off: 3.6 },
  { w: 1.0, o: 0.2, s: 0.8, off: 4.4 },
  { w: 1.0, o: 0.16, s: 0.55, off: 5.3 },
];
const SAMPLES = 150;

export interface SujitWaveformProps {
  level: ActivityLevel;
  /** Capturing now. When false the wave settles to its idle line and stops. */
  active: boolean;
  mode?: '3d' | 'flat';
  /** Accessible description of what the wave shows right now. */
  label: string;
  className?: string;
}

export function SujitWaveform({ level, active, mode = '3d', label, className }: SujitWaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const kick = useRef<() => void>(() => {});

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const reduced = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

    let raf = 0;
    let last = 0;
    let shown = 0; // smoothed amplitude 0..1
    let spin = 0;
    let travel = 0;
    let idleDrawn = false;
    let width = 0;
    let height = 0;
    let dpr = 1;
    let color = '#F2EEE7';
    let glow = true;

    const readTheme = () => {
      const cs = getComputedStyle(canvas);
      color = cs.getPropertyValue('--fg').trim() || color;
      // Light strokes glow (dark UI); dark strokes draw as ink (light UI).
      glow = luminance(color) > 0.5;
    };

    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      dpr = Math.min(2, window.devicePixelRatio || 1);
      width = Math.max(1, rect.width);
      height = Math.max(1, rect.height);
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      idleDrawn = false;
      readTheme();
      draw(performance.now());
    };

    const draw = (now: number) => {
      const dt = last ? Math.min(0.05, (now - last) / 1000) : 0;
      last = now;
      const target = activeRef.current ? level.sample() : 0;
      // Rise quickly, fall gently.
      shown += (target - shown) * (target > shown ? 0.35 : 0.12);
      if (shown < 0.002) shown = 0;

      if (shown === 0 && idleDrawn) return; // nothing changed: don't repaint
      if (!reduced) {
        spin += dt * (0.25 + 1.1 * shown);
        travel += dt * (1.2 + 2.2 * shown);
      }

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';

      const pad = 6;
      const x0 = pad;
      const x1 = width - pad;
      const cy = height / 2;
      const tilt = mode === '3d' ? height * 0.1 : 0; // the axis rises left → right
      const amp = height * 0.4 * shown;
      const cycles = 3 + 1.6 * level.busyness();

      const curves = shown === 0 ? CURVES.slice(0, 1) : CURVES;
      for (let i = curves.length - 1; i >= 0; i--) {
        const c = curves[i];
        const plane = mode === '3d' ? spin + c.off : c.off * 0.35;
        const cos = Math.cos(plane);
        const sin = Math.sin(plane);
        ctx.beginPath();
        for (let k = 0; k <= SAMPLES; k++) {
          const t = k / SAMPLES;
          const env = Math.pow(Math.sin(Math.PI * t), 1.3) * (0.3 + 0.7 * Math.exp(-(((t - 0.46) / 0.2) ** 2)));
          const v = amp * c.s * env * Math.sin(2 * Math.PI * cycles * (1 + (i - 3) * 0.035) * t - travel + c.off);
          const y = mode === '3d' ? v * cos : v * (0.75 + 0.25 * cos);
          const z = mode === '3d' ? v * sin : 0;
          const x = x0 + (x1 - x0) * t + z * 0.08;
          const py = cy + tilt * (0.5 - t) * 2 * 0.5 - y;
          if (k === 0) ctx.moveTo(x, py);
          else ctx.lineTo(x, py);
        }
        const depth = mode === '3d' ? 0.6 + 0.4 * Math.abs(cos) : 1;
        ctx.globalAlpha = shown === 0 ? 0.55 : c.o * depth;
        ctx.strokeStyle = color;
        ctx.lineWidth = (shown === 0 ? 1.25 : c.w) * (glow ? 1 : 1.1);
        if (glow && shown > 0) {
          ctx.shadowColor = color;
          ctx.shadowBlur = i === 0 ? 10 : 4;
        } else {
          ctx.shadowBlur = 0;
        }
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      idleDrawn = shown === 0;
    };

    const loop = (now: number) => {
      raf = 0;
      draw(now);
      // Keep animating only while there is something real to show.
      if (!document.hidden && (activeRef.current || shown > 0)) raf = requestAnimationFrame(loop);
    };
    const start = () => {
      if (!raf) {
        last = 0;
        raf = requestAnimationFrame(loop);
      }
    };
    kick.current = start;

    resize();
    start();
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(resize) : null;
    ro?.observe(canvas);
    const onVisible = () => !document.hidden && start();
    document.addEventListener('visibilitychange', onVisible);
    const scheme = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;
    const onTheme = () => {
      readTheme();
      idleDrawn = false;
      draw(performance.now());
    };
    scheme?.addEventListener?.('change', onTheme);
    const mo = new MutationObserver(onTheme);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class'] });
    return () => {
      cancelAnimationFrame(raf);
      ro?.disconnect();
      mo.disconnect();
      document.removeEventListener('visibilitychange', onVisible);
      scheme?.removeEventListener?.('change', onTheme);
    };
  }, [level, mode]);

  // Starting to capture restarts the loop.
  useEffect(() => {
    if (active) kick.current();
  }, [active]);

  return <canvas ref={canvasRef} role="img" aria-label={label} className={cn('block w-full', className)} />;
}

function luminance(color: string): number {
  const m = color.replace('#', '').match(/^([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i);
  if (!m) return 1;
  const [r, g, b] = m.slice(1).map((h) => parseInt(h, 16) / 255);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
