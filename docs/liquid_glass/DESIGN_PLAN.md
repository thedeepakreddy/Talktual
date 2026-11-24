# Talktual — Liquid Glass redesign plan

Owner brief (2026-11-24): the whole UI redesigned to an ultra-premium, iOS 27-style **liquid glass** system in iPhone 17 Pro **Cosmic Orange**, with depth, rich typography, and premium Three.js motion. It must surpass the owner's reference (three dark/orange phone screens with a molten orb). Buttons must not copy the reference's flat orange buttons. The UI must hold up at every screen size, without AI iconography.

This brief **supersedes** the Phase 7 constitution's visual rules: "no glass, no gradients, no glow, no pills". Every rule about honest state, accessibility and privacy is kept.

## 1. Direction — "Molten glass"
- **Two materials:** dark, warm obsidian, and orange light moving underneath it.
  - Every surface is glass floating over a slow molten field, with depth through blur, refraction highlights and soft shadow.
  - Orange is *light*, not paint: it glows from below the glass, and pools at the top and bottom of the screen.
- **Two layers of motion:**
  - *Atmosphere:* the field drifts slowly. This is decorative, explicitly not tied to sound, and frozen under reduced motion.
  - *Voice:* the orb and the Sujit wave move **only** with real recognizer events or a real mic level. Nothing simulates listening.
- **What makes it better than the reference:**
  - real 3D materials (fresnel, internal molten flow, specular)
  - layered glass with specular rims
  - an editorial serif accent
  - motion that reacts to the actual voice
  - a daylight theme of equal quality

## 2. Palette — Cosmic Orange
| Token | Dark (Obsidian) | Light (Daylight) | Use |
| --- | --- | --- | --- |
| `bg` | `#0B0705` | `#FBF5EF` | base under the molten field |
| `raised` / `surface` | `#1A120D` / `#221812` | `#FFFFFF` / `#F3EBE3` | effective colour of strong / control glass (contrast-checked) |
| `fg` / `fg-muted` / `fg-subtle` | `#FFF7F0` / `#D7C8BB` / `#A89889` | `#1B120C` / `#574A40` / `#6E5F53` | text |
| `accent` | `#FF7A1A` | `#FF7A1A` | Cosmic Orange, for fills and light |
| `accent-text` | `#FFA564` | `#B2450A` | orange as text |
| `on-accent` | `#1A0A02` | `#1A0A02` | ink on molten (≥ 7:1) |
| ember ramp | `#FFB070 → #FF9F4A → #FF7A1A → #E2560C → #7A2205` | same | gradients, field, orb |

**Glass tokens** (translucent, for fills only):
- `--glass`, `--glass-strong`: body tint
- `--glass-stroke`: hairline
- `--glass-hi`: top specular
- `--glass-lo`: bottom shade
- `--glass-drop`: elevation

## 3. Typography (round 2: Geist replaced Inter + Instrument Serif — see §9)
- **Inter Variable** (self-hosted, with the `opsz` axis): optical sizing gives the display cut at large sizes and the text cut at small ones. Stylistic set `ss03` for refined quotes, tabular numerals for times and codes.
- **Instrument Serif Italic** (self-hosted): an accent for *one or two words* in hero headlines only.
- **No third-party font requests:** files ship in the bundle.

