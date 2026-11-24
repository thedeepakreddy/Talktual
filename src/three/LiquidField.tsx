import { useEffect, useRef } from 'react';
import { frameLoop, isLightTheme, loadThree, onThemeChange, prefersReducedMotion, webglAvailable, SIMPLEX3 } from './runtime';

export type FieldMood = 'rich' | 'session' | 'calm';
const MOOD_INDEX: Record<FieldMood, number> = { rich: 0, session: 1, calm: 2 };

const FRAG = /* glsl */ `
precision highp float;
uniform float uTime;
uniform vec2 uRes;
uniform float uLight;
uniform vec3 uMood;      // weights: x = top pool, y = bottom pool, z = centre bloom
varying vec2 vUv;
${SIMPLEX3}
float fbm(vec3 p){
  float v = 0.0, a = 0.5;
  for (int i = 0; i < 3; i++) { v += a * snoise(p); p = p * 1.9 + vec3(1.7, 9.2, 3.1); a *= 0.5; }
  return v;
}
float hash(vec2 p){ p = fract(p * vec2(123.34, 456.21)); p += dot(p, p + 45.32); return fract(p.x * p.y); }
void main(){
  vec2 uv = vUv;
  float asp = uRes.x / uRes.y;
  vec2 p = (uv - 0.5) * vec2(asp, 1.0);
  float t = uTime * 0.05;
  // Silky, low-frequency domain warp: broad liquid folds, not smoke.
  vec2 q = vec2(fbm(vec3(p * 0.75, t)), fbm(vec3(p * 0.75 + 5.2, -t * 0.8)));
  float f = fbm(vec3(p * 0.7 + 1.25 * q, t * 0.5)) * 0.5 + 0.5;
  // Light pools at the edges; the middle stays obsidian for the glass to sit on.
  // Pools sized against the SHORT side, so phones and desktops get the same light shape.
  vec2 iso = uRes / min(uRes.x, uRes.y);
  vec2 tp = (uv - vec2(0.92 + 0.04 * q.x, 1.04)) * iso * vec2(1.0, 0.95);
  vec2 bp = (uv - vec2(0.15 + 0.05 * q.y, -0.12)) * iso * vec2(0.9, 1.0);
  vec2 cp = (uv - vec2(0.62, 0.6)) * iso;
  float top    = exp(-dot(tp, tp) * 3.4);
  float bottom = exp(-dot(bp, bp) * 3.0);
  float centre = exp(-dot(cp, cp) * 5.0);
  float pools = uMood.x * top + uMood.y * bottom + uMood.z * centre;
  float heat = clamp(pools * (0.62 + 0.55 * f), 0.0, 1.0);

  vec3 col;
  if (uLight < 0.5) {
    vec3 c0 = vec3(0.043, 0.027, 0.020);
    vec3 c1 = vec3(0.20, 0.05, 0.012);
    vec3 c2 = vec3(0.66, 0.20, 0.025);
    vec3 c3 = vec3(1.00, 0.46, 0.10);
    vec3 c4 = vec3(1.00, 0.74, 0.48);
    col = mix(c0, c1, smoothstep(0.04, 0.34, heat));
    col = mix(col, c2, smoothstep(0.3, 0.68, heat));
    col = mix(col, c3, smoothstep(0.64, 0.92, heat));
    col = mix(col, c4, smoothstep(0.93, 1.0, heat) * 0.45);
    float vig = smoothstep(1.35, 0.25, length(p * vec2(0.8, 1.0)));
    col *= mix(0.7, 1.0, vig);
  } else {
    vec3 l0 = vec3(0.984, 0.961, 0.937);
    vec3 l1 = vec3(1.00, 0.88, 0.77);
    vec3 l2 = vec3(1.00, 0.72, 0.50);
    vec3 l3 = vec3(1.00, 0.57, 0.28);
    col = mix(l0, l1, smoothstep(0.03, 0.38, heat));
    col = mix(col, l2, smoothstep(0.34, 0.74, heat));
    col = mix(col, l3, smoothstep(0.74, 1.0, heat) * 0.5);
  }
  // Fine film grain keeps the gradients from banding.
  float g = hash(uv * uRes + fract(uTime * 0.37) * 61.0) - 0.5;
  col += g * (uLight < 0.5 ? 0.028 : 0.016);
  gl_FragColor = vec4(col, 1.0);
}
`;

