"""Checks for the failures that should stop a licensing update or release."""

import base64
import contextlib
import io
import json
import re
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import generate


class LicensingChecks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        root_patch = patch.object(generate, "ROOT", self.root)
        root_patch.start()
        self.addCleanup(root_patch.stop)

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value.encode("utf-8"))

    def test_tampered_archive_fails_before_license_collection(self):
        self.write("web/vendor/package.tgz", "tampered archive")
        integrity = "sha256-" + base64.b64encode(bytes.fromhex(generate.digest(b"original archive"))).decode()
        with self.assertRaisesRegex(ValueError, "Archive checksum mismatch"):
            generate.archive(dict(name="package", version="1", url="file:vendor/package.tgz", integrity=integrity))

    def test_changed_lockfile_or_generator_input_invalidates_notice_check(self):
        files = {"Cargo.lock": "locked packages\n", "dev/licensing/generate.py": "audited generator\n",
            "legal/third-party-rust.txt": "license texts\n"}
        for path, content in files.items():
            self.write(path, content)
        for name in ["LICENSE", "NOTICE"]:
            self.write(name, name)
            self.write(f"web/public/{name}.txt", name)
        self.write("legal/dependency-inventory.json", json.dumps(dict(
            lockfiles={"Cargo.lock": generate.digest(files["Cargo.lock"].encode())},
            inputs={"dev/licensing/generate.py": generate.digest(files["dev/licensing/generate.py"].encode())},
            artifacts={"legal/third-party-rust.txt": generate.digest(files["legal/third-party-rust.txt"].encode())},
            packages=[])))
        with patch.object(generate, "input_files", return_value=["dev/licensing/generate.py"]):
            with contextlib.redirect_stdout(io.StringIO()):
                generate.check()
            for path, content in files.items():
                with self.subTest(path=path):
                    self.write(path, content + "changed")
                    with self.assertRaisesRegex(SystemExit, "Licensing artifacts are stale"):
                        generate.check()
                    self.write(path, content)

    def test_unreviewed_license_expression_fails(self):
        for name in ["rust-supplements", "web-supplements", "license-selections", "separate-licenses"]:
            self.write(f"legal/{name}.json", "{}")
        self.write("legal/copied-sources.json", "[]")
        with self.assertRaisesRegex(ValueError, "Review new license expression"):
            generate.supplement([dict(ecosystem="rust", name="new-package", version="1", license="New-License")])

    def test_changed_supplement_text_fails(self):
        self.write("legal/upstream/LICENSE", "changed terms")
        with self.assertRaisesRegex(ValueError, "Supplement checksum mismatch"):
            generate.document_text(dict(path="legal/upstream/LICENSE", sha256=generate.digest(b"audited terms")))

    def test_mit_transformation_must_preserve_grant_terms(self):
        source = "MIT License\n\nCopyright (c) <year> <copyright holders>\n\nPermission is hereby granted.\n"
        changed = "MIT License\n\nPermission removed.\n"
        self.write("source.txt", source)
        self.write("terms.txt", changed)
        with self.assertRaisesRegex(ValueError, "MIT template transformation mismatch"):
            generate.document_text(dict(path="terms.txt", sha256=generate.digest(changed.encode()),
                source_path="source.txt", source_sha256=generate.digest(source.encode())))

    def test_separate_license_exception_is_bound_to_version_and_archive(self):
        self.write("web/vendor/status.txt", "No license supplied. Scope license excludes this component.")
        self.write("legal/separate-licenses.json", json.dumps({"web:pagent@0.1.0": dict(
            archive_sha256="audited-archive-hash", notice="web/vendor/status.txt", reason="Reviewed exclusion")}))
        entry = dict(ecosystem="web", name="pagent", version="0.1.0", archive_sha256="audited-archive-hash", license=None, documents=[])
        generate.apply_separate_terms([entry])
        self.assertFalse(generate.missing_coverage(entry))
        self.assertIsNone(entry["selected_license"])
        for change in [{"version": "0.1.1"}, {"archive_sha256": "new-archive-hash"}, {"license": "MIT"}]:
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "review|Review"):
                generate.apply_separate_terms([{**entry, **change}])
        unknown = dict(ecosystem="web", name="other-package", version="1", license=None, documents=[])
        self.assertTrue(generate.missing_coverage(unknown))

    def test_license_selection_distinguishes_mit_from_mit_zero(self):
        expression = "CC0-1.0 OR MIT-0 OR Apache-2.0"
        generate.validate_selections({expression: "MIT-0"})
        generate.validate_selections({expression: "Apache-2.0"})
        with self.assertRaisesRegex(ValueError, "not a declared alternative"):
            generate.validate_selections({expression: "MIT"})

    def test_license_selection_preserves_required_terms_and_exceptions(self):
        invalid = [
            ("MIT AND Apache-2.0", "MIT OR Apache-2.0"),
            ("MIT AND Apache-2.0", "MIT"),
            ("(MIT OR Apache-2.0) AND Unicode-3.0", "MIT"),
            ("Apache-2.0 WITH LLVM-exception", "Apache-2.0"),
            ("Apache-2.0", "Apache-2.0 WITH LLVM-exception"),
            ("MIT OR Apache-2.0", "MIT AND Apache-2.0"),
        ]
        for declared, selected in invalid:
            with self.subTest(declared=declared, selected=selected):
                with self.assertRaisesRegex(ValueError, "not a declared alternative"):
                    generate.validate_selections({declared: selected})
        generate.validate_selections({
            "(MIT OR Apache-2.0) AND Unicode-3.0": "MIT AND Unicode-3.0",
            "MIT OR Apache-2.0 AND Unicode-3.0": "MIT",
            "ISC AND (Apache-2.0 OR ISC)": "ISC",
            "MIT OR Apache-2.0 WITH LLVM-exception": "Apache-2.0 WITH LLVM-exception",
            "Apache-2.0/MIT": "MIT",
            "apache-2.0": "Apache-2.0",
        })

    def test_license_selection_rejects_malformed_expressions(self):
        for expression in ["", "MIT OR", "MIT AND (Apache-2.0", "MIT)",
                "MIT & Apache-2.0", "MIT Apache-2.0", "MIT WITH", "(MIT) WITH LLVM-exception"]:
            with self.subTest(expression=expression), self.assertRaises(ValueError):
                generate.validate_selections({expression: expression})

    def test_shared_terms_preserve_attribution_and_distinct_license_wording(self):
        terms = ("Permission is hereby granted, free of charge, to any person obtaining a copy.\n\n"
            "THE SOFTWARE IS PROVIDED AS IS. OTHER DEALINGS IN THE SOFTWARE.")
        original = [
            "Copyright Alice\n\n" + terms + "\nAdditional notice from Alice.",
            "Copyright Bob\n\n" + terms.replace(" ", "\n"),
            "Copyright Carol\n\n" + terms.replace("free of charge", "subject to an additional condition"),
        ]
        texts = {generate.digest(text.encode()): text for text in original}
        shared = generate.shared_terms(texts)
        self.assertEqual(len(shared), 1)
        for text in original:
            rendered = generate.reference_terms(text, shared)
            restored = re.sub(r"\[Shared terms ([a-f0-9]{64})\]", lambda match: shared[match[1]], rendered)
            self.assertEqual(" ".join(restored.split()), " ".join(text.split()))
        self.assertIn("Copyright Alice", generate.reference_terms(original[0], shared))
        self.assertIn("Additional notice from Alice.", generate.reference_terms(original[0], shared))
        self.assertEqual(generate.reference_terms(original[2], shared), original[2])

    def test_rendered_notices_include_all_referenced_terms(self):
        terms = "Apache License\nVersion 2.0, January 2004\nTerms.\nEND OF TERMS AND CONDITIONS"
        documents = [dict(path="LICENSE", text="Copyright " + name + "\n" + terms) for name in ["Alice", "Bob"]]
        entries = [dict(ecosystem="rust", name="package-" + str(index), version="1", license="Apache-2.0",
            selected_license="Apache-2.0", url="https://example.invalid/source", documents=[document])
            for index, document in enumerate(documents)]
        rendered = generate.render(entries, "rust")
        self.assertEqual(rendered.count(terms), 1)
        for reference in re.findall(r"\[Shared terms ([a-f0-9]{64})\]", rendered):
            self.assertIn(f"--- shared terms {reference} ---\n{terms}", rendered)
        for name in ["Alice", "Bob"]:
            self.assertIn("Copyright " + name, rendered)

    def test_compact_inventory_preserves_every_field(self):
        metadata = dict(lockfiles={"Cargo.lock": "checksum"}, inputs={}, artifacts={})
        packages = [dict(name="example", authors=["Copyright Holder"], documents=[dict(path="LICENSE", sha256="hash")])]
        rendered = generate.render_inventory(metadata, packages)
        self.assertEqual(json.loads(rendered), dict(**metadata, packages=packages))
        self.assertEqual(sum('"name":' in line for line in rendered.splitlines()), len(packages))


if __name__ == "__main__":
    unittest.main()
