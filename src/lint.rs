//! Static spec-quality lint: findings BEFORE forge spend.
//!
//! The sieve proves what a spec SAYS; lint flags what the author probably
//! did not MEAN. Advisory by default; exactly one rule is ERR-grade:
//! contradictory invariants (the integer skeleton admits no inputs), which
//! makes every probe row impossible and any solve vacuous.

use crate::gen::{Example, Gen, Value};
use crate::probes_solver;

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub rule: &'static str,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Err,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warn => write!(f, "WARN"),
            Severity::Err => write!(f, "ERR "),
        }
    }
}

/// Lint every gen in the file. Vault optional — enables the
/// duplicate-path check when present.
pub fn lint_file(gens: &[Gen], vault: Option<&crate::vault::Vault>) -> Vec<Finding> {
    let mut out = Vec::new();
    for g in gens {
        out.extend(lint_gen(g));
        if let Some(v) = vault {
            out.extend(check_duplicate_path(g, v));
        }
    }
    out
}

fn finding(sev: Severity, rule: &'static str, path: &str, detail: String) -> Finding {
    Finding {
        severity: sev,
        rule,
        path: path.to_string(),
        detail,
    }
}

/// Lint one gen.
pub fn lint_gen(g: &Gen) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(check_unsat_invariants(g));
    for ex in &g.transparent {
        out.extend(check_example_tol(g, ex));
    }
    out.extend(check_evidence_volume(g));
    out.extend(check_hint_parseable(g));
    out.extend(check_postcondition_note(g));
    out
}

/// ERR: input-side integer skeleton unsatisfiable. The solver returns
/// NoSolution only when extraction succeeded but domains emptied — i.e.
/// genuinely contradictory constraints over scalars/lengths/flags.
fn check_unsat_invariants(g: &Gen) -> Vec<Finding> {
    match probes_solver::solve(g, 4) {
        probes_solver::Outcome::NoSolution => vec![finding(
            Severity::Err,
            "unsat-invariants",
            &g.path,
            "invariants admit NO integer inputs — no probe can ever satisfy them; \
             fix or remove the contradictory constraints"
                .to_string(),
        )],
        _ => Vec::new(),
    }
}

/// WARN: float outputs pinned with ± 0 (exact cross-tier equality).
fn check_example_tol(g: &Gen, ex: &Example) -> Vec<Finding> {
    fn floaty(v: &Value) -> bool {
        match v {
            Value::Float(_) | Value::FloatList(_) => true,
            Value::Tuple(vs) => vs.iter().any(floaty),
            _ => false,
        }
    }
    if ex.tol == 0.0 && floaty(&ex.output) {
        return vec![finding(
            Severity::Warn,
            "zero-tol-float",
            &g.path,
            format!(
                "example output {} uses ± 0 on a float result; exact equality is \
                 brittle across interpreter/native tiers — declare a tolerance",
                ex.output
            ),
        )];
    }
    if ex.tol > 1.0 {
        return vec![finding(
            Severity::Warn,
            "loose-tol",
            &g.path,
            format!("tolerance {} exceeds 1.0 — evidence weaker than typical arithmetic error", ex.tol),
        )];
    }
    Vec::new()
}

/// WARN: thin evidence volume.
fn check_evidence_volume(g: &Gen) -> Vec<Finding> {
    let mut out = Vec::new();
    if g.transparent.len() < 2 {
        out.push(finding(
            Severity::Warn,
            "thin-evidence",
            &g.path,
            format!(
                "{} transparent example(s); at least 2 recommended for honest coverage",
                g.transparent.len()
            ),
        ));
    }
    if g.opaque.is_empty() {
        out.push(finding(
            Severity::Warn,
            "no-opaque",
            &g.path,
            "no opaque (??) examples — held-out overfit gate S6 runs empty".to_string(),
        ));
    }
    out
}

/// INFO: hint does not parse as a candidate expression. Hints are advice
/// (rule 12), but machine-parseable hints correlate with forge success.
fn check_hint_parseable(g: &Gen) -> Vec<Finding> {
    let mut out = Vec::new();
    for h in &g.hints {
        let expr_src = h.trim();
        let wrapped = format!("fn @probe() -> Int {{ {expr_src} }}");
        if crate::sketch::parse(&wrapped).is_err()
            && crate::sketch::parse(&format!("fn @probe() -> F64 {{ {expr_src} }}")).is_err()
        {
            // Prose hints are fine; this is informational only.
            out.push(finding(
                Severity::Info,
                "hint-unparseable",
                &g.path,
                "hint is not a parseable candidate expression (advisory)".to_string(),
            ));
        }
    }
    out
}

