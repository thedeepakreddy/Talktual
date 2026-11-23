# Phase 8.5 — Battery, thermal and resource protocol (test I)

**Not executed.** This needs a physical phone. Do not infer battery attribution or precise thermal values from this document; record only what the device reports and what the tester observes.

## Protocol (20–30 minutes, controlled)
1. **Prepare:**
   - charge to at least 80 %, then unplug 10 minutes before starting
   - close other apps; turn off Background App Refresh for the session if the tester agrees
   - keep brightness fixed (for example 50 %, auto-brightness off); Low Power Mode **off**
   - keep the same network throughout
2. **Baseline (10 min):** Talktual open on Home, screen on, no listening. Record battery % at start and end.
3. **Session (20–30 min):**
   - open `?validate=1`, then Live translate, then Start
   - speech source: a looped consented recording or live speakers, about 50 % talk time
   - Speak translations **on**; headphones per configuration (AirPods for A, speaker for C)
4. **Every 5 minutes, record:**
   - battery %
   - perceived heat (cool / warm / hot to the touch), and whether iOS showed any temperature warning
   - whether listening is still active (header shows Listening)
   - any interruptions
5. **End:** press *Export* (test mode). Note the iOS Settings › Battery per-app % for Safari over the last hour, as reported by iOS. This is **iOS's attribution, not ours**.

## Data template
| Field | Value |
| --- | --- |
| Device model / OS / browser | |
| AirPods model / firmware (if used) | |
| Network | |
| Ambient conditions (room temperature if known; indoor or outdoor) | |
| Brightness / Low Power Mode | |
| Baseline: battery % start → end (10 min) | |
| Session duration (min) | |
| Battery % at 0 / 5 / 10 / 15 / 20 / 25 / 30 min | |
| Perceived heat at each check | |
| Temperature warning shown? | |
| Interruptions (calls, notifications, lock) | |
| iOS battery attribution for Safari (as shown) | |
| Export: `durationMin`, `reliability.asr_restarts_per_min`, `units` | |

## Interpretation (allowed claims)
- **Allowed:**
  - "Battery fell X % over Y min while listening (baseline Z % over 10 min) on device D."
  - "The phone felt warm after N min."
- **Not allowed:** watts, temperatures in °C (unless the device reports them), or attribution to recognition versus translation versus speech.
- A high `asr_restarts_per_min` (above 3) suggests the recognizer restarting often, which can cost power and cause audio-route flicker. Report it if seen.
