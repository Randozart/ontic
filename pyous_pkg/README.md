# pyous

> *Sanctified logic for Python.* Verified native kernels from specifications.

[Apache 2.0 WITH LLVM-exception](LICENSE)

## What it does

Write a specification. Get a verified native kernel callable at machine speed.

```python
import pyous as po

@po.kernel(evidence=[([2.0, 8.0], 5.831)])
def rms(xs: list[float]) -> float:
    """Root mean square."""
```

Behind the scenes: the spec is forged into candidate implementations by a
local transformer, verified against evidence and probes by a deterministic
sieve, compiled to native code via LLVM, and cached in a content-addressed
vault. Subsequent calls bind in microseconds.

## Three API levels

| Level | Style | Example |
|-------|-------|---------|
| 1 — Typed builder | Structured Python args | `po.define(name="f", params={"x": po.F64}, ...)` |
| 2 — Decorator | Type hints + docstring | `@po.kernel(evidence=...)` |
| 3 — Pure data | Dict/list | `po.gen({"name": "f", "params": {...}, ...})` |

## Requirements

- Python ≥ 3.8
- The `ontic` binary (built from [github.com/Randozart/ontic](https://github.com/Randozart/ontic))
- LLVM toolchain (`mlir-opt`, `mlir-translate`, `llc`, `clang`) for native compilation

## License

Apache 2.0 WITH LLVM-exception.
