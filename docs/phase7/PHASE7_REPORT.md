# Phase 7 — UI/UX redesign, product integration and design quality

Branch `claude/kind-archimedes-n6y11p` · baseline `8862e78` (end of the reliability task) · no PR, merge or deploy.

## 1. Summary
The interface was rebuilt around one idea: **the conversation is the screen**. Before, the conversation was hidden in a sheet behind a badge. Controls were scattered across five pills and three settings surfaces, and status was split across stacked chip rows. Now:

* The call room is a header, the live conversation and a talk dock. There is one status line and at most one banner.
* Everything adjustable lives in one **Call settings** sheet.
* An invite link opens a join screen with the code already in place.

Engine semantics are unchanged: the UI reads the session snapshot and calls the same commands as before. All 462 unit tests, 58 server tests, 8 browser suites and the reliability loop pass. A new scripted accessibility suite passes 9/9.

## 2. Design audit (before)
Full audit with evidence: [`PHASE7_DESIGN_AUDIT.md`](PHASE7_DESIGN_AUDIT.md). "Before" screenshots are in [`screens/before/`](screens/before/).

| # | Finding | Status after Phase 7 |
| --- | --- | --- |
| F1 P0 | Conversation hidden in a sheet; main screen showed only the last line | **Fixed.** Inline feed is the main surface (`call/ConversationFeed.tsx`). |
| F2 P0 | Invite link landed on Home | **Fixed.** `?join=` opens Join with the code shown; it asks only for name and language. |
| F3 P1 | Five control pills | **Fixed.** Mic and Type sit beside the talk button; modes are in settings. |
| F4 P1 | Diagnostics in the primary UI | **Fixed.** Removed. The connection type is under Settings › This call. |
| F5 P1 | Stacked status chip rows | **Fixed.** One status line (`aria-live`) and one banner (`call/status.ts`). |
| F6 P1 | Unlabelled red leave icon in the back position | **Fixed.** A labelled "End" button sits top-right ("End call" for assistive tech). A live call asks “End this call?” (Keep talking / End call); once the call is over, End leaves immediately — see §9. |
| F7 P1 | Three settings surfaces | **Fixed.** One sheet with *Talking · Processing · Assistance · Voice · This call* sections and two sub-pages. |
| F8 P2 | Skeuomorphic talk button, gradients, glow, grille texture | **Fixed.** Flat surfaces. Depth comes only from surface steps and hairlines. |
| F9 P2 | Consent as a wall of text | **Improved.** Structured list with the same text, verbatim from the engine. |
| F10 P2 | Filler copy | **Fixed.** Rewritten (see DESIGN.md › Copy). |
| F11 P2 | Disabled primary buttons with no reason | **Fixed.** Buttons stay enabled; an inline error explains and focuses the field. |
| F12 P2 | Accessibility gaps | **Fixed and verified** (§6). |
| F13 P2 | Desktop wasted width | **Improved.** 760 px conversation column. Sheets become a 400 px side panel and the column moves aside, so the conversation stays readable. |
| F14 P3 | "Channel" vs "call" vocabulary | **Fixed** in all user-facing copy and accessible names. Engine error *codes* are unchanged. |

## 3. Design direction — "Quiet instrument"
A translated call is a conversation, not a dashboard.

* Graphite neutrals, with one amber signal colour used only for talking and the single primary action. Incoming speech is blue, connected is green, danger is red.
* System font stack, so no web-font request is made.
* Dark and light themes follow the operating-system setting.
* No ornament: no gradients, glows, textures, decorative cards or icon tiles.
* Honest states: anything unavailable is either hidden or labelled "Unavailable", never a dead control.

## 4. System and tokens
`DESIGN.md` at the repository root is the normative reference: frontmatter, colour table for both themes, type roles, spacing, radius, elevation, motion, breakpoints, components and copy rules. Tokens live in `src/index.css` as CSS variables and are exposed to Tailwind through `@theme inline`.

