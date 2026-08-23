# ISSUES

## 2026-08-22 (session 1)

### 2026-08-22 — GBNF/parser parity: `%`/`@` tokenization mismatch
- **Symptom:** real Mellum2 samples died at S1 with `expected @function_name`.
- **Cause:** initial GRAMMAR allowed `ws` between `%`/`@` and the identifier
  (`"%" ident`, `"@" fname`); the Rust lexer lexes `%name`/`@name` atomically.
  Server accepted text the parser rejects.
- **Fix:** single-token grammar rules (`pid`, `atname`); no internal ws.
- **Lesson:** one grammar, two consumers — every grammar change needs a
  parse-parity test on both sides.

### 2026-08-22 — Strict grammar from token 0 loops on whitespace
- **Symptom:** grammar-constrained completions returned only newlines; model
  never reached code.
- **Cause:** `root ::= ws "fn" ...` makes unbounded leading whitespace a
  valid derivation; a Thinking-model variant happily wanders there forever
  (GBNF is a filter, not a guide).
- **Tried:** lazy grammar with word trigger `fn` + preserved_tokens. Works,
  but models that ramble before the trigger burn the whole n_predict budget.
- **Fix (kept):** prefill strategy — prompt ends with literal `fn @`, grammar
  root starts at the function name, strict from generated token 0. Prose is
  physically unreachable and Mellum2's completion-style strengths fit.

### 2026-08-22 — Shared-server connection resets mid-batch
- **Symptom:** `malformed status line ''` / reset by peer during parallel
  sampling; later both llama-server instances disappeared entirely.
- **Cause:** endpoint shared with other workloads; slot exhaustion resets
  keep-alive connections; servers eventually stopped externally.
- **Fix:** per-sample retry (3 attempts) with fresh connection + backoff;
  retryable 5xx; abandoned samples don't kill the batch; worker count via
  `ONTIC_FORGE_WORKERS` (default 2, politeness).

### 2026-08-22 — Rust bugs caught by tests before any run
- wish param splitting dropped everything after the first comma (missing
  `cur.clear()`); or-pattern `If(c,..)|Let(_,..)` illegal binding; `Tok`
  borrowed source lifetimes (fixed with 'static keyword literals); short-circuit
  test itself was wrong (a==0 forces RHS eval); probe-stage kill test needed a
  candidate that passes examples but violates invariants elsewhere.

### 2026-08-22 — Parity gap misattribution corrected
- First table blamed missing overflow flags entirely. Half the gap was the C
  reference's CONSTANT trip count enabling full unroll; Ontic's bound is
  runtime (memref dim). Correction recorded in
  benchmarks/results/2026-08-22-overflow-parity.md; specialization queued
  for eclipse-track P2.

### 2026-08-22 — llc-only pipeline skipped LLVM middle end
- mlir-translate→llc runs codegen passes only; clang references get unroll/
  vectorize/reassociate. Fixed by inserting `opt -O3` between translate and
  llc in pipeline::mlir_to_llvmir.

### 2026-08-22 — bench harness timed its own buffer initialization
- First native numbers (~5.5k ns) included per-iteration buffer init inside
  the timed loop. Moved outside; real numbers ~10x lower. Lesson: audit the
  harness before believing any timing.

### 2026-08-22 — Qwen parrots prompt meta-syntax
- Rules containing placeholders (`lowercase_name(%a: T, ...)`) were copied
  verbatim into candidates. Fix: concrete few-shot example function instead
  of abstract format description. Lesson: never show meta-tokens to small
  models under grammar constraints.

### 2026-08-22 — rms.ont unsolved by forge (open)
- Stats.meansqdev requires two dependent folds (mean, then deviation sum) +
  empty guard. All candidates honestly rejected at S3 with near-misses.
  Not a sieve bug — a capability boundary being measured honestly.

### 2026-08-22 — C23 reserved word broke generated drivers
- Struct member named `aligned` compiled as an alignment SPECIFIER under
  clang 18's default std, silently dropping the member. Generated C now uses
  base/data/off/size/stride field names. Lesson: audit GENERATED code against
  the newest compiler dialects, not just our grammar.
