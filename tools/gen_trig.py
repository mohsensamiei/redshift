#!/usr/bin/env python3
"""Regenerate crates/sim/src/trig_table.rs.

The table is committed to the repository rather than computed at build or run
time. Every peer in a lockstep match must use bit-identical values, and
floating-point evaluation is not guaranteed to agree across platforms or
toolchains. Generating once and committing the integers removes that risk.

Run from the repository root:  python3 tools/gen_trig.py
"""

import math
import subprocess

N = 4096  # entries covering a full turn
ONE = 1 << 16  # Fx scale: 1.0 == 65536
ATAN_STEPS = 1024  # arctangent table resolution over the first octant


def main() -> None:
    vals = []
    for i in range(N):
        v = math.sin(2 * math.pi * i / N) * ONE
        # Round half away from zero, in exact integer arithmetic.
        vals.append(int(math.floor(v + 0.5)) if v >= 0 else -int(math.floor(-v + 0.5)))

    assert vals[0] == 0
    assert vals[N // 4] == ONE
    assert vals[N // 2] == 0
    assert vals[3 * N // 4] == -ONE

    out = [
        "//! Generated sine table. DO NOT EDIT BY HAND.",
        "//!",
        "//! Committed rather than computed at build or run time: every peer must use",
        "//! bit-identical values, and floating-point evaluation is not guaranteed to",
        "//! agree across platforms. See docs/02-simulation.md.",
        "//!",
        f"//! {N} entries spanning a full turn, in `Fx` raw form (1.0 == {ONE}).",
        "//! Regenerate with `tools/gen_trig.py`.",
        "",
        f"pub(crate) const SIN_TABLE_LEN: usize = {N};",
        "",
        f"pub(crate) static SIN_TABLE: [i32; {N}] = [",
    ]
    for i in range(0, N, 8):
        out.append("    " + " ".join(f"{v}," for v in vals[i : i + 8]))
    out += ["];", ""]

    # --- arctangent -----------------------------------------------------
    # ATAN_TABLE[i] is atan(i / ATAN_STEPS) as a binary angle. Only the first
    # octant is stored; from_vector reconstructs the rest from the signs of dx
    # and dy and from which of |dx|, |dy| is larger. Values span 0..8192,
    # that is 0° to 45°.
    atan_vals = []
    for i in range(ATAN_STEPS + 1):
        a = math.atan(i / ATAN_STEPS) / (2 * math.pi) * 65536
        atan_vals.append(int(math.floor(a + 0.5)))

    assert atan_vals[0] == 0
    assert atan_vals[ATAN_STEPS] == 8192, atan_vals[ATAN_STEPS]
    assert all(b >= a for a, b in zip(atan_vals, atan_vals[1:])), "must be monotonic"

    out += [
        f"pub(crate) const ATAN_STEPS: i64 = {ATAN_STEPS};",
        "",
        f"pub(crate) static ATAN_TABLE: [u16; {ATAN_STEPS + 1}] = [",
    ]
    for i in range(0, len(atan_vals), 12):
        out.append("    " + " ".join(f"{v}," for v in atan_vals[i : i + 12]))
    out += ["];", ""]

    path = "crates/sim/src/trig_table.rs"
    with open(path, "w") as f:
        f.write("\n".join(out))

    # Format the output so the committed file is byte-identical to what a
    # regeneration produces. Without this, `cargo fmt` rewrites the file and the
    # CI check comparing the two would fail on every run.
    subprocess.run(["rustfmt", "--edition", "2024", path], check=True)
    print(f"wrote {path} ({N} entries, rustfmt applied)")


if __name__ == "__main__":
    main()
