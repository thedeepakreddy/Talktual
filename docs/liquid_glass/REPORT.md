# Liquid Glass redesign — report

**Brief (owner, 2026-11-24):** an ultra-premium iOS 27-style liquid glass UI in iPhone 17 Pro Cosmic Orange, with depth, rich typography and premium Three.js motion. It should be better than the supplied reference, with no AI-looking icons, and work at every screen size.

**Round 2, same day:**
- the Sujit wave as the voice visual, larger in the middle, with more strands and no ball behind it
- varied rich buttons
- custom icons
- a top minimal font

Plan: `DESIGN_PLAN.md`. The system as built: `/DESIGN.md` v3.0.0. Renders: `after/`, covering 20 states × phone/tablet/desktop × dark/light, in JPEG.

## What changed
| Area | Result |
| --- | --- |
| Tokens | Cosmic Orange palette in two themes (Obsidian, Daylight); glass layer tokens; the effective surface colours stay contrast-tested |
| Materials | Liquid glass (blur, specular rim, inner highlight/shade, drop); molten primary; prism glass; ember line; molten CSS sphere |
| Type | Geist and Geist Mono, self-hosted; display tracking −0.048em; accent words at weight 300 with the ember gradient |
| Icons | Custom 34-icon set (`src/components/icons.tsx`): 1.6 hairline, round terminals, duotone body; the generic icon library is no longer used |
| Three.js | `LiquidField` (molten backdrop: idle init, one context, 30 fps, half resolution, reduced-motion still, CSS fallback) and `VoiceOrb` (Home hero) |
| Voice | Sujit wave: 7 leads plus 14 filaments, centre-peaked envelope, white-hot lead with orange glow, tapered ends; real-signal driven only |
| Shell | Floating glass rail (desktop) and glass tab bar with a sliding lens (phone and tablet); floating glass sheets |
| Screens | Home, AIR (all states), Connect, Invite, Join, Call, Voice Studio, Settings, Lens and Rewind, every panel and sheet |

## Issues found and fixed during visual QA
1. **Field too busy:** the field read as smoke and flooded the screen. It became a silky low-frequency flow, with light pooling at the edges sized against the short side.
2. **Mottled orb:** made smooth and glossy, with inner scattering and two speculars.
3. **Glass turning red:** clear glass over hot light turned red (the latest AIR line, Settings rows, Copy code). Fixed with a warm dark tint in dark glass, lower saturation, and strong glass for text-bearing panels.
4. **Squircle corners:** `corner-shape` squared off round buttons, so it was removed.
5. **Join card:** it stretched to full height; it now hugs its content. Invite and Join are centred on desktop.
6. **AIR stage:** it collapsed while only interim text existed; it now stays large until translated lines exist.
7. **Clipped glow:** the idle wave's centre glow was clipped into a rectangle; it is now an ellipse that fits the canvas.
8. **Uppercase names:** speaker names were uppercased by the eyebrow style; they now use natural case.
9. **200 % text:** the call header scrolled sideways at 200 % text. A rem container query now collapses End to its icon, keeping the name "End call".
10. **Sheet scale animation:** the sheet's scale-in animation made targets measure 43 px mid-animation; it now springs without scaling.
11. **Field remounting (real performance defect):** the molten field remounted on every page↔session change, creating a new WebGL context and recompiling shaders, which delayed the first focus. It is now mounted once and initialised at idle.

## Verification
| Check | Result | Kind |
| --- | --- | --- |
| Typecheck, 537 unit tests, 60 server tests, build | pass | verified |
| Browser suites:<br>• smoke, recovery, compatibility, conversation, intelligence, processing<br>• accessibility (contrast, 44 px, keyboard, 200 % text, reduced motion)<br>• AIR, validation, voice wave, reliability loop | see the final run in the commit message | simulated (scripted recognizer, fake voice, stubbed providers) in real Chromium |
| Visual review | ~140 renders reviewed: phone 390, small phone 320, tablet 834, desktop 1440, dark and light; no horizontal overflow or console errors | Chromium, software WebGL |
| Safari / iOS, Android Chrome, real GPUs, VoiceOver | **not run** | **unverified**: backdrop blur, WebGL performance and battery on real phones need a device pass |

