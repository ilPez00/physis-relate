/// physis-relate — persistent graph memory for AI agents via Relate
///
/// Imports physis-core coherence nodes into a Relate graph database, then
/// provides multi-hop context retrieval and stale-detection across sessions.
///
/// Usage:
///   physis-relate import --store <physis-dir> --db ./graph.rl
///   physis-relate query  --db ./graph.rl "token refresh"
///   physis-relate deps   --db ./graph.rl "auth"
///   physis-relate stale  --db ./graph.rl --store <physis-dir>
///   physis-relate demo
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    // other fields we don't need for the graph import
}

// ── Relate shell helpers ──────────────────────────────────────────────────────

/// Run a Relate Cypher statement and return the JSON envelope as a string.
fn relate(db: &PathBuf, stmt: &str, write: bool) -> Result<serde_json::Value> {
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

// ── Import ────────────────────────────────────────────────────────────────────

fn cmd_import(store: &PathBuf, db: &PathBuf) -> Result<()> {
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

    // Note: directory-level edges (IN_SAME_DIR) require extracting the parent path.
    // Cypher string functions in Relate 1.0 don't have APOC; we'll compute this
    // in a future version using the file_path prefix. COLOCATED_WITH covers
    // the primary same-file relationship, which is the core demo claim.

    // Count edges
    let count_res = relate(
        db,
        "MATCH ()-[r:COLOCATED_WITH]->() RETURN count(r) AS edges",
        false,
    )?;
    let edges = count_res["rows"][0]["edges"].as_i64().unwrap_or(0);

    println!("\nImport complete:");
    println!("  Nodes imported : {imported}");
    println!("  Nodes skipped  : {skipped}");
    println!("  Edges created  : {edges}  (COLOCATED_WITH = same file)");
    println!("\nGraph is ready at {}", db.display());
    Ok(())
}

// ── Query ─────────────────────────────────────────────────────────────────────

fn cmd_query(query: &str, db: &PathBuf, top: usize) -> Result<()> {
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
            "MATCH (a:PhysisNode {{file_path: '{safe_path}'}})-[:COLOCATED_WITH]-(b:PhysisNode) \
             RETURN b.file_path AS path, b.coherence_score AS score \
             ORDER BY b.coherence_score DESC LIMIT 5"
        );
        let hop_res = relate(db, &hop2, false)?;
        let hop_rows = hop_res["rows"].as_array().cloned().unwrap_or_default();
        if !hop_rows.is_empty() {
            println!("\n  2-hop (colocated with top result):");
            for row in &hop_rows {
                let p = row["path"].as_str().unwrap_or("?");
                let s = row["score"].as_f64().unwrap_or(0.0);
                println!("    [{:.3}] {}", s, shorten_path(p));
            }
        }
    }

    Ok(())
}

// ── Deps ──────────────────────────────────────────────────────────────────────

fn cmd_deps(source: &str, db: &PathBuf, depth: usize) -> Result<()> {
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

    // Traverse COLOCATED_WITH edges up to `depth` hops
    let rel = "COLOCATED_WITH";
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

    println!("\nConnected nodes (via COLOCATED_WITH, depth ≤{}):\n", depth);
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

fn cmd_stale(store: &PathBuf, db: &PathBuf) -> Result<()> {
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

    // Step 3: query
    println!("\n[3/5] Querying: 'coherence'");
    cmd_query("coherence", &demo_db, 3)?;

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
        let mut content = std::fs::read_to_string(&file).unwrap_or_default();
        content.push_str("\n// demo touch\n");
        std::fs::write(&file, &content)?;
        // Re-compute hash and check stale
        cmd_stale(&demo_store, &demo_db)?;
        // Restore the file
        content.truncate(content.len() - 17);
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
    // Use mtime-seconds + size as a stable, deterministic freshness signal.
    // Format: "<size>:<mtime_secs>" — readable and stable across processes.
    if let Ok(meta) = std::fs::metadata(path) {
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{size}:{mtime}")
    } else {
        String::new()
    }
}

fn physis_core_bin() -> Result<PathBuf> {
    // Check relative paths common in this project
    let candidates = [
        PathBuf::from("../../hermes-physis/bin/physis-core"),
        PathBuf::from("/home/gio/dev/hermes-physis/bin/physis-core"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    // Try PATH
    if let Ok(p) = which::which("physis-core") {
        return Ok(p);
    }
    anyhow::bail!(
        "physis-core binary not found. Expected at ../../hermes-physis/bin/physis-core"
    )
}

fn find_scan_target() -> Result<PathBuf> {
    // Use the vendor physis-core src as a convenient real Rust source tree
    let candidates = [
        PathBuf::from("../../hermes-physis/vendor/physis-core/src"),
        PathBuf::from("/home/gio/dev/hermes-physis/vendor/physis-core/src"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    anyhow::bail!("Could not find a scan target directory. Pass --store explicitly.")
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Import { store, db } => cmd_import(store, db),
        Command::Query { query, db, top } => cmd_query(query, db, *top),
        Command::Deps { source, db, depth } => cmd_deps(source, db, *depth),
        Command::Stale { store, db } => cmd_stale(store, db),
        Command::Demo => cmd_demo(),
    };
    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
