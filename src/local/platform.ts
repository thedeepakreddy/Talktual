// Platform-provided on-device runtimes, used ONLY when the platform itself
// reports them ready. Discovery is time-bounded and never triggers a download.
//
// Evidence on the Phase 6 test hardware (docs/phase6/probe-local-capabilities.json):
//   Translator API availability() never answered; 0 speech voices; on-device
//   speech recognition availability() crashed the renderer. So none of these is
//   verified here — they report themselves 'unavailable'/'unknown' and the app
//   says so.

import { base, LOCAL_RUNTIME_VERSION, type AsrRuntime, type LocalTranslateRequest, type LocalTranslateResult, type Readiness, type RuntimeInfo, type TranslateRuntime, type VoiceRuntime } from './types';

// --- Chrome built-in Translator API --------------------------------------------------

interface TranslatorLike {
  translate(text: string, opts?: { signal?: AbortSignal }): Promise<string>;
  destroy?(): void;
}
interface TranslatorStatic {
  availability(o: { sourceLanguage: string; targetLanguage: string }): Promise<string>;
  create(o: { sourceLanguage: string; targetLanguage: string; signal?: AbortSignal }): Promise<TranslatorLike>;
}

export class BrowserTranslatorRuntime implements TranslateRuntime {
  readonly id = 'browser-translator';
  private ready = new Set<string>();
  private instances = new Map<string, TranslatorLike>();
  private readiness: Readiness = 'unknown';
  constructor(
    private readonly api: TranslatorStatic | undefined,
    private readonly pairs: Array<[string, string]>
  ) {}

  info(): RuntimeInfo {
    return {
      v: LOCAL_RUNTIME_VERSION,
      id: this.id,
      task: 'translate',
      languages: this.pairs.map(([f, t]) => `${base(f)}>${base(t)}`).filter((p) => this.ready.has(p)),
      model: { name: 'Browser built-in translator', version: 'platform', license: 'browser vendor terms', sizeKB: null, install: 'managed by the browser (Talktual never triggers a download)' },
      platform: 'Chrome with on-device translation models',
      readiness: this.readiness,
      constraints: ['Only language pairs the browser reports as available'],
      quality: 'Neural MT; quality per browser vendor (not evaluated here)',
      resources: { memoryKB: null, loadMs: null },
      verification: this.readiness === 'ready' ? 'platform-reported' : 'unverified',
    };
  }

  async discover(): Promise<Readiness> {
    if (!this.api) return (this.readiness = 'unavailable');
    let any = false;
    let downloadable = false;
    for (const [f, t] of this.pairs) {
      try {
        const a = await this.api.availability({ sourceLanguage: base(f), targetLanguage: base(t) });
        if (a === 'available') {
          this.ready.add(`${base(f)}>${base(t)}`);
          any = true;
        } else if (a === 'downloadable' || a === 'downloading') downloadable = true;
      } catch {
        /* treat as unavailable */
      }
    }
    return (this.readiness = any ? 'ready' : downloadable ? 'downloadable' : 'unavailable');
  }

  supports(from: string, to: string): boolean {
    return this.ready.has(`${base(from)}>${base(to)}`);
  }

  async load(): Promise<void> {
    /* instances are created lazily per pair */
  }

  async translate(req: LocalTranslateRequest): Promise<LocalTranslateResult> {
    if (!this.api || !this.supports(req.from, req.to)) return null;
    const key = `${base(req.from)}>${base(req.to)}`;
    let t = this.instances.get(key);
    if (!t) {
      // Only reached for pairs reported 'available' → create() does not download.
      t = await this.api.create({ sourceLanguage: base(req.from), targetLanguage: base(req.to), signal: req.signal });
      this.instances.set(key, t);
    }
    const text = await t.translate(req.text, { signal: req.signal });
    return text ? { text, coverage: 'exact' } : null;
  }

