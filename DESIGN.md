---
name: Talktual
direction: Quiet instrument
version: 1.0.0
status: provisional (L0 — first documented system; values chosen in Phase 7, see docs/phase7)
source: src/index.css (tokens), src/components/ui/* (primitives)
themes: [dark (default when the OS prefers dark), light]
---

# Talktual design system

## Direction — "Quiet instrument"
A two-person translated call is a conversation, not a dashboard. The interface stays quiet so the words carry it:

* **The conversation is the screen.** Every other element is chrome around it.
* **One loud thing:** the talk control. The signal accent belongs to talking and the single primary action on a screen, nowhere else.
* **Status in one place.** A single status line (live region) under the conversation, plus at most one banner for problems. No chip rows.
* **Settings live in one sheet**, grouped by what the user is deciding (how I talk, how it's processed, assistance), never scattered as pills.
* **No ornament.** No gradients, glows, textures, decorative cards, or icon tiles. Depth comes only from surface steps and a hairline.
* **Honest states.** Unavailable means hidden or explicitly "unavailable". Never a working-looking control that does nothing.

## Tokens (CSS custom properties in `src/index.css`, exposed to Tailwind via `@theme`)
### Color (semantic)
| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `--color-bg` | `#0E0F11` | `#F6F6F4` | page |
| `--color-raised` | `#16181B` | `#FFFFFF` | sheets, panels |
| `--color-surface` | `#1D2024` | `#EDEDEA` | inputs, controls, own messages |
| `--color-surface-hover` | `#252930` | `#E4E4E0` | hover/pressed |
| `--color-line` | `#2B2F35` | `#DADAD5` | hairlines, dividers |
| `--color-line-strong` | `#3B4048` | `#C4C4BE` | control borders |
| `--color-fg` | `#ECEDEE` | `#141517` | primary text |
| `--color-fg-muted` | `#A3A9B1` | `#555B63` | secondary text (≥ 7:1 / ≥ 6:1) |
| `--color-fg-subtle` | `#80878F` | `#6A7078` | metadata only (≥ 4.5:1) |
| `--color-accent` | `#F5A524` | `#F5A524` | talk (live), primary action fill |
| `--color-on-accent` | `#1C1305` | `#1C1305` | text/icons on accent |
| `--color-accent-text` | `#F7B24A` | `#8F5300` | accent as text |
| `--color-incoming` | `#86BFFF` | `#1F64C0` | the other person speaking / playback |
| `--color-success` | `#4CD488` | `#1B7F47` | connected dot |
| `--color-danger` | `#FF6B6E` | `#C42F35` | end call, errors |
| `--color-focus` | `#86BFFF` | `#1F64C0` | focus ring (2 px + 2 px offset) |

Soft tints (`--color-accent-soft`, `--color-incoming-soft`, `--color-danger-soft`) are the same hues at 12–16 % alpha, used only behind their own color.

### Type
System UI stack (`-apple-system, "Segoe UI", Roboto, system-ui, sans-serif`). No web-font request. Tabular numerals for codes and timers; monospace only for the 6-character call code.

| Role | Size / line | Weight |
| --- | --- | --- |
| `title` | 24 / 30 | 600 |
| `heading` | 17 / 24 | 600 |
| `body-lg` (conversation) | 17 / 25 | 400 |
| `body` | 15 / 22 | 400 |
| `label` | 13 / 18 | 500 |
| `meta` | 12 / 16 | 500 |

Rem-based, so text scales with the user's browser setting. Letter-spacing −0.01em on title only. No all-caps text.

### Space, radius, elevation, motion
* **Space:** 4 px grid (4, 8, 12, 16, 20, 24, 32, 40, 48). Side gutter 16 px on phones, 24 px on tablet and up.
* **Radius:** `--radius-sm` 8, `--radius-md` 12, `--radius-lg` 16 (sheets top corners 20), full for round controls.
* **Elevation:** level 0 (page), level 1 (`raised` + hairline), level 2 (sheet: `raised` + shadow `0 -8px 32px rgb(0 0 0 / .35)`). Nothing else casts a shadow.
* **Motion:** 120 ms (press), 200 ms (state), 280 ms (sheet), ease `cubic-bezier(.2,.8,.2,1)`. Every animation is disabled under `prefers-reduced-motion` (`MotionConfig reducedMotion="user"` plus CSS).

### Layout and breakpoints
* **Phone (< 640):** single column. The call is a header, the conversation (scrolls) and the dock.
* **Tablet (640–1023):** same structure, content column max 640, centered.
* **Desktop (≥ 1024):** conversation column max 720. Sheets open as a right-side panel (400 px) instead of a bottom sheet, so the conversation stays visible.
* **Touch targets:** ≥ 44 × 44 px (the talk control is 96 px).

## Components (`src/components/ui`)
* **Button** (`primary` accent fill · `secondary` surface · `ghost` text · `danger`), heights 48 (default) and 40 (compact). One primary per screen.
* **IconButton**: 44 px, requires `label` (becomes `aria-label`).
* **Sheet**: dialog with title, close, Escape, focus moves in on open and returns on close; bottom sheet on phones, side panel on desktop.
* **Switch**: `role="switch"`, label left, optional one-line description.
* **Segmented**: radiogroup for 2–3 mutually exclusive choices.
* **Row**: list row (label, optional value, chevron) for navigation inside sheets.
* **Banner**: one-line problem/notice (`info` | `warning` | `error`), optional action.

## Copy
Plain verbs, sentence case, no exclamation marks. Labels say what happens ("Start a call", "Join", "End"), status says what is true ("Luis is speaking", "Reconnecting…"). No marketing lines, no restating what the UI already shows.
