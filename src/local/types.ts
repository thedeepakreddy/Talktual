// Phase 6 local runtime contracts. Model-specific code lives behind these
// interfaces; the session and the policy only ever see them.

export const LOCAL_RUNTIME_VERSION = 1 as const;

export type LocalTask = 'translate' | 'asr' | 'tts' | 'intelligence';

/** unavailable: not on this platform · downloadable: platform could fetch a model (we never trigger it) · ready: usable now. */
export type Readiness = 'unknown' | 'unavailable' | 'downloadable' | 'loading' | 'ready' | 'error' | 'disabled';

/** Was this capability actually exercised on test hardware, or only reported by the platform? */
export type Verification = 'verified-test-hardware' | 'platform-reported' | 'unverified';

export interface RuntimeInfo {
  v: typeof LOCAL_RUNTIME_VERSION;
  id: string;
  task: LocalTask;
  /** Supported language pairs ("en>es") for translate; languages ("en") for asr/tts. */
  languages: string[];
  model: { name: string; version: string; license: string; sizeKB: number | null; install: string };
  platform: string;
  readiness: Readiness;
  /** Plain-language limits shown to the user ("common travel phrases only"). */
  constraints: string[];
  quality: string;
  resources: { memoryKB: number | null; loadMs: number | null };
  verification: Verification;
}

export interface LocalTranslateRequest {
  text: string;
  from: string;
  to: string;
  signal?: AbortSignal;
}

/** null = this runtime does not cover the input (caller decides what happens next). */
export type LocalTranslateResult = { text: string; coverage: 'exact' | 'template' } | null;

export interface TranslateRuntime {
  readonly id: string;
  info(): RuntimeInfo;
  /** Time-bounded; never downloads anything. */
  discover(timeoutMs: number): Promise<Readiness>;
  supports(from: string, to: string): boolean;
  load(signal?: AbortSignal): Promise<void>;
  unload(): void;
  translate(req: LocalTranslateRequest): Promise<LocalTranslateResult>;
  dispose(): void;
}

export interface VoiceRuntime {
  readonly id: string;
  info(): RuntimeInfo;
  discover(timeoutMs: number): Promise<Readiness>;
  /** A voice that synthesizes ON this device for the language, if any. */
  hasLocalVoice(lang: string): boolean;
}

export interface AsrRuntime {
  readonly id: string;
  info(): RuntimeInfo;
  discover(timeoutMs: number): Promise<Readiness>;
  supports(lang: string): boolean;
}

export const base = (lang: string) => lang.split('-')[0].toLowerCase();
export const pairKey = (from: string, to: string) => `${base(from)}>${base(to)}`;
