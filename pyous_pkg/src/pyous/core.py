"""pyous — verified native kernels from specifications.

Three API levels, all compiling to the same internal pipeline:

Level 1 — Typed builder:
    rms = po.define(
        name="Stats.rms",
        params={"xs": po.List[po.F64]},
        ret=po.F64,
        tier="wrapping",
        hints=["sqrt(sum(x*x)/len)"],
        evidence=[([2.0, 8.0], 5.830951894845301)],
    )

Level 2 — Decorator:
    @po.kernel(tier="wrapping", evidence=[([2.0, 8.0], 5.831)])
    def rms(xs: list[float]) -> float:
        \"\"\"Root mean square.\"\"\"

Level 3 — Pure data:
    rms = po.gen({
        "name": "Stats.rms",
        "params": {"xs": "List<F64>"},
        "ret": "F64",
        "evidence": [([2.0, 8.0], 5.831)],
    })

All three compile to .ont specs that feed the same forge → sieve → vault
pipeline. Cache-first: solved kernels bind in microseconds.
"""

import ctypes
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

__version__ = "0.1.0"


# ---------------------------------------------------------------------------
# Type constants and mapping
# ---------------------------------------------------------------------------

class F64:
    """Marker for Ontic F64 type."""
    pass


class Int:
    """Marker for Ontic Int type."""
    pass


class Bool:
    """Marker for Ontic Bool type."""
    pass


class List:
    """Generic list constructor: List[F64] → List<F64>."""
    _elem = None

    def __class_getitem__(cls, item):
        inst = cls()
        inst._elem = _py_to_ontic(item)
        return inst


def _py_to_ontic(t):
    """Map a Python type or marker to its Ontic type name."""
    mapping = {
        float: "F64", int: "Int", bool: "Bool",
        F64: "F64", Int: "Int", Bool: "Bool",
        "F64": "F64", "Int": "Int", "Bool": "Bool",
        "List<F64>": "List<F64>", "List<Int>": "List<Int>",
    }
    if isinstance(t, str) and t.startswith("List<"):
        return t
    if hasattr(t, "_elem") and t._elem is not None:
        return f"List<{t._elem}>"
    return mapping.get(t, t)


def _is_list_type(t: str) -> bool:
    return t.startswith("List<")


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

def _ontic_bin() -> str:
    return os.environ.get("ONTIC_BIN", "ontic")


def _vault_dir() -> Path:
    vd = os.environ.get("ONTIC_VAULT")
    if vd:
        return Path(vd)
    # Walk up from cwd looking for .ontic/vault.
    p = Path.cwd()
    while p != p.parent:
        candidate = p / ".ontic" / "vault"
        if candidate.is_dir():
            return candidate
        p = p.parent
    return Path.cwd() / ".ontic" / "vault"


def load_dotenv(path: Path | None = None):
    """Load KEY=VALUE pairs into env (never overrides existing vars)."""
    path = path or Path.cwd() / ".env"
    if not path.exists():
        return
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        k, _, v = line.partition("=")
        k, v = k.strip(), v.strip().strip("'\"")
        if k and k not in os.environ:
            os.environ[k] = v


# Load .env at import time (lowest precedence).
load_dotenv()


# ---------------------------------------------------------------------------
# Key authority + solve invocation
# ---------------------------------------------------------------------------

def _write_temp_ont(spec_text: str, tier: str) -> Path:
    """Write rendered spec text to a temp .ont file."""
    content = spec_text.strip() + "\n"
    tmp = tempfile.NamedTemporaryFile("w", suffix=".ont", delete=False)
    tmp.write(content)
    tmp.close()
    return Path(tmp.name)


def compute_key(spec_file: Path, gen_name: str | None = None) -> str:
    """Compute canonical SHA-256 via the `ontic key` subcommand."""
    cmd = [_ontic_bin(), "key", str(spec_file)]
    if gen_name:
        cmd += ["--gen", gen_name]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"ontic key failed: {r.stderr}")
    return r.stdout.strip()


def run_solve(ont_file: Path, samples: int | None = None) -> None:
    """Run ontic solve synchronously."""
    cmd = [_ontic_bin(), "solve", str(ont_file)]
    if samples:
        cmd += ["--samples", str(samples)]
    r = subprocess.run(cmd)
    if r.returncode != 0:
        raise RuntimeError(f"ontic solve failed (exit {r.returncode})")


# ---------------------------------------------------------------------------
# ctypes binding
# ---------------------------------------------------------------------------

import ctypes


class MemRefF64(ctypes.Structure):
    _fields_ = [
        ("allocated", ctypes.c_void_p),
        ("aligned", ctypes.c_void_p),
        ("offset", ctypes.c_long),
        ("size", ctypes.c_long),
        ("stride", ctypes.c_long),
    ]


