//! Spec-tree synthesis (`ontic decompose`): a paper text plus the compact
//! vault inventory goes to a spec-authoring model; the response is a set of
//! `.ont` files forming a dependency tree. Every artifact passes gen parse +
//! wish validation before anything downstream sees it (THE WALL: new
//! pen-holder, same gates). Solve order is a deterministic topological sort.

use crate::gen::{self, Gen};
use crate::forge::ForgeConfig;

/// Marker format for multi-file model responses. Kept deliberately dumb:
/// one line per boundary, no nesting, no markdown dependence beyond fences
/// being stripped by forge::extract_candidate.
pub const FILE_BEGIN: &str = "=== file: ";
pub const FILE_END_SUFFIX: &str = " ===";

/// One synthesized `.ont` file from the decomposition draft.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    /// Bare file name like `splat_alpha.ont`. Validated against a strict
    /// charset so paths stay inside the output directory.
    pub filename: String,
    pub text: String,
}

/// Parse a multi-file model response. Errors name the offending marker.
pub fn parse_tree(raw: &str) -> Result<Vec<NodeSpec>, String> {
    let mut nodes: Vec<NodeSpec> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    for line in raw.lines() {
        if let Some(rest) = line.trim().strip_prefix(FILE_BEGIN) {
            if cur.is_some() {
                return Err("file block opened while another is open".to_string());
            }
            let name = rest
                .strip_suffix(FILE_END_SUFFIX)
                .ok_or_else(|| format!("malformed file marker: `{}`", line.trim()))?
                .trim()
                .to_string();
            if !valid_filename(&name) {
                return Err(format!("invalid file name: `{}`", name));
            }
            cur = Some((name, Vec::new()));
        } else if line.trim() == "=== end ===" {
            match cur.take() {
                Some((name, body)) => nodes.push(NodeSpec {
                    filename: name,
                    text: body.join("\n").trim().to_string() + "\n",
                }),
                None => return Err("`=== end ===` without an open file block".to_string()),
            }
        } else if let Some((_, body)) = cur.as_mut() {
            body.push(line.to_string());
        }
    }
    if cur.is_some() {
        return Err("unterminated file block".to_string());
    }
    if nodes.is_empty() {
        return Err("no file blocks found in draft".to_string());
    }
    Ok(nodes)
}

/// File names must be plain identifiers with the .ont suffix — no paths,
/// no traversal, no hidden files.
fn valid_filename(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".ont") else {
        return false;
    };
    !stem.is_empty()
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validate every node: structural parse + example-vs-invariant wish gate.
/// Returns the parsed gens alongside their specs for later topo/solve use.
pub fn validate_nodes(
    nodes: &[NodeSpec],
) -> Result<Vec<(NodeSpec, Gen)>, String> {
    let mut out = Vec::new();
    for n in nodes {
        let g = gen::parse(&n.text)
            .map_err(|e| format!("{}: invalid gen: {}", n.filename, e))?;
        crate::sieve::validate_wish(&g)
            .map_err(|e| format!("{}: {}", n.filename, e))?;
        out.push((n.clone(), g));
    }
    Ok(out)
}

/// Deterministic solve order over the `use` graph (Kahn's algorithm,
/// name tie-break). A cycle is a wish error naming its members.
pub fn topo_order(gens: &[(NodeSpec, Gen)]) -> Result<Vec<usize>, String> {
    let n = gens.len();
    // Map declared dep path -> node index providing it (by gen fn path).
    let mut provider: std::collections::HashMap<String, usize> = Default::default();
    for (i, (_, g)) in gens.iter().enumerate() {
        provider.insert(g.path.clone(), i);
    }
    let mut deps_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];
    for (i, (_, g)) in gens.iter().enumerate() {
        for d in &g.deps {
            match provider.get(d) {
                Some(&j) if j != i => {
                    deps_of[j].push(i);
                    indegree[i] += 1;
                }
                Some(_) => {} // self-dep would be odd but harmless; ignore
                None => {}
                // External vault deps are fine — they already exist.
            }
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    ready.sort_by(|a, b| gens[*a].0.filename.cmp(&gens[*b].0.filename));
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop() {
        order.push(i);
        for &j in &deps_of[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                // Keep `ready` sorted by filename after each push.
                let pos = ready.partition_point(|&k| gens[k].0.filename <= gens[j].0.filename);
                ready.insert(pos, j);
            }
        }
    }
    if order.len() != n {
        let stuck: Vec<String> = (0..n)
            .filter(|i| !order.contains(i))
            .map(|i| gens[i].0.filename.clone())
            .collect();
        return Err(format!("dependency cycle among: {}", stuck.join(", ")));
    }
    Ok(order)
}

