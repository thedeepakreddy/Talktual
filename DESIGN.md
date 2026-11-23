---
name: Talktual
direction: Futuristic Audio — a precise, quiet listening instrument, in the Talktual brand (orange / charcoal / warm white)
version: 2.1.0
status: frozen for Phase 7 final (supersedes 1.0.0 "Quiet instrument"); governed by the Phase 7 Design Constitution
source: src/index.css (tokens), src/components/ui/* (primitives), src/components/shell/* (navigation), src/pages/* (destinations)
themes: [dark (showcase), light] — system by default, user-selectable (Settings › Appearance)
---

# Talktual design system

## Direction
Professional audio-instrument and communication design, not an AI product.
- **Technology shows through precision:** alignment, typography, honest state and the voice wave.
- **No ornament:** no AI iconography, gradients, glow, glass, card grids, pills or decorative motion.
- **Translation text is always the most prominent element on a session screen.**
- **Every visible state is true:**
  - unavailable capabilities (Lens, Rewind) say so plainly
  - the voice wave moves only with real recognizer events (or a real mic level)
  - nothing pulses or animates on its own

Concept exploration (A atmospheric / B instrument / C editorial) and the selection rationale are in `docs/phase7_final/PHASE7_FINAL_REPORT.md`. The selection is B's structure (metered activity trace with a time axis, speaker-label rows, borderless language pair) edited with C's typography and honest status copy.

## Tokens (`src/index.css`, exposed to Tailwind via `@theme inline`)
### Color — Talktual brand (brand sheet: `docs/brand/talktual-brand-sheet.png`)
Brand colours: Primary Orange `#F59A38` · Secondary Orange `#FFB45B` · Charcoal `#1F2328` · Warm White `#FAF7F2`. They supersede the constitution's provisional graphite and teal. The constitution's structure (three depth levels, one accent, semantic tokens) is unchanged.

| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `bg` | `#1F2328` (Charcoal) | `#FAF7F2` (Warm White) | base |
| `raised` | `#262B31` | `#FFFFFF` | surface level (sheets) |
| `surface` | `#2E343B` | `#F0ECE5` | elevated (controls, inputs) |
| `line` / `line-strong` | `#343B42` / `#4A525A` | `#E5DFD5` / `#C7BFB2` | dividers |
| `fg` | `#F2EEE7` | `#1F2328` | primary text |
| `fg-muted` | `#BDB6AC` | `#4A4F55` | supporting text |
| `fg-subtle` | `#A49D93` | `#62676D` | metadata |
| `accent` | `#F59A38` | `#F59A38` | **fills only**: primary buttons, switch on, selection tint |
| `on-accent` | `#1F2328` | `#1F2328` | text on orange (7.2:1) |
| `accent-text` | `#FFB45B` | `#A4560B`* | orange as text and icons |
| `warning` | `#E6C860` | `#6F5A00` | paused (kept distinct from the orange accent) |
| `danger` | `#F0766E` | `#B9423B` | end, errors, the listening (recording) dot |
| `success` | `#63C08F` | `#2E7D55` | connected |

\* Orange on Warm White measures 2.06:1, which fails WCAG for text (4.5:1) and for graphics (3:1). In the light theme, orange is therefore used as a fill (with charcoal text), and orange *text* uses `#A4560B`, the same hue darkened (≥ 4.56:1). Verified by `e2e/a11y.mjs`: the lowest text pair is 4.68:1 (dark) and 4.85:1 (light).

### Logo and icon
- `BrandMark` (`src/components/Brand.tsx`) is a vector rebuild of the sheet's mark: two mirrored waves split by a line, with a primary→secondary orange gradient. It has a `mono` variant (currentColor). The brand mark is the only place a gradient is used.
- `Wordmark`: the mark plus "Talktual" in bold, tracking −0.02em.
- **App icon:** `public/favicon.svg` (charcoal tile plus mark) and `public/apple-touch-icon.png` (180 px render). Theme colour `#1F2328`.

### Type (rem; follows the browser text size)
| Role | Size / line | Constitution range |
| --- | --- | --- |
| `title` (page) | 26 / 32 | 24–28 |
| `translation` (current line) | 24 / 32 | 20–26 |
| `heading` (section) | 19 / 26 | 17–20 |
| `body-lg` (transcript lines) | 18 / 26 | — |
| `body` | 15 / 22 | 15–16 |
| `label` (supporting) | 13 / 18 | 13–14 |
| `meta` | 12 / 16 | 12–13 |

The font is the brand stack: Inter / Manrope / SF Pro. They're used when installed and fall back to the system UI font. No web-font download (no third-party request). Tabular numerals for times and codes.

### Shape, depth, spacing, motion
- **Radius:** controls 10, inputs 10, small 8, surfaces 14, sheets 16. Circles only for the hold-to-talk control (ergonomic) and status dots. **No pills.**
- **Depth:** three levels — base, surface (sheets), elevated (controls). Shadow only on sheets. Content is separated by whitespace and hairline dividers, not cards.
- **Spacing:** a 4 px scale (4, 8, 12, 16, 24, 32, 48, 64). Gutters are 16 px on phones, 24 px on tablets and 40 px in desktop pages.
- **Motion:** `--duration-press` 120 ms, `--duration-state` 200 ms, `--duration-sheet` 280 ms; ease `cubic-bezier(.2,.8,.2,1)`. Motion follows real state:
  - the voice wave moves only while capturing, with real recognizer events
  - pause dims it
  - the sheet slides
  - there is no ambient animation
  - everything is disabled under `prefers-reduced-motion`

## Layout and navigation
- **Desktop (≥ 1024):** an 88 px rail on the left, with five primary destinations (Home, Connect, AIR, Lens, Rewind) and Voice and Settings in the rail footer. Content is a single column (max 720 px). Sheets open as a 400 px right panel and the session column moves aside.
- **Phone and tablet:** bottom navigation with the five primary destinations. Voice Studio and Settings are reached from Home (the list and the header icon).
- **Active sessions** (AIR, a call, invite and join) are full-screen with the navigation hidden, so a session can't be left by an accidental tab tap. Leaving AIR stops listening; leaving a live call asks first.
- A **"Skip to content"** link is the first focusable element on navigated pages.

## Components
- **Primitives (`src/components/ui`):** `Button` (primary · secondary · ghost · danger; heights 48/44), `IconButton` (44 px, labelled), `Sheet`, `Switch`, `Segmented`, `Row`/`Section`, `Banner`.
- **Shell (`src/components/shell`):** `AppShell`, `DesktopRail`, `MobileBottomNav`, `Page`.
- **AIR (`src/components/ambient`):** `AmbientScreen` with `LanguagePair`, the voice wave, the transcript rows, the dock, the AIR settings sheet and the test-mode strip.
- **Voice wave (`src/components/voice`):** `SujitWaveform` (owner-requested, replaces the activity trace). Seven sine curves share one envelope, each in its own plane around a slightly rising time axis, so they form a slowly turning 3D bundle. Strokes use `fg`: warm white with a soft glow on dark, charcoal ink without glow on light. Used in AIR (128 px) and in calls above the talk control (64 px).
  - **Honest input (`ActivityLevel`):** height comes only from real signals. The recognizer's "speech detected" holds a floor, each new interim word adds a bump that decays, and real loudness is used only where the app already holds a mic stream (the server-recognition fallback in calls). Nothing reported → one calm idle line, and the animation loop stops. Reduced motion: no turning or travel, height still follows the real level.
  - **Deviation:** the glow and the continuous motion while capturing are an owner-approved exception to "no glow, no decorative motion"; the motion is tied to capture and real events, never ambient.
- **Connect (`src/components/call`):** `ConversationFeed` (speaker-label rows), `TalkDock`, `SettingsSheet`; plus `StartCall` and `JoinCall`.
- **Pages (`src/pages`):** `HomePage`, `ConnectPage`, `LensPage`, `RewindPage`, `VoicePage`, `SettingsPage`.

## Copy
Plain verbs, sentence case, no exclamation marks, no AI vocabulary. Status says what is true ("Paused — not listening", "Not set up on this server", "Nothing is kept").
