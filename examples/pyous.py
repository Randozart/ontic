"""pyous — the Python bridge to Ontic's verified kernel vault.

Genesis workflow:
    import pyous as po
    rms = po.gen(\"\"\"
      fn Stats.rms(%xs: List<F64>) -> F64
        | %res >= 0.0
        => [2.0, 8.0] -> 5.0 +- 1e-9
    \"\"\", tier="wrapping")
    rms([2.0, 8.0])          # native speed, sieve-verified

Cache-first: the spec's canonical SHA-256 (computed by the `ontic`
binary — single source of truth) is looked up in the vault. Hits bind a
callable in microseconds; misses raise GenMissing unless auto-solve is on.

Honesty note: kernels are evidence+probe verified, not formally proven.
Provenance rides on every loaded gen via `kernel.__sieve_meta__`.
"""

import json
import os
import subprocess
import tempfile
from pathlib import Path

import ctypes


class GenMissing(RuntimeError):
    """Raised when no solved kernel exists for the given spec."""


def _ontic_bin() -> str:
    return os.environ.get("ONTIC_BIN", "ontic")


def _vault_dir() -> Path:
    return Path(os.environ.get("ONTIC_VAULT", ".ontic/vault"))


def key_for_spec(spec: str, tier: str = "wrapping") -> str:
    """Canonical SHA-256 of a spec — computed by the `ontic` binary."""
    with tempfile.NamedTemporaryFile("w", suffix=".ont", delete=False) as f:
        f.write(_render(spec, tier))
        path = f.name
    try:
        out = subprocess.run(
            [_ontic_bin(), "key", path],
            capture_output=True, text=True, check=True,
        )
        return out.stdout.strip()
    finally:
        os.unlink(path)


def _render(spec: str, tier: str) -> str:
    lines = []
    if tier == "wrapping":
        lines.append("wrapping")
    lines.append(spec.strip())
    return "\n".join(lines)


def solve(spec_file: str, samples: int | None = None) -> None:
    """Run ontic solve synchronously. Sampler comes from env/config."""
    cmd = [_ontic_bin(), "solve", spec_file]
    if samples:
        cmd += ["--samples", str(samples)]
    r = subprocess.run(cmd)
    if r.returncode != 0:
        raise GenMissing(f"ontic solve failed (exit {r.returncode})")


def load(vault_dir: str | Path, wish_path: str):
    """Bind the newest solved kernel whose signature matches wish_path.

    Returns (lib, fn, manifest). See bind() for ABI details.
    """
    import ctypes  # local: only needed on the load path

    vault = Path(vault_dir)
    best = None
    for man_path in vault.glob("*.json"):
        m = json.loads(man_path.read_text())
        sig = m.get("signature", "")
        if sig.startswith(f"fn {wish_path}("):
            if best is None or m.get("canonical", "") >= best.get(
                "canonical", ""
            ):
                best = m
    if best is None:
        raise GenMissing(
            f"{wish_path!r} not solved in {vault}; "
            "run: ontic solve <file> --hand <candidate> "
            "(or set ONTIC_AUTO_SOLVE=1)"
        )
    lib = ctypes.CDLL(str(vault / best["artifacts"]["lib"]))
    fn, ptypes, ret = _bind(lib, best["signature"], best["sketch"])
    meta = {
        "__sieve_meta__": best.get("last_solve", {}),
        "__artifacts__": best.get("artifacts", {}),
        "__canonical_sha256__": best.get("canonical_sha256",
                                         Path(man_path := "").stem if False else ""),
    }
    return fn, ptypes, ret, meta


def _bind(lib, signature: str, sketch: str):
    head = signature[3:]
    name, rest = head.split("(", 1)
    params_s, ret_s = rest.split(")", 1)
    ret_name = ret_s.replace("->", " ").strip()

    symbol = sketch.split("@", 1)[1].split("(")[0].split()[0]

    def argtypes_for(ptype: str):
        if ptype.startswith("List<"):
            # Flat-5: allocated*, aligned*, offset, size, stride
            return [ctypes.c_void_p] * 2 + [ctypes.c_long] * 3
        if ptype == "F64":
            return [ctypes.c_double]
        return [ctypes.c_long]

    f = getattr(lib, symbol)
    f.restype = {"Int": ctypes.c_long, "Bool": ctypes.c_long,
                 "F64": ctypes.c_double}[ret_name]
    at = []
    ptypes = []
    for p in params_s.split(","):
        p = p.strip()
        if not p:
            continue
        pname, ptype = p.split(":")
        ptype = ptype.strip().lstrip("%")
        at.extend(argtypes_for(ptype))
        ptypes.append((pname.lstrip("%"), ptype))
    f.argtypes = at
    return f, ptypes, ret_name


def _make_call_args(values, ptypes):
    import numpy as np  # optional; falls back to ctypes buffers

    out = []
    for (_n, pt), v in zip(ptypes, values):
        if pt.startswith("List<"):
            arr = np.ascontiguousarray(v, dtype=np.float64)
            ptr = arr.ctypes.data_as(ctypes.c_void_p)
            out += [ptr, ptr, 0, len(arr), 1]
        elif pt == "F64":
            out.append(ctypes.c_double(v))
        else:
            out.append(ctypes.c_long(int(v)))
    return out