* **Colour:** semantic tokens only (`bg`, `raised`, `surface`, `surface-hover`, `selected`, `line`, `line-strong`, `fg`, `fg-muted`, `fg-subtle`, `accent`, `on-accent`, `accent-text`, `incoming`, `success`, `danger`, `focus`, plus soft tints).
* **Type:** `title` 24/30, `heading` 17/24, `body-lg` 17/25 (conversation), `body` 15/22, `label` 13/18, `meta` 12/16. All are rem-based, so they follow the browser's text-size setting.
* **Targets:** at least 44 px everywhere. The talk control is 88 px.

### Two system-level defects found and fixed during visual iteration
1. **Unlayered element defaults beat every utility.** `button { color: inherit }` sat outside `@layer`, so every coloured button label (primary, danger, Stop, Leave) rendered in the foreground colour, e.g. white on amber. I confirmed this by measuring pixels on the screenshot, not by eye. The fix moves element defaults into `@layer base`.
2. **tailwind-merge dropped colours.** It didn't know that `text-body`/`text-label` are font sizes, so it treated them as colours and silently removed `text-on-accent` and others. The fix extends `cn()`, and `src/utils.test.ts` covers it.

## 5. Implemented screens and components
**Primitives** (`src/components/ui/`):

* `Button`: primary, secondary, ghost and danger variants; 48 px and 44 px heights.
* `IconButton`: 44 px; requires a label.
* `Sheet`: dialog with focus moved in, trapped and returned; Escape closes; bottom sheet on phones, side panel on desktop.
* `Switch` (`role=switch`), `Segmented` (radiogroup with arrow keys), `Row`/`Section`, and `Banner` (status or alert).

**Screens and composites:**

| Surface | File | Notes |
| --- | --- | --- |
| Home | `Home.tsx` | Name, language, "Start a call", "Join with a code"; inline validation. |
| Invite | `StartCall.tsx` | QR code, call code, Copy code, Share/Copy link, waiting status. Lobby logic unchanged. |
| Join | `JoinCall.tsx` | Invite path (code known) or six-box code entry with auto-join; QR scanning. Lobby logic unchanged. |
| Call room | `CallRoom.tsx` | Header with peer, state, language pair, mode and timer; Settings; End. Below that: banner, feed, suggestion, composer, dock. |
| Conversation feed | `call/ConversationFeed.tsx` | `role=log`. Own lines on a surface bubble; the other person's lines with a left rule, translation first and original underneath. Phase 4–6 markers and Phase 5 notes and signal tags (on the listener's side only). Live captions. Follows new content unless the user has scrolled up. |
| Talk dock | `call/TalkDock.tsx` | Mic on/off, hold-to-talk (pointer capture, Space key), Type. In hands-free the centre becomes "Stop". |
| Status | `call/status.ts` | Single source for the status line, banner priority and header state. Never repeats the same sentence. |
| Settings | `call/SettingsSheet.tsx` | Listening mode, language, drafts, processing mode (+ *Where things run*), clear data, assistance switches (+ *What it remembers*), Voice match (flag-gated), call code, connection, disclosure. |
| Consent | `ConsentSheet.tsx` | Same disclosure text; "Not now" / "Allow microphone". Consent prompts close settings so dialogs never stack. |
| Language picker | `LanguagePicker.tsx` | Search with full-height list on phones. |

**Integration preserved:**

* Phase 2: lifecycle, timer, auto-return after peer-left or expiry.
* Phase 4: provisional, interrupted, held and overlap markers; drafts switch; "1 message waits until you finish".
* Phase 5: notes, signals, a single dismissible suggestion, context review and reset, assistance switches.
* Phase 6: processing modes, where things run, offline/unreachable banners, typed input whenever speech input is unavailable, clear data, peer privacy marker.
* VoiceMirror: still **off by default**, shown only when the flag is on, and still behind its own disclosure.

