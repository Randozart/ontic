# Str Native ABI — Plan (2026-09-03)

**Date:** 2026-09-03
**Timestamp:** 2026-09-03 13:10
**Status:** done (2026-09-03). Commits `c7b3fe0` (lowering+S2),
`9bc7443` (driver+parity), `70fb248` (header/shim), `2b963f0` (probes),
`a320e75` (S typedef fix). 194/194 tests green; functional solve verified.

Completes survey F3 (`2026-09-02-next-work-survey.md`): Str kernels can
currently solve in interpret mode only; S2 (`check::reject_str_positions`)
fail-closes any candidate with a Str param/return with "opaque FFI ABI
pending". The grammar, oracle, builtins, and par-harness sides are done;
this arc adds the **native ABI** so Str kernels vault with real artifacts.

## ABI decision (locked in plan, per GR: patching is unacceptable)

**Str = `(char* data, long len)`** — pointer + length, NOT C strings:

1. `Value::Str` is a Rust `String` (UTF-8, may contain NULs, may be empty).
   NUL-terminated `const char*` cannot represent any of that.
2. `str_len` must equal **byte length** (`s.len()`), matching the oracle
   (interp.rs:99 `s.len() as i64`). A C `strlen` would disagree on NUL
   bytes → parity gate fails. Length field is authoritative.
3. No allocation needed: the driver embeds the UTF-8 bytes in a C array.
4. Matches the survey's own P6 wording: "(char* data, long len) per Str
   param".

**Return Str** = same 2-field struct by value (LLVM's `{ptr, i64}` return
matches `typedef struct { char* data; long len; } S;` on SysV, exactly as
tuple returns already do).

**Builtin lowering (MLIR, no externs needed):**
- `str_len(s)` → `memref.dim` of the allocated 1D `memref<?xi8>` →
  `arith.index_cast` to i64. (Same shape as `len()`, lower.rs:786.)
- `str_eq(a, b)` → length compare (`arith.cmpi eq` on the two dims) AND
  per-byte loop (`scf.for` over `min(len_a, len_b)` comparing loads;
  if lengths differ, result is 0 without the loop). Emitted i64 1/0
  (Bool convention, like `cmpi` results).

## File-by-file changes

### 1. `src/lower.rs`

- **`mlir_param_type`** (L286): `Ty::Str => ""` → `Ty::Str => "memref<?xi8>"`.
- **`mlir_ret_type`** (L295): `Ty::Str => Err(...)` → `Ty::Str => Ok("memref<?xi8>")`
  — but Str *returns* are a by-value 2-field struct `{char*, long}` in C.
  In MLIR the function returns a 1D memref (like other lists). The C
  header/driver translate memref→`S` struct. (Consistent with how list
  returns work: MLIR `memref<?xi64>` ↔ C `MR` 5-field struct.)
- **`emit_fn_tier` / `emit_fn`** — params: Str params arrive as
  `memref<?xi8>` arguments (no change needed; the param binding just gets
  the SSA name as today).
- **Str literal emission**: candidates can't *construct* Str values
  (no grammar form), so no `StrLit` emission needed. A Str param is
  passed in as a memref; bodies can only `str_len`/`str_eq` it.
- **`emit_builtin`** (L868): replace
  ```rust
  Builtin::StrLen | Builtin::StrEq => Err(...)
  ```
  with:
  - `StrLen`: emit `memref.dim` + `index_cast` (copy the `len()` shape,
    L786-796).
  - `StrEq`: needs binary lowering — route through `emit_cmp`-style
    helper. Actually `str_eq` is a Builtin2 (like `Index`), so it's
    handled in the `Expr::Builtin2` arm (L534 area), not `emit_builtin`.
    New `fn emit_str_eq(a_ssa, b_ssa, em) -> String`:
    1. `dim_a = memref.dim(a, 0)`, `dim_b = memref.dim(b, 0)`
    2. `len_eq = arith.cmpi eq, dim_a, dim_b` (i1)
    3. `scf.if len_eq -> (i64) { loop... } else { yield 0 }`
    4. loop: `scf.for i = 0 to dim_a step 1` → load `a[i]`, load
       `b[i]`, `cmpi ne`, if ne → `scf.yield 0` early; else continue.
       After loop → `scf.yield 1`.
- **`c_ret_ty`** (L2634): `Ty::Str => Err(...)` → `Ty::Str => Ok("S")`
  where `S` is `typedef struct { char* data; long len; } S;` (emitted in
  header alongside `MR`).
- **`emit_header`** (L2715): Str param → `char* {n}_d, long {n}_l`;
  Str return → `S`. Add `S` typedef to the header preamble when any
  kernel uses Str.
- **`emit_shim_c`** (L3172): Str param → `char* {n}_d, long {n}_l`;
  guard conjuncts over Str params can't be translated (no C expression
  for "string contents") — degrade to `true` (same as complex invariants).
- **`c_guard_sentinel`** (L3127): `Ty::Str => "0"` (NULL data pointer).
- **`c_guard_printf_spec`** (L3136): `Ty::Str => "%s"` (print the data,
  bounded by len for safety — actually just print len for diagnostics).

### 2. `src/check.rs`

