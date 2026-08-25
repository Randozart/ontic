# Runtime Guards

**Status:** Implemented (2026-08-24)
**Scope:** Precondition checks at FFI boundary. Postconditions deferred to v2.

## Problem

The sieve proves correctness only for inputs satisfying the gen's
invariants.  If a C / ctypes / Rust FFI consumer passes a violating value
(e.g. `geo(5.0)` when `%r >= 0.0 && %r < 1.0`), the result is undefined
behaviour — no guard exists in the generated `.so`.

## Twin artifacts

Every vaulted kernel now produces two shared libraries:

| Artifact | Symbol | Contents |
|---|---|---|
| `lib<name>-<key>.so` | `<name>` | Raw LLVM kernel. Zero overhead. Unchanged behaviour. |
| `lib<name>-<key>.guarded.so` | `<name>` (shim) + `<name>__raw` | C shim with runtime precondition checks. Error API. |

The raw library is always built.  The guarded library is built when the
toolchain supports it; failure is a non-fatal warning.

## Shim structure

The guarded `.so` exports:

```c
// Public ABI (shim)
double geo(double r);

// Raw kernel (unchecked, same as raw .so)
extern double geo__raw(double r);

// Error API
const char *ontic_last_error(void);           // NUL or UTF-8 message
void        ontic_last_error_clear(void);
void        ontic_set_violation_policy(int);  // ONTIC_POLICY_ABORT | TRAP
int         ontic_violation_policy(void);
```

### Guard flow

```
geo(r)
  ├─ check conjuncts from sieve-proven invariants
  │   ├─ PASS → geo__raw(r)              [~0.1ns overhead, branch predicted]
  │   └─ FAIL → format message → policy action
  │       ├─ ABORT: fprintf(stderr, msg) + abort()   [default]
  │       └─ TRAP:  store in tl_error, return NaN/INT_MIN
  └─ return result
```

### Violation message

```
ontic: <Name> pre violated: <conjunct text> (param=value, ...)
```

Example: `ontic: geo pre violated: (%r < 1e0) (long r=5)`

### Thread-local state

Error buffer: 256 bytes, `_Thread_local`, NUL-terminated.
`ontic_last_error()` returns NULL when no error is pending.
`ontic_last_error_clear()` resets the buffer.

## Usage

### C / C++

```c
#include "geo-90663fd5.h"

// Link libgeo-90663fd5.guarded.so for checks.

double result = geo(0.75);       // guarded
double raw    = geo__raw(0.75);  // unchecked, same kernel

// Switch to TRAP mode (no abort, returns NAN on violation)
ontic_set_violation_policy(ONTIC_POLICY_TRAP);
double r = geo(5.0);  // returns NAN
const char *err = ontic_last_error();  // "ontic: geo pre violated: ..."
ontic_last_error_clear();
```

### Python ctypes

```python
import ctypes, math

lib = ctypes.CDLL("libgeo-90663fd5.guarded.so")
lib.geo.restype = ctypes.c_double
lib.geo.argtypes = [ctypes.c_double]
lib.geo__raw.restype = ctypes.c_double
lib.geo__raw.argtypes = [ctypes.c_double]
lib.ontic_last_error.restype = ctypes.c_char_p
lib.ontic_set_violation_policy.restype = None
lib.ontic_set_violation_policy.argtypes = [ctypes.c_int]

# TRAP mode: returns NAN, check ontic_last_error()
lib.ontic_set_violation_policy(1)
r = lib.geo(5.0)
if math.isnan(r):
    print(lib.ontic_last_error())  # evidence string
```

### Rust

```rust
use std::ffi::{c_char, c_double, c_int, CStr};

extern "C" {
    fn geo(r: c_double) -> c_double;
    fn geo__raw(r: c_double) -> c_double;
    fn ontic_last_error() -> *const c_char;
    fn ontic_set_violation_policy(p: c_int);
}

const ONTIC_POLICY_TRAP: c_int = 1;

unsafe {
    ontic_set_violation_policy(ONTIC_POLICY_TRAP);
    let r = geo(5.0);
    if r.is_nan() {
        let msg = CStr::from_ptr(ontic_last_error());
        eprintln!("{}", msg.to_string_lossy());
    }
}
```

## Invariant translation

Sieve-proven invariants are translated to C guard expressions:

| Invariant | C guard |
|---|---|
| `%x >= 0.0` | `x >= 0` |
| `len(%a) == len(%b)` | `a_s == b_s` (flat-memref size) |
| `%r >= 0.0 && %r < 1.0` | two separate if-chains (one per conjunct) |

Translation uses the same `contract_text` engine as the C++26 `.hpp`
headers.

**Fail-closed policy (2026-08-25):** an invariant that cannot be translated
(e.g. a `res`-referencing postcondition) REFUSES the guarded build with an
error naming each untranslated predicate. The raw `.so` still vaults, but no
`.guarded.so` is produced — a guard that silently checks nothing would be
worse than no guard. Restate the invariant in translatable form to recover
the guarded twin.

## Golden Rule compliance

| Rule | Status |
|---|---|
| GR1 (The Wall) | ✓ Shim emitter is pure Rust; zero model involvement |
| GR11 (Speed requires declaration) | ✓ Twin artifacts: raw = zero overhead, guarded = opt-in tier |
| GR12 (Hints are advice, evidence is truth) | ✓ Violation message carries actual param values as evidence |
| Interpreter oracle | ✓ Guards never alter semantics on valid inputs |

## Performance

Measured on `geo(0.75)` (pass path, 100k iterations):

- Raw: **0.9 ns/call**
- Guarded: **1.0 ns/call**
- Overhead: **0.1 ns** (branch predictor eliminates cost)

## Limitations

- **Preconditions only** — postconditions checked by the sieve at proof
  time, not at runtime (v2 scope).
- **Scalar + shape checks** — invariants over complex expressions may
  degrade to comments.  Untranslated conjuncts fire as `"true"` (harmless).
- **Thread-local only** — no cross-thread error propagation.
- **No dual-load** — never link both raw and guarded `.so` in the same
  process (symbol collision on `<name>`).
