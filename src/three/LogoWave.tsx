import { useEffect, useRef } from 'react';
import { cn } from '../utils';
import type { ActivityLevel } from '../components/voice/activityLevel';
import { frameLoop, isLightTheme, loadThree, onThemeChange, prefersReducedMotion, webglAvailable } from './runtime';

// ============================================================================
// LogoWave — the Talktual mark as living liquid glass.
//
// The logo's two waves (upper: big crest then small; lower: small trough then
// big) are two slabs of orange glass. Each outer edge is a simulated elastic
// string (masses, springs, tension, damping; 240 Hz sub-steps): ripples travel,
// reflect at the ends and settle exactly back into the logo.
//
// Voice (REAL level only): drives the strings and kicks splashes; sustained
// speech morphs the logo into a full-width living waveform with the same
// interleaved crests, the split line becoming a line of light, with a 3D swirl
// mid-transition; after a calm it springs back into the logo.
//
// RGB reflection: three colour copies of each edge follow the liquid at
// different rates. At rest they coincide (white); in motion they fan apart —
// chromatic dispersion that comes from the physics, not a filter.
// ============================================================================

const N = 128; // string nodes per wave
const FILL_ROWS = 10;
const GLOW_ROWS = 6;
const ROWS = FILL_ROWS + 1 + GLOW_ROWS;

// Logo geometry (from the 96×48 mark): baselines at 22.5 / 25.5; crests 21.5 / 15.
const CREST_BIG = 21.5 / 47;
const CREST_SMALL = 15 / 47;
const GAP_HALF = 1.5 / 47;

const hump = (u: number, a: number, b: number) => (u <= a || u >= b ? 0 : Math.sin((Math.PI * (u - a)) / (b - a)) ** 2);
const LOGO_TOP = new Float32Array(N);
const LOGO_BOT = new Float32Array(N);
const U = new Float32Array(N);
for (let i = 0; i < N; i++) {
  const u = -1 + (2 * i) / (N - 1);
  U[i] = u;
  LOGO_TOP[i] = CREST_BIG * hump(u, -1, 0.191) + CREST_SMALL * hump(u, 0.191, 1);
  LOGO_BOT[i] = CREST_SMALL * hump(u, -1, -0.17) + CREST_BIG * hump(u, -0.17, 1);
}

