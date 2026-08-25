<p align="center">
  <img src="ontic-logo.svg" width="128" height="128" alt="Ontic logo"/>
</p>

# Ontic

**A DSL whose products are verified native libraries.**

You write a specification. A local transformer proposes implementations.
A deterministic seven-stage sieve proves them against your evidence.
What survives is machine code you can call from any language.

---

## Writing gens

A gen is a specification: signature, invariants, evidence. You never write
implementation code — the transformer proposes candidates and the sieve
discards everything that doesn't hold up.

### A reduction

```ont
fn Stats.rms(%xs: List<F64>) -> F64
  | %res >= 0.0
  hint "root mean square: sqrt(sum(x*x)/len)"
  => [2.0, 8.0] -> 5.830951894845301 ± 1e-9
  => [1.0, 0.0] -> 1.0 ± 1e-9
  ?? [3.0] -> 1.7320508075688772 ± 1e-12
```

| Marker | Meaning |
|--------|---------|
| `\|%res >= 0` | invariant: guides forge AND bounds probe oracle |
| `=> [2,8] -> 5.83 ± tol` | **transparent** evidence: forge sees this |
| `?? [3] -> 1.73 ± tol` | **opaque** evidence: held out; overfit killer |
| `hint "..."` | author guidance for the forge (advice, never evidence) |

### A list transform

```ont
fn Transform.translate_scale(%pts: List<F64>, %s: F64, %off: F64) -> List<F64>
  hint "scale then translate: map(%v in %pts) { %v * %s + %off }"
  => [1.0, 2.0], 2.0, 10.0 -> [12.0, 14.0]
  => [3.0], 1.0, 0.0 -> [3.0] ± 0
  ?? [0.0, 0.0], 1.0, 7.0 -> [7.0, 7.0] ± 1e-9
```

### A matrix operation

```ont
fn Linalg.matvec(%mat: List<F64>, %vec: List<F64>) -> List<F64>
  hint "row-major NxN; use map over row indices, inner fold per row"
  => [1.0, 0.0, 0.0, 1.0], [3.0, 7.0] -> [3.0, 7.0] ± 1e-12
  => [2.0, 0.0, 0.0, 3.0], [1.0, 1.0] -> [2.0, 3.0] ± 1e-12
  ?? [1.0, 2.0, 3.0, 4.0], [1.0, 1.0] -> [3.0, 7.0] ± 1e-12
```

### What happens when you solve

```bash
ontic solve stats.ont --sampler-backend gemini --samples 8
```

```
forge     : 8 candidates proposed by Gemini flash-lite under GBNF constraints
S1 parse  : grammar-constrained sketch accepted
S2 types  : typechecker validates body against signature
S3 visible: transparent evidence matches exactly (± tolerance)
S4 hidden : opaque evidence matches — overfit rejected
S5 probes : 256+ random inputs checked against invariants
S6 shape  : constant-guard ratio below threshold — no memorization
S7 bench  : compiled performance ranking among survivors

VAULTED Stats.rms (b388bfeb...)
HEADER  : .ontic/vault/rms-b388bfeb.h
LIB     : .ontic/vault/librms-b388bfeb.so
OUS     : .ontic/vault/rms-b388bfeb.ous
```

Three artifacts produced per solved gen: C header, shared library, and a
single-file `.ous` bundle containing everything needed to use the kernel
without Ontic installed.

---

## Consuming verified kernels

### From C

```c
#include "rms-b388bfeb.h"    /* double rms(void*a, void*b, long o, long s, long st); */

double buf[2] = {2.0, 8.0};
double result = rms(buf, buf, 0, 2, 1);   /* → 5.831 */
```

### From Python

