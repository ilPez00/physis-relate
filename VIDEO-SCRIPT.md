# Neo4j Relate — video

**File:** `demo.mp4` — 75 s, 1280×800, H.264, silent.
**Recorded:** 2026-11-26 against the current `physis-relate` binary.

## What it shows, frame by frame

The recording is a real terminal session running `./target/release/physis-relate demo`.
No mockup, no edited output.

| Time | On screen |
|---|---|
| 0:00–0:08 | Title: physis-relate, physis-core scan → Relate graph → query/stale check |
| 0:08–0:14 | The graph is a Relate `.rl` file, so it survives between sessions |
| 0:14–0:40 | Scan (10 nodes), import (21 SIMILAR_TO edges), semantic query top 3 |
| 0:40–0:52 | `similar provenance` — the kNN neighbours, sim ≈ 0.987 |
| 0:52–0:62 | Stale check: 10/10 fresh |
| 0:62–0:72 | Touch `temporal.rs`, re-check → `STALE (hash changed)`, 9 fresh / 1 stale |
| 0:72–0:75 | `All 5 steps passed. physis-relate is working correctly.` |

## Re-record it

```bash
bash /tmp/record-relate.sh
```

That script runs the demo inside `xterm -hold` (so the output stays on screen)
while `ffmpeg -f x11grab` captures `:99` with a fixed `-t 75` so the file always
finalises. Both details matter: without `-hold` the terminal closed before the
capture, and without `-t` the `moov` atom was never written, producing a black
file.

## Devpost needs narration

The rules say the video should show the demo and be 1–2 minutes. This cut is
silent. Record a voiceover against it before submitting, or re-record with audio.

## Submission status

| Item | State |
|---|---|
| Public repo `ilPez00/physis-relate` | pushed, in sync with `origin/main` |
| `demo.mp4` | committed (75 s) |
| `build-disclosure.md` | complete |
| `README.md` | current measured numbers |
| Devpost entry | **not filed** — needs the owner's browser login at relate.devpost.com |

Deadline: Dec 11, 2026, ~09:00 CET.
