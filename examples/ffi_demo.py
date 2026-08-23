#!/usr/bin/env python3
"""ffi_demo.py — call an Ontic kernel from Python via ctypes.

Usage: python3 examples/ffi_demo.py [vault_dir] [wish_path]
Defaults: .ontic/vault  Stats.mean
"""

import json
import ctypes
import sys
from pathlib import Path

# Flat MemRef ABI per List<T> param: (ptr, ptr, offset, size, stride)
CTYPES_FOR = {
    "Int": ctypes.c_long,
    "Bool": ctypes.c_long,
    "F64": ctypes.c_double,
    "List<Int>": ctypes.c_void_p,   # expanded: (a, b, o, s, st)
    "List<F64>": ctypes.c_void_p,
}


def load_kernel(vault: Path, wish_path: str):
    """Find the newest vault entry whose signature matches wish_path."""
    best = None
    for man in vault.glob("*.json"):
        m = json.loads(man.read_text())
        sig = m.get("signature", "")
        if sig.startswith(f"fn {wish_path}("):
            if best is None or m.get("canonical", "") >= best.get("canonical", ""):
                best = m
    if best is None:
        sys.exit(f"kernel {wish_path!r} not found in {vault} — solve it first")
    so = vault / best["artifacts"]["lib"]
    header = vault / best["artifacts"]["header"]
    return best, so, header


FLAT_LIST_ARGS = [
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_long,
    ctypes.c_long,
    ctypes.c_long,
]


def param_argtypes(ptype: str):
    if ptype.startswith("List<"):
        return FLAT_LIST_ARGS
    return [CTYPES_FOR[ptype]]


def bind(lib, signature: str, symbol: str):
    """Bind restype/argtypes ON THE FUNCTION OBJECT and return it.

    NOTE: CDLL-level defaults stopped propagating on modern Python;
    per-function binding is required.
    """
    head = signature[len("fn "):]
    name, rest = head.split("(", 1)
    params_s, ret_s = rest.split(")", 1)
    ret_name = ret_s.replace("->", " ").strip()
    fn = getattr(lib, symbol)
    fn.restype = CTYPES_FOR[ret_name]
    argtypes = []
    param_types = []
    for p in params_s.split(","):
        p = p.strip()
        if not p:
            continue
        pname, ptype = p.split(":")
        ptype = ptype.strip()
        argtypes.extend(param_argtypes(ptype))
        param_types.append((pname.strip().lstrip("%"), ptype))
    fn.argtypes = argtypes
    return fn, param_types, ret_name


def make_c_args(values, param_types):
    """Build the flat C argument list from friendly Python values."""
    out = []
    vi = 0
    for pname, ptype in param_types:
        v = values[vi]
        vi += 1
        if ptype == "F64":
            out.append(ctypes.c_double(v))
        elif ptype == "Int":
            out.append(ctypes.c_long(v))
        elif ptype == "Bool":
            out.append(ctypes.c_long(int(v)))
        else:  # List types: buffer, buffer, offset=0, size=n, stride=1
            arr = (ctypes.c_double * len(v))(*v)
            out.extend([arr, arr, 0, len(v), 1])
    return out


def main():
    vault = Path(sys.argv[1] if len(sys.argv) > 1 else ".ontic/vault")
    wish_path = sys.argv[2] if len(sys.argv) > 2 else "Stats.mean"
    manifest, so, _header = load_kernel(vault, wish_path)

    lib = ctypes.CDLL(str(so))
    # The exported symbol is the candidate's own @name (from its sketch),
    # not necessarily the gen path.
    sk = manifest.get("sketch", "")
    sym = sk.split("@", 1)[1].split("(")[0].split()[0] if "@" in sk else manifest["name"]
    kernel, param_types, _ret = bind(lib, manifest["signature"], sym)

    # Demo input: one value per parameter (lists get a fixed sample).
    values = []
    for _, ptype in param_types:
        values.append([1.0, 2.0, 3.0, 4.0] if ptype.startswith("List") else 2)
    c_args = make_c_args(values, param_types)
    result = kernel(*c_args)

    pretty = ", ".join(f"{n}={v}" for (n, _), v in zip(param_types, values))
    print(f"{wish_path}({pretty}) = {result}")


if __name__ == "__main__":
    main()
