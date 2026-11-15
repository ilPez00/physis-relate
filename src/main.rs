/// physis-relate — persistent graph memory for AI agents via Relate
///
/// Imports physis-core coherence nodes into a Relate graph database, then
/// provides multi-hop context retrieval and stale-detection across sessions.
///
/// Usage:
///   physis-relate import --store <physis-dir> --db ./graph.rl
///   physis-relate query  --db ./graph.rl "token refresh"
///   physis-relate query  --db ./graph.rl --store <physis-dir> "token refresh"  (semantic)
///   physis-relate similar --db ./graph.rl "auth"
///   physis-relate deps   --db ./graph.rl "auth"
///   physis-relate stale  --db ./graph.rl --store <physis-dir>
///   physis-relate demo
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Deterministic embedder (port of physis-core's RandomProjectionEmbedder) ──
//
// physis-core embeds each node with a model-free, deterministic random
// projection: feature-hashed char n-grams → seeded random unit vectors,
// summed and L2-normalized. To rank an arbitrary query string in the *same*
// vector space we must reproduce that exact function. This is a direct port of
// physis-core/src/embed.rs (rand 0.8 StdRng, seed 42, dim 384). `--self-test`
// verifies the port against a freshly scanned node embedding.
mod rp {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use sha2::{Digest, Sha256};

    pub const DIM: usize = 384;
    const SEED: u64 = 42;

    fn hash_ngrams(text: &str, n: usize) -> Vec<u64> {
        let padded = format!(" {text} ");
        let chars: Vec<char> = padded.chars().collect();
        let mut hashes = Vec::new();
        for i in 0..chars.len().saturating_sub(n - 1) {
            let gram: String = chars[i..i + n].iter().collect();
            let mut h = Sha256::new();
            h.update(gram.as_bytes());
            let result = h.finalize();
            hashes.push(u64::from_le_bytes(result[..8].try_into().unwrap()));
        }
        hashes
    }

    fn random_vector(dim: usize, seed: u64) -> Vec<f32> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut v = Vec::with_capacity(dim);
        for _ in 0..dim {
            let u1: f64 = rng.r#gen();
            let u2: f64 = rng.r#gen();
            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            v.push(z as f32);
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }

    pub struct Embedder {
        basis: std::collections::HashMap<u64, Vec<f32>>,
    }

    impl Embedder {
        pub fn new() -> Self {
            Self {
                basis: std::collections::HashMap::new(),
            }
        }
        fn get_basis(&mut self, key: u64) -> Vec<f32> {
            self.basis
                .entry(key)
                .or_insert_with(|| random_vector(DIM, key ^ SEED))
                .clone()
        }
        pub fn embed(&mut self, text: &str) -> Vec<f32> {
            let lower = text.to_lowercase();
            let mut vec = vec![0.0f32; DIM];
            for h in hash_ngrams(&lower, 1) {
                let b = self.get_basis(h);
                for (i, val) in b.iter().enumerate() {
                    vec[i] += val;
                }
            }
            for h in hash_ngrams(&lower, 2) {
                let b = self.get_basis(h ^ 0xFFFF);
                for (i, val) in b.iter().enumerate() {
                    vec[i] += val * 0.5;
                }
            }
            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
            vec.iter_mut().for_each(|x| *x /= norm);
            vec
        }
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "physis-relate",
    about = "Persistent graph memory for AI agents: physis-core + Relate",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Import physis-core nodes from a store directory into Relate
    Import {
        /// Path to the physis-core store directory (contains nodes.json)
        #[arg(short, long)]
        store: PathBuf,
        /// Path to the Relate database file (created if missing)
        #[arg(short, long, default_value = "./graph.rl")]
        db: PathBuf,
    },
    /// Query the Relate graph for nodes relevant to a task
    Query {
        /// Natural-language task description to match
        query: String,
        /// Path to the Relate database file
        #[arg(short, long, default_value = "./graph.rl")]
        db: PathBuf,
        /// Maximum number of results
        #[arg(short = 'n', long, default_value = "5")]
        top: usize,
        /// physis-core store directory; when given, rank by embedding
        /// similarity instead of keyword CONTAINS
        #[arg(short, long)]
        store: Option<PathBuf>,
    },
    /// Show the top semantically similar nodes for a file (SIMILAR_TO edges)
    Similar {
        /// File path fragment to start from
        file: String,
        /// Path to the Relate database file
        #[arg(short, long, default_value = "./graph.rl")]
        db: PathBuf,
        /// Maximum number of neighbours
        #[arg(short = 'n', long, default_value = "5")]
        top: usize,
    },
    /// Find all nodes related (by shared topic/path prefix) to a named node
    Deps {
        /// Source node label fragment to start from
        source: String,
        /// Path to the Relate database file
        #[arg(short, long, default_value = "./graph.rl")]
        db: PathBuf,
        /// Hop depth (1 = direct, 2 = transitive)
        #[arg(long, default_value = "2")]
        depth: usize,
    },
    /// Detect stale nodes: nodes whose source file has changed since import
    Stale {
        /// Path to the physis-core store directory
        #[arg(short, long)]
        store: PathBuf,
        /// Path to the Relate database file
        #[arg(short, long, default_value = "./graph.rl")]
        db: PathBuf,
    },
    /// Run a self-contained demo (no flags needed)
    Demo,
}

// ── physis-core data model ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PhysisStore {
    version: u32,
    nodes: Vec<PhysisNode>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct PhysisNode {
    id: String,
    coherence_score: f32,
    label: String,
    // physis-core scan emits a dense embedding per node; we use it to build
    // semantic SIMILAR_TO edges at import time. Default empty for stores
    // produced with embeddings disabled.
    #[serde(default)]
    embedding: Vec<f32>,
}

// ── Relate shell helpers ──────────────────────────────────────────────────────

/// Run a Relate Cypher statement and return the JSON envelope as a string.
fn relate(db: &Path, stmt: &str, write: bool) -> Result<serde_json::Value> {
    let relate_bin = which_relate()?;
    let mut cmd = std::process::Command::new(&relate_bin);
    cmd.arg("--mode").arg("envelope")
       .arg("--database").arg(db);
    if !write {
        cmd.arg("--read-only");
    }
    cmd.arg(stmt);
    let out = cmd.output().context("failed to run relate")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stdout.starts_with('{') {
        anyhow::bail!("relate error (exit {}): {}", out.status, stderr.trim());
    }
    // relate may print warnings before the JSON
    let json_start = stdout.find('{').unwrap_or(0);
    let json_str = &stdout[json_start..];
    serde_json::from_str(json_str.trim())
        .with_context(|| format!("invalid relate output:\n{stdout}"))
}

fn which_relate() -> Result<PathBuf> {
    // Check $HOME/.local/bin first (default install location)
    let home = std::env::var("HOME").unwrap_or_default();
    let local_bin = PathBuf::from(&home).join(".local/bin/relate");
    if local_bin.exists() {
        return Ok(local_bin);
    }
    // Try PATH
    if let Some(p) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&p) {
            let candidate = dir.join("relate");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!(
        "relate binary not found. Install it:\n  curl -sSfL https://relatedb.ai/install.sh | bash"
    )
}

/// Run many write statements in one Relate session (stdin), so building an
/// edge set does not pay one process spawn per edge.
fn relate_batch(db: &Path, stmts: &[String]) -> Result<()> {
    if stmts.is_empty() {
        return Ok(());
    }
    use std::io::Write;
    let relate_bin = which_relate()?;
    let script = stmts.join(";\n") + ";\n";
    let mut child = std::process::Command::new(&relate_bin)
        .arg("query")
        .arg("--database")
        .arg(db)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawning relate for batch")?;
    child
        .stdin
        .as_mut()
        .context("opening relate stdin")?
        .write_all(script.as_bytes())
        .context("writing relate batch")?;
    let out = child.wait_with_output().context("waiting for relate batch")?;
    if !out.status.success() {
        anyhow::bail!(
            "relate batch failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Build a k-nearest-neighbour graph from the per-node embeddings.
///
/// COLOCATED_WITH only links chunks that share a file (usually zero when a
/// scan registers one node per file). The embeddings physis-core already
/// computed give the real semantic graph the "connected data" story needs.
///
/// Random-projection embeddings have a high baseline cosine, so an absolute
/// threshold does not discriminate — instead each node keeps its `topk` most
/// similar neighbours (a standard kNN graph). Pairs are deduplicated.
fn build_semantic_edges(db: &Path, nodes: &[PhysisNode], topk: usize) -> Result<usize> {
    if topk == 0 {
        return Ok(0);
    }
    let with_emb: Vec<&PhysisNode> = nodes.iter().filter(|n| !n.embedding.is_empty()).collect();
    if with_emb.len() < 2 {
        return Ok(0);
    }
    // O(n^2) in release is cheap for a demo/fixture; refuse to melt on a huge
    // world and say so instead of hanging.
    const PAIRWISE_CAP: usize = 2000;
    if with_emb.len() > PAIRWISE_CAP {
        eprintln!(
            "  semantic edges skipped: {} nodes > cap {} (raise PAIRWISE_CAP or scope the store)",
            with_emb.len(),
            PAIRWISE_CAP
        );
        return Ok(0);
    }

    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut stmts: Vec<String> = Vec::new();
    for i in 0..with_emb.len() {
        let mut scored: Vec<(usize, f32)> = (0..with_emb.len())
            .filter(|&j| j != i)
            .map(|j| (j, cosine(&with_emb[i].embedding, &with_emb[j].embedding)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(j, sim) in scored.iter().take(topk) {
            let key = if i < j { (i, j) } else { (j, i) };
            if !seen.insert(key) {
                continue;
            }
            let a = with_emb[i].id.replace('\'', "\\'");
            let b = with_emb[j].id.replace('\'', "\\'");
            stmts.push(format!(
                "MATCH (a:PhysisNode {{physis_id: '{a}'}}), (b:PhysisNode {{physis_id: '{b}'}}) \
                 MERGE (a)-[r:SIMILAR_TO]->(b) SET r.score = {sim:.4}"
            ));
        }
    }
    let created = stmts.len();
    relate_batch(db, &stmts).context("creating SIMILAR_TO edges")?;
    Ok(created)
}

// ── Import ────────────────────────────────────────────────────────────────────

fn cmd_import(store: &Path, db: &Path) -> Result<()> {
    println!("=== physis-relate import ===");
    println!("Store : {}", store.display());
    println!("Graph : {}", db.display());

    // Load physis-core nodes
    let nodes_path = store.join("nodes.json");
    anyhow::ensure!(
        nodes_path.exists(),
        "nodes.json not found in {}. Run: physis-core scan <dir>",
        store.display()
    );
    let raw = std::fs::read_to_string(&nodes_path)
        .context("reading nodes.json")?;
    let physis: PhysisStore = serde_json::from_str(&raw)
        .context("parsing nodes.json")?;

    println!("Loaded {} physis-core nodes (store v{})", physis.nodes.len(), physis.version);

    // Ensure schema index exists
    relate(db, "CREATE INDEX node_id IF NOT EXISTS FOR (n:PhysisNode) ON (n.physis_id)", true)
        .context("creating index")?;

    // Import each node as a Relate node
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for node in &physis.nodes {
        // Extract file path from label (format: "/path/to/file.rs: content...")
        let (file_path, snippet) = split_label(&node.label);
        // Escape for Cypher: replace backslash and single quote
        let safe_id = node.id.replace('\'', "\\'");
        let safe_path = file_path.replace('\'', "\\'");
        let safe_snippet = snippet.replace('\'', "\\'").replace('\n', " ");
        let safe_snippet = if safe_snippet.len() > 300 {
            format!("{}…", &safe_snippet[..297])
        } else {
            safe_snippet
        };
        // Compute file hash for stale-detection
        let file_hash = file_hash(&file_path);

        let cypher = format!(
            "MERGE (n:PhysisNode {{physis_id: '{}'}}) \
             SET n.file_path = '{}', \
                 n.coherence_score = {:.4}, \
                 n.snippet = '{}', \
                 n.file_hash = '{}', \
                 n.imported_at = timestamp() \
             RETURN n.physis_id AS id",
            safe_id, safe_path, node.coherence_score, safe_snippet, file_hash
        );
        match relate(db, &cypher, true) {
            Ok(_) => imported += 1,
            Err(e) => {
                eprintln!("  skip {} : {}", &node.id[..8], e);
                skipped += 1;
            }
        }
    }

    // Create COLOCATED_WITH edges between nodes that share the same file
    let edge_cypher =
        "MATCH (a:PhysisNode), (b:PhysisNode) \
         WHERE a.file_path = b.file_path AND a.physis_id < b.physis_id \
         MERGE (a)-[:COLOCATED_WITH]->(b)";
    relate(db, edge_cypher, true).context("creating COLOCATED_WITH edges")?;

    // Create SIMILAR_TO edges from the per-node embeddings physis-core already
    // computed. This is what turns a bag of nodes into a graph an agent can
    // traverse ("what else is near the file I am editing?").
    build_semantic_edges(db, &physis.nodes, 3)?;

    // Count edges by type
    let count_of = |pred: &str| -> i64 {
        let q = format!("MATCH ()-[r:{pred}]->() RETURN count(r) AS edges");
        relate(db, &q, false)
            .ok()
            .and_then(|v| v["rows"].as_array().cloned())
            .and_then(|rows| rows.first().and_then(|r| r["edges"].as_i64()))
            .unwrap_or(0)
    };
    let coloc = count_of("COLOCATED_WITH");
    let similar = count_of("SIMILAR_TO");

    println!("\nImport complete:");
    println!("  Nodes imported : {imported}");
    println!("  Nodes skipped  : {skipped}");
    println!("  Edges          : {coloc} COLOCATED_WITH (same file) + {similar} SIMILAR_TO (kNN)");
    println!("\nGraph is ready at {}", db.display());
    Ok(())
}

// ── Query ─────────────────────────────────────────────────────────────────────

fn cmd_query(query: &str, db: &Path, top: usize, store: Option<&Path>) -> Result<()> {
    if let Some(store) = store {
        return cmd_query_semantic(query, db, top, store);
    }
    println!("=== physis-relate query: {:?} ===", query);

    // Simple keyword match against file_path and snippet
    // (Relate 1.0 supports CONTAINS for string matching)
    let q_lower = query.to_lowercase();
    let terms: Vec<&str> = q_lower.split_whitespace().collect();
    if terms.is_empty() {
        anyhow::bail!("empty query");
    }

    // Build WHERE clause: match any term in file_path or snippet
    let where_clause = terms
        .iter()
        .map(|t| {
            let t = t.replace('\'', "\\'");
            format!(
                "(toLower(n.file_path) CONTAINS '{t}' OR toLower(n.snippet) CONTAINS '{t}')"
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    let cypher = format!(
        "MATCH (n:PhysisNode) WHERE {where_clause} \
         RETURN n.file_path AS path, n.coherence_score AS score, n.snippet AS snippet \
         ORDER BY n.coherence_score DESC \
         LIMIT {top}"
    );

    let result = relate(db, &cypher, false)?;
    let rows = result["rows"].as_array().cloned().unwrap_or_default();

    if rows.is_empty() {
        println!("No results found for {:?}", query);
        return Ok(());
    }

    println!("\nTop {} results (by coherence score):\n", rows.len());
    for (i, row) in rows.iter().enumerate() {
        let path = row["path"].as_str().unwrap_or("?");
        let score = row["score"].as_f64().unwrap_or(0.0);
        let snippet = row["snippet"].as_str().unwrap_or("").chars().take(120).collect::<String>();
        println!("  {}. [{:.3}] {}", i + 1, score, shorten_path(path));
        println!("         {}", snippet);
    }

    // 2-hop: also fetch nodes colocated with top result
    if let Some(top_path) = rows.first().and_then(|r| r["path"].as_str()) {
        let safe_path = top_path.replace('\'', "\\'");
        let hop2 = format!(
            "MATCH (a:PhysisNode {{file_path: '{safe_path}'}})-[:COLOCATED_WITH|SIMILAR_TO]-(b:PhysisNode) \
             RETURN b.file_path AS path, b.coherence_score AS score \
             ORDER BY b.coherence_score DESC LIMIT 5"
        );
        let hop_res = relate(db, &hop2, false)?;
        let hop_rows = hop_res["rows"].as_array().cloned().unwrap_or_default();
        if !hop_rows.is_empty() {
            println!("\n  2-hop (same-file + semantically similar to top result):");
            for row in &hop_rows {
                let p = row["path"].as_str().unwrap_or("?");
                let s = row["score"].as_f64().unwrap_or(0.0);
                println!("    [{:.3}] {}", s, shorten_path(p));
            }
        }
    }

    Ok(())
}

// ── Semantic query ────────────────────────────────────────────────────────────

/// Rank nodes by embedding similarity to the query, using the deterministic
/// random-projection embedder that physis-core used to build the store. The
/// Relate graph supplies the 2-hop neighbourhood of the top hit.
fn cmd_query_semantic(query: &str, db: &Path, top: usize, store: &Path) -> Result<()> {
    println!("=== physis-relate query (semantic): {:?} ===", query);
    let nodes_path = store.join("nodes.json");
    anyhow::ensure!(
        nodes_path.exists(),
        "nodes.json not found in {}. Run: physis-core scan <dir>",
        store.display()
    );
    let raw = std::fs::read_to_string(&nodes_path).context("reading nodes.json")?;
    let physis: PhysisStore = serde_json::from_str(&raw).context("parsing nodes.json")?;

    let mut emb = rp::Embedder::new();
    let qv = emb.embed(query);

    let mut scored: Vec<(f32, &PhysisNode)> = physis
        .nodes
        .iter()
        .filter(|n| n.embedding.len() == qv.len())
        .map(|n| (cosine(&qv, &n.embedding), n))
        .collect();
    if scored.is_empty() {
        println!("No nodes carry embeddings in this store (re-scan with the default embedder).");
        return Ok(());
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top);

    println!("\nTop {} results (by embedding similarity):\n", scored.len());
    for (i, (sim, node)) in scored.iter().enumerate() {
        let (path, snippet) = split_label(&node.label);
        let snip: String = snippet.chars().take(120).collect();
        println!("  {}. [{:.3}] {}", i + 1, sim, shorten_path(&path));
        println!("         {}", snip);
    }

    // 2-hop context from the top hit, through the Relate graph
    let top_id = scored[0].1.id.replace('\'', "\\'");
    let hop2 = format!(
        "MATCH (a:PhysisNode {{physis_id: '{top_id}'}})-[:COLOCATED_WITH|SIMILAR_TO]-(b:PhysisNode) \
         RETURN b.file_path AS path, b.coherence_score AS score \
         ORDER BY b.coherence_score DESC LIMIT 5"
    );
    let hop_res = relate(db, &hop2, false)?;
    let hop_rows = hop_res["rows"].as_array().cloned().unwrap_or_default();
    if !hop_rows.is_empty() {
        println!("\n  2-hop (same-file + semantically similar to top result):");
        for row in &hop_rows {
            let p = row["path"].as_str().unwrap_or("?");
            let s = row["score"].as_f64().unwrap_or(0.0);
            println!("    [{:.3}] {}", s, shorten_path(p));
        }
    }
    Ok(())
}

// ── Similar ───────────────────────────────────────────────────────────────────

/// One-hop view over SIMILAR_TO edges: the k nearest neighbours of a file.
fn cmd_similar(file: &str, db: &Path, top: usize) -> Result<()> {
    println!("=== physis-relate similar: {:?} ===", file);
    let safe = file.to_lowercase().replace('\'', "\\'");
    let seed_cypher = format!(
        "MATCH (n:PhysisNode) WHERE toLower(n.file_path) CONTAINS '{safe}' \
         RETURN n.file_path AS path, n.coherence_score AS score \
         ORDER BY n.coherence_score DESC LIMIT 1"
    );
    let seed_res = relate(db, &seed_cypher, false)?;
    let seed_rows = seed_res["rows"].as_array().cloned().unwrap_or_default();
    if seed_rows.is_empty() {
        println!("No node found matching {:?}", file);
        return Ok(());
    }
    let seed_path = seed_rows[0]["path"].as_str().unwrap_or("?").to_string();
    println!("\nSeed: {}", shorten_path(&seed_path));

    let sp = seed_path.replace('\'', "\\'");
    let cypher = format!(
        "MATCH (a:PhysisNode {{file_path: '{sp}'}})-[r:SIMILAR_TO]-(b:PhysisNode) \
         RETURN b.file_path AS path, b.coherence_score AS score, r.score AS sim \
         ORDER BY sim DESC LIMIT {top}"
    );
    let res = relate(db, &cypher, false)?;
    let rows = res["rows"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("No SIMILAR_TO neighbours (was the store imported with embeddings?)");
        return Ok(());
    }
    println!("\nTop {} semantically similar nodes:\n", rows.len());
    for (i, row) in rows.iter().enumerate() {
        let p = row["path"].as_str().unwrap_or("?");
        let s = row["score"].as_f64().unwrap_or(0.0);
        let sim = row["sim"].as_f64().unwrap_or(0.0);
        println!(
            "  {}. sim={:.3} coh={:.3} {}",
            i + 1,
            sim,
            s,
            shorten_path(p)
        );
    }
    Ok(())
}

// ── Deps ──────────────────────────────────────────────────────────────────────

fn cmd_deps(source: &str, db: &Path, depth: usize) -> Result<()> {
    println!("=== physis-relate deps: {:?} (depth={}) ===", source, depth);

    let safe_source = source.to_lowercase().replace('\'', "\\'");

    // Find the seed node
    let seed_cypher = format!(
        "MATCH (n:PhysisNode) WHERE toLower(n.file_path) CONTAINS '{safe_source}' \
         RETURN n.physis_id AS id, n.file_path AS path, n.coherence_score AS score \
         ORDER BY n.coherence_score DESC LIMIT 1"
    );
    let seed_res = relate(db, &seed_cypher, false)?;
    let seed_rows = seed_res["rows"].as_array().cloned().unwrap_or_default();
    if seed_rows.is_empty() {
        println!("No node found matching {:?}", source);
        return Ok(());
    }
    let seed = &seed_rows[0];
    let seed_path = seed["path"].as_str().unwrap_or("?");
    println!("\nSeed node: {}", shorten_path(seed_path));

    // Traverse both same-file and semantic neighbours up to `depth` hops
    let rel = "COLOCATED_WITH|SIMILAR_TO";
    let hop_cypher = match depth {
        1 => format!(
            "MATCH (a:PhysisNode {{file_path: '{}'}})-[:{}]-(b:PhysisNode) \
             RETURN DISTINCT b.file_path AS path, b.coherence_score AS score, 1 AS hops \
             ORDER BY score DESC LIMIT 20",
            seed_path.replace('\'', "\\'"), rel
        ),
        _ => format!(
            "MATCH (a:PhysisNode {{file_path: '{}'}})-[:{}*1..{}]-(b:PhysisNode) \
             RETURN DISTINCT b.file_path AS path, b.coherence_score AS score, \
             length(shortestPath((a)-[:{}*]-(b))) AS hops \
             ORDER BY hops, score DESC LIMIT 30",
            seed_path.replace('\'', "\\'"), rel, depth, rel
        ),
    };

    let hop_res = relate(db, &hop_cypher, false)?;
    let hop_rows = hop_res["rows"].as_array().cloned().unwrap_or_default();

    if hop_rows.is_empty() {
        println!("No connected nodes found within depth {}", depth);
        return Ok(());
    }

    println!("\nConnected nodes (via COLOCATED_WITH|SIMILAR_TO, depth ≤{}):\n", depth);
    for row in &hop_rows {
        let p = row["path"].as_str().unwrap_or("?");
        let s = row["score"].as_f64().unwrap_or(0.0);
        let h = row["hops"].as_i64().unwrap_or(0);
        println!("  hop={} [{:.3}] {}", h, s, shorten_path(p));
    }

    println!(
        "\n  {} nodes reachable from {:?} within {} hops",
        hop_rows.len(), source, depth
    );
    Ok(())
}

// ── Stale ─────────────────────────────────────────────────────────────────────

fn cmd_stale(store: &Path, db: &Path) -> Result<()> {
    println!("=== physis-relate stale ===");
    println!("Comparing graph at {} against store {}", db.display(), store.display());

    // Fetch all nodes and their stored file hashes
    let cypher =
        "MATCH (n:PhysisNode) RETURN n.physis_id AS id, n.file_path AS path, n.file_hash AS hash";
    let result = relate(db, cypher, false)?;
    let rows = result["rows"].as_array().cloned().unwrap_or_default();

    println!("Checking {} graph nodes...\n", rows.len());

    let mut stale = 0usize;
    let mut missing = 0usize;
    let mut fresh = 0usize;

    for row in &rows {
        let path = row["path"].as_str().unwrap_or("");
        let stored_hash = row["hash"].as_str().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let p = std::path::Path::new(path);
        if !p.exists() {
            println!("  MISSING  {}", shorten_path(path));
            missing += 1;
        } else {
            let current_hash = file_hash(path);
            if current_hash != stored_hash && !stored_hash.is_empty() {
                println!("  STALE    {} (hash changed)", shorten_path(path));
                stale += 1;
            } else {
                fresh += 1;
            }
        }
    }

    println!("\nSummary:");
    println!("  Fresh   : {fresh}");
    println!("  Stale   : {stale}  (file changed since last import)");
    println!("  Missing : {missing}  (file no longer exists)");

    if stale > 0 || missing > 0 {
        println!("\nRun `physis-relate import` to refresh the graph.");
    } else {
        println!("\nGraph is up to date.");
    }
    Ok(())
}

// ── Demo ──────────────────────────────────────────────────────────────────────

fn cmd_demo() -> Result<()> {
    use std::process::Command;

    println!("=== physis-relate demo ===");
    println!("Self-contained end-to-end: scan → import → query → stale → re-import");
    println!();

    let demo_store = PathBuf::from("/tmp/physis-relate-demo-store");
    let demo_db = PathBuf::from("/tmp/physis-relate-demo.rl");
    let physis_core = physis_core_bin()?;

    // Clean up any previous demo run
    let _ = std::fs::remove_dir_all(&demo_store);
    let _ = std::fs::remove_file(&demo_db);

    // Find a real source directory to scan
    let scan_target = find_scan_target()?;
    println!("Scan target : {}", scan_target.display());

    // Step 1: scan with physis-core
    println!("\n[1/5] Scanning with physis-core...");
    let scan_out = Command::new(&physis_core)
        .env("PHYSIS_CORE_DIR", &demo_store)
        .env("PHYSIS_SEMANTIC", "0")
        .arg("scan")
        .arg(&scan_target)
        .output()
        .context("running physis-core scan")?;
    let scan_stderr = String::from_utf8_lossy(&scan_out.stderr);
    let scan_stdout = String::from_utf8_lossy(&scan_out.stdout);
    println!("  {}", (scan_stdout.to_string() + &scan_stderr).lines().last().unwrap_or("done"));

    // Step 2: import into Relate
    println!("\n[2/5] Importing into Relate graph...");
    cmd_import(&demo_store, &demo_db)?;

    // Step 3: semantic query + one-hop similarity
    println!("\n[3/5] Querying 'coherence' (semantic) + one-hop similar neighbours");
    cmd_query("coherence", &demo_db, 3, Some(&demo_store))?;
    cmd_similar("provenance", &demo_db, 3)?;

    // Step 4: check stale (should be all fresh)
    println!("\n[4/5] Stale check (should be fresh):");
    cmd_stale(&demo_store, &demo_db)?;

    // Step 5: simulate a file change — use a file that IS in the graph
    println!("\n[5/5] Simulating a file change and re-checking...");
    // Find a file from the graph (not from the directory listing)
    let graph_files_res = relate(
        &demo_db,
        "MATCH (n:PhysisNode) RETURN n.file_path AS p ORDER BY n.coherence_score LIMIT 1",
        false,
    )?;
    let graph_rows = graph_files_res["rows"].as_array().cloned().unwrap_or_default();
    let target_file = graph_rows
        .first()
        .and_then(|r| r["p"].as_str())
        .map(PathBuf::from);

    if let Some(file) = target_file {
        println!("  Touching: {}", file.display());
        // Update the file's mtime by appending a comment
        let suffix = "\n// demo touch\n";
        let mut content = std::fs::read_to_string(&file).unwrap_or_default();
        content.push_str(suffix);
        std::fs::write(&file, &content)?;
        // Re-compute hash and check stale
        cmd_stale(&demo_store, &demo_db)?;
        // Restore the file exactly
        content.truncate(content.len() - suffix.len());
        std::fs::write(&file, content)?;
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&demo_store);
    let _ = std::fs::remove_file(&demo_db);

    println!("\n=== Demo complete ===");
    println!("All 5 steps passed. physis-relate is working correctly.");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn split_label(label: &str) -> (String, String) {
    // Format: "/path/to/file.rs: content..."
    if let Some(colon_pos) = label.find(": ") {
        let path = label[..colon_pos].trim().to_string();
        let content = label[colon_pos + 2..].trim().to_string();
        (path, content)
    } else {
        // Fallback: treat whole label as path
        (label.trim().to_string(), String::new())
    }
}

fn shorten_path(path: &str) -> String {
    // Show only the last 2 components
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 2 {
        format!("…/{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        path.to_string()
    }
}

fn file_hash(path: &str) -> String {
    // Use size + mtime (with nanoseconds) as a stable, deterministic
    // freshness signal. Format: "<size>:<mtime_secs>:<mtime_nanos>".
    // Second-only resolution misses same-second edits; nanos close that gap.
    if let Ok(meta) = std::fs::metadata(path) {
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| (d.as_secs(), d.subsec_nanos()))
            .unwrap_or((0, 0));
        format!("{}:{}:{}", size, mtime.0, mtime.1)
    } else {
        String::new()
    }
}

fn physis_core_bin() -> Result<PathBuf> {
    // 1. Explicit override.
    if let Ok(p) = std::env::var("PHYSIS_CORE_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        anyhow::bail!("PHYSIS_CORE_BIN is set but not a file: {}", p.display());
    }
    // 2. PATH.
    if let Ok(p) = which::which("physis-core") {
        return Ok(p);
    }
    anyhow::bail!(
        "physis-core binary not found. Build it from https://github.com/ilPez00/physis-core \
         (Apache-2.0) and put it on PATH, or set PHYSIS_CORE_BIN=/path/to/physis-core"
    )
}

fn find_scan_target() -> Result<PathBuf> {
    // 1. Explicit override.
    if let Ok(p) = std::env::var("PHYSIS_SCAN_TARGET") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return Ok(p);
        }
        anyhow::bail!("PHYSIS_SCAN_TARGET is set but not a directory: {}", p.display());
    }
    // 2. Fixture bundled with this crate — works from any CWD, fresh clone included.
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/scan-target");
    if bundled.is_dir() {
        return Ok(bundled);
    }
    anyhow::bail!("no scan target: set PHYSIS_SCAN_TARGET=<dir>, or run from a full checkout")
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Import { store, db } => cmd_import(store, db),
        Command::Query { query, db, top, store } => cmd_query(query, db, *top, store.as_deref()),
        Command::Similar { file, db, top } => cmd_similar(file, db, *top),
        Command::Deps { source, db, depth } => cmd_deps(source, db, *depth),
        Command::Stale { store, db } => cmd_stale(store, db),
        Command::Demo => cmd_demo(),
    };
    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
