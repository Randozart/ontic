<p align="center">
  <img src="ontic-logo.svg" width="128" height="128" alt="Ontic logo"/>
</p>

# Ontic

**A DSL whose products are verified native libraries.**

Write a specification (`.ont`) — signatures, invariants, example evidence.
A local transformer proposes implementations. A deterministic seven-stage
sieve proves them against your evidence. The output: a shared library + C
header that Python, C, C++, Rust — anything with an FFI — consumes at
native speed.

```python
import pyous as po

rms = po.gen("""
  fn Stats.rms(%xs: List<F64>) -> F64
    | %res >= 0.0
    => [2.0, 8.0] -> 5.830951894845301 ± 1e-9
""", tier="wrapping")

rms([2.0, 8.0])   # → 5.830951894845301 at native speed
```

## THE WALL

> **The transformer generates candidates. Everything else is deterministic
> Rust.** The model proposes; it never validates, ranks, or decides.

Trust scales with sieve strength, not model strength. Every capability gain
came from verifier work while the model stayed frozen.

---

## Two file formats

| | `.ont` — Source Spec | `.ous` — Kernel Bundle |
|---|---|---|
| **What** | Human-written specification: signatures, invariants, evidence | Machine-produced verified artifact: manifest + sketch + MLIR + object + header |
| **Who writes it** | You | The compiler (after forge + sieve pass) |
| **Editable** | Yes — this is where you make changes | No — immutable build output; edit `.ont` and re-solve instead |
| **Portable** | Needs Ontic installed to process | Self-contained: unpack + link + call on any machine with a C compiler |
| **Contains** | `fn` signatures, `\|` invariants, `=>` transparent evidence, `??` opaque evidence, hints, tier declarations | Manifest JSON, sketch source, MLIR text, LLVM object code, C header |

### `.ont` example

```
wrapping
fn Stats.rms(%xs: List<F64>) -> F64
  | %res >= 0.0
  hint "root mean square: sqrt(sum(x*x)/len)"
  => [2.0, 8.0] -> 5.830951894845301 ± 1e-9
  ?? [3.0] -> 1.7320508075688772 ± 1e-12
```

- `wrapping` — overflow semantics tier (see below)
- `| %res >= ...` — invariant: guides forge AND bounds probe oracle
- `=> [2,8] -> 5.83` — **transparent** evidence: forge sees this
- `?? [3] -> 1.73` — **opaque** evidence: held out; overfit killer

### `.ous` lifecycle

```
ontic pack Stats.mean -o stats.ous     # bundle from vault
# ship stats.ous anywhere
ontic unpack stats.ous -d ./lib        # extract .so + .h
gcc my_app.c -L./lib -lmean -o app     # link and call
```

---

## PyOus — Python bridge

[`pip install pyous`](https://pypi.org/project/pyous/)

PyOus loads `.ous` kernels into Python via ctypes with zero-copy numpy
support. Three API levels, all producing the same native callables.

### Level 1 — Typed builder

```python
import pyous as po

rms = po.define(
    name="Stats.rms",
    params={"xs": po.List[po.F64]},
    ret=po.F64,
    tier="wrapping",
    evidence=[
        ([2.0, 8.0], 5.830951894845301),
        ([1.0, 0.0], 1.0),
    ],
    opaque=[
        ([3.0], 1.7320508075688772),
    ],
)
rms([2.0, 8.0])  # → native speed
```

### Level 2 — Decorator

```python
@po.kernel(tier="wrapping", evidence=[([21], 42)])
def twice(n: int) -> int:
    """Double the input."""
```

Type hints auto-map (`float→F64`, `list[float]→List<F64>`). Docstring
becomes forge guidance. Body is documentation only — never executed.

### Level 3 — Pure data

```python
rms = po.gen({
    "name": "Stats.rms",
    "params": {"xs": "List<F64>"},
    "ret": "F64",
    "tier": "wrapping",
    "invariants": ["res >= 0"],
    "evidence": [([2.0, 8.0], 5.831)],
})
```

### Cache-first economics

Cold genesis (forge → sieve → compile) costs seconds to minutes depending
on sampler. Warm cache hits bind in microseconds. Every kernel persists in
the vault — solve once, use forever. Ship `.ous` files to colleagues without
Ontic installed.

---

## Sieve pipeline

Every candidate passes through seven deterministic stages:

| Stage | Check | Kill condition |
|-------|-------|---------------|
| S1 parse | grammar-constrained sketch | malformed |
| S2 well-formed | typechecker | type error |
| S3 transparent | visible examples | wrong output |
| S4 held-out | hidden examples | **overfit** |
| S5 probes | random inputs ∩ invariants | violation / runtime error |
| S6 shape | constant-guard ratio + table scan | memorization structure |
| S7 bench | compiled performance ranking | tiebreak only |

Overfit rejection is behavioral (S4+S5) AND structural (S6). Neither alone
is sufficient.

## Overflow tiers

| Tier | Syntax | Semantics |
|------|--------|-----------|
| wrapping | `wrapping` line in gen | mod 2^64; bit-exact interp↔native; LLVM free to optimize |
| checked *(default)* | absent | overflow kills candidates in sieve; traps natively |
| proven *(M3)* | automatic | Z3 absence proof ⇒ flag-free codegen |

Speed requires declaration (Golden Rule 16).

---

## Setup

Requires: Python ≥ 3.8, system LLVM toolchain (`llc`, `clang`), Rust ≥ 1.70.

```bash
git clone https://github.com/Randozart/ontic.git
cd ontic
cargo build --release
cargo test --lib

# Install pyous bridge
cd pyous_pkg && pip install -e . && cd ..

# Solve a gen from spec
./target/release/ontic solve examples/stats-mean.ont \
  --sampler-backend gemini --samples 8

# Call from Python
python3 -c "
import pyous as po
mean = po.load('.ontic/vault', 'Stats.mean')
print(mean([1.0, 2.0, 3.0]))
"
```

Environment variables:

| Variable | Purpose | Default |
|----------|---------|---------|
| `ONTIC_BIN` | Path to ontic binary | PATH lookup |
| `ONTIC_VAULT` | Vault directory | `.ontic/vault` |
| `ONTIC_FORGE` | llama-server endpoint | `127.0.0.1:8279` |
| `ONTIC_SAMPLER` | Backend: llama/openai/gemini | `llama` |
| `ONTIC_MODEL` | Model name (cloud) | varies by provider |
| `GEMINI_API_KEY` | Gemini API key | required for cloud |
| `ONTIC_MLIR_BIN` | LLVM toolchain directory | `/usr/lib/llvm-18/bin` |
| `ONTIC_FORGE_WORKERS` | Parallel sampling workers | `2` |

## License

[Apache 2.0 WITH LLVM-exception](LICENSE) — compiled output exempt from
source distribution requirements.
