# Phase 7 final — UI/UX redesign and product integration — completion report

Governing documents: the *Phase 7 Final UI/UX Redesign Execution Specification* and the binding *Design Constitution* (which wins on visual style).

## 1. Verdict
**Engineering complete for the web platform scope; real-device visual and behavioural validation is pending.**
- The redesign is implemented, integrated with real engine state, inspected and corrected in four screenshot passes, accessibility-checked and regression-tested.
- Everything was run in Chromium, with phone and tablet emulation. Safari, Firefox and physical devices are **unverified** (§9).

**Branch and boundaries:**
- Branch `claude/kind-archimedes-n6y11p`, on top of `main` `41cdb8f` (Phase 8 merge) and the Phase 8.5 commits.
- No merge, PR, deploy, credential change or paid call was made during this phase. Phase 9 was not started.

## 2. Tools and references actually used
| Tool | Used | How |
| --- | --- | --- |
| design-superpowers `/creative` skill | ✔ | Maturity detection (L1); concept method: design read, theme statement, three concepts from distinct families |
| design `/design-critique` skill | ✔ | Critique of the final renders → the visible-label fix for dock controls (§6, pass 4) |
| Playwright + Chromium (pre-installed) | ✔ | Concept renders, 4 iteration passes, 120 final screenshots, a11y and E2E suites |
| Web search (public sources) | ✔ | Apple Live Translation (AirPods), Voice Memos (time-axis waveform), Google Translate conversation mode, Apple HIG accessibility (§3) |
| **Mobbin connector** | ✘ **blocked** | Connected, but every call returns "requires a paid plan". Upgrading is a paid action and was not done |
| Figma connector | not used | Connected, but no Figma file was provided; creating one is an external write that needs approval |
| Superdesign skill | not used | External generation service (account or credits); generated variants aren't references |

## 3. Audit findings (before; screenshots in `before/`)
1. Navigation had no destinations. Everything branched from one Home form; Voice, Settings, Lens and Rewind didn't exist as places.
2. Palette and shape didn't match the constitution: an amber accent, round icon buttons, a pulsing dot, and synthetic "level bars" that animated whether or not the sound level changed (**prohibited**: motion not reflecting real activity).
3. Call turns used chat bubbles for your own lines (the constitution says "no excessive chat bubbles").
4. Only system theme; no user choice.
5. **Capabilities:**
   - **Lens:** no camera or text recognition exists.
   - **Rewind:** nothing is retained, by policy.
   - **VoiceMirror:** behind a build flag.
   - **Voice:** server TTS, or the browser voice.
6. **Tooling defect:** `@types/react` was missing, so JSX was type-checked against fallback typings. Added; zero new errors.

