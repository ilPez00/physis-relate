# Build Disclosure — physis-relate

Neo4j Relate Hackathon 2026

## Code written during the contest window (Nov 11 – Dec 11, 2026)

| File | Description |
|---|---|
| `src/main.rs` | All application code: CLI, import, query, deps, stale, demo |
| `README.md` | Documentation |
| `Cargo.toml` | Project manifest |
| `fixtures/scan-target/` | Demo scan input: ten unmodified modules from physis-core v0.2.0 (Apache-2.0) |
| `build-disclosure.md` | This file |

## Reused prior work (with license)

| Component | License | Source | Use |
|---|---|---|---|
| physis-core binary v0.2.0 | Apache-2.0 | github.com/ilPez00/physis-core | Scan repos into coherence graph |
| physis-core vendor source | Apache-2.0 | hermes-physis/vendor/physis-core | Reference for JSON output format |
| BGE-small ONNX embedder | MIT | bundled in hermes-physis | Optional semantic embedder |
| Relate CLI + crate v1.0.1 | Apache-2.0 | relatedb.ai | Graph persistence and traversal |

## What is NOT included

- physis-next source (private, commercial) — no source exposed
- hermes-physis plugin code — not used in this entry
- Any code from other contest entries
