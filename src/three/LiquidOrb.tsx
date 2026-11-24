import { useEffect, useRef } from 'react';
import { cn } from '../utils';
import type { ActivityLevel } from '../components/voice/activityLevel';
import { frameLoop, isLightTheme, loadThree, onThemeChange, prefersReducedMotion, webglAvailable, SIMPLEX3 } from './runtime';

// ============================================================================
// Liquid Orb — a luminous glass sphere holding a twisting ribbon of light.
// When speech is reported the sphere flattens into a line of light and the
// ribbon unfurls into the Sujit wave: a full-width bundle of glowing strands
// that turns in depth. In silence it springs back into the orb.
//
// Physics: the morph and a jelly squash are spring–damper systems (slight
// overshoot), level is attack/release smoothed, and the morph uses hysteresis
// so it never flickers. Honest by construction: ripples, squash and the morph
// are driven ONLY by `level` (real recognizer events / real mic). Without a
// level the orb never becomes a wave.
// ============================================================================

const ROT = /* glsl */ `
vec3 rotX(vec3 p, float a){ float c = cos(a), s = sin(a); return vec3(p.x, c*p.y - s*p.z, s*p.y + c*p.z); }
vec3 rotY(vec3 p, float a){ float c = cos(a), s = sin(a); return vec3(c*p.x + s*p.z, p.y, -s*p.x + c*p.z); }
float ease(float e){ e = clamp(e, 0.0, 1.0); return e*e*(3.0 - 2.0*e); }
`;

// ---- Sphere body: milky, luminous glass with flowing warm patches ----------
const SPHERE_VERT = /* glsl */ `
uniform float uMorph, uLevel, uFlow, uSquash;
varying vec3 vN; varying vec3 vObj; varying vec3 vPos;
${SIMPLEX3}
${ROT}
void main(){
  float ee = ease(uMorph);
  vec3 p = position;
  float wob = snoise(normal * 1.8 + vec3(uFlow * 0.6)) * (0.012 + 0.075 * uLevel) * (1.0 - ee);
  p += normal * wob;
  // Jelly squash, then the collapse into a line of light.
  p.x *= (1.0 - 0.5 * uSquash) * mix(1.0, 1.9, ee);
  p.y *= (1.0 + uSquash) * mix(1.0, 0.035, ee);
  p.z *= mix(1.0, 0.25, ee);
  vObj = normal;
  vN = normalize(normalMatrix * normal);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  vPos = mv.xyz;
  gl_Position = projectionMatrix * mv;
}
`;
const SPHERE_FRAG = /* glsl */ `
precision highp float;
uniform float uMorph, uLevel, uFlow, uLight;
varying vec3 vN; varying vec3 vObj; varying vec3 vPos;
${SIMPLEX3}
${ROT}
void main(){
  vec3 N = normalize(vN);
  vec3 V = normalize(-vPos);
  float ndv = clamp(dot(N, V), 0.0, 1.0);
  float fres = pow(1.0 - ndv, 2.0);
  vec3 o = vObj;
  // Very low-frequency drift: broad, silky pools of colour (never blotchy).
  float n = snoise(o * 0.75 + vec3(uFlow * 0.09, -uFlow * 0.06, uFlow * 0.05));
  vec3 dTop = normalize(vec3(0.35 + 0.25 * sin(uFlow * 0.21), 0.92, 0.25));
  vec3 dBot = normalize(vec3(-0.3 + 0.2 * cos(uFlow * 0.17), -0.9, 0.3));
  vec3 dSide = normalize(vec3(-0.95, 0.1, 0.3));

  vec3 milk   = vec3(1.0, 0.985, 0.965);
  vec3 cream  = vec3(1.0, 0.95, 0.9);
  vec3 amber  = vec3(1.0, 0.7, 0.42);
  vec3 cosmic = vec3(1.0, 0.44, 0.07);
  vec3 ember  = vec3(0.96, 0.3, 0.04);

  // Light lives INSIDE the glass: a bright milky body…
  vec3 col = mix(milk, cream, smoothstep(0.3, 0.9, 1.0 - ndv));
  // …with saturated, well-defined pools of colour near the poles.
  float top = smoothstep(0.56, 0.86, dot(o, dTop) + 0.12 * n);
  float bot = smoothstep(0.6, 0.9, dot(o, dBot) - 0.1 * n);
  float side = smoothstep(0.62, 0.95, dot(o, dSide) + 0.1 * n);
  col = mix(col, cosmic, top * 0.95);
  col = mix(col, ember, bot * 0.9);
  col = mix(col, amber, side * 0.4);
  // The limb takes on colour (light travelling through more glass)…
  col = mix(col, mix(amber, cosmic, 0.35), smoothstep(0.5, 0.88, fres) * 0.5);
  // …and the very edge is a crisp, luminous line.
  col = mix(col, vec3(1.0, 0.96, 0.88), smoothstep(0.88, 1.0, fres) * 0.85);
  // Louder voice: brighter from within (not a bigger corona).
  col *= 1.0 + 0.22 * uLevel;
  float shell = 1.0 - smoothstep(0.3, 0.82, uMorph);
  float a = mix(0.95, 1.0, fres) * shell;
  gl_FragColor = vec4(col * a, a);
}
`;

