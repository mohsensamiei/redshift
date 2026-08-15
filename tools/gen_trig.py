#!/usr/bin/env python3
"""Regenerate crates/sim/src/trig_table.rs.

The table is committed to the repository rather than computed at build or run
time. Every peer in a lockstep match must use bit-identical values, and
floating-point evaluation is not guaranteed to agree across platforms or
toolchains. Generating once and committing the integers removes that risk.

Run from the repository root:  python3 tools/gen_trig.py
"""

import math

N = 4096  # entries covering a full turn
ONE = 1 << 16  # Fx scale: 1.0 == 65536


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

    with open("crates/sim/src/trig_table.rs", "w") as f:
        f.write("\n".join(out))
    print(f"wrote crates/sim/src/trig_table.rs ({N} entries)")


if __name__ == "__main__":
    main()
