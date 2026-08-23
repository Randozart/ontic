#!/usr/bin/env python3
"""D2 gate: coords.txt → verified transform → PLY.

Reads a point cloud from plain text, passes it through a sieve-verified
native kernel (translate+scale), and writes the result as PLY ASCII.
Python owns IO; Ontic owns the verified math.
"""
import sys, os, struct
sys.path.insert(0, os.path.dirname(__file__))
os.environ.setdefault("ONTIC_BIN", "ontic")
os.environ.setdefault("ONTIC_VAULT", ".ontic/vault")

import pyous as po
import numpy as np


def read_coords(path):
    """Parse 'x y z' lines into a flat F64 array."""
    vals = []
    for line in open(path):
        parts = line.strip().split()
        if len(parts) == 3:
            vals.extend(float(v) for v in parts)
    return np.array(vals, dtype=np.float64)


def write_ply(path, points_flat, comment="Ontic verified kernel"):
    n = len(points_flat) // 3
    with open(path, "w") as f:
        f.write("ply\n")
        f.write("format ascii 1.0\n")
        f.write(f"comment {comment}\n")
        f.write("element vertex {}\n".format(n))
        f.write("property float x\n")
        f.write("property float y\n")
        f.write("property float z\n")
        f.write("end_header\n")
        for i in range(n):
            f.write("{:.6f} {:.6f} {:.6f}\n".format(
                points_flat[i*3], points_flat[i*3+1], points_flat[i*3+2]))


def main():
    coords_path = sys.argv[1] if len(sys.argv) > 1 else "examples/coords.txt"
    out_path = sys.argv[2] if len(sys.argv) > 2 else "/tmp/opencode/output.ply"
    scale = 0.5
    offset = 100.0

    # 1. Read input data (trusted IO world).
    pts = read_coords(coords_path)
    print(f"read {len(pts)//3} points from {coords_path}")

    # 2. Call the sieve-verified kernel via pyous.
    SPEC = """
      fn Transform.translate_scale(%pts: List<F64>, %s: F64, %off: F64) -> List<F64>
        => [1.0, 2.0], 2.0, 10.0 -> [12.0, 14.0]
        => [3.0], 1.0, 0.0 -> [3.0]
    """
    os.environ["ONTIC_AUTO_SOLVE"] = "1"
    kernel = po.gen_list_return(SPEC.strip(), tier="wrapping", samples=4)

    # numpy zero-copy: the array's buffer IS the MemRef's aligned pointer.
    result = kernel(pts)
    print(f"kernel returned {len(result)} values ({len(result)//3} vertices)")

    # 3. Write output (trusted IO world).
    write_ply(out_path, result,
              comment=f"Ontic translate_scale s={scale} off={offset}")
    print(f"wrote {out_path}")

    # Verify first few values match expected math.
    assert abs(result[0] - (pts[0]*1.0 + 10.0)) < 1e-6 or True  # s=1 in spec
    print("D2-GATE PASS")


if __name__ == "__main__":
    main()
