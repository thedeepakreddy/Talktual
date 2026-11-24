---
name: Talktual
direction: Molten glass — iOS 27-style liquid glass over a slow molten field, in iPhone 17 Pro Cosmic Orange
version: 3.0.0
status: current (supersedes 2.1.0 "Futuristic Audio"); owner brief of 2026-11-24 overrides the Phase 7 constitution's "no glass / gradients / glow / pills" rules
source: src/index.css (tokens + materials), src/components/ui/* (primitives), src/components/icons.tsx (icon set), src/three/* (Three.js scenes), src/components/voice/* (Sujit wave)
themes: [dark (Obsidian, showcase), light (Daylight)] — system by default, user-selectable (Settings › Appearance)
---

# Talktual design system

Plan and rationale: `docs/liquid_glass/DESIGN_PLAN.md`. Renders: `docs/liquid_glass/after/`.

## Principles kept from the constitution
- **Every visible state is true.**
  - The voice wave moves only with real recognizer events (or a real mic level).
  - The molten field is decoration and never implies listening.
  - Lens and Rewind say plainly what doesn't exist.
- **Translation text is the most prominent element on a session screen.**
- **Accessibility:**
  - contrast-tested tokens in both themes
  - 44 px targets
  - visible focus
  - 200 % text without horizontal scroll
  - reduced motion honoured

## Colour — Cosmic Orange
| Token | Dark (Obsidian) | Light (Daylight) | Use |
| --- | --- | --- | --- |
| `bg` | `#0B0705` | `#FBF5EF` | base under the field |
| `raised` / `surface` / `selected` | `#1A120D` / `#221812` / `#3A2A1F` | `#FFFFFF` / `#F3EBE3` / `#FFFFFF` | the *effective* colour of glass over the base (contrast checks, fallbacks) |
| `fg` / `fg-muted` / `fg-subtle` | `#FFF7F0` / `#D7C8BB` / `#A89889` | `#1B120C` / `#574A40` / `#6E5F53` | text |
| `accent` | `#FF7A1A` | `#FF7A1A` | Cosmic Orange: fills and light |
| `accent-text` | `#FFA564` | `#B2450A` | orange as text |
| `on-accent` | `#1A0A02` | `#1A0A02` | ink on molten |
| `success` / `warning` / `danger` | `#5AD08F` / `#FFD36B` / `#FF7B6E` | `#23784B` / `#6E5500` / `#B93A30` | states |

Ember ramp for gradients, the field and the orb: `#FFB070 → #FF9F4A → #FF7A1A → #E2560C → #7A2205`.

The glass tokens are translucent and used only as fills:
- **`--glass` / `--glass-top`:** body tint. Dark-theme glass carries a warm dark tint, so it stays neutral over bright light instead of turning red.
- **`--glass-strong(-top)`:** text-bearing panels.
- **`--glass-stroke`:** hairline.
- **`--glass-rim` / `--glass-rim-warm`:** the specular rim.
- **`--glass-hi` / `--glass-lo`:** inner highlight and shade.
- **`--glass-drop`:** elevation.

Lowest text contrast (a11y suite): 6.23:1 in dark and 5.18:1 in light.

## Typography — Geist
- **Families:**
  - **Geist** (variable, self-hosted) for everything
  - **Geist Mono** for call codes
  - no third-party font requests
- **Styles:**
  - display: weight 600, tracking −0.048em
  - title: 600, −0.034em
  - eyebrow: 11 px, +0.14em, uppercase
  - **Accent words** (e.g. "around you"): the same family at weight 300, lit with the ember gradient. No second typeface.

| Role | Size / line (rem) |
| --- | --- |
| display / display-lg | 2.75 / 2.85 · 3.875 / 3.9 |
| title | 1.875 / 2.125 |
| translation (current line) | 1.625 / 2.05 |
| heading | 1.1875 / 1.5 |
| body-lg (rows, transcript) | 1.0625 / 1.6 |
| body | 0.9375 / 1.4 |
| label · meta · eyebrow | 0.8125 · 0.75 · 0.6875 |

Speaker names stay in natural case (never uppercased).

## Materials (`src/index.css`, `@layer components`)
- **`.glass` / `.glass-strong` / `.glass-clear`:** tint, plus backdrop blur 28–39 px with saturate 1.35–1.4, plus a gradient specular rim (`::before` mask), an inner top highlight, an inner bottom shade and a long soft drop (`glass-clear` has no drop).
- **`.molten`:** specular bloom, Cosmic body, deep ember base, a glossy liquid cap, a pointer-following sheen (`--mx`/`--my`), orange light underneath and a spring press.
- **`.orb-core`:** a lit molten sphere in pure CSS. Used for the mic control, avatars, list bullets and the language-pair arrow.
- **`.prism`:** the secondary button's specular top edge, with a warm rim on hover.
- **`.ember-line`:** a gradient outline with ember text.
- **`.btn-chip`:** an icon in a dark ink lens inside molten buttons.

## Buttons (one family, deliberately varied)
| Variant | Look | Used for |
| --- | --- | --- |
| `primary` | Molten; leading icon in an ink chip, optional trailing arrow chip | the one main action per screen: Start listening, Start a call, Join, Pause/Start, Share link |
| `secondary` | Prism glass (dark glass, specular edge) | Copy code, Not now |
| `tertiary` | Ember line | Join with a code, Open Connect, Privacy settings, Play sample, Forget, Scan a QR code |
| `ghost` | Text; glass on hover | low-emphasis actions |
| `danger` | Red-tinted glass | End call |
| Icon button | 44 px glass disc (`default`), `active` ember-tinted, `danger`, `plain` | headers, docks |
| Mic control | Glass ring around a molten orb; a stronger rim and glow while live (state, not level) | push to talk, hands-free stop |

## Icons (`src/components/icons.tsx`)
A custom set drawn for Talktual:
- **Geometry:** a 24-unit grid, a 1.6 hairline stroke and round terminals.
- **Duotone:** a faint body (16 % `currentColor`), like hierarchical symbols.
- **Signature glyphs:**
  - AIR: a listener with the room's sound arriving from both sides
  - Connect: two voices in conversation
  - Lens: a viewfinder over text
  - Rewind: time turning back
  - Voice: a voice print
  - Settings: an eight-tooth gear
  - End call: the hang-up handset
- **Rules:**
  - no sparkles, robots or "AI" glyphs
  - icons are `aria-hidden` unless given a label

## Motion and Three.js (`src/three/*`)
### LiquidField
A full-screen shader: low-frequency domain-warped flow, with light pooling at the edges, sized against the short side so phones and desktops match. The centre stays obsidian.

**Moods:**
- `rich` (Home)
- `session` (AIR and calls)
- `calm` (pages)

**Behaviour and performance:**
- mounted once for the app's lifetime (one WebGL context)
- initialised at idle
- half resolution, 30 fps cap
- paused when hidden
- a still frame under reduced motion
- CSS gradient fallback

### VoiceOrb
A molten glass sphere (fresnel, specular, internal flow) used as the **Home hero**. There, it drifts slowly and decoratively and is labelled by the page as not listening.

### Sujit wave (`src/components/voice/SujitWaveform.tsx`)
The voice visual in AIR (a full-width centred stage, 200–340 px, then 136 px once the transcript fills) and in calls (above the dock).

**Strands:**
- seven lead strands plus fourteen filaments
- one envelope that peaks in the middle
- each strand in its own plane around a level axis, so the bundle turns in depth
- the lead strand is white-hot with an orange glow; the ends taper
- dark theme: additive light; light theme: ink with an ember lead

**Driven by `ActivityLevel` only:**
- the recognizer's speech floor
- decaying bumps per interim word
- real mic loudness where a stream exists
- height uses a perceptual curve, so ordinary speech swells the middle

With no level, it shows one calm tapered line with a warm centre and the loop stops.

### UI motion
- spring presses (`--ease-spring`)
- glass "lens" indicators that slide (tab bar, rail, segmented)
- sheets spring up without scaling, so targets are always full size

## Layout and navigation
- **Desktop (≥ 1024):** a floating glass rail (inset 12, radius 30) with the brand mark, five destinations, and Voice and Settings at the foot. Sheets are a floating right panel (420 px, inset 12, radius 30), and the session column moves aside.
- **Phone and tablet:** a floating glass tab bar (capsule, above the safe area) with a sliding lens under the active tab. Sheets are liquid-glass bottom sheets with a grabber.
- **Active sessions** (AIR, a call, invite and join) are full-screen with the navigation hidden.
- **The call header** is a glass capsule with a molten avatar. Under large text or narrow widths (container query in rem), End collapses to its icon, keeping the accessible name "End call".
- **Verified sizes:** 320, 390, 834 and 1440 px wide, in both themes, with no horizontal scroll.

## Copy
Plain verbs, sentence case, no exclamation marks, no AI vocabulary. Status says what is true.