**Scale** (rem, follows the user's text size):

| Style | Size / line height | Tracking | Notes |
| --- | --- | --- | --- |
| display | 2.75 / 1.02 (phone), 3.75 (lg) | −0.04em | |
| title | 1.875 / 1.1 | −0.03em | |
| translation | 1.625 / 1.25 | −0.015em | |
| heading | 1.1875 | −0.01em | |
| body-lg | 1.0625 | | iOS body |
| body | 0.9375 | | |
| label | 0.8125 | | |
| meta | 0.75 | | |
| eyebrow | 0.6875 | +0.14em | uppercase |

## 4. Shape, depth, light
- **Radii:** capsules for buttons, segmented controls and tab bars; cards 28; sheets 34; inputs 18; squircle smoothing via `corner-shape` where supported.
- **Depth:** z0 molten field → z1 glass panels (blur 28, saturate 180%) → z2 controls (stronger rim) → z3 sheets and tab bar (strongest blur, drop shadow).
- **Glass recipe:** a translucent tint, a specular gradient rim (bright top-left, warm bottom-right), an inner top highlight, an inner bottom shade, and a soft long drop.
- **Molten button (primary):**
  - three-layer gradient: specular bloom on top, Cosmic body, deep ember base
  - a glossy liquid cap (inner highlight)
  - a pointer-following sheen
  - an outer orange light that grows on hover and press
  - dark ink label
  - presses with a short spring
- **Glass button (secondary):** a clear glass capsule. **Ghost:** text, with glass on hover. **Danger:** glass tinted red.

## 5. Three.js scenes (lazy-loaded; WebGL fallback is CSS)
1. **LiquidField:** a full-screen backdrop shader.
   - Domain-warped fbm molten flow on an ember ramp, glow pools (top and bottom, per route), vignette and fine grain.
   - Rendered at reduced resolution, capped at 30 fps, paused when hidden, a single frame under reduced motion.
   - Moods: `rich` (Home), `session` (AIR and calls: dark centre, light from below), `calm` (settings pages).
2. **VoiceOrb:** a molten glass sphere.
   - Fresnel rim, internal flowing ember, specular highlight, a self-shadowed side.
   - Displacement and glow come **only** from the real voice level.
   - Slow internal flow only while actually capturing; frozen otherwise.
   - Used in the AIR stage, crossed by the Sujit wave, and as the Home hero, where it is idle and labelled "Not listening".

## 6. Components
- **Shell, desktop:** a floating glass rail (inset 12, radius 28) with the brand mark, 5 destinations, and Voice and Settings at the foot.
- **Shell, phone and tablet:** a floating glass tab bar (a capsule above the safe area) with a sliding glass "lens" under the active tab.
- **Page header:** display title; optional serif accent; glass icon buttons.
- **Sheets:**
  - phones: a liquid glass bottom sheet with a grabber
  - desktop: a floating right panel (inset 12, radius 28)
- **Rows and sections:** inset-grouped glass cards (iOS settings language), with section eyebrows.
- **Switch:** a glass track with a molten fill when on, and a glass knob with a specular highlight.
- **Segmented:** a glass capsule with a sliding lens.
- **Banner:** a glass card with a tinted rim and an icon in a tinted squircle.
- **Mic button:** a glass ring around a molten orb (a real 3D-looking sphere in CSS), with a stronger rim light while live (state, not level).
- **Icons:** lucide line icons only, a consistent 1.75 stroke, in glass squircles where they introduce a feature. No sparkles or "AI" glyphs.

## 7. Screens
| Screen | Plan |
| --- | --- |
| Home | Wordmark; display headline with a serif accent; the idle orb; glass language-pair capsule; real status; molten "Start listening"; a "More ways to talk" rail of glass cards (Connect, Voice Studio, Lens, Rewind) |
| AIR | Glass header (back, title, status, settings); language-pair capsule; stage with orb and Sujit wave; transcript: the latest line large on glass, earlier lines quieter; floating glass dock (Speech, molten Start/Pause, Stop) |
| Connect | Glass entry card (name, language); molten Start, glass Join; invite with a large tabular code, a white QR tile, share and copy; join with a code field |
| Call | Glass header (peer, languages, timer, settings, red-glass End); feed: own lines as molten-tinted glass bubbles on the right, peer lines with large type on the left; wave above the dock; glass dock with the mic orb |
| Voice Studio, Settings | Inset-grouped glass cards; same content and controls |
| Lens, Rewind | Honest "not available" glass hero cards, routed to the real features |

## 8. Guarantees
- Accessibility:
  - token contrast stays tested (a11y suite)
  - 44 px targets
  - focus rings on glass
  - 200 % text with no horizontal scroll
  - reduced motion respected (the field freezes; voice visuals keep following the level without travel)
- Breakpoints: verified at 320, 390, 430 (phone), 768, 1024 (tablet) and 1440 (desktop), in both themes.
- Performance: Three.js loads after first paint; one renderer per scene; device pixel ratio ≤ 1.5 for the field and ≤ 2 for the orb; loops stop when hidden or idle.
- Honesty: no fake listening; Lens and Rewind remain truthful placeholders.

## 9. Owner round 2 (same day) — applied
| Request | Decision |
| --- | --- |
| "Use the Sujit wave; next level; no ball behind it; wave in the middle; extra waves; bigger in the middle" | The AIR stage is now **wave only**: centred, full width, 200–340 px. It has 7 lead strands plus 14 filaments; a centre-peaked envelope; a perceptual height curve, so ordinary speech swells the middle; and a white-hot lead strand with an orange glow and tapered ends. The orb is no longer in AIR; it remains only as the Home hero. |
| "Different rich button designs across the app" | A deliberately varied family: Molten with an ink icon chip and a trailing arrow chip (main actions), Prism glass (secondary), Ember line (tertiary), Ghost, Danger glass. A glass-ring molten orb for the mic. |
| "Icons ultra rich minimal; redesign; no AI style" | A custom 34-icon set (`src/components/icons.tsx`): hairline 1.6 stroke, round terminals, a faint duotone body. The generic library was removed from every screen. |
| "Top minimal premium font that fits" | **Geist** for everything, **Geist Mono** for codes, self-hosted. Accent words use the same family at weight 300 with the ember gradient, so there is no second typeface. |
