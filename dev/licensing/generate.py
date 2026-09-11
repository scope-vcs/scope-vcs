#!/usr/bin/env python3
"""Collect licenses from checksum-verified archives named by Scope's lockfiles."""

import argparse
import base64
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tempfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path(tempfile.gettempdir()) / "scope-license-archives"
LOCKFILES = ["Cargo.lock", "cli/Cargo.lock", "web/pnpm-lock.yaml"]
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|copyright|notice|ofl|unlicense)([._-].*)?$", re.I)
SHARED_TERMS = re.compile(
    r"Permission\s+is\s+hereby\s+granted,.*?OTHER\s+DEALINGS\s+IN\s+THE\s+SOFTWARE\."
    r"|Apache\s+License\s+Version\s+2\.0,\s+January\s+2004.*?END\s+OF\s+TERMS\s+AND\s+CONDITIONS",
    re.S,
)


def digest(data, algorithm="sha256"):
    return hashlib.new(algorithm, data).hexdigest()


def packages():
    import yaml

    crates = {}
    for lock in ["Cargo.lock", "cli/Cargo.lock"]:
        for package in tomllib.loads((ROOT / lock).read_text())['package']:
            source = package.get("source")
            if not source:
                continue
            if source != "registry+https://github.com/rust-lang/crates.io-index":
                raise ValueError(f"Unsupported Cargo source: {source}")
            name, version = package["name"], package["version"]
            key = f"{name}@{version}"
            entry = crates.setdefault(key, dict(ecosystem="rust", name=name, version=version,
                url=f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                integrity="sha256-" + base64.b64encode(bytes.fromhex(package["checksum"])).decode(), lockfiles=[]))
            if entry["integrity"] != "sha256-" + base64.b64encode(bytes.fromhex(package["checksum"])).decode():
                raise ValueError(f"Conflicting crate checksums: {key}")
            entry["lockfiles"].append(lock)
    result = list(crates.values())
    lock = yaml.safe_load((ROOT / "web/pnpm-lock.yaml").read_text())
    for key, package in lock["packages"].items():
        name, version = key.rsplit("@", 1)
        resolution = package["resolution"]
        url = resolution.get("tarball")
        if not url:
            url = f"https://registry.npmjs.org/{name}/-/{name.rsplit('/', 1)[-1]}-{version}.tgz"
        result.append(dict(ecosystem="web", name=name, version=package.get("version", version),
            url=url, integrity=resolution["integrity"], lockfiles=["web/pnpm-lock.yaml"]))
    return sorted(result, key=lambda item: (item["ecosystem"], item["name"], item["version"]))


def archive(package):
    algorithm, encoded = package["integrity"].split("-", 1)
    expected = base64.b64decode(encoded)
    cache = CACHE / expected.hex()
    if package["url"].startswith("file:"):
        data = (ROOT / "web" / package["url"][5:]).read_bytes()
    elif cache.exists():
        data = cache.read_bytes()
    else:
        with urllib.request.urlopen(package["url"], timeout=120) as response:
            data = response.read()
        if hashlib.new(algorithm, data).digest() == expected:
            cache.write_bytes(data)
    if hashlib.new(algorithm, data).digest() != expected:
        raise ValueError(f"Archive checksum mismatch: {package['name']} {package['version']}")
    return data


def collect(package):
    data = archive(package)
    documents = []
    metadata = None
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as bundle:
        for member in bundle.getmembers():
            if not member.isfile():
                continue
            path = member.name.split("/", 1)[-1]
            if path == ("Cargo.toml" if package["ecosystem"] == "rust" else "package.json"):
                raw = bundle.extractfile(member).read().decode("utf-8")
                metadata = tomllib.loads(raw)["package"] if package["ecosystem"] == "rust" else json.loads(raw)
            is_document = Path(path).suffix.lower() not in {".rs", ".js", ".ts", ".c", ".h", ".cc", ".cpp", ".json", ".map"}
            if is_document and (LICENSE_NAME.match(Path(path).name) or any(part.lower() == "licenses" for part in Path(path).parts[:-1])):
                raw = bundle.extractfile(member).read()
                text = raw.decode("utf-8-sig").replace("\r\n", "\n").strip()
                if text:
                    documents.append(dict(path=path, sha256=digest(raw), text=text))
    if metadata is None:
        raise ValueError(f"Missing package manifest: {package['name']}")
    license_value = metadata.get("license", metadata.get("licenses"))
    if isinstance(license_value, (dict, list)):
        license_value = json.dumps(license_value, sort_keys=True)
    return dict(**package, license=license_value, authors=metadata.get("authors", metadata.get("author")), repository=metadata.get("repository"), archive_sha256=digest(data),
        documents=sorted(documents, key=lambda document: document["path"]))


