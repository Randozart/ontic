"""pyous — verified native kernels from specifications.

    import pyous as po

    # Level 1: typed builder
    rms = po.define(
        name="Stats.rms",
        params={"xs": po.List[po.F64]},
        ret=po.F64,
        evidence=[([2.0, 8.0], 5.831)],
    )

    # Level 2: decorator
    @po.kernel(evidence=[([2.0, 8.0], 5.831)])
    def rms(xs: list[float]) -> float: ...

    # Level 3: pure data
    rms = po.gen({"name": "Stats.rms", "params": {"xs": "List<F64>"}, ...})

All three compile to the same .ont spec and produce identical native
callables backed by sieve-verified machine code.
"""

__version__ = "0.1.0"

from .core import (
    Bool,
    F64,
    GenMissing,
    Int,
    List,
    define,
    gen,
    kernel,
    load,
)