// ---- Glass overlay: rim light, chromatic fringe, speculars (over the ribbon) --
const GLASS_FRAG = /* glsl */ `
precision highp float;
uniform float uMorph, uLevel, uLight;
varying vec3 vN; varying vec3 vObj; varying vec3 vPos;
void main(){
  vec3 N = normalize(vN);
  vec3 V = normalize(-vPos);
  float ndv = clamp(dot(N, V), 0.0, 1.0);
  float fres = pow(1.0 - ndv, 3.0);
  vec3 L = normalize(vec3(-0.45, 0.62, 0.66));
  vec3 L2 = normalize(vec3(0.6, -0.5, 0.6));
  float spec = pow(max(dot(N, normalize(L + V)), 0.0), 150.0);
  float sheen = pow(max(dot(N, normalize(L + V)), 0.0), 14.0);
  float spec2 = pow(max(dot(N, normalize(L2 + V)), 0.0), 60.0);
  // A thin prismatic fringe at the very edge (warm gold one side, rose the other).
  float fr = smoothstep(0.55, 1.0, fres);
  vec3 fringe = mix(vec3(1.0, 0.82, 0.38), vec3(1.0, 0.36, 0.55), smoothstep(-0.6, 0.6, vObj.x));
  vec3 col = vec3(1.0) * spec * 1.3 + vec3(1.0, 0.9, 0.8) * sheen * 0.12 + vec3(1.0, 0.75, 0.5) * spec2 * 0.35 + fringe * fr * 0.55;
  float shell = 1.0 - smoothstep(0.3, 0.82, uMorph);
  float a = clamp(spec * 1.3 + sheen * 0.1 + spec2 * 0.3 + fr * 0.45, 0.0, 1.0) * shell;
  gl_FragColor = vec4(col * shell, a * 0.35);
}
`;