const MOODS: Record<FieldMood, [number, number, number]> = {
  rich: [1.0, 0.92, 0.16],
  session: [0.34, 0.78, 0.0],
  calm: [0.62, 0.5, 0.0],
};

/**
 * The molten atmosphere behind every screen. Decorative: it never reflects
 * sound or listening. Lazy-loaded, rendered at reduced resolution (it is
 * soft by nature), capped at 30 fps, paused when hidden, a still frame under
 * reduced motion. Without WebGL the CSS gradient on <body> remains.
 */
export function LiquidField({ mood }: { mood: FieldMood }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const moodRef = useRef(mood);
  const setMood = useRef<(m: FieldMood) => void>(() => {});

  useEffect(() => {
    moodRef.current = mood;
    setMood.current(mood);
  }, [mood]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || !webglAvailable()) return;
    let disposed = false;
    let cleanup = () => {};

    // Start when the main thread is idle, so the first taps never wait on shader compilation.
    const idle = (fn: () => void) =>
      typeof window.requestIdleCallback === 'function' ? window.requestIdleCallback(fn, { timeout: 1500 }) : window.setTimeout(fn, 200);
    idle(() => void loadThree().then((THREE) => {
      if (disposed) return;
      const renderer = new THREE.WebGLRenderer({ antialias: false, alpha: false, powerPreference: 'low-power' });
      const scale = Math.min(1, (window.devicePixelRatio || 1) * 0.5);
      renderer.setPixelRatio(scale);
      const canvas = renderer.domElement;
      canvas.setAttribute('aria-hidden', 'true');
      canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity 900ms cubic-bezier(.22,1,.36,1)';
      host.appendChild(canvas);

      const scene = new THREE.Scene();
      const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
      const target = new THREE.Vector3(...MOODS[moodRef.current]);
      const uniforms = {
        uTime: { value: 0 },
        uRes: { value: new THREE.Vector2(1, 1) },
        uLight: { value: isLightTheme() ? 1 : 0 },
        uMood: { value: target.clone() },
      };
      const material = new THREE.ShaderMaterial({
        uniforms,
        vertexShader: 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }',
        fragmentShader: FRAG,
        depthTest: false,
        depthWrite: false,
      });
      const mesh = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), material);
      scene.add(mesh);

      const resize = () => {
        const w = host.clientWidth || window.innerWidth;
        const h = host.clientHeight || window.innerHeight;
        renderer.setSize(w, h, false);
        uniforms.uRes.value.set(w * scale, h * scale);
      };
      resize();

      const reduced = prefersReducedMotion();
      const start = performance.now() / 1000 - 40; // start mid-flow, not from a blank frame
      const loop = frameLoop((t) => {
        uniforms.uTime.value = reduced ? 40 : t - start;
        // Glide the light to the new route's mood.
        const m = uniforms.uMood.value;
        m.lerp(target, reduced ? 1 : 0.06);
        renderer.render(scene, camera);
        const settled = m.distanceTo(target) < 0.002;
        return !reduced || !settled;
      }, 30);

      setMood.current = (next) => {
        target.set(...MOODS[next]);
        loop.wake();
      };
      requestAnimationFrame(() => (canvas.style.opacity = '1'));
      loop.wake();

      const ro = new ResizeObserver(() => {
        resize();
        loop.wake();
      });
      ro.observe(host);
      const offTheme = onThemeChange(() => {
        uniforms.uLight.value = isLightTheme() ? 1 : 0;
        loop.wake();
      });
      const onLost = (e: Event) => {
        e.preventDefault();
        canvas.style.opacity = '0'; // CSS atmosphere shows through
      };
      canvas.addEventListener('webglcontextlost', onLost);

      cleanup = () => {
        loop.stop();
        ro.disconnect();
        offTheme();
        canvas.removeEventListener('webglcontextlost', onLost);
        material.dispose();
        mesh.geometry.dispose();
        renderer.dispose();
        canvas.remove();
      };
    }));

    return () => {
      disposed = true;
      cleanup();
    };
  }, []);

  return <div ref={hostRef} aria-hidden="true" data-mood={MOOD_INDEX[mood]} className="pointer-events-none fixed inset-0 -z-10 overflow-hidden" />;
}
