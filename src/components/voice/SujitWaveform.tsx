import { useEffect, useRef } from 'react';
import { cn } from '../../utils';
import type { ActivityLevel } from './activityLevel';

/**
 * Sujit Waveform — a 3D bundle of light. Seven lead strands and a sheaf of fine
 * filaments share one envelope that peaks in the middle; each strand lies in
 * its own plane around a level time axis, so the bundle turns in depth. The
 * lead strand burns white-hot with an orange glow; the ends taper to nothing.
 * Dark theme: additive light. Light theme: ink with an ember lead.
 *
 * Honest by construction: height comes ONLY from `level` (real recognizer
 * events or a real microphone stream). No level → one calm, tapered line and
 * the animation loop stops.
 */
interface Strand {
  w: number; // line width
  o: number; // opacity
  s: number; // amplitude scale
  off: number; // plane / phase offset
  k: number; // frequency multiplier
  lead?: boolean;
}

const LEADS: Strand[] = [
  { w: 2.1, o: 1.0, s: 1.0, off: 0.0, k: 1.0, lead: true },
  { w: 1.4, o: 0.7, s: 0.88, off: 0.85, k: 1.03 },
  { w: 1.2, o: 0.55, s: 0.74, off: 1.75, k: 0.97 },
  { w: 1.1, o: 0.45, s: 0.95, off: 2.6, k: 1.06 },
  { w: 1.0, o: 0.36, s: 0.62, off: 3.5, k: 0.94 },
  { w: 1.0, o: 0.3, s: 0.8, off: 4.35, k: 1.09 },
  { w: 1.0, o: 0.26, s: 0.55, off: 5.25, k: 0.91 },
];
// Fine filaments clustered around the leads: the "many threads" richness.
const FILAMENTS: Strand[] = Array.from({ length: 14 }, (_, i) => ({
  w: 0.7,
  o: 0.16 + 0.18 * ((i * 7) % 5) / 4,
  s: 0.45 + 0.5 * (((i * 3) % 7) / 6),
  off: (i * 0.449 + 0.2) % (Math.PI * 2),
  k: 0.92 + 0.16 * (((i * 5) % 9) / 8),
}));
const STRANDS = [...FILAMENTS, ...LEADS.slice().reverse()];
const SAMPLES = 180;

export interface SujitWaveformProps {
  level: ActivityLevel;
  /** Capturing now. When false the wave settles to its idle line and stops. */
  active: boolean;
  /** Accessible description of what the wave shows right now. */
  label: string;
  className?: string;
}