def supplement(entries):
    for entry in json.loads((ROOT / "legal/copied-sources.json").read_text(encoding="utf-8")):
        for document in entry["documents"]:
            document["text"] = document_text(document)
        entries.append(entry)
    by_key = {f"{entry['ecosystem']}:{entry['name']}@{entry['version']}": entry for entry in entries}
    configuration = {}
    for path in ["legal/rust-supplements.json", "legal/web-supplements.json"]:
        configuration.update(json.loads((ROOT / path).read_text(encoding="utf-8")))
    for key, addition in configuration.items():
        if key not in by_key:
            raise ValueError(f"Remove stale license supplement: {key}")
        entry = by_key[key]
        entry["license_evidence"] = addition["reason"]
        if addition.get("upstream_license"):
            if entry["archive_sha256"] != addition["archive_sha256"] or entry["license"] is not None:
                raise ValueError(f"Review upstream license supplement for changed package: {key}")
            entry["upstream_license"] = addition["upstream_license"]
        for document in addition.get("documents", []):
            entry["documents"].append(dict(**document, text=document_text(document)))
        if addition.get("declared_metadata"):
            text = json.dumps(dict(name=entry["name"], version=entry["version"], license=entry["license"], authors=entry["authors"]), indent=2, ensure_ascii=False)
            entry["documents"].append(dict(path="Published package license and author metadata", sha256=digest(text.encode()), text=text))
        if addition.get("archive_document"):
            spec = addition["archive_document"]
            with tarfile.open(fileobj=io.BytesIO(archive(entry)), mode="r:gz") as bundle:
                member = next(member for member in bundle if member.name.split("/", 1)[-1] == spec["path"])
                raw = bundle.extractfile(member).read()
            text = raw.decode("utf-8-sig").replace("\r\n", "\n")
            start = text.index(spec["start"])
            text = text[start:]
            entry["documents"].append(dict(path=spec["path"] + " (license section)", sha256=digest(raw), text=text.strip()))
    # Parent documents are resolved after local supplements, for native package siblings.
    for key, addition in configuration.items():
        if addition.get("from_package"):
            parent = by_key[addition["from_package"]]
            if not parent["documents"]:
                raise ValueError(f"Missing parent license texts: {addition['from_package']}")
            by_key[key]["documents"].extend(parent["documents"])
    selections = json.loads((ROOT / "legal/license-selections.json").read_text())
    validate_selections(selections)
    for entry in entries:
        declaration = entry["license"] or entry.get("upstream_license")
        if declaration:
            if declaration not in selections:
                raise ValueError(f"Review new license expression: {declaration}")
            entry["selected_license"] = selections[declaration]
    return sorted(entries, key=lambda item: (item["ecosystem"], item["name"], item["version"]))


def validate_selections(selections):
    """Keep every selected alternative within the declared combinations of terms."""
    for declared, selected in selections.items():
        if not license_choices(selected) <= license_choices(declared):
            raise ValueError(f"Selected license is not a declared alternative: {declared} -> {selected}")


def license_choices(expression):
    """Expand AND/OR expressions into alternatives, keeping WITH exceptions attached."""
    tokens = re.findall(r"[A-Za-z0-9][A-Za-z0-9.+-]*|[()/]", expression)
    if "".join(tokens) != re.sub(r"\s+", "", expression):
        raise ValueError(f"Invalid license expression: {expression}")
    position = 0

    def take(token):
        nonlocal position
        if position < len(tokens) and tokens[position] == token:
            position += 1
            return True
        return False

    def identifier():
        nonlocal position
        if position == len(tokens) or tokens[position] in {"AND", "OR", "WITH", "(", ")", "/"}:
            raise ValueError(f"Expected license identifier: {expression}")
        value = tokens[position].lower()
        position += 1
        return value

    def term():
        if take("("):
            choices = alternatives()
            if not take(")"):
                raise ValueError(f"Unclosed license expression: {expression}")
            return choices
        value = identifier()
        if take("WITH"):
            value += " WITH " + identifier()
        return {frozenset([value])}

    def conjunction():
        choices = term()
        while take("AND"):
            following = term()
            choices = {left | right for left in choices for right in following}
        return choices

    def alternatives():
        choices = conjunction()
        # Published crate manifests in this inventory also use slash for alternatives.
        while take("OR") or take("/"):
            choices |= conjunction()
        return choices

    choices = alternatives()
    if position != len(tokens):
        raise ValueError(f"Unexpected license expression token: {expression}")
    return choices