def gen(spec: str, tier: str = "wrapping", samples: int | None = None,
        vault_dir: str | None = None):
    """Genesis: spec text -> callable native kernel.

    Cache hit binds instantly; miss raises GenMissing unless
    ONTIC_AUTO_SOLVE=1 (then runs `ontic solve` with your configured
    sampler and retries the lookup).
    """
    vd = Path(vault_dir or _vault_dir())
    key = key_for_spec(spec, tier)

    man_path = vd / f"{key}.json"
    complete = False
    if man_path.exists():
        m = json.loads(man_path.read_text())
        # A hit requires the shared library artifact; older/stale entries
        # (pre-artifact solves) are treated as misses and re-solved.
        complete = bool(m.get("artifacts", {}).get("lib"))
    if not complete:
        if os.environ.get("ONTIC_AUTO_SOLVE") == "1":
            tmp = tempfile.NamedTemporaryFile("w", suffix=".ont", delete=False)
            tmp.write(_render(spec, tier))
            tmp.close()
            solve(tmp.name, samples)
            os.unlink(tmp.name)
        else:
            raise GenMissing(
                f"no solved kernel for this gen (key {key[:12]}…).\n"
                "Fix: ontic solve <file.ont> --hand <candidate>\n"
                "or:  export ONTIC_AUTO_SOLVE=1"
            )

    manifest = json.loads(man_path.read_text())
    lib = ctypes.CDLL(str(vd / manifest["artifacts"]["lib"]))
    fn, ptypes, _ret = _bind(lib, manifest["signature"], manifest["sketch"])

    meta = dict(manifest.get("last_solve", {}))
    meta["artifacts"] = manifest.get("artifacts", {})
    meta["canonical_sha256"] = key

    def kernel(*values):
        args = _make_call_args(values, ptypes)
        return fn(*args)

    kernel.__sieve_meta__ = meta
    kernel.__ptypes__ = ptypes
    return kernel


class MemRefF64(ctypes.Structure):
    """Flat-MemRef descriptor (for struct-return kernels)."""
    _fields_ = [
        ("allocated", ctypes.c_void_p),
        ("aligned", ctypes.c_void_p),
        ("offset", ctypes.c_long),
        ("size", ctypes.c_long),
        ("stride", ctypes.c_long),
    ]


class ListF64Kernel:
    """Callable wrapper for List<F64> → List<F64> kernels.
    
    Native ABI: each List<F64> param = 5 flat args; F64 params = 1 double.
    restype = MemRefF64 struct; ctypes auto-handles hidden sret pointer.
    """
    def __init__(self, lib, symbol, num_scalars: int):
        self._lib = lib
        self._symbol = symbol
        self._num_scalars = num_scalars

    def __call__(self, pts, *scalars):
        import numpy as np
        arr = np.ascontiguousarray(pts, dtype=np.float64)
        n = len(arr)

        fn = getattr(self._lib, self._symbol)
        fn.restype = MemRefF64
        # Flat-5 for the list + one double per scalar
        fn.argtypes = [
            ctypes.c_void_p, ctypes.c_void_p,
            ctypes.c_long, ctypes.c_long, ctypes.c_long,
        ] + [ctypes.c_double] * self._num_scalars

        ptr = arr.ctypes.data_as(ctypes.c_void_p)
        args = [ptr, ptr, ctypes.c_long(0), ctypes.c_long(n), ctypes.c_long(1)]
        args.extend(ctypes.c_double(s) for s in scalars)
        result = fn(*args)

        if result.size == 0 or not result.aligned:
            return np.array([], dtype=np.float64)
        out_t = ctypes.c_double * result.size
        buf = out_t.from_address(result.aligned)
        return np.frombuffer(buf, dtype=np.float64).copy()

def gen_list_return(spec: str, tier: str = "wrapping",
                    samples: int | None = None,
                    vault_dir: str | None = None):
    """Genesis for List<F64> → List<F64> kernels."""
    import numpy as np

    vd = Path(vault_dir or _vault_dir())
    key = key_for_spec(spec, tier)
    man_path = vd / f"{key}.json"
    complete = False
    if man_path.exists():
        m = json.loads(man_path.read_text())
        complete = bool(m.get("artifacts", {}).get("lib"))
    if not complete:
        if os.environ.get("ONTIC_AUTO_SOLVE") == "1":
            tmp = tempfile.NamedTemporaryFile("w", suffix=".ont", delete=False)
            tmp.write(_render(spec, tier))
            tmp.close()
            solve(tmp.name, samples)
            os.unlink(tmp.name)
        else:
            raise GenMissing(f"key {key[:12]} not solved")

    manifest = json.loads(man_path.read_text())
    so_path = vd / manifest["artifacts"]["lib"]
    sk = manifest.get("sketch", "")
    sym = sk.split("@", 1)[1].split("(")[0].split()[0] if "@" in sk else "f"

    lib = ctypes.CDLL(str(so_path))
    # Count scalar params from the stored sketch signature.
    sig = manifest.get("signature", "")
    num_scalars = 0
    inner = sig.split("(", 1)[1].split(")")[0] if "(" in sig else ""
    for p in inner.split(","):
        p = p.strip()
        if p and "List" not in p:
            num_scalars += 1
    kernel = ListF64Kernel(lib, sym, num_scalars)
    kernel.__sieve_meta__ = manifest.get("last_solve", {})
    return kernel