/// Compact one-line-per-entry signature inventory for decomposer prompts.
/// Context-budget guard: explicit truncation marker, never silent.
pub fn inventory_block(entries: &[String]) -> String {
    const MAX: usize = 60;
    let mut out = String::from("AVAILABLE VAULT CORES (call by full path):\n");
    for e in entries.iter().take(MAX) {
        out.push_str("  ");
        out.push_str(e);
        out.push('\n');
    }
    if entries.len() > MAX {
        out.push_str(&format!("  [+{} more]\n", entries.len() - MAX));
    }
    out
}

/// The decomposition prompt: language reference encoding every hard-won
/// lesson, the compact vault inventory, then the paper. Output contract:
/// one `=== file: <stem>.ont ===` block per function.
pub fn build_decompose_prompt(paper: &str, inventory: &str) -> String {
    let langref = include_str!("ask_langref.txt");
    format!(
        "{langref}\n\n{inventory}\n\nPAPER TEXT FOLLOWS:\n{paper}\n"
    )
}

/// Normalized signature summary used for differential draft comparison:
/// per file, the declared dep set + each gen's `path(params) -> ret`.
pub fn normalize_tree(gens: &[(NodeSpec, Gen)]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (spec, g) in gens {
        let params: Vec<String> =
            g.params.iter().map(|(n, t)| format!("%{}: {}", n, t.name())).collect();
        lines.push(format!(
            "{} | uses [{}] | {}({}) -> {} | ex={} inv={}",
            spec.filename,
            g.deps.join(","),
            g.path,
            params.join(", "),
            g.ret.name(),
            g.transparent.len(),
            g.invariants.len(),
        ));
    }
    lines.sort();
    lines
}

/// Human-readable diff of two normalized drafts (line-set symmetric
/// difference). Empty string means drafts agree.
pub fn draft_diff(a: &[String], b: &[String]) -> String {
    let mut out = String::new();
    for l in a {
        if !b.contains(l) {
            out.push_str(&format!("  only in draft A: {}\n", l));
        }
    }
    for l in b {
        if !a.contains(l) {
            out.push_str(&format!("  only in draft B: {}\n", l));
        }
    }
    out
}

/// Resolve a --spec-backend flag into a ForgeConfig for sample_text.
/// `file:<path>` short-circuits network entirely: the named file's contents
/// ARE the response (offline tests, reproducible replays).
pub enum SpecSource {
    Model(ForgeConfig),
    File(String),
}

pub fn resolve_spec_source(flag: Option<&str>, base: ForgeConfig) -> Result<SpecSource, String> {
    match flag {
        Some(s) if s.starts_with("file:") => Ok(SpecSource::File(s[5..].to_string())),
        Some(other) => {
            let mut cfg = base;
            apply_backend_name(&mut cfg, other)?;
            Ok(SpecSource::Model(cfg))
        }
        None => Ok(SpecSource::Model(base)),
    }
}

fn apply_backend_name(cfg: &mut ForgeConfig, name: &str) -> Result<(), String> {
    cfg.backend = match name {
        "openai" | "openai-compat" => crate::forge::Backend::OpenAICompat,
        "gemini" | "gemini-native" => crate::forge::Backend::GeminiNative,
        "uniform" => crate::forge::Backend::Uniform,
        _ => return Err(format!("unknown spec backend `{}`", name)),
    };
    Ok(())
}