// ---- Ribbon / strands: the S-band inside the orb ↔ the Sujit wave -----------
const RIBBON_VERT = /* glsl */ `
attribute float aOff; attribute float aAmp; attribute float aK; attribute float aW; attribute float aA; attribute float aLead;
uniform float uMorph, uLevel, uFlow, uTravel, uSpin, uR, uHalfW, uSquash, uTime, uBusy;
varying float vV; varying float vU; varying float vAlpha; varying float vInside; varying float vTaper;
${ROT}
void main(){
  float u = uv.x * 2.0 - 1.0;
  float v = uv.y * 2.0 - 1.0;
  float e = clamp(uMorph, 0.0, 1.0);
  float ee = ease(uMorph);
  float mid = sin(3.14159265 * e); // peaks mid-transition

  // Orb mode: an S-shaped band inside the sphere that twists as it travels.
  float R = uR;
  vec3 cO = vec3(u * R * 0.98, R * 0.36 * sin(3.14159265 * u + 0.2 * sin(uFlow * 0.9)), 0.0);
  float tw = 0.95 * u + 0.3 * sin(uFlow * 0.7) + 0.55;
  vec3 bO = normalize(vec3(-0.22 * u, cos(tw), sin(tw)));
  vec3 pO = cO + bO * v * R * 0.36 * (0.7 + 0.3 * cos(3.14159265 * u * 0.9));
  pO = rotY(rotX(pO, -0.42), 0.35 * sin(uSpin));
  pO.y *= 1.0 + uSquash;
  pO.x *= 1.0 - 0.5 * uSquash;
  float inside = 1.0 - smoothstep(R * 0.84, R * 0.99, length(pO.xy));

  // Wave mode: the Sujit wave — centre-peaked envelope, harmonics, 3D planes.
  float env = pow(max(0.0, 1.0 - u * u), 1.5) * (0.16 + 0.84 * exp(-pow(u / 0.38, 2.0)));
  float amp = R * 1.08 * pow(max(uLevel, 0.0), 0.55) * aAmp;
  float ph = 3.14159265 * (2.6 + 1.4 * uBusy) * aK * u - uTravel + aOff;
  float s = (sin(ph) + 0.3 * sin(2.0 * ph + aOff * 1.7 - uTravel * 0.6)) / 1.2;
  float y = amp * env * s;
  // The lead keeps facing you (always legible); the strands turn in depth around it.
  float plane = aLead > 0.5 ? 0.22 * sin(uSpin) : uSpin * 1.6 + aOff;
  float wHalf = (0.007 + 0.03 * aLead) * aW * (0.8 + 0.4 * uLevel);
  vec3 pW = vec3(u * uHalfW * 0.985, y * cos(plane), y * sin(plane)) + vec3(0.0, 1.0, 0.0) * v * wHalf * 2.0;

  vec3 p = mix(pO, pW, ee);
  // The transformation itself: a swirl and a ripple that peak halfway.
  p = rotX(p, mid * (2.2 + aOff * 0.15));
  p.y += mid * 0.3 * R * sin(u * 8.5 + uTime * 5.5 + aOff);
  p.z += mid * 0.3 * R * cos(u * 6.5 - uTime * 4.5 + aOff);

  vV = v;
  vU = u;
  vInside = mix(inside, 1.0, ee);
  vTaper = 1.0 - smoothstep(0.72, 1.0, abs(u)) * ee;
  // Only the lead ribbon lives in the orb; the strands fan out as it becomes a wave.
  vAlpha = aLead > 0.5 ? 1.0 : aA * smoothstep(0.15, 0.85, e);
  gl_Position = projectionMatrix * modelViewMatrix * vec4(p, 1.0);
}
`;
const RIBBON_FRAG = /* glsl */ `
precision highp float;
uniform float uMorph, uLight, uFlow;
uniform vec3 uAccent;
varying float vV; varying float vU; varying float vAlpha; varying float vInside; varying float vTaper;
float ease(float e){ e = clamp(e, 0.0, 1.0); return e*e*(3.0 - 2.0*e); }
void main(){
  float ee = ease(uMorph);
  float av = abs(vV);
  // Orb: a frosted cream band, warm inner light, fine ember edges.
  float edge = smoothstep(0.86, 0.985, av);
  float centre = 1.0 - smoothstep(0.0, 0.85, av);
  vec3 band = mix(vec3(1.0, 0.985, 0.96), vec3(1.0, 0.8, 0.58), 0.3 + 0.25 * sin(vU * 2.2 + uFlow * 0.6));
  band = mix(band, vec3(1.0, 0.62, 0.3), centre * 0.42);   // warm light inside the band
  band = mix(band, vec3(0.88, 0.3, 0.04), edge);           // fine ember edge lines
  band *= 1.06;                                            // brighter than the glass around it
  float orbA = mix(0.9, 1.0, edge) * vInside;
  // Wave: a white-hot core in an orange halo (ink and ember on light themes).
  float core = exp(-pow(vV * 3.4, 2.0));
  float halo = exp(-pow(vV * 1.25, 2.0));
  vec3 hot = uLight < 0.5 ? mix(uAccent, vec3(1.0, 0.985, 0.95), core) : mix(vec3(1.0, 0.5, 0.14), vec3(0.5, 0.13, 0.02), core);
  float waveA = (core * 0.95 + halo * 0.42) * vTaper;
  vec3 col = mix(band, hot, ee);
  float a = mix(orbA, waveA, ee) * vAlpha;
  float additive = uLight < 0.5 ? ee * 0.85 : 0.0;
  gl_FragColor = vec4(col * a, a * (1.0 - additive));
}
`;

