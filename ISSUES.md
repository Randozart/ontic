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