class MemRefI64(ctypes.Structure):
    _fields_ = [
        ("allocated", ctypes.c_void_p),
        ("aligned", ctypes.c_void_p),
        ("offset", ctypes.c_long),
        ("size", ctypes.c_long),
        ("stride", ctypes.c_long),
    ]


_SCALAR_RESTYPE = {"Int": ctypes.c_long, "Bool": ctypes.c_long,
                   "F64": ctypes.c_double}
_MEMREF_RESTYPE = {"List<F64>": MemRefF64, "List<Int>": MemRefI64}


def _argtypes_for(ptype: str):
    if _is_list_type(ptype):
        return [ctypes.c_void_p] * 2 + [ctypes.c_long] * 3
    if ptype == "F64":
        return [ctypes.c_double]
    return [ctypes.c_long]


def _bind(lib, signature: str, sketch: str):
    head = signature[3:]
    name, rest = head.split("(", 1)
    params_s, ret_s = rest.split(")", 1)
    ret_name = ret_s.replace("->", " ").strip()

    symbol = sketch.split("@", 1)[1].split("(")[0].split()[0]

    f = getattr(lib, symbol)
    if ret_name in _MEMREF_RESTYPE:
        f.restype = _MEMREF_RESTYPE[ret_name]
    else:
        f.restype = _SCALAR_RESTYPE[ret_name]

    at = []
    ptypes = []
    for p in params_s.split(","):
        p = p.strip()
        if not p:
            continue
        pname, ptype = p.split(":")
        ptype = ptype.strip().lstrip("%")
        at.extend(_argtypes_for(ptype))
        ptypes.append((pname.lstrip("%"), ptype))
    f.argtypes = at

    is_list_ret = ret_name.startswith("List<")
    return f, ptypes, ret_name, is_list_ret


def _make_args(values, ptypes):
    try:
        import numpy as np
        has_np = True
    except ImportError:
        np = None
        has_np = False

    out = []
    for (_n, pt), v in zip(ptypes, values):
        if _is_list_type(pt):
            if has_np:
                arr = np.ascontiguousarray(v, dtype=np.float64)
                ptr = arr.ctypes.data_as(ctypes.c_void_p)
            else:
                buf = (ctypes.c_double * len(v))(*v)
                ptr = ctypes.cast(buf, ctypes.c_void_p)
            out += [ptr, ptr, 0, len(v), 1]
        elif pt == "F64":
            out.append(ctypes.c_double(v))
        else:
            out.append(ctypes.c_long(int(v)))
    return out


def _extract_list_result(memref, ret_name: str):
    n = memref.size
    if n == 0 or not memref.aligned:
        import numpy as np
        dtype = np.float64 if "F64" in ret_name else np.int64
        return np.array([], dtype=dtype)
    buf = (ctypes.c_double * n).from_address(memref.aligned)
    import numpy as np
    arr = np.frombuffer(buf, dtype=np.float64).copy()
    if "I64" in ret_name or "Int" in ret_name:
        return arr.astype(np.int64)
    return arr


# ---------------------------------------------------------------------------
# Vault access
# ---------------------------------------------------------------------------

def _find_manifest(vault_dir: Path, key: str):
    man_path = vault_dir / f"{key}.json"
    if not man_path.exists():
        return None, None
    m = json.loads(man_path.read_text())
    if not m.get("artifacts", {}).get("lib"):
        return None, None  # stale entry without artifacts
    return m, man_path


# ---------------------------------------------------------------------------
# Invariant serialization
# ---------------------------------------------------------------------------

def _serialize_invariants(invs) -> list:
    if invs is None:
        return []
    out = []
    for inv in invs:
        if isinstance(inv, str):
            out.append(inv.lstrip("|").strip())
        elif callable(inv):
            try:
                import inspect
                src = inspect.getsource(inv).strip()
                # Strip lambda prefix: "lambda r: expr" → "expr"
                if src.startswith("lambda"):
                    body = src.split(":", 1)[-1].strip()
                    out.append(body)
                else:
                    out.append(src)
            except Exception:
                raise ValueError(
                    f"cannot serialize invariant {inv}; use string format "
                    "(e.g. 'res >= 0')"
                )
        else:
            raise ValueError(f"invariant must be callable or string, got {inv}")
    return out


# ---------------------------------------------------------------------------
# Spec rendering (.ont text generation from structured data)
# ---------------------------------------------------------------------------

