"""Run the Python wrapper with a synthetic mount table for differential tests."""

import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from bwrap_wrapper import mounts, runtime

try:
    runtime.run(sys.argv[1:], __file__, lambda: mounts.read_file(os.environ["BWRAP_TEST_MOUNTS"]))
except (OSError, ValueError) as error:
    print(f"bwrap-wrapper: {error}", file=sys.stderr)
    raise SystemExit(1) from None