## 6. Accessibility and viewport checks (`node e2e/a11y.mjs`, 9/9)
| Check | Method | Result |
| --- | --- | --- |
| Contrast, dark | WCAG formula on 20 token pairs read from `index.css` | Pass; lowest text pair 4.94:1 |
| Contrast, light | Same | Pass; lowest text pair 4.94:1 (`fg-subtle` raised from 4.26 → 4.94) |
| Touch targets | Every visible interactive element on Home, Invite, Join, Consent, Call and Settings at 390 px | All ≥ 44 × 44 (40 px controls and a 15 px footer link fixed) |
| 200 % text | Root font at 200 % on the phone call and settings screens | No horizontal scroll; talk button stays on screen |
| Keyboard | Tab order on Home; whole call by keyboard; sheet focus in, trapped for 40 tabs, Escape, focus returned; Space push-to-talk ignored in text fields | Pass. Found and fixed a real bug: the sheet's focus effect re-ran on every engine update and bounced focus out mid-call. |
| Live regions / landmarks | Status line `aria-live=polite`; feed `role=log`; banners `status`/`alert` | Pass |
| Reduced motion | `prefers-reduced-motion: reduce` → no animation or transition over 10 ms | Pass |

Not done: a screen-reader session (VoiceOver/TalkBack/NVDA) and an automated axe audit (deliberately no new dependency). These are listed as next conditions.

## 7. Visual evidence
`docs/phase7/screens/after/` holds real renders from Playwright Chromium against the real signalling server, with providers stubbed and speech scripted. Viewports are phone 390×844 (touch, mobile), tablet 834×1112 and desktop 1440×900. The dark theme is captured at all three; the light theme (`*-light.png`) at phone and desktop.

There are 22 captures per viewport and theme (including the End-call confirmation): home, name required, invite, code entry, invite join, consent, idle, holding with live caption, received, conversation (both sides), settings (top and scrolled), where things run, memory, language, hands-free, typing, reconnecting / peer away, and peer left. Regenerate with `node e2e/screens.mjs <outDir> phone,tablet,desktop 1 dark|light`.

I inspected each screen and iterated on it. Changes made from inspection:

* the contrast defects in §4
* the desktop side panel covering the conversation
* the weak selected state on segmented controls
* signal tags removed from your own lines
* the destructive "Clear this call's data" styling
* sub-page scroll reset
* triple repetition of "X left the call"
* 44 px banner actions and suggestion dismiss

The "reconnecting" captures show the *peer-away* banner on the other side. On the guest itself, P2P carried the call through the dropped socket, so no reconnecting banner appeared; that path is asserted in `recovery.mjs` on relay-only.

## 8. Tools and skills actually used
* **design-superpowers `/design` skill:** maturity detection (L0, no prior DESIGN.md), then the proceed-with-provisional-values path. The resulting `DESIGN.md` is marked provisional.
* **Playwright + Chromium** (pre-installed): screenshots, the accessibility suite and all E2E runs. Rendered images were inspected directly.
* **Figma MCP: available but not used.** No Figma file was provided, and creating one would be an external write that wasn't approved. No Figma validation is claimed.
* Not used: superdesign (external service), axe-core (new dependency), real devices and screen readers.

## 9. Tests and regressions
| Suite | Result |
| --- | --- |
| `npm run test:all` (lint/tsc, 462 unit, 58 server, build) | Pass |
| smoke 17/17 · recovery 8/8 · compat 5/5 · conversation 8/8 · intelligence 7/7 · processing 7/7 · VoiceMirror 8/8 · a11y 9/9 | Pass |
| `REPRO_ASSERT=1 e2e/repro-loop.mjs` 10/10 | Pass |

**New tests:**

* `call/status.test.ts`: banner priority, hints, no repetition.
* `App.test.tsx`: invite link → join (F2), inline validation (F11), home navigation.
* `utils.test.ts`: class merging.
* `e2e/a11y.mjs`.