/// INFO: res-referencing invariants constrain the output; guarded twins
/// enforce preconditions only.
fn check_postcondition_note(g: &Gen) -> Vec<Finding> {
    let has_post = g
        .invariants
        .iter()
        .any(|inv| crate::lower::expr_refs_res(inv));
    if has_post {
        return vec![finding(
            Severity::Info,
            "postcondition-guarded-note",
            &g.path,
            "res-referencing invariant(s): the guarded .so enforces preconditions \
             only; postconditions are sieve evidence, not runtime checks"
                .to_string(),
        )];
    }
    Vec::new()
}

/// INFO: same gen path solved under multiple content addresses.
fn check_duplicate_path(g: &Gen, vault: &crate::vault::Vault) -> Vec<Finding> {
    let versions = vault
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.signature.starts_with(&format!("fn {}(", g.path)))
        .count();
    if versions > 1 {
        return vec![finding(
            Severity::Info,
            "duplicate-vault-path",
            &g.path,
            format!("{versions} vault entries share this path; resolution prefers \
                     verifiable manifests then the greatest key"),
        )];
    }
    Vec::new()
}

#[allow(unused_imports)]
use crate::lower as _lower_reexport;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen;

    #[test]
    fn test_contradictory_invariants_is_err() {
        let g = gen::parse(
            "fn C.bad(%n: Int) -> Int\n  | %n >= 5\n  | %n <= 4\n  => 5 -> 5\n",
        )
        .unwrap();
        let findings = lint_gen(&g);
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "unsat-invariants" && f.severity == Severity::Err),
            "{findings:?}"
        );
    }

    #[test]
    fn test_zero_tol_float_warns() {
        let g = gen::parse("fn Z.f(%x: F64) -> F64\n  => 0.5 -> 1.0 ± 0\n").unwrap();
        assert!(
            lint_gen(&g)
                .iter()
                .any(|f| f.rule == "zero-tol-float" && f.severity == Severity::Warn),
            "{findings:?}",
            findings = lint_gen(&g)
        );
    }

    #[test]
    fn test_thin_and_no_opaque_warn() {
        let g = gen::parse("fn T.f(%x: Int) -> Int\n  => 1 -> 2\n").unwrap();
        let rules: Vec<&str> = lint_gen(&g).iter().map(|f| f.rule).collect();
        assert!(rules.contains(&"thin-evidence"), "{rules:?}");
        assert!(rules.contains(&"no-opaque"), "{rules:?}");
    }

    #[test]
    fn test_healthy_spec_has_no_err_or_warn() {
        let g = gen::parse(
            "fn H.good(%xs: List<Int>) -> Int\n  | len(%xs) >= 1\n  hint \"fold x in %xs, acc from 0 { acc + x }\"\n  => [1, 2] -> 3\n  => [5] -> 5\n  ?? [4] -> 4\n",
        )
        .unwrap();
        let bad: Vec<_> = lint_gen(&g)
            .into_iter()
            .filter(|f| f.severity != Severity::Info)
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
    }

    #[test]
    fn test_prose_hint_is_info_only() {
        let g = gen::parse(
            "fn P.f(%x: Int) -> Int\n  hint \"sum of squares via fold\"\n  => 1 -> 2\n  ?? 3 -> 4\n",
        )
        .unwrap();
        let findings = lint_gen(&g);
        assert!(findings.iter().any(|f| f.rule == "hint-unparseable"));
        assert!(!findings.iter().any(|f| f.severity == Severity::Err));
    }

    #[test]
    fn test_postcondition_note() {
        let g = gen::parse(
            "fn R.f(%x: Int) -> Int\n  | res >= 0\n  => 1 -> 2\n  ?? 3 -> 4\n",
        )
        .unwrap();
        assert!(
            lint_gen(&g)
                .iter()
                .any(|f| f.rule == "postcondition-guarded-note")
        );
    }
}