def missing_coverage(entry):
    return not (entry["license"] or entry.get("upstream_license")) or not entry["documents"] or (
        not entry.get("license_evidence") and not any("/" not in doc["path"] for doc in entry["documents"]))


def document_text(document):
    raw = (ROOT / document["path"]).read_bytes()
    if digest(raw) != document["sha256"]:
        raise ValueError(f"Supplement checksum mismatch: {document['path']}")
    if document.get("source_path"):
        source = (ROOT / document["source_path"]).read_bytes()
        if digest(source) != document["source_sha256"]:
            raise ValueError(f"Canonical source checksum mismatch: {document['source_path']}")
        expected = source.replace(b"Copyright (c) <year> <copyright holders>\n\n", b"", 1)
        if raw != expected:
            raise ValueError(f"MIT template transformation mismatch: {document['path']}")
    return raw.decode("utf-8-sig").replace("\r\n", "\n").strip()


def shared_terms(texts):
    """Share repeated terms only when all words and punctuation are identical."""
    matches = [match.group() for text in texts.values() for match in SHARED_TERMS.finditer(text)]
    counts = Counter(" ".join(text.split()) for text in matches)
    terms = {}
    for text in matches:
        normalized = " ".join(text.split())
        if counts[normalized] > 1:
            terms.setdefault(digest(normalized.encode()), text)
    return terms


def reference_terms(text, terms):
    def replace(match):
        reference = digest(" ".join(match.group().split()).encode())
        return f"[Shared terms {reference}]" if reference in terms else match.group()
    return SHARED_TERMS.sub(replace, text)


def render(entries, ecosystem):
    output = ["Third-party licenses for Scope", "",
        "Generated from checksum-verified dependency archives by dev/licensing/generate.py.",
        "Includes every locked dependency, including build, development, optional, and",
        "platform-specific packages. An entry does not imply it is linked into every build.",
        "Third-party components retain their own licenses. Scope's Apache-2.0 license",
        "does not replace those terms. References to shared terms resolve to the",
        "complete text at the end of this document; package-specific notices remain",
        "with each license document.", ""]
    texts = {}
    for entry in entries:
        if entry["ecosystem"] != ecosystem:
            continue
        output.extend(["=" * 78, f"{entry['name']} {entry['version']}",
            f"Declared license: {entry['license'] or 'not declared'}", f"Selected license: {entry['selected_license']}", f"Source: {entry['url']}", ""])
        if entry.get("upstream_license"):
            output.append("Upstream license: " + entry["upstream_license"])
        if entry.get("authors"):
            output.append("Published author metadata: " + json.dumps(entry["authors"], ensure_ascii=False))
        if entry.get("repository"):
            output.append("Repository: " + json.dumps(entry["repository"], ensure_ascii=False))
        if entry.get("license_evidence"):
            output.extend([entry["license_evidence"], ""])
        for document in entry["documents"]:
            reference = digest(document["text"].encode())
            texts[reference] = document["text"]
            output.append(f"{document['path']}: text {reference}")
        output.append("")
    output.extend(["=" * 78, "License and notice texts", ""])
    terms = shared_terms(texts)
    for reference, text in sorted(texts.items()):
        output.extend([f"--- text {reference} ---", reference_terms(text, terms), ""])
    output.extend(["=" * 78, "Shared license terms", ""])
    for reference, text in sorted(terms.items()):
        output.extend([f"--- shared terms {reference} ---", text, ""])
    return "\n".join(line.rstrip() for line in "\n".join(output).split("\n")).rstrip() + "\n"