export function SujitWaveform({ level, active, label, className }: SujitWaveformProps) {
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
    let shown = 0;
    let spin = 0;
    let travel = 0;
    let idleDrawn = false;
    let width = 0;
    let height = 0;
    let dpr = 1;
    let ink = '#fff7f0';
    let lit = true; // dark UI: additive light; light UI: ink
    let accent = '#ff7a1a';

    const readTheme = () => {
      const cs = getComputedStyle(canvas);
      ink = cs.getPropertyValue('--fg').trim() || ink;
      accent = cs.getPropertyValue('--accent').trim() || accent;
      lit = luminance(ink) > 0.5;
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

    // Ends fade to nothing; full strength through the middle.
    const taper = (color: string, alpha: number) => {
      const g = ctx.createLinearGradient(0, 0, width, 0);
      g.addColorStop(0, withAlpha(color, 0));
      g.addColorStop(0.14, withAlpha(color, alpha * 0.55));
      g.addColorStop(0.5, withAlpha(color, alpha));
      g.addColorStop(0.86, withAlpha(color, alpha * 0.55));
      g.addColorStop(1, withAlpha(color, 0));
      return g;
    };

    const path = (c: Strand, amp: number, cycles: number) => {
      const cy = height / 2;
      const plane = spin + c.off;
      const cos = Math.cos(plane);
      const sin = Math.sin(plane);
      ctx.beginPath();
      for (let k = 0; k <= SAMPLES; k++) {
        const t = k / SAMPLES;
        // Envelope peaks in the middle: the voice swells at the centre.
        const env = Math.pow(Math.sin(Math.PI * t), 1.6) * (0.16 + 0.84 * Math.exp(-(((t - 0.5) / 0.17) ** 2)));
        const v = amp * c.s * env * Math.sin(2 * Math.PI * cycles * c.k * t - travel + c.off);
        const y = v * cos;
        const z = v * sin;
        const x = t * width + z * 0.06;
        if (k === 0) ctx.moveTo(x, cy - y);
        else ctx.lineTo(x, cy - y);
      }
      return 0.55 + 0.45 * Math.abs(cos);
    };

    const draw = (now: number) => {
      const dt = last ? Math.min(0.05, (now - last) / 1000) : 0;
      last = now;
      const target = activeRef.current ? level.sample() : 0;
      shown += (target - shown) * (target > shown ? 0.32 : 0.1);
      if (shown < 0.002) shown = 0;
      if (shown === 0 && idleDrawn) return;
      if (!reduced) {
        spin += dt * (0.22 + 1.0 * shown);
        travel += dt * (1.1 + 2.4 * shown);
      }

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';

      if (shown === 0) {
        // Idle: one calm tapered line with a soft light at its centre.
        const cy = height / 2;
        ctx.globalCompositeOperation = 'source-over';
        ctx.beginPath();
        ctx.moveTo(0, cy);
        ctx.lineTo(width, cy);
        ctx.strokeStyle = taper(ink, lit ? 0.55 : 0.4);
        ctx.lineWidth = 1.25;
        ctx.stroke();
        // Soft light at the centre, an ellipse that always fits inside the canvas.
        const r = Math.min(width * 0.18, 120);
        const ry = Math.min(r, height / 2) / r;
        ctx.save();
        ctx.translate(width / 2, cy);
        ctx.scale(1, ry);
        const glow = ctx.createRadialGradient(0, 0, 0, 0, 0, r);
        glow.addColorStop(0, withAlpha(accent, lit ? 0.22 : 0.14));
        glow.addColorStop(1, withAlpha(accent, 0));
        ctx.fillStyle = glow;
        ctx.fillRect(-r, -r, r * 2, r * 2);
        ctx.restore();
        idleDrawn = true;
        return;
      }
      idleDrawn = false;

      // Perceptual curve: ordinary speech already swells the middle.
      const amp = height * 0.49 * Math.pow(shown, 0.55);
      const cycles = 3 + 1.6 * level.busyness();
      ctx.globalCompositeOperation = lit ? 'lighter' : 'source-over';

      for (const c of STRANDS) {
        const depth = path(c, amp, cycles);
        if (c.lead) {
          // Orange halo under a white-hot core.
          ctx.shadowColor = accent;
          ctx.shadowBlur = lit ? 18 : 8;
          ctx.strokeStyle = taper(accent, lit ? 0.55 : 0.5);
          ctx.lineWidth = lit ? 5 : 3.2;
          ctx.stroke();
          ctx.shadowBlur = 0;
          ctx.strokeStyle = taper(lit ? '#ffffff' : darken(accent), 1);
          ctx.lineWidth = c.w;
          ctx.stroke();
        } else {
          ctx.strokeStyle = taper(lit ? '#ffe6d2' : ink, c.o * depth * (lit ? 0.9 : 0.75));
          ctx.lineWidth = c.w;
          ctx.stroke();
        }
      }
      ctx.globalCompositeOperation = 'source-over';
    };

    const loop = (now: number) => {
      raf = 0;
      draw(now);
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
  }, [level]);

  useEffect(() => {
    if (active) kick.current();
  }, [active]);

  return <canvas ref={canvasRef} role="img" aria-label={label} className={cn('block w-full', className)} />;
}

function rgb(color: string): [number, number, number] {
  const m = color.replace('#', '').match(/^([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i);
  if (!m) return [255, 255, 255];
  return [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)];
}
function withAlpha(color: string, a: number): string {
  const [r, g, b] = rgb(color);
  return `rgba(${r},${g},${b},${Math.max(0, Math.min(1, a))})`;
}
function darken(color: string): string {
  const [r, g, b] = rgb(color);
  return `rgb(${Math.round(r * 0.72)},${Math.round(g * 0.6)},${Math.round(b * 0.5)})`;
}
function luminance(color: string): number {
  const [r, g, b] = rgb(color).map((v) => v / 255);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
