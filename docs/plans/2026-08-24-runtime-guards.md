# Runtime Guards — Twin Artifacts

**Date:** 2026-08-24
**Author:** session
**Status:** Approved
**Depends on:** contracted headers (completed); flat-memref ABI v1; vault infrastructure.

## 1. Problem

Plain C / ctypes / Rust FFI consumers call kernel symbols directly. If the
caller passes a value violating the sieve-proven preconditions (e.g. `geo(5.0)`
when the invariant is `%r >= 0.0 && %r < 1.0`), the output is **silent
undefined behavior**. The sieve proved correctness only for valid inputs.

C++26 `pre()` annotations exist in the `.hpp` but that requires a
C++26-contract-enabled compiler. The vast majority of consumers (Python ctypes,
Rust `extern "C"`, plain C) have no guard at all today.

## 2. Design

### 2.1 Twin artifacts per vaulted kernel

| Artifact | Contents |
|---|---|
| `lib<name>-<key8>.so` | **Raw.** Current behavior. Zero overhead. Symbol: `<name>` = LLVM kernel. |
| `lib<name>-<key8>.guarded.so` | **Guarded.** `<name>` = C shim; `<name>__raw` = LLVM kernel. Policy + error API. |
| `<name>-<key8>.hpp` | C++26 twin, unchanged (existing pre() under ONTIC_CONTRACTS). |

Guarded `.so` exports five symbols beyond the kernel:

```c
// Policy constants
#define ONTIC_POLICY_ABORT 0   // default: print message, abort()
#define ONTIC_POLICY_TRAP  1   // store message, return sentinel

// Thread-local error state
const char *ontic_last_error(void);           // NUL or UTF-8 msg
void        ontic_last_error_clear(void);

// Policy control
void ontic_set_violation_policy(int policy);
int  ontic_violation_policy(void);
```

Error buffer: 256 bytes, thread-local, NUL-terminated.

Sentinel values on TRAP policy:
- F64 kernels return `NAN`
- Int/Bool kernels return `LONG_MIN` (0x8000000000000000)

### 2.2 Shim structure

Generated C file (one per kernel, compiled into the guarded `.so`):

```c
#include <math.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>

// Thread-local storage
static _Thread_local char tl_error[256];
static _Thread_local int  tl_policy = 0; // ONTIC_POLICY_ABORT

const char *ontic_last_error(void) {
    return tl_error[0] ? tl_error : NULL;
}
void ontic_last_error_clear(void) { tl_error[0] = '\0'; }
void ontic_set_violation_policy(int p) { tl_policy = p; }
int  ontic_violation_policy(void) { return tl_policy; }

// Raw kernel (MLIR-emitted)
extern double geo__raw(double r);

// Shimmed public symbol
double geo(double r) {
    if (r < 0.0 || r >= 1.0) {
        snprintf(tl_error, sizeof(tl_error),
                 "ontic: EvalSeries.geo_sum pre violated: "
                 "%r >= 0.0 && %r < 1.0 (got r=%.17g)", r);
        if (tl_policy == 0) { // ABORT
            fprintf(stderr, "%s\n", tl_error);
            abort();
        }
        return NAN;
    }
    return geo__raw(r);
}
```

For list params (flat-memref ABI), shape checks map `len(a) == len(b)` to
the `size` field of the 5-tuple (`_a, _b, _o, _s, _st` → `_s` is length).
Conjuncts over `len()` are translated using existing `contract_text` with
a small extension: `len(x)` in the invariant maps to `x_s` (the size
argument).

### 2.3 Invariant translation

`contract_text` (lower.rs) already handles: `<`, `<=`, `>=`, `>`, `==`, `!=`,
`&&`, `||`, `!`, scalar arithmetic, `len(x)`. These map directly to C.

Extension for guarded path: `contract_text_c_guard(invariant, params) -> Option<String>`
differs from `contract_text` only in how `len(x)` resolves — it produces the
flat-memref size argument name `x_s` instead of `.size()` in C++ syntax.

Conjuncts that cannot be translated degrade to a comment in the guard message
field (matching existing hpp behavior for untranslated invariants). Never
silently dropped.

### 2.4 MLIR emission for guarded mode

When building the guarded variant, `emit_func` names the MLIR function
`<name>__raw` instead of `<name>`. The public symbol `<name>` is provided by
the compiled C shim. This avoids any symbol collision between raw and guarded
`.so` files.

### 2.5 Build wiring

`build_shared_so` gains an optional `shim_source: Option<&str>` parameter.
When provided:

1. Write shim to a temp `.c` file
2. Compile: `llc <mlir.ll> -o <tmp.o>` (produces the raw kernel as `<name>__raw`)
3. Compile shim: `cc -c <shim.c> -o <shim.o>`
4. Link: `cc -shared -o <guarded.so> <tmp.o> <shim.o> -lm -lpthread`

When `shim_source` is None, current behavior preserved exactly.

### 2.6 Vault entry changes

```json
{
  "name": "EvalSeries.geo_sum",
  "key": "90663fd5...",
  "raw_lib": "libgeo-90663fd5.so",
  "guarded_lib": "libgeo-90663fd5.guarded.so",
  "guarded_shim": "geo-90663fd5.guarded.c"
}
```

`ontic vault ls` shows both artifacts. `ontic unpack` extracts both `.so`
files when the guarded variant exists.