const FILL_VERT = /* glsl */ `
attribute float aU; attribute float aT; attribute float aG; attribute float aH; attribute float aVel;
varying float vU; varying float vT; varying float vG; varying float vH; varying float vVel;
void main(){
  vU = aU; vT = aT; vG = aG; vH = aH; vVel = aVel;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
const FILL_FRAG = /* glsl */ `
precision highp float;
uniform float uTime, uLevel, uLight, uLw, uSweep;
varying float vU; varying float vT; varying float vG; varying float vH; varying float vVel;
vec3 ramp(float x){
  vec3 a = vec3(0.886, 0.337, 0.047), b = vec3(1.0, 0.478, 0.102), c = vec3(1.0, 0.69, 0.44);
  float t = clamp(x * 0.5 + 0.5, 0.0, 1.0);
  return t < 0.5 ? mix(a, b, t * 2.0) : mix(b, c, (t - 0.5) * 2.0);
}
void main(){
  float thick = smoothstep(0.0, uLw * 0.01, vH);
  if (vG > 0.0) {
    // Light leaving the glass edge: a tight bloom that hugs the crest.
    float g = exp(-pow(vG * 2.3, 2.0)) * (0.5 + 0.55 * uLevel) * smoothstep(uLw * 0.01, uLw * 0.08, vH);
    vec3 col = mix(ramp(vU), vec3(1.0, 0.86, 0.72), 0.3) * g;
    if (uLight > 0.5) { float a = g * 0.28; gl_FragColor = vec4(vec3(1.0, 0.62, 0.32) * a, a); }
    else { gl_FragColor = vec4(col * 0.75, 0.0); }
    return;
  }
  float d = min(vT, 1.0 - vT) * vH;          // distance to the nearest long edge
  float p = smoothstep(0.0, uLw * 0.085, d); // 0 at the rim → 1 inside
  float film = 1.0 - p;
  vec3 col = ramp(vU) * 1.1;
  // Lit from within.
  col = mix(col, vec3(1.0, 0.88, 0.74), 0.38 * p * (1.0 - min(1.0, abs(vT - 0.55) * 1.7)));
  // Glass thickness: a slightly deeper band just inside the edge.
  float thickBand = smoothstep(0.25, 0.55, film) * (1.0 - smoothstep(0.7, 0.95, film));
  col *= 1.0 - 0.2 * thickBand;
  // Caustics: faint lines of light drifting through the glass.
  float caustic = pow(0.5 + 0.5 * sin(vU * 17.0 + vT * 5.0 + uTime * 0.9), 14.0) + pow(0.5 + 0.5 * sin(vU * 11.0 - vT * 7.0 - uTime * 0.6), 18.0);
  col += vec3(1.0, 0.93, 0.85) * caustic * 0.12 * p;
  // Thin-film RGB reflection on the rims, drifting with time and position.
  vec3 irid = 0.5 + 0.5 * cos(6.28318 * (film * 1.25 + vU * 0.42 + uTime * 0.12 + vec3(0.0, 0.33, 0.67)));
  col = mix(col, irid, film * film * 0.7);
  // Depth: the glass deepens toward the split line…
  col *= mix(0.78, 1.0, smoothstep(0.0, 0.45, vT));
  // …a glossy liquid cap catches the light just under the crest…
  float cap = exp(-pow((vT - 0.8) * 5.5, 2.0)) * p;
  col = mix(col, vec3(1.0, 0.95, 0.88), cap * 0.32);
  // …and a crisp line of light runs along the crest.
  float crest = exp(-pow((1.0 - vT) * vH / (uLw * 0.011), 2.0));
  col += vec3(1.0, 0.97, 0.93) * crest * 0.95;
  // A reflection band sweeping across the glass, split into R, G and B.
  float sx = vU * 1.3 - vT * 0.45 - uSweep;
  vec3 sweep = vec3(exp(-pow((sx + 0.04) * 5.0, 2.0)), exp(-pow(sx * 5.0, 2.0)), exp(-pow((sx - 0.04) * 5.0, 2.0)));
  col += sweep * 0.42 * p;
  // Faster liquid runs hotter.
  col *= 1.0 + 0.22 * uLevel + 0.7 * clamp(abs(vVel), 0.0, 0.45);
  float a = mix(0.88, 1.0, film) * thick;
  gl_FragColor = vec4(col * a, a);
}
`;

const LINE_VERT = /* glsl */ `
attribute float aV; attribute float aFade;
varying float vV; varying float vFade;
void main(){ vV = aV; vFade = aFade; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }
`;
const LINE_FRAG = /* glsl */ `
precision highp float;
uniform vec3 uColor; uniform float uIntensity; uniform float uLight;
varying float vV; varying float vFade;
void main(){
  float core = exp(-pow(vV * 2.2, 2.0));
  float a = core * uIntensity * vFade;
  if (uLight > 0.5) gl_FragColor = vec4(uColor * a, a);
  else gl_FragColor = vec4(uColor * a, 0.0);
}
`;

export interface LogoWaveProps {
  /** Real voice level. Without it the logo never ripples or becomes a wave. */
  level?: ActivityLevel | null;
  /** Capturing now: a faint travelling breath along the glass (a true state). */
  active?: boolean;
  /** Decorative slow drift (Home); never implies listening, never becomes a wave. */
  ambient?: boolean;
  /** Accessible description. Omit for a decorative mark (aria-hidden). */
  label?: string;
  className?: string;
}

export function LogoWave({ level, active = false, ambient = false, label, className }: LogoWaveProps) {
  const hostRef = useRef<HTMLDivElement>(null);
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

    void loadThree()
      .then((THREE) => {
        if (disposed) return;
        let renderer: InstanceType<typeof THREE.WebGLRenderer>;
        try {
          renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, premultipliedAlpha: true, powerPreference: 'high-performance' });
        } catch {
          return; // the CSS mark remains
        }
        renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
        renderer.setClearColor(0x000000, 0);
        const canvas = renderer.domElement;
        canvas.setAttribute('aria-hidden', 'true');
        canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity 700ms cubic-bezier(.22,1,.36,1)';
        host.appendChild(canvas);

        const scene = new THREE.Scene();
        const camera = new THREE.PerspectiveCamera(28, 1, 0.1, 50);
        camera.position.set(0, 0, 7);
        const group = new THREE.Group();
        scene.add(group);

        const light = { value: isLightTheme() ? 1 : 0 };
        const premul = {
          transparent: true,
          depthTest: false,
          depthWrite: false,
          side: THREE.DoubleSide,
          blending: THREE.CustomBlending,
          blendSrc: THREE.OneFactor,
          blendDst: THREE.OneMinusSrcAlphaFactor,
        } as const;
        const fillUniforms = { uTime: { value: 0 }, uLevel: { value: 0 }, uLight: light, uLw: { value: 1 }, uSweep: { value: -2 } };

        // ---- Glass slabs (one per wave) -----------------------------------------
        const makeFill = () => {
          const verts = N * ROWS;
          const geo = new THREE.BufferGeometry();
          const pos = new Float32Array(verts * 3);
          const aU = new Float32Array(verts);
          const aT = new Float32Array(verts);
          const aG = new Float32Array(verts);
          const aH = new Float32Array(verts);
          const aVel = new Float32Array(verts);
          for (let r = 0; r < ROWS; r++)
            for (let i = 0; i < N; i++) {
              const k = r * N + i;
              aU[k] = U[i];
              aT[k] = r <= FILL_ROWS ? r / FILL_ROWS : 1;
              aG[k] = r <= FILL_ROWS ? 0 : (r - FILL_ROWS) / GLOW_ROWS;
            }
          const index: number[] = [];
          for (let r = 0; r < ROWS - 1; r++)
            for (let i = 0; i < N - 1; i++) {
              const a = r * N + i;
              const b = a + 1;
              const c = a + N;
              const d = c + 1;
              index.push(a, c, b, b, c, d);
            }
          geo.setIndex(index);
          const posAttr = new THREE.BufferAttribute(pos, 3).setUsage(THREE.DynamicDrawUsage);
          const hAttr = new THREE.BufferAttribute(aH, 1).setUsage(THREE.DynamicDrawUsage);
          const vAttr = new THREE.BufferAttribute(aVel, 1).setUsage(THREE.DynamicDrawUsage);
          geo.setAttribute('position', posAttr);
          geo.setAttribute('aU', new THREE.BufferAttribute(aU, 1));
          geo.setAttribute('aT', new THREE.BufferAttribute(aT, 1));
          geo.setAttribute('aG', new THREE.BufferAttribute(aG, 1));
          geo.setAttribute('aH', hAttr);
          geo.setAttribute('aVel', vAttr);
          const mesh = new THREE.Mesh(geo, new THREE.ShaderMaterial({ uniforms: fillUniforms, vertexShader: FILL_VERT, fragmentShader: FILL_FRAG, ...premul }));
          mesh.frustumCulled = false;
          mesh.renderOrder = 1;
          group.add(mesh);
          return { mesh, pos, aH, aVel, posAttr, hAttr, vAttr };
        };

        // ---- Thin lines of light (RGB edge copies, the split line) -----------------
        const makeLine = (color: [number, number, number], order: number) => {
          const verts = N * 2;
          const geo = new THREE.BufferGeometry();
          const pos = new Float32Array(verts * 3);
          const aV = new Float32Array(verts);
          const aFade = new Float32Array(verts);
          for (let i = 0; i < N; i++) {
            aV[i * 2] = -1;
            aV[i * 2 + 1] = 1;
          }
          const index: number[] = [];
          for (let i = 0; i < N - 1; i++) {
            const a = i * 2;
            index.push(a, a + 1, a + 2, a + 1, a + 3, a + 2);
          }
          geo.setIndex(index);
          const posAttr = new THREE.BufferAttribute(pos, 3).setUsage(THREE.DynamicDrawUsage);
          const fadeAttr = new THREE.BufferAttribute(aFade, 1).setUsage(THREE.DynamicDrawUsage);
          geo.setAttribute('position', posAttr);
          geo.setAttribute('aV', new THREE.BufferAttribute(aV, 1));
          geo.setAttribute('aFade', fadeAttr);
          const uniforms = { uColor: { value: new THREE.Color(...color) }, uIntensity: { value: 0.6 }, uLight: light };
          const mesh = new THREE.Mesh(geo, new THREE.ShaderMaterial({ uniforms, vertexShader: LINE_VERT, fragmentShader: LINE_FRAG, ...premul }));
          mesh.frustumCulled = false;
          mesh.renderOrder = order;
          group.add(mesh);
          return { mesh, pos, aFade, posAttr, fadeAttr, uniforms };
        };

        // Simulation state per wave: the string, its velocity, and three lagging colour copies.
        const waves = [
          { side: 1, rest: LOGO_TOP, y: Float32Array.from(LOGO_TOP), v: new Float32Array(N), echo: [Float32Array.from(LOGO_TOP), Float32Array.from(LOGO_TOP), Float32Array.from(LOGO_TOP)], fill: makeFill() },
          { side: -1, rest: LOGO_BOT, y: Float32Array.from(LOGO_BOT), v: new Float32Array(N), echo: [Float32Array.from(LOGO_BOT), Float32Array.from(LOGO_BOT), Float32Array.from(LOGO_BOT)], fill: makeFill() },
        ];
        const DARK_RGB: [number, number, number][] = [
          [1.0, 0.22, 0.12],
          [0.22, 1.0, 0.32],
          [0.18, 0.38, 1.0],
        ];
        const LIGHT_RGB: [number, number, number][] = [
          [0.86, 0.26, 0.04],
          [0.6, 0.16, 0.02],
          [0.98, 0.5, 0.12],
        ];
        const lines = waves.map((_, w) => DARK_RGB.map((c, ch) => makeLine(c, 2 + w * 3 + ch)));
        const split = makeLine([1.0, 0.9, 0.8], 9);
        const ECHO_RATE = [4.8, 11, 24]; // R lags most, B least

        const paintColours = () => {
          const lt = light.value > 0.5;
          lines.forEach((set) => set.forEach((l, ch) => l.uniforms.uColor.value.setRGB(...(lt ? LIGHT_RGB : DARK_RGB)[ch])));
        };
        paintColours();

        // Layout.
        let halfW = 2;
        let halfH = 1.7;
        let Lw = 1;
        const resize = () => {
          const w = host.clientWidth;
          const h = host.clientHeight;
          if (!w || !h) return;
          renderer.setSize(w, h, false);
          camera.aspect = w / h;
          camera.updateProjectionMatrix();
          halfH = Math.tan((camera.fov * Math.PI) / 360) * camera.position.z;
          halfW = halfH * camera.aspect;
          Lw = Math.min(halfW * 0.6, halfH * 1.3);
          fillUniforms.uLw.value = Lw;
        };
        resize();

        const reduced = prefersReducedMotion();
        const morph = { x: 0, v: 0, target: 0 };
        let shown = 0;
        let slow = 0;
        let prevShown = 0;
        let loudSince = -1;
        let quietSince = -1;
        let time = 0;
        let travel = 0;
        let kicks = 0;
        let lastData = '';
        const C2 = 1.7 * 1.7; // wave speed² (u-units / s)
        const DU = 2 / (N - 1);
        const KS = 34; // restoring spring to the rest shape
        const DAMP = 4.2;

        const tgt = new Float32Array(N);
        const envWave = (u: number) => Math.pow(Math.max(0, 1 - u * u), 1.3) * (0.25 + 0.75 * Math.exp(-((u / 0.45) ** 2)));

        const simulate = (dt: number) => {
          const s = state.current;
          const now = performance.now();
          const raw = s.active && s.level ? s.level.sample() : 0;
          shown += (raw - shown) * (1 - Math.exp(-(raw > shown ? 18 : 5) * dt));
          slow += (shown - slow) * (1 - Math.exp(-(shown > slow ? 8 : 2.5) * dt));
          if (shown < 0.002) shown = 0;
          if (slow < 0.002) slow = 0;
          const busy = s.active && s.level ? s.level.busyness() : 0;

          // Hysteresis: become the wave after sustained speech; return after a calm.
          if (shown > 0.3) {
            quietSince = -1;
            if (loudSince < 0) loudSince = now;
            if (now - loudSince > 140) morph.target = 1;
          } else if (shown < 0.1) {
            loudSince = -1;
            if (quietSince < 0) quietSince = now;
            if (now - quietSince > 1100) morph.target = 0;
          }
          if (!s.active || s.ambient) morph.target = 0;

          const k = reduced ? 260 : 24;
          const zeta = reduced ? 1 : 0.6;
          const flowing = !reduced && (s.active || s.ambient);
          if (flowing) travel += dt * (1.6 + 3.4 * shown);
          time += dt;

          // Splash: a kick wherever the voice jumps.
          const rise = shown - prevShown;
          prevShown = shown;
          if (rise > 0.06 && !reduced) {
            const u0 = ((kicks++ * 0.618034) % 1) * 1.6 - 0.8;
            for (const w of waves) for (let i = 1; i < N - 1; i++) w.v[i] += rise * 3.2 * Math.exp(-(((U[i] - u0) / 0.12) ** 2)) * w.side * (kicks % 2 ? 1 : -1);
          }

          const sub = Math.max(1, Math.ceil(dt / (1 / 240)));
          const h = Math.min(dt, 0.1) / sub;
          for (let n = 0; n < sub; n++) {
            morph.v += (k * (morph.target - morph.x) - 2 * zeta * Math.sqrt(k) * morph.v) * h;
            morph.x += morph.v * h;
          }
          morph.x = Math.max(-0.1, Math.min(1.12, morph.x));
          const m = Math.max(0, Math.min(1, morph.x));
          const me = m * m * (3 - 2 * m);
          const kWave = Math.PI * (3.2 + 1.6 * busy);
          const ampWave = ((halfH * 0.8) / Lw) * Math.pow(slow, 0.5);
          const breath = s.ambient ? 0.016 : s.active ? 0.01 : 0;
          const drive = reduced ? 0 : shown * 46;
          const freq = 2 * Math.PI * (1.1 + 1.6 * busy);

          for (const w of waves) {
            const ph = w.side > 0 ? 0 : Math.PI;
            // The shape the string wants to hold this frame (logo ↔ living wave).
            for (let i = 0; i < N; i++) {
              const u = U[i];
              const waveRest = ampWave * envWave(u) * (0.5 + 0.5 * Math.sin(kWave * u - travel + ph));
              tgt[i] = i === 0 || i === N - 1 ? 0 : w.rest[i] * (1 - me) + waveRest * me + breath * Math.sin(6 * u - travel * 0.6 + ph) * Math.sin(Math.PI * (u + 1) * 0.5);
            }
            for (let n = 0; n < sub; n++) {
              for (let i = 1; i < N - 1; i++) {
                const u = U[i];
                const target = tgt[i];
                // Tension acts on the DEVIATION from the target shape: at rest the
                // string holds the logo exactly; disturbances travel as waves.
                const lap = (w.y[i - 1] - tgt[i - 1] - 2 * (w.y[i] - target) + (w.y[i + 1] - tgt[i + 1])) / (DU * DU);
                const force = drive * envWave(u) * Math.sin(freq * time - 8 * u + ph) * 0.02;
                const acc = KS * (target - w.y[i]) + C2 * lap - DAMP * w.v[i] + force;
                w.v[i] += acc * h;
              }
              for (let i = 1; i < N - 1; i++) w.y[i] += w.v[i] * h;
              w.y[0] = 0;
              w.y[N - 1] = 0;
              w.v[0] = 0;
              w.v[N - 1] = 0;
            }
            // Colour copies follow at different speeds: dispersion in motion.
            for (let ch = 0; ch < 3; ch++) {
              const a = 1 - Math.exp(-ECHO_RATE[ch] * dt);
              const e = w.echo[ch];
              for (let i = 0; i < N; i++) e[i] += (w.y[i] - e[i]) * a;
            }
          }

          fillUniforms.uTime.value = time;
          fillUniforms.uLevel.value = shown;
          if (flowing) fillUniforms.uSweep.value = ((time * (0.22 + 0.4 * shown)) % 3.2) - 1.6;

          const data = morph.x.toFixed(2);
          if (data !== lastData) {
            host.dataset.morph = data;
            lastData = data;
          }

          // Settled when the strings rest on their shapes and nothing drives them.
          let energy = 0;
          for (const w of waves) for (let i = 0; i < N; i++) energy += w.v[i] * w.v[i] + (w.y[i] - w.rest[i]) ** 2 * (1 - me);
          const settled = energy < 1e-6 && Math.abs(morph.target - morph.x) < 0.002 && Math.abs(morph.v) < 0.002;
          return { me, m, keep: s.active || flowing || shown > 0 || !settled };
        };

        const build = (me: number, m: number) => {
          const widthHalf = Lw + (halfW * 0.97 - Lw) * me;
          const gapHalf = Lw * (GAP_HALF + (0.012 - GAP_HALF) * me);
          const glowH = Lw * (0.1 + 0.06 * fillUniforms.uLevel.value);
          const mid = Math.sin(Math.PI * m);
          group.rotation.x = mid * 0.55 * Math.sin(time * 2.3);
          group.rotation.y = mid * 0.22 * Math.cos(time * 1.7);
          group.rotation.z = mid * 0.06 * Math.sin(time * 3.1);

          for (let wi = 0; wi < waves.length; wi++) {
            const w = waves[wi];
            const f = w.fill;
            const base = w.side * gapHalf;
            for (let i = 0; i < N; i++) {
              const x = U[i] * widthHalf;
              const hgt = Math.max(0, w.y[i]) * Lw;
              const z = w.v[i] * 0.012 * Lw + mid * 0.25 * Lw * Math.sin(U[i] * 7 + time * 4 + wi);
              for (let r = 0; r < ROWS; r++) {
                const k = r * N + i;
                const yy = r <= FILL_ROWS ? base + w.side * (r / FILL_ROWS) * hgt : base + w.side * (hgt + ((r - FILL_ROWS) / GLOW_ROWS) * glowH);
                f.pos[k * 3] = x;
                f.pos[k * 3 + 1] = yy;
                f.pos[k * 3 + 2] = z;
                f.aH[k] = hgt;
                f.aVel[k] = w.v[i];
              }
            }
            f.posAttr.needsUpdate = true;
            f.hAttr.needsUpdate = true;
            f.vAttr.needsUpdate = true;

            // RGB edge copies, offset along the curve's normal.
            for (let ch = 0; ch < 3; ch++) {
              const l = lines[wi][ch];
              const e = w.echo[ch];
              const half = Lw * (0.006 + 0.004 * fillUniforms.uLevel.value);
              let spread = 0;
              for (let i = 0; i < N; i++) spread = Math.max(spread, Math.abs(e[i] - w.y[i]));
              l.uniforms.uIntensity.value = (light.value > 0.5 ? 0.35 : 0.55) + Math.min(0.35, spread * 4);
              for (let i = 0; i < N; i++) {
                const x = U[i] * widthHalf;
                const y = base + w.side * (Math.max(0, e[i]) * Lw + (2 - ch) * Lw * 0.004 * Math.min(1, e[i] / 0.05));
                const i0 = Math.max(0, i - 1);
                const i1 = Math.min(N - 1, i + 1);
                const dx = (U[i1] - U[i0]) * widthHalf;
                const dy = w.side * (Math.max(0, e[i1]) - Math.max(0, e[i0])) * Lw;
                const len = Math.hypot(dx, dy) || 1;
                const nx = -dy / len;
                const ny = dx / len;
                const z = w.v[i] * 0.012 * Lw + mid * 0.25 * Lw * Math.sin(U[i] * 7 + time * 4 + wi);
                const fade = Math.min(1, Math.max(0, (e[i] * Lw) / (Lw * 0.02))) * (1 - Math.max(0, (Math.abs(U[i]) - 0.9) / 0.1));
                for (let side = 0; side < 2; side++) {
                  const k = i * 2 + side;
                  const sgn = side ? 1 : -1;
                  l.pos[k * 3] = x + nx * half * sgn;
                  l.pos[k * 3 + 1] = y + ny * half * sgn;
                  l.pos[k * 3 + 2] = z;
                  l.aFade[k] = fade;
                }
              }
              l.posAttr.needsUpdate = true;
              l.fadeAttr.needsUpdate = true;
            }
          }

          // The logo's split line becomes a line of light as it turns into the wave.
          const sh = Lw * 0.005;
          split.uniforms.uIntensity.value = me * (0.35 + 0.8 * fillUniforms.uLevel.value) * (light.value > 0.5 ? 0.5 : 1);
          for (let i = 0; i < N; i++) {
            const x = U[i] * widthHalf;
            const fade = Math.pow(Math.max(0, 1 - U[i] * U[i]), 0.6);
            for (let side = 0; side < 2; side++) {
              const k = i * 2 + side;
              split.pos[k * 3] = x;
              split.pos[k * 3 + 1] = side ? sh : -sh;
              split.pos[k * 3 + 2] = 0;
              split.aFade[k] = fade;
            }
          }
          split.posAttr.needsUpdate = true;
          split.fadeAttr.needsUpdate = true;
        };

        let first = true;
        const loop = frameLoop((_t, dt) => {
          const { me, m, keep } = simulate(dt || 1 / 60);
          build(me, m);
          renderer.render(scene, camera);
          if (first) {
            first = false;
            canvas.style.opacity = '1';
            host.dataset.webgl = 'on';
          }
          return keep;
        }, 60);
        state.current.wake = loop.wake;
        loop.wake();

        const ro = new ResizeObserver(() => {
          resize();
          loop.wake();
        });
        ro.observe(host);
        const offTheme = onThemeChange(() => {
          light.value = isLightTheme() ? 1 : 0;
          paintColours();
          loop.wake();
        });
        const onLost = (e: Event) => {
          e.preventDefault();
          loop.stop();
          canvas.style.opacity = '0';
          delete host.dataset.webgl; // the CSS mark shows again
        };
        canvas.addEventListener('webglcontextlost', onLost);

        cleanup = () => {
          loop.stop();
          ro.disconnect();
          offTheme();
          canvas.removeEventListener('webglcontextlost', onLost);
          group.traverse((o) => {
            const mesh = o as InstanceType<typeof THREE.Mesh>;
            if (mesh.isMesh) {
              mesh.geometry.dispose();
              (mesh.material as InstanceType<typeof THREE.ShaderMaterial>).dispose();
            }
          });
          renderer.dispose();
          renderer.forceContextLoss(); // free the GPU context now, not at garbage collection
          canvas.remove();
          delete host.dataset.webgl;
          delete host.dataset.morph;
        };
      })
      .catch(() => {
        /* Three.js failed to load: the CSS mark remains */
      });

    return () => {
      disposed = true;
      cleanup();
    };
  }, []);

  return (
    <div ref={hostRef} className={cn('group relative', className)} data-morph="0.00" {...(label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}>
      {/* The mark in CSS/SVG until WebGL is ready (or without it). */}
      <svg
        viewBox="0 0 96 48"
        className="pointer-events-none absolute left-1/2 top-1/2 h-auto w-[min(56%,420px)] -translate-x-1/2 -translate-y-1/2 transition-opacity duration-700 group-data-[webgl=on]:opacity-0"
        aria-hidden="true"
        style={{ filter: 'drop-shadow(0 0 18px rgb(255 122 26 / 0.45))' }}
      >
        <defs>
          <linearGradient id="lw-fallback" x1="0" y1="0" x2="96" y2="0" gradientUnits="userSpaceOnUse">
            <stop offset="0" stopColor="#E2560C" />
            <stop offset="0.5" stopColor="#FF7A1A" />
            <stop offset="1" stopColor="#FFB070" />
          </linearGradient>
        </defs>
        <path d="M1 22.5C11 22.5 15 1 29.5 1C44 1 47 22.5 57 22.5C63.5 22.5 65.5 7.5 75 7.5C84.5 7.5 86.5 22.5 95 22.5Z" fill="url(#lw-fallback)" />
        <path d="M1 25.5C9.5 25.5 12 40.5 21.5 40.5C31 40.5 33.5 25.5 40 25.5C50 25.5 53 47 67.5 47C82 47 85 25.5 95 25.5Z" fill="url(#lw-fallback)" />
      </svg>
    </div>
  );
}
