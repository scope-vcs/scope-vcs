"""Copy legal sources to the locations where GitHub and the web server publish them."""

import argparse
import re
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COPIES = {
    "legal/SECURITY.md": ".github/SECURITY.md",
    "legal/security.txt": "web/public/.well-known/security.txt",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail if a published copy differs from its source.")
    args = parser.parse_args()
    expires = re.search(r"^Expires: (\S+)$", (ROOT / "legal/security.txt").read_text(encoding="utf-8"), re.M)
    if expires is None or datetime.fromisoformat(expires[1]) <= datetime.now(timezone.utc):
        raise SystemExit("legal/security.txt needs an Expires date in the future.")
    stale = []
    for source, target in COPIES.items():
        content = (ROOT / source).read_text(encoding="utf-8")
        destination = ROOT / target
        if args.check:
            if not destination.exists() or destination.read_text(encoding="utf-8") != content:
                stale.append(target)
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding="utf-8")
    if stale:
        raise SystemExit("Legal copies are stale. Run python3 dev/legal/distribute.py:\n" + "\n".join(stale))


if __name__ == "__main__":
    main()