**References informing decisions:**
- [Apple: Live Translation with AirPods](https://support.apple.com/guide/airpods/dev9c215ca94/web): audio in the ear, transcript on the phone, tap to play again. This is AIR's audio plus transcript split and its single "Play again".
- [Apple Voice Memos](https://support.apple.com/guide/voice-memos/vma2c8c0a040/mac): a horizontal, time-based waveform with time markings. This is the AIR activity trace with its −15 s…now axis, instead of an orb.
- [Google Translate conversation mode](https://support.google.com/translate/answer/6142474): each speaker reads their own language. Connect shows your line in your language and theirs in yours.
- [Apple HIG — Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility): readability comes from size and contrast in both appearances. Translation gets the largest type token, and contrast is validated in both themes.

## 4. Concepts and decision (`concepts/`)
| | A Atmospheric | B Instrument | C Editorial |
| --- | --- | --- | --- |
| Composition | Full-bleed field, a single centred translation, a wide trace | Language-pair strip, trace with a time axis, speaker-label rows, tabular metadata | Typographic page, translation column, a thin activity line, index list |
| Verdict | Rejected: slogan-sized Home heading; **showed waveform bars while not listening (fake activity)**; weak navigation | **Selected structure**, after removing its boxed IN/OUT strip (a card) and its uppercase HUD labels | **Selected typography and copy honesty**; rejected its activity line (it reads as a progress bar) |

**Final:** B's structure, C's typography and honest one-line statuses, and A's large current translation.

## 5. Design system (frozen in `DESIGN.md`)
- **Palette:** constitution graphite and teal, with **two documented light-theme deviations** required by WCAG AA. Light secondary `#6B737B` → `#5F676F`; light accent `#327F84` → `#2C7479`. The light `line-strong` was also tuned to 1.6:1.
- **Type:** page title 26, translation 24, section 19, transcript 18, body 15, supporting 13, metadata 12.
- **Radii:** controls and inputs 10, surfaces 14, sheets 16; no pills.
- **Depth:** three levels.
- **Motion tokens:** 120 / 200 / 280 ms.
- **Theme:** System / Light / Dark (Settings › Appearance), applied before first paint.
- **Shell:**
  - desktop: an 88 px rail (five destinations, plus Voice and Settings in the footer)
  - phone and tablet: a bottom bar (five destinations)
  - active sessions are full-screen with navigation hidden
  - a "Skip to content" link

## 6. Implementation status
| Destination | Status | Notes |
| --- | --- | --- |
| Home | ✔ | AIR first: language pair, real processing and output status, **real blockers** (no speech recognition, insecure context, mic blocked). A quiet list for Connect, Voice Studio, Lens and Rewind |
| AIR | ✔ | See below |
| Connect | ✔ | Entry (name, language, Start / Join); invite; join (invite link and code); call with speaker-label rows, the latest incoming translation at 24 px, labelled dock. All Phase 4–6 markers preserved |
| Lens | ✔ (honest) | "Not available in this version". Doesn't open the camera; links to AIR and Connect. **Deviation:** no camera OCR exists, and inventing it is prohibited |
| Rewind | ✔ (honest) | "Nothing is kept" with the real retention facts. No search box (there's nothing to search). **Deviation:** retention isn't offered by policy |
| Voice Studio | ✔ | Speak in AIR and volume (stored preferences, used as AIR's defaults); a real sample through the playback controller; server natural-voice status; device voices (on-device or network); VoiceMirror stated truthfully per build |
| Settings & Privacy | ✔ | Language, theme, processing mode (plain language), microphone permission (live), AIR and call consent status with "Forget", "What AIR captures" (the real disclosure), "What Talktual keeps", and clear cached translations |

**AIR, in detail:**
- **Language pair strip:** "They speak" and "You hear", tappable.
- **Transcript:** divider-separated; the newest translation at 24 px, earlier ones at 18 px, originals secondary, the live interim in italics.
- **Activity trace:** the last 15 s on a time axis. It rises only on real recognizer events, stays flat in silence, and dims when paused.
- **Dock:** Speech on/off · Start / Pause / Resume / Start again · Stop / Clear, all labelled.
- **States:** ready, consent, listening, held for speech, paused (amber), stopped (distinct), unavailable (with Settings), stalled, offline, hidden. Autostart from Home.

**Engine semantics: unchanged.** Consent gates, processing-mode enforcement, cancellation, half-duplex, retention and caches are untouched. The UI only calls existing commands.

## 7. Visual iteration evidence
| Folder | Contents |
| --- | --- |
| `before/` | the previous UI |
| `concepts/` | concepts A, B and C |
| `iter1/` | pass 1, phone dark |
| `iter2/` | pass 2, desktop dark and phone light |
| `after/` | final: **120 renders**, i.e. 20 states × phone/tablet/desktop × dark/light |

| Pass | Found | Fixed |
| --- | --- | --- |
| 1 | Dashed silence pattern in the trace; stopped labelled "Not listening"; page titles cramped on phone; Lens actions stretched with a centred ghost link | Continuous baseline; "Stopped" label; header spacing; inline actions |
| 2 | Scroll position carried across destinations (back/forward and links) | Scroll reset on every route change |
| 3 | Page-title padding still ineffective (the `safe-t` utility overrode it); "Live translate" naming left in the disclosure and engine messages | Header padding uses `max(spacing, safe area)`; AIR naming throughout |
| 4 (critique) | Icon-only dock controls were ambiguous (speaker-slash: muted, or speech off?) | Visible labels: "Speech on / Text only", "Stop", "Clear", "Mic", "Type" |

**Tests found these too:**
- light `line-strong` at 1.49:1 → 1.6:1
- skip-link padding applying while hidden → focus-only padding

## 8. Accessibility
`e2e/a11y.mjs`, 11/11, Chromium:
- contrast of 20 token pairs in both themes (lowest text pair ≥ 4.5:1)
- 44 px targets on all phone screens (calls and AIR)
- keyboard: skip link → Connect tab order; whole call by keyboard; sheet focus trap and return; Space push-to-talk
- 200 % text with no horizontal scroll
- reduced motion
- live regions and labelled logs

Plus: the AIR trace has a text alternative; status is never colour-only (every dot has a label).

## 9. Tests and coverage (verified · simulated · unverified)
| Suite | Result | Kind |
| --- | --- | --- |
| lint/typecheck (now with real React types), 518 unit, 60 server, build | pass | verified |
| smoke 17 · recovery 8 · compat 5 · conversation 8 · intelligence 7 · processing 7 · VoiceMirror 8 · a11y 11 · AIR 13 · validation 2 · reliability loop 10 | pass (see the final run in the summary) | simulated (scripted recognizer, fake voice, stubbed providers) in real Chromium |
| Safari / iOS, Firefox, physical phones, screen readers (VoiceOver, TalkBack, NVDA) | **not run** | **unverified**; use `docs/phase8_5/DEVICE_TEST_PROTOCOL.md` plus a VoiceOver pass |

New tests:
- App flows: Connect start and join, Home primary action and navigation, Lens and Rewind honesty, theme preference
- E2E entry paths: Connect at `#/connect`, AIR at `#/air`
- a11y: skip link and the Connect tab order

## 10. Known limitations and defects
- **Lens and Rewind** are honest placeholders until a camera-OCR capability and an opt-in retention policy exist.
- **Voice choice:** voice selection isn't user-controllable; the engine picks a matching voice automatically, and the page says so.
- **Phone navigation:** Voice Studio and Settings aren't in the phone bottom bar (five slots are approved). They're reached from Home and its header.
- **Mobbin** reference research couldn't be performed (paid plan).
- **Autostart:** AIR autostart from Home may need a second tap on iOS if Safari requires a fresh user gesture for recognition. Unverified on device.

## 11. Rollback
The Phase 7-final commits are view-layer. Revert them (`git revert fa76ae9^..HEAD` on this branch), or check out `src/` from `1249459`. There are no data, engine or server changes, other than the dev-only `@types/react`.

## 12. Brand application (after the owner shared the brand sheet)
The owner's brand sheet (`docs/brand/talktual-brand-sheet.png`) supersedes the constitution's provisional palette. Applied:
- **Palette tokens:** charcoal and Warm White bases, orange accent, Secondary Orange as dark-theme accent text.
- **Type stack:** Inter / Manrope / SF Pro, with no web-font download.
- **Mark and wordmark:** a vector rebuild of the logo, plus the wordmark.
- **App icon:** `favicon.svg` plus a 180 px touch icon.

**Accessibility handling:** orange on Warm White is 2.06:1, so in light mode orange is a fill (with charcoal text). Orange text and the activity trace use a darkened `#A4560B`. The paused state moved to a yellow warning, distinct from the orange accent. a11y 11/11; contrast lowest text pair 4.68 (dark) and 4.85 (light). The `after/` evidence set was re-rendered in the brand.

## 13. Next steps
1. **Device pass:** iPhone Safari, Android Chrome and VoiceOver, on Home, AIR and Connect, in both themes.
2. **Mobbin reference review** if the plan is upgraded.
3. A decision on Lens (on-device OCR approach) and Rewind (opt-in retention, which needs counsel review per Phase 8.5).

**Phase 9 was not started.**
