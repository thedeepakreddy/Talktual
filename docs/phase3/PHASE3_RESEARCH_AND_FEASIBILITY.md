# Phase 3 Research & Feasibility — VoiceMirror

**Question:** how can Talktual make the translated speech a listener hears preserve selected characteristics of the speaker's voice, given this product's constraints?

**Evidence used:** (1) primary sources fetched in this session (GitHub READMEs/LICENSE files; listed in §6); (2) a measured probe on synthetic voices (`eval/voicemirror/feasibility_probe.py`). Anything that couldn't be verified here is marked **not verified**. ElevenLabs, MDN, Hugging Face and coqui.ai were unreachable from this environment.

## 1. Constraints that decide feasibility

| Constraint | Value |
| --- | --- |
| Product | Browser app; listener hears either server TTS (ElevenLabs, one fixed voice) or the browser's `speechSynthesis` |
| Hardware here | 4 vCPU, no GPU |
| Model hub | Hugging Face blocked: no neural TTS/VC/speaker-encoder weights downloadable, except packages that bundle weights on PyPI |
| Paid APIs | Not authorized (no cloning calls, no new accounts) |
| Voice data | Only synthetic (espeak-ng) or explicitly consented; none consented exists |
| Phase 2 contracts | Lifecycle, consent, capture gates, segment IDs, bounded buffers must not regress |

## 2. Measured probe: what does "sounding like the speaker" depend on?

12 synthetic speakers (6 espeak timbres × 2 pitch/tempo settings) enrol in English; the listener hears Spanish. Similarity uses Resemblyzer's GE2E speaker encoder (cosine to the speaker's enrolment), plus top-1 identification among the 12.

| Output voice | Mean cosine | Identified correctly |
| --- | --- | --- |
| Today: one fixed voice | 0.746 | 1/12 |
| Default timbre + the speaker's exact pitch & tempo (*prosody oracle*) | 0.773 | 2/12 |
| Nearest **other** stock timbre + the speaker's pitch & tempo (*selection oracle*) | 0.838 | 0/12 |
| The speaker's own timbre + pitch + tempo (*perfect-clone oracle*) | **0.941** | **12/12** |
| Different speakers compared with each other (reference) | 0.767 | — |

**Findings**
1. **Timbre dominates identity.** Matching pitch and tempo perfectly barely moves speaker similarity (+0.03) and stays at the different-speaker level. Any approach that doesn't reproduce timbre can't claim identity preservation.
2. **Stock-voice selection gives resemblance without identity** (0.838 but 0/12): it sounds *more like* the speaker without being mistakable *for* them. That's also a safety property (no impersonation capability).
3. **Only cloning/conversion reaches identity** (0.941, 12/12), and that's exactly the class of approach blocked here (models, GPU, paid calls, consented data).

Caveats: synthetic formant voices are far less varied than human voices; GE2E "works best on English" (Resemblyzer README). These numbers rank approaches. They don't predict human perception (§5).

## 3. Approaches compared

| # | Approach | Representative options (verified facts) |
| --- | --- | --- |
| A | **Speaker-conditioned zero-shot TTS** (cross-lingual cloning) | OpenVoice V1/V2: **MIT**, "free for commercial and research use", V2 native EN/ES/FR/ZH/JA/KO, "zero-shot cross-lingual voice cloning". CosyVoice: code **Apache-2.0**, 9 languages, "multi-lingual/cross-lingual zero-shot voice cloning", ~150 ms bi-streaming latency claimed; weights licence **not verified**; README disclaimer "academic purposes only". F5-TTS: code MIT, **weights CC-BY-NC** (non-commercial); GPU RTF 0.04–0.15 on an L20. XTTS-v2: code MPL-2.0; weights licence (CPML) **not verified** here; "16 languages", "<200 ms" streaming claimed. Provider: ElevenLabs Instant Voice Cloning (paid; requirements **not verified**) |
| B | **Voice conversion after TTS** | kNN-VC: **MIT**, any-to-any, clean target speech; "improvement diminishes beyond 5 minutes" of reference. RVC: **MIT** code, per-speaker training on ≥10 min of audio. ElevenLabs speech-to-speech (paid; **not verified**) |
| C | **Nearest stock voice by speaker embedding** | Needs an embedding model on the device (e.g. a GE2E-class encoder; Resemblyzer is Apache-2.0, bundled weights) and embeddings of the provider's stock voices |
| D | **Characteristics matching** (pitch register, pitch liveliness, tempo) | Signal processing only (YIN pitch, energy envelope); browser `speechSynthesis` pitch/rate; provider speed (ElevenLabs `voice_settings.speed` is already sent by Phase 2 code; its allowed range **not verified**) |
| E | **Stock voice by register** (low/mid/high) + speed | Server-side catalog of provider voice IDs; no biometric model |

