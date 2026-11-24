import { useEffect, useRef } from 'react';
import { cn } from '../utils';
import type { ActivityLevel } from '../components/voice/activityLevel';
import { frameLoop, isLightTheme, loadThree, onThemeChange, prefersReducedMotion, webglAvailable, SIMPLEX3 } from './runtime';

const VERT = /* glsl */ `
uniform float uFlow;
uniform float uLevel;
varying vec3 vN;
varying vec3 vObjN;
varying vec3 vPos;
${SIMPLEX3}
void main(){
  vec3 n = normal;
  // Surface ripples only from the real voice level (a faint, fixed relief otherwise).
  float d = snoise(n * 1.7 + vec3(0.0, 0.0, uFlow));
  float d2 = snoise(n * 3.6 - vec3(uFlow * 0.7));
  float disp = d * (0.012 + 0.11 * uLevel) + d2 * 0.03 * uLevel;
  vec3 p = position + n * disp;
  vObjN = n;
  vN = normalize(normalMatrix * n);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  vPos = mv.xyz;
  gl_Position = projectionMatrix * mv;
}
`;

const FRAG = /* glsl */ `
precision highp float;
uniform float uFlow;
uniform float uLevel;
uniform float uLight;
varying vec3 vN;
varying vec3 vObjN;
varying vec3 vPos;
${SIMPLEX3}
void main(){
  vec3 N = normalize(vN);
  vec3 V = normalize(-vPos);
  float ndv = max(dot(N, V), 0.0);
  float fres = pow(1.0 - ndv, 2.4);
  vec3 L = normalize(vec3(-0.55, 0.75, 0.55));
  float diff = max(dot(N, L), 0.0);

  // Molten interior seen through glass: broad, slow, smooth bands (no grit).
  vec3 q = vObjN * 1.05 + vec3(uFlow * 0.3, -uFlow * 0.2, uFlow * 0.16);
  float flow = snoise(q) * 0.5 + 0.5;

  vec3 deep   = vec3(0.20, 0.035, 0.006);
  vec3 ember  = vec3(0.74, 0.22, 0.025);
  vec3 cosmic = vec3(1.00, 0.47, 0.10);
  vec3 hot    = vec3(1.00, 0.76, 0.50);

  float heat = diff * 0.86 + flow * 0.16 + uLevel * 0.2;
  vec3 col = mix(deep, ember, smoothstep(0.08, 0.46, heat));
  col = mix(col, cosmic, smoothstep(0.42, 0.8, heat));
  col = mix(col, hot, smoothstep(0.88, 1.12, heat));
  // Light scattering inside the glass: the core glows toward the viewer.
  col += cosmic * pow(ndv, 3.0) * 0.18 * (0.6 + flow * 0.8);

  // Soft shadowed limb (lower right).
  float shade = smoothstep(0.0, 0.9, dot(N, normalize(vec3(0.6, -0.5, 0.3))));
  col *= mix(1.0, 0.32, shade);

  // Glass: crisp key specular, a broad sheen, a window reflection below, a warm rim.
  vec3 H = normalize(L + V);
  float spec = pow(max(dot(N, H), 0.0), 140.0);
  float sheen = pow(max(dot(N, H), 0.0), 10.0);
  vec3 L2 = normalize(vec3(0.45, -0.7, 0.55));
  float spec2 = pow(max(dot(N, normalize(L2 + V)), 0.0), 34.0);
  col += vec3(1.0, 0.96, 0.9) * spec * 1.25 + vec3(1.0, 0.78, 0.55) * sheen * 0.1 + vec3(1.0, 0.6, 0.3) * spec2 * 0.22;
  col += vec3(1.0, 0.5, 0.14) * fres * (0.5 + 0.8 * uLevel);
  if (uLight > 0.5) col = mix(col, col * 1.06 + 0.03, 0.5);

  float alpha = smoothstep(0.0, 0.08, ndv) * 0.98 + fres * 0.02;
  gl_FragColor = vec4(col, alpha);
}
`;

export interface VoiceOrbProps {
  /** Real voice level source. Without it the orb never ripples. */
  level?: ActivityLevel | null;
  /** Capturing now: the interior flows slowly (a true state, not a level). */
  active?: boolean;
  /** Decorative idle drift (Home hero only; never implies listening). */
  ambient?: boolean;
  className?: string;
}

