# Device test results — one copy per device and configuration

> Fill in by hand. Attach the exported `talktual-validation-*.json` files. **Do not paste transcripts of bystanders.** For test J, copy only consented scripted phrases.

## Context
| Field | Value |
| --- | --- |
| Date / tester | |
| Build (commit) / URL | |
| Device model | |
| OS + version | |
| Browser + version | |
| AirPods / headset model + firmware | |
| Network (Wi-Fi / cellular, approx. speed) | |
| Room, approx. noise dBA (meter app name) | |
| Processing mode (Cloud / Hybrid / On-device) | |
| Headphones switch (on/off) · Speak translations (on/off) · Volume | |

## Results
| Test | Steps done (Y/N) | Result (Pass / Fail / Blocked) | Key numbers (from export) | Notes (route observed, quality, anomalies) | Export file |
| --- | --- | --- | --- | --- | --- |
| A iPhone mic + AirPods out | | | phrases ok _/10; mark→audio p50/p95 | input route actually used: | |
| B AirPods mic + out | | | | playback quality during / after Pause / after Back: | |
| C iPhone mic + speaker | | | half-duplex holds; echo dropped | | |
| D wired headset | | | | | |
| E route changes | | | route_changes; restarts | state matched reality? | |
| F permission / background / lock | | | interruptions | | |
| G pause / stop / offline / cancel | | | cancels; drops | response < 1 s? | |
| H speech conditions | | | humanMarked p50/p90/p95; engineReported | | |
| I resource session | | | see battery sheet | | |
| J native-speaker review | | | mean adequacy; omissions | | |

## Summary
- Overall (Pass / Partial / Fail / Blocked):
- Blocking issues found:
- Recommendation for DR-8.5-1 (web-first / native spike):