  unload(): void {
    for (const t of this.instances.values()) t.destroy?.();
    this.instances.clear();
  }

  dispose(): void {
    this.unload();
  }
}

// --- Speech synthesis voices that run on the device ----------------------------------

interface VoiceLike {
  lang: string;
  localService: boolean;
}

export class LocalVoiceRuntime implements VoiceRuntime {
  readonly id = 'local-voices';
  private langs = new Set<string>();
  private readiness: Readiness = 'unknown';
  constructor(private readonly getVoices: (() => VoiceLike[]) | undefined) {}

  info(): RuntimeInfo {
    return {
      v: LOCAL_RUNTIME_VERSION,
      id: this.id,
      task: 'tts',
      languages: [...this.langs],
      model: { name: 'Operating-system voices (localService)', version: 'platform', license: 'OS vendor terms', sizeKB: null, install: 'installed by the OS / user' },
      platform: 'browser speechSynthesis',
      readiness: this.readiness,
      constraints: ['Only voices the browser marks as on-device (localService); network voices are never used in on-device mode'],
      quality: 'Varies by OS voice',
      resources: { memoryKB: null, loadMs: null },
      verification: this.readiness === 'ready' ? 'platform-reported' : 'unverified',
    };
  }

  async discover(): Promise<Readiness> {
    // Browsers load the voice list asynchronously: wait briefly (bounded) for it.
    let voices = this.getVoices?.() ?? [];
    for (let i = 0; i < 10 && this.getVoices && !voices.length; i++) {
      await new Promise((r) => setTimeout(r, 100));
      voices = this.getVoices();
    }
    this.langs = new Set(voices.filter((v) => v.localService).map((v) => base(v.lang)));
    return (this.readiness = this.langs.size ? 'ready' : 'unavailable');
  }

  hasLocalVoice(lang: string): boolean {
    return this.langs.has(base(lang));
  }
}

// --- On-device speech recognition (Chrome processLocally) ----------------------------

interface SpeechRecognitionStatic {
  available?(o: { langs: string[]; processLocally: boolean }): Promise<string>;
}

/**
 * DISABLED by default: on the test hardware `available({processLocally:true})`
 * crashed the renderer (it would take the whole call down). Enabled only when
 * the build sets VITE_ONDEVICE_ASR=1, for real-device verification.
 */
export class OnDeviceAsrRuntime implements AsrRuntime {
  readonly id = 'ondevice-webspeech';
  private langs = new Set<string>();
  private readiness: Readiness = 'disabled';
  constructor(
    private readonly api: SpeechRecognitionStatic | undefined,
    private readonly enabled: boolean,
    private readonly candidates: string[]
  ) {}

  info(): RuntimeInfo {
    return {
      v: LOCAL_RUNTIME_VERSION,
      id: this.id,
      task: 'asr',
      languages: [...this.langs],
      model: { name: 'Browser on-device speech recognition', version: 'platform', license: 'browser vendor terms', sizeKB: null, install: 'browser language pack (Talktual never installs it)' },
      platform: 'Chrome with processLocally support',
      readiness: this.readiness,
      constraints: this.enabled ? ['Only languages the browser reports installed'] : ['Turned off in this build: the availability check crashed the browser tab on the test hardware'],
      quality: 'Not evaluated',
      resources: { memoryKB: null, loadMs: null },
      verification: 'unverified',
    };
  }

  async discover(): Promise<Readiness> {
    if (!this.enabled) return (this.readiness = 'disabled');
    if (!this.api?.available) return (this.readiness = 'unavailable');
    for (const l of this.candidates) {
      try {
        if ((await this.api.available({ langs: [l], processLocally: true })) === 'available') this.langs.add(base(l));
      } catch {
        /* unavailable */
      }
    }
    return (this.readiness = this.langs.size ? 'ready' : 'unavailable');
  }

  supports(lang: string): boolean {
    return this.langs.has(base(lang));
  }
}
