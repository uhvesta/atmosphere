"""The generated notices name every crate they were given and every crate the
shipped binaries link, and aren't empty."""

import os
import sys
import unittest
from pathlib import Path

NOTICES, CRATES, LINKED = sys.argv[1:4]
sys.argv = sys.argv[:1]


def runfiles_dir():
    found = os.environ.get("RUNFILES_DIR")
    if found:
        return Path(found)
    for parent in Path(__file__).resolve().parents:
        if parent.name.endswith(".runfiles"):
            return parent
    raise ValueError("Bazel runfiles directory is missing")


def read(rlocation_path):
    path = (runfiles_dir() / rlocation_path).resolve(strict=True)
    return path.read_text(encoding="utf-8", errors="replace")


class NoticesTest(unittest.TestCase):
    def test_names_every_crate(self):
        crates = [line for line in read(CRATES).splitlines() if line]
        self.assertTrue(crates, "the crate list is empty")
        headings = {line for line in read(NOTICES).splitlines() if line.startswith("Crate: ")}
        missing = [crate for crate in crates if "Crate: " + crate not in headings]
        self.assertEqual(missing, [])

    def test_covers_every_linked_crate(self):
        linked = [line for line in read(LINKED).splitlines() if line]
        self.assertTrue(linked, "the linked crate list is empty")
        headings = {line for line in read(NOTICES).splitlines() if line.startswith("Crate: ")}
        # The linked list takes versions from repository names, where a
        # build tag's "+" is spelled "-".
        named = {heading.replace("+", "-") for heading in headings}
        missing = [crate for crate in linked if "Crate: " + crate.replace("+", "-") not in named]
        self.assertEqual(missing, [])

    def test_not_empty(self):
        text = read(NOTICES)
        self.assertGreater(len(text.strip()), 1000)

    def test_every_crate_has_license_text(self):
        # A crate's section holds a license file or standard text, so the
        # section is longer than its three heading lines.
        sections = read(NOTICES).split("Crate: ")[1:]
        bare = [s.splitlines()[0] for s in sections if len(s.split("\n" + "-" * 80 + "\n", 1)[-1].strip()) < 50]
        self.assertEqual(bare, [])


if __name__ == "__main__":
    unittest.main()
