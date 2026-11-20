"""Parse the Linux mount table for WSL2 Windows drive mounts."""

from pathlib import Path
import re

OCTAL_ESCAPE = re.compile(r"\\([0-7]{3})")


def parse(contents):
    mounts = set()
    for line in re.split(r"[\r\n]", contents):
        fields = line.split(maxsplit=4)
        if len(fields) < 4 or fields[2] != "9p":
            continue
        if "aname=drvfs" in re.split(r"[,;]", fields[3]):
            mounts.add(OCTAL_ESCAPE.sub(lambda m: chr(int(m[1], 8)), fields[1]))
    return sorted(mounts, key=lambda path: (path.count("/"), path))


def read_file(path="/proc/mounts"):
    return parse(Path(path).read_text(encoding="utf-8"))