## Honest-state guarantees kept
- **The molten field:** decorative; never tied to sound.
- **The Home orb:** drifts slowly, decoratively, on a page that never claims to listen.
- **The Sujit wave** moves only with:
  - recognizer "speech detected"
  - new interim words
  - real mic loudness where the call already holds a stream

  With no signal it is one calm tapered line, and its loop stops.
- **Lens and Rewind** remain truthful placeholders.

## Known limitations
- WebGL and backdrop blur cost GPU time. Frame caps, idle init, pause-when-hidden and reduced motion help, but the battery impact on real phones is unmeasured.
- Geist has no Arabic, Hebrew, Devanagari or CJK glyphs, so those scripts use the system font, as before.
- The tablet and desktop session screens use a centred column; this is intentional for readability.

## Round 3 — the Liquid Orb (owner reference: luminous glass sphere with an inner ribbon)
| Request | Done |
| --- | --- |
| The reference orb's light, quality, reflection and style, in liquid glass, in UI colours | `LiquidOrb`: milky self-lit glass, orange/ember pools, a coloured limb, a crisp luminous edge, a prismatic fringe, speculars, an inner S-ribbon with ember edges, and fine lens streaks |
| "Luminescence like the reference, not a glowing sun" | The corona was removed: the light comes from inside the glass, with only a tight bloom that follows the silhouette |
| Reacts to sound; physics | Ripples, jelly squash and inner brightness follow the real level; spring–damper morph and squash; hysteresis |
| Orb becomes the wave (crazy animation) and returns | The sphere flattens into a lens, then a line; the ribbon unfurls into the Sujit wave with 13 strands fanning out in 3D; a mid-transition swirl and ripple; it springs back after silence |
| No Three.js or animation bugs | GPU contexts released on unmount; context-loss fallback; the browser test fails on any page error, console error or WebGL/Three.js warning; deterministic fake-clock frame captures checked |

Evidence:
- `docs/voice_wave/orb-to-wave-sequence-dark.jpg`: the orb → lens → wave → lens → orb sequence, captured with a fake clock
- `docs/voice_wave/orb-and-wave-dark-light.jpg`: the orb and the wave in both themes

The Liquid Orb replaced the earlier molten `VoiceOrb` on Home.

## Round 4 — the logo's wave as the voice visual (replaces the orb)
| Request | Done |
| --- | --- |
| "Crazy animation from the app logo's wave", Liquid Glass, premium | `LogoWave`: the mark's two waves as orange liquid-glass slabs (inner light, cap, crest line, caustics) that unfold into a full-width living waveform with the same crest DNA, and fold back |
| RGB colour reflection | Iridescent thin-film rims; an RGB-split sweeping reflection; three lagging colour copies of each edge, so dispersion appears with motion |
| Next-level physics | Elastic-string edges (128 nodes, 240 Hz), tension on the deviation from the target shape (holds the logo exactly at rest), splashes on voice jumps, spring morph with overshoot, 3D swirl mid-transition |
| More orange smoke in the other corners | Ridged-noise smoke tendrils rising from top-left and bottom-right |
| No Three.js / animation bugs | Found and fixed during QA: string tension flattening the logo, glow stubs on near-zero slabs, and a `Vector4.distanceTo` runtime error in the field (would have thrown every frame). The browser test now covers Home as well as AIR and fails on any error or WebGL warning |

Evidence: `docs/voice_wave/logo-to-wave-sequence-dark.jpg` (fake-clock frames) and `logo-and-wave-dark-light.jpg`.

## Round 5 — Home: "Conversation" choreography
The Home mark now acts out what Talktual does on an 11 s loop (decorative; Home never listens):
1. The top wave is plucked at its big crest: it speaks.
2. The bottom wave answers from its big trough.
3. Both answer once more.
4. Together they flow into the full living waveform (3D swirl, lit split line, faster RGB sweep).
5. They fold back into the logo with a spring overshoot and rest.

**Touch:**
- The glass tilts toward the pointer (spring-damped).
- Brushing across a wave plucks it in the direction of the motion.
- A tap or click splashes both waves.
- Reduced motion turns all of this off.

Evidence: `docs/voice_wave/home-conversation-sequence.jpg`.
