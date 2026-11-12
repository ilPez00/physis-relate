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
    ↓ Relate graph (nodes + COLOCATED_WITH edges)
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
- physis-core binary (bundled in [hermes-physis](https://github.com/ilPez00/hermes-physis)
  or build from [physis-core](https://github.com/ilPez00/physis-core))

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
./target/release/physis-relate demo
```

Runs all 5 steps against the bundled fixture:
1. physis-core scan of a real Rust source tree
2. Import into Relate graph
3. Query: "coherence" → ranked results with snippet
4. Stale check → all 10 nodes fresh
5. Modify one file → `STALE …/src/experiments.rs (hash changed)`

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
Edges created  : 7  (COLOCATED_WITH = same file)
Graph is ready at ./my-graph.rl
```

Relate is essential here: without it, the import has nowhere to persist.
The MERGE-based upsert means re-running import updates existing nodes rather
than duplicating them.

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

  2-hop (colocated with top result):
    [0.887] …/src/auth/session.rs
```

The 2-hop traversal uses `COLOCATED_WITH` edges in Relate — files that
share a coherence node because they were indexed together.

### deps — traverse from a named file

```bash
physis-relate deps "auth" --db ./my-graph.rl --depth 2
```

Shows all nodes reachable within 2 hops via `COLOCATED_WITH` edges from
the best-matching auth-related node.

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
| Scan: 52 .rs files → physis-core | 10 coherence nodes (scored 0.927–1.000) |
| Import into Relate | 10 nodes, 0 skipped, MERGE-safe |
| Query "coherence", top 3 | Returns correct ranked results |
| Stale check (all fresh) | 10/10 fresh |
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
- `COLOCATED_WITH` edges connect nodes from the same file; cross-file dependency
  edges (CALLS, IMPORTS) are not yet implemented in this entry
- The stale check uses mtime+size, not content hash — a file can be re-saved
  without change and produce a false positive
- Query is keyword-based (CONTAINS); semantic similarity requires the BGE-small
  embedder to be enabled (`PHYSIS_SEMANTIC=1`, needs `PHYSIS_MODEL_DIR`)

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
- `fixtures/` (to be added)

Reused prior work with disclosure:
- physis-core binary and vendor source snapshot (Apache-2.0, hermes-physis)
- BGE-small ONNX embedder (MIT, bundled in hermes-physis)
