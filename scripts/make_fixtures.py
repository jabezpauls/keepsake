#!/usr/bin/env python3
"""Phase-1 fixture generator.

The integration tests generate synthetic JPEG dumps on the fly via
``tests/support/mod.rs`` — no checked-in binary data is required. This
script exists for the rare case where a human wants to reproduce the
fixture tree on disk (e.g. to exercise ``tauri dev`` manually). It
shells out to ``cargo test`` with a filter that only instantiates the
helper and copies the resulting directory to the requested output path.

Usage::

    scripts/make_fixtures.py --out /tmp/iphone_dump

The output directory is overwritten.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


TEMPLATE = """\
#[path = "support/mod.rs"]
mod support;

#[test]
fn emit_dump() {{
    let root = std::path::PathBuf::from(std::env::var("MV_FIXTURE_OUT").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    support::make_iphone_dump(&root);
}}
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True, help="destination dir")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    tests_dir = repo_root / "crates" / "core" / "tests"

    if args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True)

    with tempfile.NamedTemporaryFile(
        "w", dir=tests_dir, suffix="_emit_fixtures.rs", delete=False
    ) as fh:
        fh.write(TEMPLATE)
        tmp_path = Path(fh.name)

    try:
        env = dict(os.environ)
        env["MV_FIXTURE_OUT"] = str(args.out.resolve())
        subprocess.run(
            [
                "cargo",
                "test",
                "-p",
                "mv-core",
                "--test",
                tmp_path.stem,
                "--",
                "emit_dump",
                "--nocapture",
            ],
            check=True,
            cwd=repo_root,
            env=env,
        )
    finally:
        tmp_path.unlink(missing_ok=True)

    files = list(args.out.rglob("*"))
    print(f"wrote {len(files)} fixture files under {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
