# Counsel brief — Talktual "Live translate" (ambient translation) in the EU / Hungary

**This is a factual brief prepared for qualified counsel. It is not legal advice and makes no claim about legality or compliance.** Legal references are listed for counsel's verification. They are not conclusions.

Prepared: 2026-11-23 · Product state: `main` at the Phase 8 merge (`41cdb8f`), plus Phase 8.5 fixes on the working branch.

## 1. What the feature does (facts)
1. A user deliberately starts "Live translate" on their phone or computer. The app then listens through the device microphone to **speech of people near the user**, who may not use Talktual and have not been asked.
2. The browser's built-in speech recognition converts speech to text:
   - **Chrome/Edge:** Google or Microsoft cloud services
   - **Safari:** Apple
   - **On-device recognition:** used only where the browser supports it; not available for Hungarian on the tested hardware
3. The text is sent to the Talktual server and translated by a provider the operator configures: MyMemory (Translated srl, Italy), Google Gemini or DeepL. Text may instead be translated on the device for some pairs.
4. Optionally, the translated text is spoken through ElevenLabs (via the Talktual server) or by the device's own voice.
5. **Retention (verified in code and tests):**
   - Talktual stores no audio.
   - Text exists in the app's memory until the user clears it or leaves; it is never written to storage.
   - The server keeps translated text in an in-memory cache for **up to 2 minutes** (configurable; can be disabled), without user identifiers. Server logs contain no text.
   - Browser storage holds only a consent flag and a mode preference.
6. **Indicators and control:**
   - listening is visible at all times (indicator, tab title, control)
   - it stops immediately on Pause, leaving the screen, going offline or losing the mic
   - it never runs in the background
7. **Disclosure shown to the user before first use** (`src/ambient/disclosure.ts`) covers what is captured, who processes it per mode, what is kept, and guidance about people nearby.
8. **No notice is given to the people nearby** by the app.

## 2. Data flows
| Data | From → To | Roles to be assessed |
| --- | --- | --- |
| Room audio (third parties' voices) | Device mic → browser vendor recognizer | User? Talktual operator? Browser vendor (Apple/Google/Microsoft): processor or independent controller? |
| Recognised text | Device → Talktual server (EU/other hosting, operator's choice) → translation provider | Operator as controller? Provider as processor or controller (terms differ: MyMemory, Gemini, DeepL) |
| Translated text | Server → ElevenLabs (if TTS on) | Processor? |
| IP address | Device → Talktual server | Operator |

Cross-border: recognition vendors and some providers (Google, Microsoft, ElevenLabs) may process outside the EU/EEA.

## 3. Legal references for counsel to assess
- **GDPR Art. 2(2)(c) — household exemption.** Does a private user's ambient translation fall within it? The CJEU read it narrowly where public space is captured (**C-212/13, Ryneš**, 11 Dec 2014, under Directive 95/46/EC; https://curia.europa.eu/jcms/jcms/P_152318). Talktual does not record or store, which may be relevant.
- **GDPR roles (Art. 4(7)/(8), Art. 26, 28):** the operator's role regarding third-party speech processed via the browser's recognizer, which Talktual does not control. Does the operator determine purposes and means of processing bystanders' voices?
- **Lawful basis (Art. 6)** for the operator's processing of bystander speech (recognised text) — e.g. legitimate interests and a balancing test. **Special categories (Art. 9)** could be incidentally revealed by speech content.
- **Transparency (Art. 12–14):** feasibility of informing bystanders; whether Art. 14(5)(b) (disproportionate effort) is relevant.
- **Minimisation and storage limitation (Art. 5(1)(c),(e)):** present design is no audio storage, memory-only text, and a 2-minute server cache (disableable).
- **Security (Art. 32)** and **international transfers (Chapter V;** EU–US Data Privacy Framework status of each vendor).
- **EDPB Guidelines 02/2021 on Virtual Voice Assistants (v2.0, 7 July 2021):** treatment of "accidental users" and other people's voices captured by voice systems; filtering and deletion expectations (https://www.edpb.europa.eu/system/files/2021-07/edpb_guidelines_202102_on_vva_v2.0_adopted_en.pdf).
- **Hungary:**
  - **Civil Code (Act V of 2013) §2:48** — consent requirements for making and using recordings of a person's **voice**. Does real-time recognition without recording constitute a "recording" or a "use"? Exceptions for public crowds and public life.
  - **Act CXII of 2011 (Infotv.)** — the national data-protection framework and the NAIH's role.
  - **Criminal Code (Act C of 2012) §422** — illicit data acquisition (secret surveillance, recording of private communications). Is the scope engaged by covert listening to others' conversations in private settings?
  - NAIH guidance on audio recordings (https://www.naih.hu/files/2016_05_09_tajekoztato_hangfelvetelekrol.pdf).
- **ePrivacy / communications confidentiality:** likely not engaged (no electronic communications service is intercepted), but please confirm.
- **EU AI Act:** whether any obligation applies (speech recognition and translation, not emotion recognition or biometric identification). Please confirm the classification.

## 4. Questions for counsel
1. Is ambient translation by a private user within the household exemption? Does the answer differ for cafés (public) versus private homes or offices?
2. What is Talktual's operator role for bystander speech: controller, joint controller with the user, or none for the browser's recognition step?
3. Which lawful basis applies to the operator's processing of bystanders' recognised text and its translation? Is a legitimate-interests assessment needed?
4. Is any **bystander notice** required or advisable (a visible on-screen sign the user can show, or a spoken notice)? Is the current user-facing guidance sufficient?
5. Does Hungarian Civil Code §2:48 apply to real-time processing without recording? If so, what consent or exception applies?
6. How should the browser vendors' processing (Apple/Google/Microsoft recognition) be described in the privacy notice, given Talktual doesn't control their retention?
7. Is the 2-minute server cache acceptable, or should it be disabled for ambient traffic (configurable today)?
8. Transfer mechanisms for the cloud recognizer and providers. Should EU-only providers be preferred (e.g. DeepL) for EU users?
9. Are there places or contexts where the feature should be discouraged or blocked (workplaces, healthcare, courts, schools)?
10. Is a DPIA (Art. 35) required or advisable?

## 5. Unresolved facts (to establish before or with counsel)
- The hosting region of the production Talktual server, and which translator and TTS are configured in production.
- Each provider's current terms (processor or controller; retention; training use).
- Whether the iOS recognition for Safari runs on device or on server for hu-HU on target devices (device test).
