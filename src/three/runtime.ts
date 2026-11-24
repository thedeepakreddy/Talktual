// Shared plumbing for the Three.js scenes: lazy loading (Three.js arrives after
// first paint), capability checks and a frame loop that respects reduced
// motion, hidden tabs and a frame-rate cap.

export type ThreeModule = typeof import('three');

let threePromise: Promise<ThreeModule> | null = null;
export function loadThree(): Promise<ThreeModule> {
  threePromise ??= import('three');
  return threePromise;
}

export function webglAvailable(): boolean {
  if (typeof document === 'undefined') return false;
  try {
    const c = document.createElement('canvas');
    return Boolean(c.getContext('webgl2') || c.getContext('webgl'));
  } catch {
    return false;
  }
}

export function prefersReducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Light or dark, following <html data-theme> and the system preference. */
export function isLightTheme(): boolean {
  if (typeof document === 'undefined') return false;
  const forced = document.documentElement.getAttribute('data-theme');
  if (forced === 'light') return true;
  if (forced === 'dark') return false;
  return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: light)').matches;
}

/** Calls `cb` whenever the theme may have changed. */
export function onThemeChange(cb: () => void): () => void {
  const mo = new MutationObserver(cb);
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  const mq = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: light)') : null;
  mq?.addEventListener?.('change', cb);
  return () => {
    mo.disconnect();
    mq?.removeEventListener?.('change', cb);
  };
}

/**
 * requestAnimationFrame loop capped at `fps`. `frame` returns false when there
 * is nothing left to animate; the loop then sleeps until `wake()`.
 */
export function frameLoop(frame: (t: number, dt: number) => boolean, fps = 60) {
  let raf = 0;
  let last = 0;
  let prev = 0;
  let running = false;
  const minGap = 1000 / fps - 1;
  const tick = (now: number) => {
    raf = 0;
    if (document.hidden) {
      running = false;
      return;
    }
    if (now - last >= minGap) {
      const dt = prev ? Math.min(0.1, (now - prev) / 1000) : 0;
      prev = now;
      last = now;
      if (!frame(now / 1000, dt)) {
        running = false;
        prev = 0;
        return;
      }
    }
    raf = requestAnimationFrame(tick);
  };
  const wake = () => {
    if (running) return;
    running = true;
    raf = requestAnimationFrame(tick);
  };
  const onVis = () => !document.hidden && wake();
  document.addEventListener('visibilitychange', onVis);
  return {
    wake,
    stop() {
      cancelAnimationFrame(raf);
      running = false;
      document.removeEventListener('visibilitychange', onVis);
    },
  };
}

/** Ashima/stegu 3D simplex noise (MIT), shared by the shaders. */
export const SIMPLEX3 = /* glsl */ `
vec3 mod289(vec3 x){return x-floor(x*(1.0/289.0))*289.0;}
vec4 mod289(vec4 x){return x-floor(x*(1.0/289.0))*289.0;}
vec4 permute(vec4 x){return mod289(((x*34.0)+10.0)*x);}
vec4 taylorInvSqrt(vec4 r){return 1.79284291400159-0.85373472095314*r;}
float snoise(vec3 v){
  const vec2 C=vec2(1.0/6.0,1.0/3.0);
  const vec4 D=vec4(0.0,0.5,1.0,2.0);
  vec3 i=floor(v+dot(v,C.yyy));
  vec3 x0=v-i+dot(i,C.xxx);
  vec3 g=step(x0.yzx,x0.xyz);
  vec3 l=1.0-g;
  vec3 i1=min(g.xyz,l.zxy);
  vec3 i2=max(g.xyz,l.zxy);
  vec3 x1=x0-i1+C.xxx;
  vec3 x2=x0-i2+C.yyy;
  vec3 x3=x0-D.yyy;
  i=mod289(i);
  vec4 p=permute(permute(permute(i.z+vec4(0.0,i1.z,i2.z,1.0))+i.y+vec4(0.0,i1.y,i2.y,1.0))+i.x+vec4(0.0,i1.x,i2.x,1.0));
  float n_=0.142857142857;
  vec3 ns=n_*D.wyz-D.xzx;
  vec4 j=p-49.0*floor(p*ns.z*ns.z);
  vec4 x_=floor(j*ns.z);
  vec4 y_=floor(j-7.0*x_);
  vec4 x=x_*ns.x+ns.yyyy;
  vec4 y=y_*ns.x+ns.yyyy;
  vec4 h=1.0-abs(x)-abs(y);
  vec4 b0=vec4(x.xy,y.xy);
  vec4 b1=vec4(x.zw,y.zw);
  vec4 s0=floor(b0)*2.0+1.0;
  vec4 s1=floor(b1)*2.0+1.0;
  vec4 sh=-step(h,vec4(0.0));
  vec4 a0=b0.xzyw+s0.xzyw*sh.xxyy;
  vec4 a1=b1.xzyw+s1.xzyw*sh.zzww;
  vec3 p0=vec3(a0.xy,h.x);
  vec3 p1=vec3(a0.zw,h.y);
  vec3 p2=vec3(a1.xy,h.z);
  vec3 p3=vec3(a1.zw,h.w);
  vec4 norm=taylorInvSqrt(vec4(dot(p0,p0),dot(p1,p1),dot(p2,p2),dot(p3,p3)));
  p0*=norm.x;p1*=norm.y;p2*=norm.z;p3*=norm.w;
  vec4 m=max(0.5-vec4(dot(x0,x0),dot(x1,x1),dot(x2,x2),dot(x3,x3)),0.0);
  m=m*m;
  return 105.0*dot(m*m,vec4(dot(p0,x0),dot(p1,x1),dot(p2,x2),dot(p3,x3)));
}
`;