/// Fetch one draft through the resolved source.
pub fn fetch_draft(src: &SpecSource, prompt: &str) -> Result<String, String> {
    match src {
        SpecSource::File(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("file backend {}: {}", path, e)),
        SpecSource::Model(cfg) => crate::forge::sample_text(prompt, cfg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFT_A: &str = "\
=== file: inner.ont ===
wrapping
fn Inner.double(%x: Int) -> Int
  => 2 -> 4
?? 3 -> 6
=== end ===
=== file: outer.ont ===
wrapping
use Inner.double
fn Outer.quad(%x: Int) -> Int
  hint \"double(double(x))\"
  => 2 -> 8
?? 3 -> 12
=== end ===
";

    #[test]
    fn test_parse_tree_two_nodes() {
        let nodes = parse_tree(DRAFT_A).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].filename, "inner.ont");
        assert_eq!(nodes[1].filename, "outer.ont");
        assert!(nodes[0].text.starts_with("wrapping"));
    }

    #[test]
    fn test_parse_tree_rejects_traversal_and_garbage() {
        assert!(parse_tree("=== file: ../evil.ont ===\n=== end ===").is_err());
        assert!(parse_tree("=== file: noext ===\n=== end ===").is_err());
        assert!(parse_tree("no markers at all").is_err());
        assert!(parse_tree("=== file: a.ont ===\nbody").is_err()); // unterminated
        assert!(parse_tree("=== end ===").is_err()); // no open block
    }

    #[test]
    fn test_validate_nodes_passes_good_draft() {
        let nodes = parse_tree(DRAFT_A).unwrap();
        let gens = validate_nodes(&nodes).unwrap();
        assert_eq!(gens.len(), 2);
        assert_eq!(gens[0].1.path, "Inner.double");
    }

    #[test]
    fn test_validate_nodes_rejects_invariant_violating_example() {
        let bad = "=== file: bad.ont ===\nwrapping\nfn B.f(%n: Int, %a: List<Int>) -> Int\n  | len(%a) == %n * %n\n  => 1, [7] -> 7\n?? 2, [1,2,3,4] -> 10\n=== end ===";
        let nodes = parse_tree(bad).unwrap();
        // example n=1 with 1-elem list is fine; make it violate:
        let bad = bad.replace("=> 1, [7] -> 7", "=> 2, [7] -> 7");
        let nodes = parse_tree(&bad).unwrap();
        assert!(validate_nodes(&nodes).is_err());
    }

    #[test]
    fn test_topo_order_leaves_first() {
        let nodes = parse_tree(DRAFT_A).unwrap();
        let gens = validate_nodes(&nodes).unwrap();
        let order = topo_order(&gens).unwrap();
        assert_eq!(order, vec![0, 1]); // inner before outer (name tie also ok)
    }

    #[test]
    fn test_topo_order_detects_cycle() {
        let cyc = "\
=== file: a.ont ===
wrapping
use B.g
fn A.f(%x: Int) -> Int
  => 1 -> 2
?? 3 -> 6
=== end ===
=== file: b.ont ===
wrapping
use A.f
fn B.g(%x: Int) -> Int
  => 1 -> 2
?? 3 -> 6
=== end ===
";
        let gens = validate_nodes(&parse_tree(cyc).unwrap()).unwrap();
        let err = topo_order(&gens).unwrap_err();
        assert!(err.contains("cycle"), "{}", err);
    }

    #[test]
    fn test_normalize_and_diff() {
        let gens = validate_nodes(&parse_tree(DRAFT_A).unwrap()).unwrap();
        let na = normalize_tree(&gens);
        assert!(draft_diff(&na, &na).is_empty());
        let mut nb = na.clone();
        nb.pop();
        assert!(draft_diff(&na, &nb).contains("only in draft A"));
    }

    #[test]
    fn test_inventory_block_truncates_explicitly() {
        let entries: Vec<String> = (0..70).map(|i| format!("F.f{}() -> F64", i)).collect();
        let block = inventory_block(&entries);
        assert!(block.contains("[+10 more]"));
    }
}