def render_inventory(metadata, packages):
    """Keep one dependency per line so changes remain reviewable without JSON padding."""
    header = json.dumps(metadata, indent=2, ensure_ascii=False).removesuffix("\n}")
    records = ",\n".join("    " + json.dumps(entry, ensure_ascii=False) for entry in packages)
    return header + ',\n  "packages": [\n' + records + "\n  ]\n}\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail if committed artifacts differ.")
    parser.add_argument("--audit-only", action="store_true", help="Report missing coverage without writing artifacts.")
    args = parser.parse_args()
    check_first_party()
    if args.check:
        check()
        return
    CACHE.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=12) as executor:
        entries = supplement(list(executor.map(collect, packages())))
    missing = [f"{entry['ecosystem']} {entry['name']}@{entry['version']} ({entry['license']})"
        for entry in entries if missing_coverage(entry)]
    if missing:
        print("Missing license declaration or license text:\n" + "\n".join(missing))
        (CACHE / "audit.json").write_text(json.dumps(entries, indent=2), encoding="utf-8")
        raise SystemExit(1)
    if args.audit_only:
        print(f"Audited {len(entries)} dependency and copied-source entries.")
        return
    inventory = [{**entry, "documents": [{key: value for key, value in document.items() if key != "text"}
        for document in entry["documents"]]} for entry in entries]
    outputs = {
        "legal/third-party-rust.txt": render(entries, "rust"),
        "web/public/third-party-licenses.txt": render(entries, "web"),
        "web/public/LICENSE.txt": (ROOT / "LICENSE").read_text(encoding="utf-8"),
        "web/public/NOTICE.txt": (ROOT / "NOTICE").read_text(encoding="utf-8"),
    }
    outputs["legal/dependency-inventory.json"] = render_inventory(dict(
        lockfiles={path: digest((ROOT / path).read_text(encoding="utf-8").encode()) for path in LOCKFILES},
        inputs={path: input_digest(path) for path in input_files()},
        artifacts={path: digest(content.encode()) for path, content in outputs.items()}), inventory)
    for path, content in outputs.items():
        target = ROOT / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content.encode("utf-8"))
    print(f"Generated notices for {len(entries)} dependency versions.")


def check():
    """Check freshness offline using only Python's standard library."""
    inventory = json.loads((ROOT / "legal/dependency-inventory.json").read_text(encoding="utf-8"))
    stale = []
    for path, expected in {**inventory["lockfiles"], **inventory["artifacts"]}.items():
        target = ROOT / path
        if not target.exists() or digest(target.read_text(encoding="utf-8").encode()) != expected:
            stale.append(path)
    if sorted(inventory["inputs"]) != input_files():
        stale.append("licensing input file list")
    for path, expected in inventory["inputs"].items():
        target = ROOT / path
        if not target.exists() or input_digest(path) != expected:
            stale.append(path)
    for name in ["LICENSE", "NOTICE"]:
        if (ROOT / name).read_text(encoding="utf-8") != (ROOT / "web/public" / f"{name}.txt").read_text(encoding="utf-8"):
            stale.append(name)
    if stale:
        raise SystemExit("Licensing artifacts are stale. Run python dev/licensing/generate.py:\n" + "\n".join(stale))
    print(f"Checked notices for {len(inventory['packages'])} dependency versions.")


def input_files():
    paths = ["dev/licensing/generate.py", "dev/licensing/requirements.txt", "legal/license-selections.json",
        "legal/rust-supplements.json", "legal/web-supplements.json", "legal/copied-sources.json"]
    paths.extend(path.relative_to(ROOT).as_posix() for path in (ROOT / "legal/upstream").glob("*"))
    paths.extend(path.relative_to(ROOT).as_posix() for path in (ROOT / "web/vendor").glob("*.tgz"))
    return sorted(paths)


def input_digest(path):
    target = ROOT / path
    if path.startswith(("legal/upstream/", "web/vendor/")):
        return digest(target.read_bytes())
    return digest(target.read_text(encoding="utf-8").encode())


def check_first_party():
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    if workspace["package"]["license"] != "Apache-2.0":
        raise ValueError("Workspace license must be Apache-2.0")
    for member in [*workspace["members"], "cli"]:
        package = tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]
        if package.get("license") not in ["Apache-2.0", {"workspace": True}]:
            raise ValueError(f"First-party license must be Apache-2.0: {member}")
    if json.loads((ROOT / "web/package.json").read_text())["license"] != "Apache-2.0":
        raise ValueError("Web license must be Apache-2.0")


if __name__ == "__main__":
    main()