def render_ont(name: str, params: dict, ret: str, tier: str = "checked",
               invariants=None, hints=None, evidence=None, opaque=None,
               deps=None) -> str:
    def val_str(v, ptype):
        """Render a value according to its declared param type."""
        if _is_list_type(ptype):
            inner = ", ".join(str(x) for x in v)
            return f"[{inner}]"
        return str(v)

    lines = []
    if tier == "wrapping":
        lines.append("wrapping")
    for d in deps or []:
        lines.append(f"use {d}")
    params_s = ", ".join(f"%{n}: {t}" for n, t in params.items())
    lines.append(f"fn {name}({params_s}) -> {ret}")

    for inv in _serialize_invariants(invariants):
        lines.append(f"| {inv}")
    for h in hints or []:
        lines.append(f'hint "{h}"')

    ptypes = list(params.values())
    def _render_evidence_line(marker, inputs, output):
        """Render => or ?? line using param types."""
        if not isinstance(inputs, (list, tuple)):
            inputs = [inputs]
        # If only one param and its type is a List, wrap entire inputs as one value.
        if len(ptypes) == 1 and _is_list_type(ptypes[0]):
            ins_parts = ["[" + ", ".join(str(x) for x in inputs) + "]"]
            out_s = ", ".join(str(x) for x in (
                output if isinstance(output, (list, tuple)) else [output]))
            lines.append(f"{marker} {ins_parts[0]} -> {out_s}")
        else:
            ins_parts = []
            out_parts = []
            for (inp, pt) in zip(inputs, ptypes):
                if _is_list_type(pt):
                    ins_parts.append("[" + ", ".join(str(x) for x in inp) + "]")
                elif pt == "F64":
                    ins_parts.append(str(float(inp)))
                else:
                    ins_parts.append(str(int(inp)))
            if isinstance(output, (list, tuple)):
                out_parts.append("[" + ", ".join(str(x) for x in output) + "]")
            elif ret == "F64":
                out_parts.append(str(float(output)))
            else:
                out_parts.append(str(output))
            lines.append(f"{marker} {', '.join(ins_parts)} -> {', '.join(out_parts)}")

    for inputs, output in evidence or []:
        _render_evidence_line("=>", inputs, output)
    for inputs, output in opaque or []:
        _render_evidence_line("??", inputs, output)

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Internal: solve-or-vault pipeline
# ---------------------------------------------------------------------------

def _ensure_solved(key: str, spec_text: str, samples: int | None,
                   vault_dir: Path) -> dict:
    """Return manifest if solved; auto-solve on miss if ONTIC_AUTO_SOLVE=1."""
    manifest, _ = _find_manifest(vault_dir, key)
    if manifest:
        return manifest
    auto = os.environ.get("ONTIC_AUTO_SOLVE", "")
    if auto != "1":
        raise GenMissing(
            f"no solved kernel (key {key[:12]}…).\n"
            "Fix: ontic solve <file.ont> --hand <candidate>\n"
            "or:  export ONTIC_AUTO_SOLVE=1"
        )
    tmp = _write_temp_ont(spec_text, "checked")
    try:
        run_solve(tmp, samples)
    finally:
        tmp.unlink(missing_ok=True)
    manifest, _ = _find_manifest(vault_dir, key)
    if not manifest:
        raise GenMissing("solve completed but kernel still not in vault")
    return manifest


# ---------------------------------------------------------------------------
# Level 3 — Pure data DSL
# ---------------------------------------------------------------------------

class GenMissing(RuntimeError):
    """Raised when no solved kernel exists for the given spec."""


def gen(spec_dict: dict, samples: int | None = None,
        vault_dir: str | None = None) -> callable:
    """Level 3: pure data specification.

    Example:
        rms = po.gen({
            "name": "Stats.rms",
            "params": {"xs": "List<F64>"},
            "ret": "F64",
            "tier": "wrapping",
            "invariants": ["res >= 0"],
            "evidence": [([2.0, 8.0], 5.831)],
            "hints": ["sqrt(sum(x*x)/len)"],
        })
    """
    name = spec_dict["name"]
    params = spec_dict["params"]
    ret = spec_dict.get("ret", "F64")
    tier = spec_dict.get("tier", "wrapping")
    invariants = spec_dict.get("invariants")
    hints = spec_dict.get("hints") or []
    evidence = spec_dict.get("evidence", [])
    opaque = spec_dict.get("opaque", [])
    deps = spec_dict.get("deps", [])
    if deps:
        hints.append('Available: ' + ', '.join(deps))

    spec_text = render_ont(name, params, ret, tier,
                           invariants, hints, evidence, opaque, deps)
    vd = Path(vault_dir or _vault_dir())
    key = compute_key_from_text(spec_text)

    manifest = _ensure_solved(key, spec_text, samples, vd)
    lib = ctypes.CDLL(str(vd / manifest["artifacts"]["lib"]))
    fn, ptypes, ret_name, is_list = _bind(
        lib, manifest["signature"], manifest["sketch"])

    meta = dict(manifest.get("last_solve", {}))
    meta["artifacts"] = manifest["artifacts"]

    def kernel_fn(*values):
        args = _make_args(values, ptypes)
        result = fn(*args)
        if is_list:
            return _extract_list_result(result, ret_name)
        return result

    kernel_fn.__sieve_meta__ = meta
    kernel_fn.__ret_type__ = ret_name
    kernel_fn.__ptypes__ = ptypes
    return kernel_fn


