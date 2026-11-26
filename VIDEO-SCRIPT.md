# Neo4j Relate — Video Script (physis-relate demo)

**Target:** <2 minutes (Devpost requirement)
**File:** demo.mp4 — already recorded at 1:50, committed to /home/gio/dev/physis-relate/demo.mp4
**Recorded:** Xvfb + ffmpeg x11grab, 1280x720, 30fps, H.264

---

## Video contents (already recorded, 5 steps)

### 0:00–0:12 — Intro
"Without persistent graph memory, agents re-derive the same relationships on every session. We're fixing that."

### 0:12–0:30 — Scan
Run `physis-core scan` on the fixtures/scan-target/ tree → 225 files registered, coherence index 0.957.

### 0:30–1:05 — Import + Query
Import into Relate graph → 10 nodes, 21 SIMILAR_TO edges. Query "coherence" → ranked results with 2-hop neighbours.

### 1:05–1:30 — Update cycle
Edit one file (temporal.rs), re-check stale → `STALE …/temporal.rs (hash changed)` detected correctly.

### 1:30–1:50 — Stale guard + Benchmark
Query a stale node → flagged, not silently returned. Benchmark: 8/10 queries correct.

### 1:50–2:00 — Why Relate matters
"Remove Relate: the graph is session-only and 2-hop traversal is gone."

---

## Devpost submission items (login-gated, operator step)

| Item | Status |
|---|---|
| Public repo: ilPez00/physis-relate | ✅ Pushed to origin/main |
| Demo video: demo.mp4 | ✅ Committed (1:50, under 2-min limit) |
| build-disclosure.md | ✅ Complete |
| README.md | ✅ Complete |
| Devpost submission | ⬜ Requires GitHub login at relate.devpost.com |

Deadline: Dec 11, ~09:00 CET
