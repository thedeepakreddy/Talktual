---
name: Talktual
direction: Futuristic Audio — a precise, quiet listening instrument
version: 2.0.0
status: frozen for Phase 7 final (supersedes 1.0.0 "Quiet instrument"); governed by the Phase 7 Design Constitution
source: src/index.css (tokens), src/components/ui/* (primitives), src/components/shell/* (navigation), src/pages/* (destinations)
themes: [dark (showcase), light] — system by default, user-selectable (Settings › Appearance)
---

# Talktual design system

## Direction
Professional audio-instrument and communication design, not an AI product.
- **Technology shows through precision:** alignment, typography, honest state and the activity trace.
- **No ornament:** no AI iconography, gradients, glow, glass, card grids, pills or decorative motion.
- **Translation text is always the most prominent element on a session screen.**
- **Every visible state is true:**
  - unavailable capabilities (Lens, Rewind) say so plainly
  - the listening trace shows only real recognizer events
  - nothing pulses or animates on its own

Concept exploration (A atmospheric / B instrument / C editorial) and the selection rationale are in `docs/phase7_final/PHASE7_FINAL_REPORT.md`. The selection is B's structure (metered activity trace with a time axis, speaker-label rows, borderless language pair) edited with C's typography and honest status copy.

## Tokens (`src/index.css`, exposed to Tailwind via `@theme inline`)
### Color
| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `bg` | `#101317` | `#F3F4F3` | base |
| `raised` | `#191D22` | `#FFFFFF` | surface level (sheets) |
| `surface` | `#22272D` | `#E8EBEC` | elevated level (controls, inputs) |
| `line` / `line-strong` | `#262B31` / `#363C43` | `#DCE0E2` / `#C5CBCF` | dividers, control borders, trace baseline |
| `fg` | `#D8E0E5` | `#20262D` | primary text |
| `fg-muted` | `#A4ADB5` | `#4C545C` | supporting text |
| `fg-subtle` | `#89939D` | `#5F676F`* | metadata |
| `accent` | `#4EADB0` | `#2C7479`* | primary action, selection, live activity only |
| `on-accent` | `#0B1F20` | `#FFFFFF` | text on accent |
| `warning` | `#D9A441` | `#8A5A00` | paused |
| `danger` | `#E5706A` | `#B9423B` | end, errors, the listening (recording) dot |
| `success` | `#5CB98A` | `#2E7D55` | connected |

\* **Deviation from the constitution's proposed light values,** made after the contrast validation it requires. Light secondary `#6B737B` measured 4.36:1 on base and 4.02:1 on elevated. Light accent `#327F84` measured 4.23:1 on base. Both fail WCAG AA for text. Replaced with `#5F676F` (≥ 4.79:1) and `#2C7479` (≥ 4.52:1; white on it ≥ 5:1). All dark values pass as proposed.

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

The font is the system UI stack (no web font). Tabular numerals for times and codes.

### Shape, depth, spacing, motion
- **Radius:** controls 10, inputs 10, small 8, surfaces 14, sheets 16. Circles only for the hold-to-talk control (ergonomic) and status dots. **No pills.**
- **Depth:** three levels — base, surface (sheets), elevated (controls). Shadow only on sheets. Content is separated by whitespace and hairline dividers, not cards.
- **Spacing:** a 4 px scale (4, 8, 12, 16, 24, 32, 48, 64). Gutters are 16 px on phones, 24 px on tablets and 40 px in desktop pages.
- **Motion:** `--duration-press` 120 ms, `--duration-state` 200 ms, `--duration-sheet` 280 ms; ease `cubic-bezier(.2,.8,.2,1)`. Motion follows real state:
  - the trace advances only while capturing
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
- **AIR (`src/components/ambient`):** `AmbientScreen` with `LanguagePair`, `ActivityTrace` (listening canvas), the transcript rows, the dock, the AIR settings sheet and the test-mode strip.
- **Connect (`src/components/call`):** `ConversationFeed` (speaker-label rows), `TalkDock`, `SettingsSheet`; plus `StartCall` and `JoinCall`.
- **Pages (`src/pages`):** `HomePage`, `ConnectPage`, `LensPage`, `RewindPage`, `VoicePage`, `SettingsPage`.

## Copy
Plain verbs, sentence case, no exclamation marks, no AI vocabulary. Status says what is true ("Paused — not listening", "Not set up on this server", "Nothing is kept").
