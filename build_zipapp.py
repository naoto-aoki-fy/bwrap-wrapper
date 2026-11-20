"""Build a standalone executable with the standard-library zipapp module."""

import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
import zipapp

ROOT = Path(__file__).resolve().parent


def build(output_dir, interpreter):
    if not Path(interpreter).is_absolute() or any(c in interpreter for c in " \t\r\n\0"):
        raise ValueError("the interpreter must be an absolute path without whitespace or NUL")

    _, marker = (ROOT / "bwrap").read_bytes().splitlines()[:2]
    shebang = b"#!" + os.fsencode(interpreter) + b" -ISBXutf8\n"
    # Linux reads at most 256 bytes when parsing a script's interpreter.
    if len(shebang) > 256:
        raise ValueError("the interpreter path is too long for a Linux shebang")

    output_dir = Path(output_dir).resolve()
    destination = output_dir / "bwrap"
    if destination == ROOT / "bwrap":
        raise ValueError("choose an output directory separate from the source launcher")

    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output_dir) as tmp:
        staging = Path(tmp) / "app"
        package = staging / "bwrap_wrapper"
        package.mkdir(parents=True)
        for module in (ROOT / package.name).glob("*.py"):
            shutil.copyfile(module, package / module.name)
        for name in ("LICENSE", "NOTICE"):
            shutil.copyfile(ROOT / name, staging / name)
        (staging / "__main__.py").write_text(
            "import sys\nfrom bwrap_wrapper.runtime import main\n\nmain(sys.argv[0])\n",
            encoding="utf-8",
        )

        archive = Path(tmp) / "bwrap"
        with archive.open("wb") as stream:
            stream.write(shebang + marker + b"\n")
            zipapp.create_archive(staging, stream, compressed=True)
        archive.chmod(0o755)
        archive.replace(destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    parser.add_argument("--python", default=sys.executable, help="absolute interpreter path")
    args = parser.parse_args()
    try:
        print(build(args.output_dir, args.python))
    except (OSError, ValueError) as error:
        parser.exit(1, f"build_zipapp.py: {error}\n")


if __name__ == "__main__":
    main()