// ---- Light: halo, anamorphic streaks, the glow along the wave ---------------
const LIGHT_VERT = /* glsl */ `
varying vec2 vXY;
void main(){ vec4 w = modelMatrix * vec4(position, 1.0); vXY = w.xy; gl_Position = projectionMatrix * viewMatrix * w; }
`;
const LIGHT_FRAG = /* glsl */ `
precision highp float;
uniform float uMorph, uLevel, uR, uHalfW, uHalfH, uLight, uActive, uSquash;
uniform vec3 uAccent;
varying vec2 vXY;
float ease(float e){ e = clamp(e, 0.0, 1.0); return e*e*(3.0 - 2.0*e); }
void main(){
  float ee = ease(uMorph);
  vec2 p = vXY;
  float R = uR;
  float lum = 0.6 + 0.15 * uActive + 0.35 * uLevel;
  // Tight bloom hugging the glass edge (no corona), a whisper of haze.
  // Follow the sphere's real silhouette (squash, then the collapse into a line).
  vec2 sc = vec2((1.0 - 0.5 * uSquash) * mix(1.0, 1.9, ee), (1.0 + uSquash) * mix(1.0, 0.035, ee));
  float sd = length(p / sc) - R * 0.985;          // signed: < 0 inside the glass
  float dd = sd > 0.0 ? sd : -sd * 5.0;           // fades fast inward: a ring, never a filled disc
  float rim = exp(-dd * dd / (R * R * 0.0035)) * 0.7 * lum * (1.0 - ee);
  float bloom = exp(-dd * dd / (R * R * 0.04)) * 0.2 * lum * (1.0 - ee);
  float glowWide = exp(-dot(p, p) / (R * R * 4.0)) * 0.05 * lum * (1.0 - 0.6 * ee);
  // Fine horizontal lens streaks.
  float streak = exp(-pow(p.y / (0.012 * R), 2.0)) * exp(-pow(p.x / (uHalfW * 0.7), 2.0)) * (0.3 + 0.35 * uLevel);
  float streak2 = (exp(-pow((p.y - R * 0.48) / (0.01 * R), 2.0)) + exp(-pow((p.y + R * 0.5) / (0.01 * R), 2.0))) * exp(-pow(p.x / (uHalfW * 0.45), 2.0)) * 0.1;
  float streaks = (streak + streak2) * (1.0 - 0.5 * ee);
  float glow = rim + bloom;
  // Along the wave: a soft glow only as tall as the wave itself.
  float band = exp(-pow(p.y / (0.06 * R + R * 0.7 * pow(max(uLevel, 0.0), 0.55)), 2.0)) * (1.0 - smoothstep(uHalfW * 0.35, uHalfW * 0.95, abs(p.x))) * ee * 0.2;
  vec3 warmWhite = vec3(1.0, 0.86, 0.7);
  vec3 col = warmWhite * rim + mix(warmWhite, uAccent, 0.5) * bloom + uAccent * (glowWide + band) + vec3(1.0, 0.84, 0.68) * streaks;
  // Everything fades out before the canvas edge: no visible box.
  float fade = 1.0 - smoothstep(0.62, 1.0, length(p / vec2(uHalfW, uHalfH)));
  col *= fade;
  float a = clamp(glow + glowWide + band + streaks, 0.0, 1.0) * fade;
  // Light theme: a warm peach tint over the page (never a grey smudge).
  if (uLight > 0.5) { float aE = a * 0.3; gl_FragColor = vec4(vec3(1.0, 0.8, 0.6) * aE, aE); }
  else { gl_FragColor = vec4(col, 0.0); }
}
`;