## 4. Decision matrix

Scores: ●●● good · ●● fair · ● poor. "Identity" uses the probe's oracles as upper bounds.

| Criterion | A zero-shot TTS | B voice conversion | C embedding → stock voice | D characteristics | E register → stock voice |
| --- | --- | --- | --- | --- | --- |
| Identity potential (probe ceiling) | ●●● (≈ clone 0.94, 12/12) | ●●● (≈ clone) | ●● resemblance (0.84, 0/12) | ● (0.77, 2/12) | ● to ●● (measured in the evaluation) |
| Added latency | ● to ●● (GPU model on the critical path; 150–200 ms claimed by sources, not measured) | ● (an extra model pass) | ●●● (selection once per call) | ●●● (analysis off the critical path; 1–2 ms per audio second measured) | ●●● |
| Multilingual / cross-lingual | ●● to ●●● (OpenVoice V2: 6 languages natively; CosyVoice: 9; Talktual has 31) | ●●● (language-agnostic) | ●●● | ●●● (language-agnostic signals; tempo calibration is language-sensitive) | ●●● |
| Consent / misuse risk | ● highest: creates a reusable synthetic voice of a person (impersonation) | ● same | ●● biometric embedding on device; output is someone else's voice | ●●● coarse, non-invertible descriptor | ●●● |
| Data retention | Reference audio or voice model must exist server-side/at provider for the call; deletion obligations | Same | Embedding on device only (if designed so) | None (memory only, per call) | None |
| Deployment | GPU service or paid API | GPU service or paid API | Ship an ML model to browsers (~17 MB class) + catalog embeddings | Pure TS, already in app | Server config |
| Cost | GPU hours or per-character provider fees | Same | Low | ~0 | ~0 |
| Licensing | OpenVoice MIT ✓; CosyVoice weights ?; F5 weights NC ✗; XTTS weights ? | kNN-VC MIT ✓; RVC MIT ✓ (training-heavy) | Encoder licence to check | none | none |
| **Feasible in this environment** | ✗ (no models, no GPU, no paid calls, no consented data) | ✗ (same) | ✗ (no provider catalog audio; model packaging out of scope) | **✓** | **✓** |

## 5. Recommendation and decision

**Decision for Phase 3: build D + E ("characteristics" tier) as the controlled prototype, plus the VoiceMirror seam that higher tiers plug into. Design, but don't build or activate, A/B/C.**

Why:
* D/E are the only options that are feasible *and* safe to run here, and they deliver what they promise: pitch register and pace follow the speaker. The probe shows they **won't** deliver identity, so the product must not present them as voice cloning (the UI copy says "does not copy or clone your voice").
* The seam (opt-in, revocation, a per-segment hint resolved at playback, server-owned voice selection, privacy-safe diagnostics) is exactly what A/B/C will need, so the Phase 3 work isn't throwaway.
* Real identity preservation needs **A (zero-shot cloning) with OpenVoice V2 (MIT) self-hosted on a GPU, or a paid provider**. Both need owner decisions: cost, GPU infrastructure, a consent/verification design for creating someone's synthetic voice, and retention/deletion terms. None of that is authorized in Phase 3.

**Next-tier entry conditions (owner decisions)**
1. Budget and hosting choice (GPU service vs provider), plus legal review of provider terms and any consent-verification requirement (not verified here).
2. A consented, licensed human voice dataset for evaluation. Synthetic results don't transfer to human perception.
3. Human listening tests (MOS/similarity) as the acceptance metric. Speaker-encoder cosine is a proxy.

## 6. Sources
Fetched successfully in this session: github.com/myshell-ai/OpenVoice (+ raw LICENSE), github.com/SWivid/F5-TTS, github.com/FunAudioLLM/CosyVoice, github.com/bshall/knn-vc (+ LICENSE), github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI, github.com/resemble-ai/Resemblyzer, github.com/coqui-ai/TTS.
Unreachable (DNS/policy): huggingface.co (XTTS-v2 card), coqui.ai/cpml, elevenlabs.io (cloning, delete-voice, speech-to-speech, TTS settings docs), developer.mozilla.org (`SpeechSynthesisUtterance.pitch/rate`). Claims depending on those are marked *not verified*.
Tools used locally: espeak-ng 1.51 (synthetic speech), Resemblyzer 0.1.4 (GE2E encoder; README: "works best on English"), librosa 1.0.1 (pYIN), pocketsphinx 5.1.1 (bundled en-US model). All are evaluation-only and not shipped.