/**
 * Molten glass sphere. Ripples and rim light follow ONLY the real level; the
 * interior flows only while capturing (or a slow drift for the decorative
 * Home orb). Still frame under reduced motion. CSS orb fallback without WebGL.
 */
export function VoiceOrb({ level, active = false, ambient = false, className }: VoiceOrbProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const haloRef = useRef<HTMLDivElement>(null);
  const state = useRef({ level, active, ambient, wake: () => {} });
  state.current.level = level;
  state.current.active = active;
  state.current.ambient = ambient;

  useEffect(() => {
    state.current.wake();
  }, [active, ambient, level]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || !webglAvailable()) return;
    let disposed = false;
    let cleanup = () => {};

    void loadThree().then((THREE) => {
      if (disposed) return;
      const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'high-performance', premultipliedAlpha: false });
      renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
      renderer.setClearColor(0x000000, 0);
      const canvas = renderer.domElement;
      canvas.setAttribute('aria-hidden', 'true');
      canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity 700ms cubic-bezier(.22,1,.36,1)';
      host.appendChild(canvas);
      host.dataset.webgl = 'on';

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(30, 1, 0.1, 20);
      camera.position.set(0, 0, 4.6);
      const uniforms = { uFlow: { value: 7.3 }, uLevel: { value: 0 }, uLight: { value: isLightTheme() ? 1 : 0 } };
      const material = new THREE.ShaderMaterial({ uniforms, vertexShader: VERT, fragmentShader: FRAG, transparent: true });
      const geometry = new THREE.IcosahedronGeometry(1, 64);
      const mesh = new THREE.Mesh(geometry, material);
      mesh.rotation.set(0.2, -0.5, 0);
      scene.add(mesh);

      const resize = () => {
        const w = host.clientWidth;
        const h = host.clientHeight;
        if (!w || !h) return;
        renderer.setSize(w, h, false);
        camera.aspect = w / h;
        // Keep the sphere fully inside the smaller side.
        const fit = w < h ? h / w : 1;
        camera.position.z = 4.6 * fit;
        camera.updateProjectionMatrix();
      };
      resize();

      const reduced = prefersReducedMotion();
      let shown = 0;
      let rendered = false;
      const loop = frameLoop((_t, dt) => {
        const s = state.current;
        const target = s.active && s.level ? s.level.sample() : 0;
        shown += (target - shown) * (target > shown ? 0.3 : 0.08);
        if (shown < 0.002) shown = 0;
        uniforms.uLevel.value = shown;
        const flowing = !reduced && (s.active || s.ambient);
        if (flowing) {
          uniforms.uFlow.value += dt * (s.active ? 0.32 + shown * 1.4 : 0.09);
          mesh.rotation.y += dt * (s.active ? 0.05 + shown * 0.25 : 0.025);
        }
        if (haloRef.current) haloRef.current.style.opacity = String(0.55 + shown * 0.45);
        if (haloRef.current) haloRef.current.style.transform = `scale(${1 + shown * 0.12})`;
        renderer.render(scene, camera);
        rendered = true;
        // Sleep when nothing can change: not flowing and the level has settled.
        return flowing || shown > 0 || !rendered;
      }, 60);
      state.current.wake = loop.wake;
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

      cleanup = () => {
        loop.stop();
        ro.disconnect();
        offTheme();
        material.dispose();
        geometry.dispose();
        renderer.dispose();
        canvas.remove();
        delete host.dataset.webgl;
      };
    });

    return () => {
      disposed = true;
      cleanup();
    };
  }, []);

  return (
    <div className={cn('relative', className)} aria-hidden="true">
      {/* Orange light the orb casts into the glass around it. */}
      <div
        ref={haloRef}
        className="pointer-events-none absolute inset-[-28%] rounded-full opacity-60 transition-transform duration-300"
        style={{ background: 'radial-gradient(closest-side, color-mix(in oklab, var(--accent) 42%, transparent), transparent 72%)', filter: 'blur(8px)' }}
      />
      <div ref={hostRef} className="group absolute inset-0">
        {/* CSS sphere until WebGL is ready (or without WebGL). */}
        <div className="orb-core absolute inset-[9%] rounded-full transition-opacity duration-500 group-data-[webgl=on]:opacity-0" />
      </div>
    </div>
  );
}