interface Strand {
  off: number;
  amp: number;
  k: number;
  w: number;
  a: number;
  lead: number;
}
const STRANDS: Strand[] = [
  { off: 0, amp: 1, k: 1, w: 1.1, a: 1, lead: 1 },
  ...Array.from({ length: 13 }, (_, i) => ({
    off: (i + 1) * 0.483,
    amp: 0.42 + 0.6 * (((i * 5) % 7) / 6),
    k: 0.86 + 0.28 * (((i * 3) % 8) / 7),
    w: i < 5 ? 1.0 : 0.7,
    a: i < 5 ? 0.7 - i * 0.08 : 0.2 + 0.14 * (((i * 7) % 4) / 3),
    lead: 0,
  })),
];

export interface LiquidOrbProps {
  /** Real voice level. Without it the orb never ripples or becomes a wave. */
  level?: ActivityLevel | null;
  /** Capturing now: the light inside flows (a true state, not a level). */
  active?: boolean;
  /** Decorative slow drift (Home); never implies listening. */
  ambient?: boolean;
  /** Accessible description. Omit for a decorative orb (aria-hidden). */
  label?: string;
  className?: string;
}

export function LiquidOrb({ level, active = false, ambient = false, label, className }: LiquidOrbProps) {
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
          return; // CSS orb remains
        }
        renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
        renderer.setClearColor(0x000000, 0);
        const canvas = renderer.domElement;
        canvas.setAttribute('aria-hidden', 'true');
        canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity 800ms cubic-bezier(.22,1,.36,1)';
        host.appendChild(canvas);

        const scene = new THREE.Scene();
        const camera = new THREE.PerspectiveCamera(28, 1, 0.1, 50);
        camera.position.set(0, 0, 7);
        const accent = new THREE.Color('#ff7a1a');

        const shared = {
          uMorph: { value: 0 },
          uLevel: { value: 0 },
          uFlow: { value: 3.1 },
          uTravel: { value: 0 },
          uSpin: { value: 0.35 },
          uSquash: { value: 0 },
          uTime: { value: 0 },
          uBusy: { value: 0 },
          uR: { value: 1 },
          uHalfW: { value: 2 },
          uHalfH: { value: 1.7 },
          uLight: { value: isLightTheme() ? 1 : 0 },
          uActive: { value: 0 },
          uAccent: { value: accent },
        };
        const premul = {
          transparent: true,
          depthTest: false,
          depthWrite: false,
          blending: THREE.CustomBlending,
          blendSrc: THREE.OneFactor,
          blendDst: THREE.OneMinusSrcAlphaFactor,
        } as const;

        const lightPlane = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.ShaderMaterial({ uniforms: shared, vertexShader: LIGHT_VERT, fragmentShader: LIGHT_FRAG, ...premul }));
        lightPlane.renderOrder = 0;

        const sphereGeo = new THREE.SphereGeometry(1, 128, 96);
        const body = new THREE.Mesh(sphereGeo, new THREE.ShaderMaterial({ uniforms: shared, vertexShader: SPHERE_VERT, fragmentShader: SPHERE_FRAG, ...premul }));
        body.renderOrder = 1;
        const glass = new THREE.Mesh(sphereGeo, new THREE.ShaderMaterial({ uniforms: shared, vertexShader: SPHERE_VERT, fragmentShader: GLASS_FRAG, ...premul }));
        glass.renderOrder = 3;

        const base = new THREE.PlaneGeometry(1, 1, 360, 8);
        const ribbonGeo = new THREE.InstancedBufferGeometry();
        ribbonGeo.index = base.index;
        ribbonGeo.setAttribute('position', base.getAttribute('position'));
        ribbonGeo.setAttribute('uv', base.getAttribute('uv'));
        const attr = (pick: (s: Strand) => number) => new THREE.InstancedBufferAttribute(new Float32Array(STRANDS.map(pick)), 1);
        ribbonGeo.setAttribute('aOff', attr((s) => s.off));
        ribbonGeo.setAttribute('aAmp', attr((s) => s.amp));
        ribbonGeo.setAttribute('aK', attr((s) => s.k));
        ribbonGeo.setAttribute('aW', attr((s) => s.w));
        ribbonGeo.setAttribute('aA', attr((s) => s.a));
        ribbonGeo.setAttribute('aLead', attr((s) => s.lead));
        ribbonGeo.instanceCount = STRANDS.length;
        const ribbon = new THREE.Mesh(ribbonGeo, new THREE.ShaderMaterial({ uniforms: shared, vertexShader: RIBBON_VERT, fragmentShader: RIBBON_FRAG, side: THREE.DoubleSide, ...premul }));
        ribbon.renderOrder = 2;
        ribbon.frustumCulled = false;
        body.frustumCulled = false;
        glass.frustumCulled = false;

        scene.add(lightPlane, body, ribbon, glass);

        const resize = () => {
          const w = host.clientWidth;
          const h = host.clientHeight;
          if (!w || !h) return;
          renderer.setSize(w, h, false);
          camera.aspect = w / h;
          camera.updateProjectionMatrix();
          const halfH = Math.tan((camera.fov * Math.PI) / 360) * camera.position.z;
          const halfW = halfH * camera.aspect;
          shared.uHalfW.value = halfW;
          shared.uHalfH.value = halfH;
          // The orb fits the smaller side with room for its light.
          const R = Math.min(halfH * 0.6, halfW * 0.52);
          shared.uR.value = R;
          body.scale.setScalar(R);
          glass.scale.setScalar(R);
          lightPlane.scale.set(halfW * 2.4, halfH * 2.4, 1);
        };
        resize();

        const reduced = prefersReducedMotion();
        // Springs (semi-implicit Euler, fixed sub-steps for stability).
        const morph = { x: 0, v: 0, target: 0 };
        const squash = { x: 0, v: 0 };
        let shown = 0;
        let prevShown = 0;
        let loudSince = -1;
        let quietSince = -1;
        let time = 0;
        let lastData = '';

        const step = (dt: number) => {
          const s = state.current;
          const now = performance.now();
          const raw = s.active && s.level ? s.level.sample() : 0;
          const rate = raw > shown ? 18 : 5;
          shown += (raw - shown) * (1 - Math.exp(-rate * dt));
          if (shown < 0.002) shown = 0;

          // Hysteresis: become a wave after sustained speech; return after a calm.
          if (shown > 0.3) {
            quietSince = -1;
            if (loudSince < 0) loudSince = now;
            if (now - loudSince > 140) morph.target = 1;
          } else if (shown < 0.1) {
            loudSince = -1;
            if (quietSince < 0) quietSince = now;
            if (now - quietSince > 1100) morph.target = 0;
          }
          if (!s.active) morph.target = 0;

          const k = reduced ? 260 : 24;
          const zeta = reduced ? 1 : 0.6;
          const sub = Math.max(1, Math.ceil(dt / (1 / 120)));
          const h = dt / sub;
          for (let i = 0; i < sub; i++) {
            morph.v += (k * (morph.target - morph.x) - 2 * zeta * Math.sqrt(k) * morph.v) * h;
            morph.x += morph.v * h;
            // Jelly: kicked by rising level, wobbles out.
            squash.v += (-150 * squash.x - 2 * 0.3 * Math.sqrt(150) * squash.v) * h;
            squash.x += squash.v * h;
          }
          const rise = shown - prevShown;
          if (rise > 0.01 && !reduced) squash.v += rise * 2.2;
          prevShown = shown;
          squash.x = Math.max(-0.12, Math.min(0.12, squash.x));
          morph.x = Math.max(-0.1, Math.min(1.12, morph.x));

          const flowing = !reduced && (s.active || s.ambient);
          time += dt;
          if (flowing) {
            shared.uFlow.value += dt * (s.active ? 0.38 + 1.7 * shown : 0.1);
            shared.uSpin.value += dt * (s.active ? 0.12 + 0.9 * shown : 0.05);
            shared.uTravel.value += dt * (1.3 + 3.2 * shown);
          }
          shared.uTime.value = time;
          shared.uLevel.value = shown;
          shared.uMorph.value = morph.x;
          shared.uSquash.value = squash.x;
          shared.uActive.value += ((s.active ? 1 : 0) - shared.uActive.value) * (1 - Math.exp(-4 * dt));
          shared.uBusy.value = s.level && s.active ? s.level.busyness() : 0;

          const data = morph.x.toFixed(2);
          if (data !== lastData) {
            host.dataset.morph = data;
            lastData = data;
          }
          const settled = Math.abs(morph.target - morph.x) < 0.002 && Math.abs(morph.v) < 0.002 && Math.abs(squash.x) < 0.001 && Math.abs(squash.v) < 0.001;
          return s.active || flowing || shown > 0 || !settled || Math.abs(shared.uActive.value - (s.active ? 1 : 0)) > 0.01;
        };

        let firstFrame = true;
        const loop = frameLoop((_t, dt) => {
          const keep = step(dt);
          renderer.render(scene, camera);
          if (firstFrame) {
            firstFrame = false;
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
          shared.uLight.value = isLightTheme() ? 1 : 0;
          loop.wake();
        });
        const onLost = (e: Event) => {
          e.preventDefault();
          loop.stop();
          canvas.style.opacity = '0';
          delete host.dataset.webgl; // CSS orb shows again
        };
        canvas.addEventListener('webglcontextlost', onLost);

        cleanup = () => {
          loop.stop();
          ro.disconnect();
          offTheme();
          canvas.removeEventListener('webglcontextlost', onLost);
          for (const m of [lightPlane, body, glass, ribbon]) (m.material as InstanceType<typeof THREE.ShaderMaterial>).dispose();
          lightPlane.geometry.dispose();
          sphereGeo.dispose();
          base.dispose();
          ribbonGeo.dispose();
          renderer.dispose();
          renderer.forceContextLoss(); // free the GPU context now, not at garbage collection
          canvas.remove();
          delete host.dataset.webgl;
          delete host.dataset.morph;
        };
      })
      .catch(() => {
        /* Three.js failed to load: the CSS orb remains */
      });

    return () => {
      disposed = true;
      cleanup();
    };
  }, []);

  return (
    <div
      ref={hostRef}
      className={cn('group relative', className)}
      data-morph="0.00"
      {...(label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true })}
    >
      {/* CSS orb: shown until WebGL is ready, or without WebGL. */}
      <div className="liquid-orb-fallback pointer-events-none absolute left-1/2 top-1/2 aspect-square h-[62%] max-w-[62%] -translate-x-1/2 -translate-y-1/2 rounded-full transition-opacity duration-700 group-data-[webgl=on]:opacity-0" />
    </div>
  );
}