**Updated:** `CallRoom.test.tsx`, `JoinCall.test.tsx` and every browser suite (new selectors, same assertions on engine behaviour). One assertion was dropped: compat's "Batch transcribe" label, a diagnostics string removed under F4. That suite still asserts the server-STT call and the absence of hands-free.

**Simulated vs real:** all browser runs use desktop Chromium with fake media devices, a scripted Web Speech shim and stubbed providers. Phone results come from viewport, touch and mobile emulation. **No real-device, Safari/iOS, Firefox or screen-reader validation was performed.**

**Design decisions (resolved):**

* **End call asks first while the call is live** (connected, waiting or reconnecting): a small sheet, “End this call?”, with “Keep talking” focused by default. When the call has already ended or failed, End leaves immediately. Covered by `CallRoom.test.tsx` and `smoke.mjs`.
* **Hands-free stays in settings.** It is a deliberate mode switch, not a per-turn action. While it is on, the always-visible banner and the centre “Stop” control make it obvious and one tap to turn off, so a dock toggle would add clutter without adding safety.

## 10. Privacy, consent and processing semantics
Unchanged:

* The disclosure text is engine-supplied and rendered verbatim.
* Nothing is captured before "Allow microphone".
* The hands-free indicator is always visible while hands-free is on.
* Processing-mode enforcement is in the engine, and the UI only calls `setProcessingMode`.
* "On-device only" still shows typing when speech input is unavailable.
* VoiceMirror is off by default, with its own disclosure.
* No logging was added.

Copy changes touched only wording: "channel" → "call" in three engine notice strings, and the share text.

## 11. Limitations
* The light theme is checked at phone and desktop only; tablet uses the same rules.
* There is no real-device or screen-reader pass (see §9).
* `DESIGN.md` values are provisional (L0), and no Figma library exists.
* The feed is not virtualised. That is fine for call-length transcripts, but not for very long sessions.
* Banner priority shows one problem at a time by design. A lower-priority notice waits until the higher one clears.

## 12. Files
**New:**

* `DESIGN.md`
* `src/components/ui/*`
* `src/components/call/{ConversationFeed,TalkDock,SettingsSheet,status}.tsx/ts` (+ `status.test.ts`)
* `src/components/{Brand,LanguageField}.tsx`
* `src/{App,utils}.test.ts(x)`
* `e2e/{a11y,screens}.mjs`
* `docs/phase7/*`

**Rewritten:** `src/index.css`, `App.tsx`, `Home.tsx`, `StartCall.tsx` (presentation), `JoinCall.tsx` (presentation), `CallRoom.tsx`, `ConsentSheet.tsx`, `LanguagePicker.tsx`.

**Removed:** `AssistPanel.tsx`, `PrivacyPanel.tsx`.

**Small edits:** `utils.ts`, `engine/CommunicationSession.ts` (three user-facing strings), `e2e/harness.mjs` and every suite, `package.json` (`test:e2e` now includes a11y; `screens` script).

## 13. Rollback
The redesign is view-only, so any Phase 7 commit can be reverted without touching engine, server or data. For example, `git revert <phase-7 commits>`, or reset the UI to `8862e78` with `git checkout 8862e78 -- src/ e2e/ package.json`. Then rerun `npm run test:all` and the browser suites. There are no migrations, stored-data changes or flags to unwind.

## 14. Next conditions (before production)
1. Real-device pass: iOS Safari, Android Chrome, mid-range Android at 200 % text, with VoiceOver and TalkBack.
2. Firefox and Safari desktop runs of the browser suites.
3. Moderated usability check of the invite → join → first exchange journey, with two people on two phones.
4. Watch usability sessions for whether the End confirmation feels like friction, and whether people find hands-free in settings.
5. Optionally, a Figma library from `DESIGN.md`, if the team wants design-side ownership. That needs an approved Figma file.

**Engineering completion:** done for the scope above, with all automated checks green.
**Production readiness:** not established. It depends on items 1–3.