### 2.7 Header changes

`emit_header` gains a guarded section when the guarded artifact is present:

```c
// ONTIC GUARDED SECTION — link libgeo-90663fd5.guarded.so for runtime checks
// Default policy: ABORT on violation. Switch to TRAP at runtime:
//   ontic_set_violation_policy(ONTIC_POLICY_TRAP);
//   double result = geo(0.5);           // guarded
//   double raw    = geo__raw(0.5);      // unchecked, same kernel
//
const char *ontic_last_error(void);
void        ontic_last_error_clear(void);
void        ontic_set_violation_policy(int policy);
int         ontic_violation_policy(void);
#define    ONTIC_POLICY_ABORT 0
#define    ONTIC_POLICY_TRAP  1
```

`<name>__raw` declaration included only in the guarded header variant.
Raw `.h` file (existing) remains unchanged.

## 3. Phases

### P1 — Shim emitter (`lower.rs`)

**Functions to add:**

| Function | Purpose |
|---|---|
| `emit_shim_c(name, params, ret, key8, invariants)` | Generate complete C shim source |
| `contract_text_c_guard(invariant, params)` | Translate invariant → C expression (len → size field) |
| `c_guard_sentinel(ty) -> &'static str` | Return "NAN" / "LONG_MIN" per type |

**Intent comments required on every new function.**
Cyclomatic ≤15. Nesting ≤6.

**Verification:**
- Unit tests: shim text contains expected conjuncts for `geosum` (scalar list)
- Unit tests: `contract_text_c_guard` handles `len(a) == len(b)` → `a_s == b_s`
- Unit tests: untranslated conjuncts produce `// untranslated:` comments
- `cargo test --lib` green

### P2 — Build wiring (`pipeline.rs`, `main.rs`)

**Changes:**
- `build_shared_so`: add `shim_source: Option<&str>`, dual-compile when present
- `emit_func` / call sites: when `guard=true` suffix name with `__raw`
- `cmd_solve`: build raw + guarded; guarded failure = warning, raw always lands
- `cmd_lib_build`: same dual-build pattern
- Vault: store `guarded_lib` + `guarded_shim` fields; shim `.c` persists for repro
- `ontic vault ls`: show both artifacts

**Verification:**
- geosum end-to-end: both `.so` files exist, raw `geo` and guarded `geo`
- `geo(0.75)` via guarded → bit-exact same result as raw
- `geo(5.0)` via guarded → abort fires (exit code 134); `geo__raw(5.0)` → UB (sanitizer catches)
- `.so` sizes comparable (shim adds <2KB)

### P3 — Header + Python (`lower.rs`, pyous path)

**Changes:**
- `emit_header`: guarded-section decls when artifact present
- pyous/ctypes: after FFI call, if `ontic_last_error()` returns non-NULL → raise `OnticViolation(msg)` (TRAP mode only; ABORT mode never reaches Python)
- `ontic venv check --guarded`: verify guarded `.so` links clean

**Verification:**
- ctypes: `geo(0.75)` → correct value, no error
- ctypes: switch to TRAP → `geo(5.0)` → `OnticViolation` with evidence string
- ctypes: `ontic_last_error()` returns None after `ontic_last_error_clear()`
- `cargo test --lib` + full `ontic eval` suite green (sieve untouched)

### P4 — Bench + docs

**Changes:**
- ns/call comparison: raw vs guarded (passing input, expect ≤2ns delta)
- `docs/GUARDS.md`: ABI spec, policy semantics, GR11 framing ("guards are a declared tier, separate artifact")
- `docs/IDENTITY.md`: updated guard bullet

**Verification:**
- Benchmark results recorded
- Docs render clean
- `make test` full green

## 4. Golden Rule compliance

| Rule | Status |
|---|---|
| GR1 — Model output only as candidate | ✓ Shim emitter is pure Rust; zero model involvement |
| GR11 — Speed requires declaration | ✓ Twin artifacts: raw = zero overhead, guarded = opt-in tier. Consumer chooses at link time. |
| GR12 — Hints are advice, evidence is truth | ✓ Violation message carries actual param values as evidence. Policy is runtime-configurable. |
| Interpreter oracle | ✓ Guards never alter semantics on valid inputs; interp behavior unchanged |

## 5. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `contract_text` subset misses a conjunct | Degrades to comment in guard + `"untranslated"` metadata. Never silent. Matches hpp behavior. |
| Shim overhead on hot loops | Twin artifact design: hot path links raw `.so`, zero overhead. Guarded is opt-in. |
| Thread-local not portable (C11 _Thread_local) | Fallback: `__thread` (GCC/Clang), `__declspec(thread)` (MSVC). All major compilers support one form. |
| Symbol collision if raw + guarded loaded in same process | Guarded lib exports `name` (shim) + `name__raw` (kernel). Raw lib exports `name`. Dynamic linker picks last-loaded. Document: never load both simultaneously. |
| Guarded `.so` build fails but raw succeeds | Non-fatal: raw always lands. Warning printed. Vault records `"guarded": null`. |

## 6. Out of scope (v2)

- Postcondition checking (return value validation)
- Shape-only guard mode (skip scalar checks for perf-sensitive paths)
- `ontic guard config` CLI for per-kernel policy overrides
- Integration with Praetor's verification path (guards are runtime, Praetor is static)