[`pip install pyous`](https://pypi.org/project/pyous/)

PyOus loads `.ous` kernels into Python via ctypes with zero-copy numpy
support. Three API levels, all producing the same native callables.

#### Typed builder

```python
import pyous as po

rms = po.define(
    name="Stats.rms",
    params={"xs": po.List[po.F64]},
    ret=po.F64,
    evidence=[
        ([2.0, 8.0], 5.830951894845301),
        ([1.0, 0.0], 1.0),
    ],
    opaque=[
        ([3.0], 1.7320508075688772),
    ],
)
rms([2.0, 8.0])  # → 5.830951894845301 at native speed
```

#### Decorator

```python
@po.kernel(evidence=[([21], 42)])
def twice(n: int) -> int:
    """Double the input."""
```

Type hints auto-map (`float→F64`, `list[float]→List<F64>`). Docstring
becomes forge guidance. Body is documentation only — never executed.

#### Pure data

```python
rms = po.gen({
    "name": "Stats.rms",
    "params": {"xs": "List<F64>"},
    "ret": "F64",
    "invariants": ["res >= 0"],
    "evidence": [([2.0, 8.0], 5.831)],
})
```

### Cache-first economics

Cold genesis (forge → sieve → compile) costs seconds to minutes depending
on sampler. Warm cache hits bind in microseconds. Every kernel persists in
the vault — solve once, use forever.

---

## THE WALL

> **The transformer generates candidates. Everything else is deterministic
> Rust.** The model proposes; it never validates, ranks, or decides.

Trust scales with sieve strength, not model strength. Every capability gain
came from verifier work while the model stayed frozen.

## Overflow semantics

Integer arithmetic is checked: overflow kills candidates in the sieve and
traps natively. Keep intermediate values in range via invariants or small
domains. A future `proven` tier (M3) plans Z3 absence proofs for flag-free
codegen.

Speed requires declaration.

## Setup

Requires: Python ≥ 3.8, system LLVM toolchain (`llc`, `clang`), Rust ≥ 1.70.

```bash
git clone https://github.com/Randozart/ontic.git
cd ontic
cargo build --release
cargo test --lib
```

Environment variables:

| Variable | Purpose | Default |
|----------|---------|---------|
| `ONTIC_BIN` | Path to ontic binary | PATH lookup |
| `ONTIC_VAULT` | Vault directory | `.ontic/vault` |
| `ONTIC_FORGE` | llama-server endpoint | `127.0.0.1:8279` |
| `ONTIC_SAMPLER` | Backend: llama/openai/gemini/uniform | `llama` |
| `ONTIC_MODEL` | Model name (cloud) | varies by provider |
| `GEMINI_API_KEY` | Gemini API key | required for cloud |
| `ONTIC_MLIR_BIN` | LLVM toolchain directory | `/usr/lib/llvm-18/bin` |
| `ONTIC_FORGE_WORKERS` | Parallel sampling workers | `2` |


## From paper to pipeline

`ontic decompose` turns research-paper text into a tree of `.ont`
specifications, gates it through one human confirmation, then solves the
tree leaves-first — verified kernels composing into verified kernels:

```bash
curl -s https://arxiv.org/html/2308.04079v1 | \
  python3 -c "import sys,html,re; t=sys.stdin.read(); \
    t=re.sub(r'<[^>]+>',' ',t); print(html.unescape(t))" > gs.txt
ontic decompose gs.txt --spec-backend gemini --outdir tree/
# review the PROPOSED TREE table, confirm, and each node is solved+vaulted
```

The vault records which cores fed which solves (`ontic vault` shows reuse
counts), so every paper deposits reusable math for the next one. See
`docs/reports/2026-08-23-research-report-p3.md` for a full run against the
3D Gaussian Splatting paper.

Export/import whole slices of the vault as `.nous` packages
(*Native Ontic Unified Store* — νοῦς, the intellect):

```bash
ontic vault export Stats.mean --out mean.nous   # dep chain included
ontic vault import mean.nous --verify           # re-runs the sieve locally
```

Imports land `attested`; `--verify` promotes only what the local
deterministic sieve re-proves. See docs/NOUS.md.

## License

[Apache 2.0 WITH LLVM-exception](LICENSE)