- **`reject_str_positions`** (L61): remove the Str rejection. Keep the
  tuple rejection. Str params and returns now pass S2.
- No other check changes: `infer` already handles Str (Builtin::StrLen
  → Int, Builtin::StrEq → Bool).

### 3. `src/pipeline.rs`

- **`CK` enum** (L92): add `CK::Str` — `proto() => "char*, long"`.
- **`RetSpec`** (L76): add `RetSpec::Str`.
- **`eval_c_source`** (L399):
  - `CK::Str` param: emit
    ```c
    static char s{N}[] = "{escaped_utf8}";
    ```
    and pass `s{N}, {len}`.
  - `RetSpec::Str` return:
    ```c
    S r = {fname}({args});
    // print bounded: fwrite(r.data, 1, r.len, stdout)
    ```
    The eval harness parses stdout as the result — for Str returns,
    compare the raw bytes (not numeric parse). This requires a small
    change to `eval_native`: when `ret == RetSpec::Str`, return the
    stdout string instead of parsing floats.
- **`eval_native`** (L568): add a `RetSpec::Str` arm that returns the
  raw stdout as a `String` (new return type variant, or a separate
  `eval_native_str` fn to avoid changing the existing signature).
- **`differential_parity`** (main.rs:689): add a `Ty::Str` ret arm —
  compare oracle `Value::Str` vs native stdout bytes exactly.
- **`bench_c_source_from_row`** (L260): `CK::Str` param — same as eval
  (embed bytes array). RetSpec::Str — measure the call, print ns.

### 4. `src/main.rs`

- **`differential_parity`** (L689):
  - `ret_spec` match: `Ty::Str => RetSpec::Str`.
  - `kinds` match: `Ty::Str => CK::Str`.
  - param stream: `Value::Str(s)` → push `(s, s.len())` into a new
    `lists_str: Vec<(String, usize)>` or a parallel `strs: Vec<String>`.
  - comparison: `RetSpec::Str` → byte-exact compare.
- **`proven_equivalence_gate`** (L~750): unchanged (Str returns are
  outside the proven subset by design — `subset_ok` requires Int/Bool).
- **`emit_and_store`**: no change (the pipeline is shape-agnostic).

### 5. `src/interp.rs`

- No changes. The oracle already handles `Value::Str` for `str_len` and
  `str_eq` (L99, L327).

## Tests (per Golden Rule 9: tests or it doesn't exist)

| Test | Location | What it proves |
|------|----------|----------------|
| `str_len_lowers_to_memref_dim` | lower.rs | `str_len(%s)` emits `memref.dim` + `index_cast` |
| `str_eq_lowers_to_byte_loop` | lower.rs | `str_eq(a, b)` emits length check + `scf.for` byte compare |
| `str_param_emits_memref_xi8` | lower.rs | Str param → `memref<?xi8>` in MLIR |
| `str_return_emits_memref_xi8` | lower.rs | Str return → `memref<?xi8>` in MLIR |
| `check_allows_str_params` | check.rs | S2 no longer rejects Str params/returns |
| `differential_parity_str` | pipeline.rs or main.rs | Str kernel: oracle `Value::Str("hello")` == native stdout `"hello"` |
| `str_eq_parity` | pipeline.rs | `str_eq("ab", "ab")` → 1; `str_eq("ab", "ac")` → 0, both native+oracle |
| `str_len_parity` | pipeline.rs | `str_len("hello")` → 5, native == oracle |
| `header_emits_str_abi` | lower.rs | `.h` has `char* s_d, long s_l` params + `S` typedef |
| `shim_handles_str` | lower.rs | shim compiles (clang smoke) with Str params |

## Sequencing

- **Commit 1** (`feat(str): MLIR lowering + check unblock`):
  lower.rs (mlir types, StrLen/StrEq emission), check.rs (remove Str
  rejection). Tests: 4 lower + 1 check.
- **Commit 2** (`feat(str): native driver + parity`):
  pipeline.rs (CK::Str, RetSpec::Str, eval_c_source, eval_native),
  main.rs (differential_parity Str arm). Tests: 3 parity.
- **Commit 3** (`feat(str): header/shim C ABI`):
  lower.rs (c_ret_ty, emit_header, emit_shim_c, c_guard_sentinel).
  Tests: 2 header/shim.
- **Commit 4** (`docs(str): docs + changelog`):
  README, CHANGES.md, plan status update.

## Risk / honest scope

- **NUL bytes in strings**: the C `char[]` initializer can embed NULs
  (`"\0"`) but the array length is the C sizeof, not a strlen. The `len`
  field is authoritative, so this is correct. Test with a NUL-containing
  string to prove it.
- **Empty strings**: `static char s0[] = "";` is valid C (zero-length
  array is a GCC/Clang extension; use `static char s0[1] = "";` or
  `static char s0[] = {0};` to be safe). The len field is 0.
- **Non-ASCII UTF-8**: `Value::Str` is a Rust String; the C array holds
  the UTF-8 bytes. `str_len` returns byte length (matching oracle). No
  issue.
- **mlir-opt validation**: `memref<?xi8>` is valid MLIR; the par-harness
  validates the emitted IR. If mlir-opt is absent, validation is skipped
  (same as today for non-Str kernels).
