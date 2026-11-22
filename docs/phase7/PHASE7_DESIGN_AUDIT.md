# Phase 7 — Design Audit (before)

Evidence: real renders captured with `node e2e/screens.mjs docs/phase7/screens/before phone,desktop` (two Chromium pages, real app and server, providers stubbed, scripted speech). These are 13 states × 2 viewports.

## Inventory
* **Screens:**
  * Home (name, language, Start/Join)
  * StartCall (QR, code, copy/share, waiting)
  * JoinCall (scan/enter tabs, code boxes)
  * CallRoom (top bar, language rail, notice, Phase 4 chips, Phase 6 chips, Phase 5 suggestion, "speaker panel", PTT, meta line, 5 pills + disclosure link)
* **Sheets:** Transcript, Privacy & processing (Phase 6), Conversation assistance (Phase 5), Consent, VoiceMirror consent, Language picker.
* **Flags affecting UI:** `VITE_CONVERSATION_LAYER`, `VITE_COMMUNICATION_INTELLIGENCE`, `VITE_PROCESSING_MODES` (on by default), `VITE_VOICEMIRROR` (off).
* **Named capabilities:** AIR, FACE, LENS and REWIND **do not exist in the codebase**, so nothing is presented for them. "VOICE" corresponds to VoiceMirror (flag-off prototype), which stays behind its flag.
* **Design system:** ad-hoc. Apple dark-mode color literals, Tailwind arbitrary values everywhere (`text-[13px]`, `bg-white/[0.06]` …), and three sheet implementations with copy-pasted chrome. There's no shared Button, Sheet or Switch.

## Findings (prioritized)
| # | Severity | Finding | Evidence |
| --- | --- | --- | --- |
| F1 | **P0** | The conversation (the product) is hidden in a modal sheet behind a badge. The main screen shows only the *last* line, inside a decorative "speaker grille" panel | phone-05/08 |
| F2 | **P0** | Opening an invite link lands on the generic home screen with "Start a channel" as the primary action. The code isn't acknowledged until the user finds "Join a channel" | phone-03 |
| F3 | P1 | Control sprawl: 5 pills (Hands-free, Cloud, Type, Assist, How your voice is processed) wrap into misshapen 2-line pills; mode and feature toggles compete with the talk control | phone-05, desktop-05 |
| F4 | P1 | Diagnostics in the primary UI ("Direct · Live transcribe · 67ms · System voice") | phone-08 |
| F5 | P1 | Status shown as up to three stacked chip rows (Phase 4, Phase 6, notice) plus a hint line. No single place to look for "what's happening" | code |
| F6 | P1 | Destructive "leave" sits top-left (the usual *back* position) as an unlabeled red icon, with no confirmation | phone-05 |
| F7 | P1 | Three separate settings surfaces (Privacy sheet, Assistance sheet, inline pills) for one concept: "how this call is processed" | code |
| F8 | P2 | Skeuomorphic PTT (conic bezel, specular dome, glow, ripples), a radial-gradient page background, a glowing app tile, and a perforated grille texture: decoration, not information | all |
| F9 | P2 | Consent is a 5-paragraph wall with no structure | phone-04 |
| F10 | P2 | Filler copy ("31 languages. You can switch mid-call.", "Whatever you say appears here, translated.", "They scan this, pick their own language, and you're talking.") | phone-01/02 |
| F11 | P2 | Disabled primary buttons look broken (30 % opacity blue) before a name is typed, and nothing explains why | phone-01 |
| F12 | P2 | Accessibility: sheets have no focus management or return focus; several targets are 32 px (sheet close, chips); `--text-3` (32 % white) fails contrast for meaningful text; no `prefers-reduced-motion` handling for sheet springs | code |
| F13 | P2 | Desktop is a phone column floating in a large empty canvas, with no use of width | desktop-05 |
| F14 | P3 | "Channel" vocabulary (radio metaphor) is inconsistent with "call" elsewhere (leave *channel*, *call* ended) | copy |

## Priority journeys
1. **Host:** open app → name/language → start → share → partner joins → hold to talk → read and hear the reply → end.
2. **Guest:** open invite link → name/language → join → same loop.
3. **Recovery:** connection drop, partner away, mic permission denied, translation unavailable, offline / on-device only.
4. **Privacy:** understand and change processing mode; clear data; consent.
