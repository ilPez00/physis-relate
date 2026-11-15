# physis-relate

**Persistent graph memory for AI agents: physis-core × Relate**

Neo4j Relate Hackathon 2026 entry. Theme: *Context, Connected*.

Agents forget everything between sessions. `physis-relate` fixes that:
it imports the coherence graph built by [physis-core](https://github.com/ilPez00/physis-core)
into a [Relate](https://relatedb.ai) database, giving agents cross-session
memory, keyword-ranked retrieval, and automatic stale detection when source
files change.

---

## What it does

```
physis-core scan <repo>
    ↓ nodes.json (file chunks with coherence scores)
physis-relate import
    ↓ Relate graph (nodes + COLOCATED_WITH + SIMILAR_TO edges)
physis-relate query "token refresh"
    ↓ ranked results + 2-hop context
physis-relate stale
    ↓ STALE …/auth.rs (hash changed) → agent knows to re-index
```

**Without Relate:** the graph lives in-memory per session. Two sessions
cannot share context, and there is no cross-session traversal.

**With Relate:** the graph persists to a `.rl` file. Any session can query it,
the stale checker catches file changes, and re-indexing is incremental (MERGE,
not DELETE + INSERT).

---

## Install

Prerequisites:
- Rust 1.88+ (`rustup update`)
- Relate CLI (`curl -sSfL https://relatedb.ai/install.sh | bash`)
- physis-core v0.2.0 binary, built from [physis-core](https://github.com/ilPez00/physis-core)
  (Apache-2.0):

```bash
git clone https://github.com/ilPez00/physis-core
cd physis-core && cargo build --release
# either put it on PATH, or point at it explicitly:
export PHYSIS_CORE_BIN="$PWD/target/release/physis-core"
```

No other absolute paths are assumed: the demo scans the bundled fixture
(`fixtures/scan-target/`), overridable with `PHYSIS_SCAN_TARGET=<dir>`.

```bash
git clone https://github.com/ilPez00/physis-relate
cd physis-relate
cargo build --release
# binary at: ./target/release/physis-relate
```

Add `~/.local/bin` to PATH if relate is not found:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

---

## 60-second demo (self-contained)

```bash
export PHYSIS_CORE_BIN=/path/to/physis-core   # or put physis-core on PATH
./target/release/physis-relate demo
```

Runs all 5 steps against the bundled fixture (`fixtures/scan-target/`, ten
sample Rust modules taken unmodified from physis-core v0.2.0, Apache-2.0 — see
`fixtures/scan-target/README.md`). Works from any working directory; override
the tree with `PHYSIS_SCAN_TARGET=<dir>`.

1. physis-core scan of the fixture tree
2. Import into Relate graph (COLOCATED_WITH + SIMILAR_TO kNN edges)
3. Semantic query + `similar` → embedding-ranked results, 2-hop context, kNN neighbours
4. Stale check → all nodes fresh
5. Modify one file → `STALE …/scan-target/experiments.rs (hash changed)`

Expected output ends with:

```
=== Demo complete ===
All 5 steps passed. physis-relate is working correctly.
```

---

## Commands

### import — load a physis-core store into Relate

```bash
# Scan a repo first
PHYSIS_CORE_DIR=./my-store physis-core scan ./my-repo

# Import into Relate
physis-relate import --store ./my-store --db ./my-graph.rl
```

Output:
```
Nodes imported : 42
Nodes skipped  : 0
Edges          : 7 COLOCATED_WITH (same file) + 120 SIMILAR_TO (kNN)
Graph is ready at ./my-graph.rl
```

Relate is essential here: without it, the import has nowhere to persist.
The MERGE-based upsert means re-running import updates existing nodes rather
than duplicating them.

Two edge types are built at import:

- **`COLOCATED_WITH`** — chunks that share a source file (usually zero when a
  scan registers one node per file).
- **`SIMILAR_TO`** — a k-nearest-neighbour graph over the dense embeddings
  physis-core already computes for every node. Each node keeps its 3 most
  similar neighbours, with the cosine similarity stored as `r.score`.

Random-projection embeddings (the default, no model required) have a high
baseline cosine — every pair lands in 0.82–0.99 — so an absolute threshold
does not discriminate. A top-K graph is the correct construction: it yields a
real, connected graph regardless of the embedding's absolute scale. Edges are
batched through one Relate session (stdin), not one process per edge.

### query — find relevant nodes

```bash
physis-relate query "token refresh" --db ./my-graph.rl --top 5
```

Output:
```
Top 5 results (by coherence score):

  1. [0.941] …/src/auth/token.rs
         Handles token creation, refresh and rotation. refresh_token validates
  ...

  2-hop (same-file + semantically similar to top result):
    [0.887] …/src/auth/session.rs
```

The 2-hop traversal walks both edge types in Relate: `COLOCATED_WITH` (same
file) and `SIMILAR_TO` (semantically near). An agent editing `token.rs` sees
`session.rs` as related context even though the files are not co-located.

#### Semantic mode (`--store`)

Keyword `CONTAINS` is the default. Pass the store directory to rank by
**embedding similarity** instead:

```bash
physis-relate query "token refresh" --store ./my-store --db ./my-graph.rl
```

This embeds the query with the *same* deterministic random-projection function
physis-core used to build the store (a direct port of `embed.rs`; verified to
reproduce physis-core's own `search` ranking score-for-score), ranks every node
by cosine similarity, and then pulls the 2-hop neighbourhood of the top hit
from Relate. The two systems divide the work: physis-core supplies the vector,
Relate supplies the relationships.

### deps — traverse from a named file

```bash
physis-relate deps "auth" --db ./my-graph.rl --depth 2
```

Shows all nodes reachable within 2 hops via `COLOCATED_WITH|SIMILAR_TO` edges
from the best-matching auth-related node — the change-impact view: "if I touch
this file, what else is in its neighbourhood?"

### similar — nearest semantic neighbours of a file

```bash
physis-relate similar "auth" --db ./my-graph.rl --top 5
```

One-hop view over `SIMILAR_TO` edges, ranked by the stored cosine score:

```
Seed: …/src/provenance.rs

Top 4 semantically similar nodes:

  1. sim=0.987 coh=1.000 …/src/temporal.rs
  2. sim=0.985 coh=0.966 …/src/coherence_dimensions.rs
  3. sim=0.985 coh=0.971 …/src/contradiction.rs
  4. sim=0.981 coh=0.973 …/src/config_run.rs
```

This is the direct answer to "what is this file near?" without an embedding
call at query time — the edges are already in the graph.

### stale — detect changed files

```bash
physis-relate stale --store ./my-store --db ./my-graph.rl
```

Compares every node's stored file hash (size:mtime) against the current
filesystem state:

```
  STALE    …/src/auth/token.rs (hash changed)

Summary:
  Fresh   : 41
  Stale   : 1
  Missing : 0

Run `physis-relate import` to refresh the graph.
```

The stale check is the agent's freshness gate: before trusting a query
result, the agent checks whether the underlying files have changed.

---

## Why Relate is essential

Remove Relate and:
- `import` has no target — the graph cannot persist between processes
- `query` and `deps` cannot traverse edges — there is no edge store
- `stale` cannot compare against a stored hash — there is nothing to compare to
- Two agents running simultaneously cannot see each other's observations

Relate's embedded, file-based model (`.rl` file, no server, no signup) is
the right fit: the same graph file can be shared between a CI job, an agent
session, and a local developer query without coordination.

---

## Measured results (reproducible)

Run `physis-relate demo` to reproduce:

| Step | Result |
|---|---|
| Scan: 10 sample .rs files → physis-core | 9 coherence nodes (0.926–1.000); files >5 KB are skipped by the scanner |
| Import into Relate | 9 nodes, 0 skipped, 19 SIMILAR_TO edges (kNN), MERGE-safe |
| Query "coherence", top 3 | experiments.rs 0.720 · coherence_query.rs 0.687 · coherence_dimensions.rs 0.677 |
| Query "coherence" `--store` (semantic) | Embedding ranking matches physis-core `search` score-for-score |
| `similar provenance` | temporal.rs sim=0.987 coh=1.000 |
| Stale check (all fresh) | 9/9 fresh |
| Stale check (after file edit) | 1 STALE detected correctly |

The semiotic benchmark for physis-core (independent reproducible fixture):
```
PHYSIS_CORE_DIR=/tmp/bench ./bin/physis-core bench-semiotic benchmarks/semiotic_cases.json
# calibration: top-3 76.4% vs random baseline 1.9%
```

---

## Honest limits

- physis-core produces one node per file (not per function/symbol); symbol-level
  graphs require physis-next's world model (private, commercial)
- `SIMILAR_TO` edges are a kNN graph over dense embeddings, not a true call/
  import graph — they capture "semantically near", not "depends on". Real
  `CALLS`/`IMPORTS` edges need a symbol-level parser (physis-next) and are not
  in this entry
- The stale check uses mtime+size, not content hash — a file can be re-saved
  without change and produce a false positive
- Query is keyword-based (CONTAINS). The graph edges are semantic (from the
  embeddings); enabling the BGE-small embedder (`PHYSIS_SEMANTIC=1`, needs
  `PHYSIS_MODEL_DIR`) improves edge quality but is not required — the default
  random-projection embeddings still produce a usable kNN graph

---

## License

Apache-2.0.

Reuses [physis-core](https://github.com/ilPez00/physis-core) (Apache-2.0)
and [Relate](https://relatedb.ai) (Apache-2.0).

---

## Build disclosure

New code written during the contest window (Nov 11–Dec 11, 2026):
- `src/main.rs` — all physis-relate application code
- `README.md`, `Cargo.toml`
- `fixtures/scan-target/` — ten sample modules from physis-core v0.2.0
  (Apache-2.0), used as the demo's scan input; not compiled with this crate

Reused prior work with disclosure:
- physis-core binary and vendor source snapshot (Apache-2.0, hermes-physis)
- BGE-small ONNX embedder (MIT, bundled in hermes-physis)