def deps_block(deps: list) -> str:
    """Render AVAILABLE FUNCTIONS block for dep signatures."""
    out = ""
    for d in deps:
        out += d + "\n"
    return out


def compute_key_from_text(spec_text: str) -> str:
    tmp = _write_temp_ont(spec_text, "checked")
    try:
        return compute_key(tmp)
    finally:
        tmp.unlink(missing_ok=True)


# ---------------------------------------------------------------------------
# Level 1 — Typed builder
# ---------------------------------------------------------------------------

def define(name: str, params: dict, ret: type | str = F64,
           tier: str = "wrapping", invariants=None,
           hints=None, evidence=None, opaque=None,
           deps=None, samples: int | None = None,
           vault_dir: str | None = None):
    """Level 1: typed builder — structured Python arguments.

    Example:
        rms = po.define(
            name="Stats.rms",
            params={"xs": po.List[po.F64]},
            ret=po.F64,
            tier="wrapping",
            evidence=[([2.0, 8.0], 5.831)],
        )
    """
    ontic_params = {}
    for n, t in params.items():
        ontic_params[n] = _py_to_ontic(t)

    # Convert ret from Python type to Ontic string.
    if isinstance(ret, type) or hasattr(ret, '_elem'):
        ret = _py_to_ontic(ret)
    elif not isinstance(ret, str):
        ret = str(ret)

    spec_dict = {
        "name": name,
        "params": ontic_params,
        "ret": ret,
        "tier": tier,
        "invariants": invariants,
        "hints": hints or [],
        "evidence": evidence or [],
        "opaque": opaque or [],
        "deps": deps or [],
    }
    return gen(spec_dict, samples=samples, vault_dir=vault_dir)


# ---------------------------------------------------------------------------
# Level 2 — Decorator
# ---------------------------------------------------------------------------

def kernel(tier: str = "wrapping", evidence=None, opaque=None,
           invariants=None, hints=None, deps=None,
           samples: int | None = None, vault_dir: str | None = None):
    r'''Level 2: decorator with Python type hints.

    Example:
        @po.kernel(tier="wrapping", evidence=[([2.0, 8.0], 5.831)])
        def rms(xs: list[float]) -> float:
            "Root mean square."
    '''
    import inspect

    def decorator(fn):
        sig = inspect.signature(fn)
        params = {}
        for pname, param in sig.parameters.items():
            ann = param.annotation
            if ann == inspect.Parameter.empty:
                raise ValueError(
                    f"parameter %{pname} missing type annotation"
                )
            if isinstance(ann, type):
                params[pname] = _py_to_ontic(ann)
            else:
                # Handle list[float] etc via string representation
                s = str(ann)
                if "float" in s:
                    params[pname] = _py_to_ontic(list) if "list" in s else "F64"
                elif "int" in s:
                    params[pname] = _py_to_ontic(list) if "list" in s else "Int"
                else:
                    params[pname] = s
        ret_ann = sig.return_annotation
        if ret_ann == inspect.Signature.empty:
            raise ValueError("return type annotation required")
        if isinstance(ret_ann, type):
            ret = _py_to_ontic(ret_ann)
        else:
            s = str(ret_ann)
            ret = f"List<{_py_to_ontic(float)}>" if "list" in s.lower() else _py_to_ontic(ret_ann)

        fn_hints = hints or []
        docstring = inspect.getdoc(fn)
        if docstring:
            fn_hints = [docstring] + fn_hints

        return define(
            name=fn.__name__,
            params=params,
            ret=ret,
            tier=tier,
            invariants=invariants,
            hints=fn_hints,
            evidence=evidence,
            opaque=opaque,
            deps=deps,
            samples=samples,
            vault_dir=vault_dir,
        )

    return decorator


def load(vault_dir: str | None = None, wish_path: str | None = None):
    """Load a pre-solved kernel by path or vault lookup."""
    vd = Path(vault_dir or _vault_dir())
    lib_path = vd / "lib.so"  # fallback; real path resolved by caller
    return OusiaKernel(lib_path, "unknown")


class OusiaKernel:
    """Thin ctypes wrapper for direct symbol calls."""

    def __init__(self, path, name):
        self._lib = ctypes.CDLL(str(path))
        self._fn = getattr(self._lib, name)
        self._fn.restype = ctypes.c_double

    def __call__(self, *args):
        return self._fn(*args)
