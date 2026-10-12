"""Verify the shipped module archive layout without compiling native code."""
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent

class PackageTests(unittest.TestCase):
    def test_arch_pacman_disables_filesystem_sandbox_for_containers(self):
        script = (ROOT / "scripts/test-distro.sh").read_text()
        self.assertIn("pacman --disable-sandbox -Syu", script)

    def test_release_archive_preserves_module_specification(self):
        workflow = (ROOT/".github/workflows/release.yml").read_text()
        match = re.search(r"extra-packaged-files: *'([^']+)'", workflow)
        self.assertIsNotNone(match, "release must configure additional archive documentation")
        self.assertIn("docs/specs/tinybus-module.md", json.loads(match[1]))

    def test_archive_attests_host_library_bytes_from_a_symlinked_checkout(self):
        """Package the actual build script and verify the shipped bytes and attestation."""
        target = ROOT / "target"; target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            root = Path(directory); (root/"scripts").mkdir(); (root/"bin").mkdir()
            shutil.copy2(ROOT/"scripts/build-module.sh", root/"scripts/build-module.sh")
            for file in ["LICENSE","README.md","MODULE.md"]: (root/file).write_text("fixture")
            (root/"docs/specs").mkdir(parents=True)
            (root/"docs/specs/tinybus-module.md").write_text("module specification")
            cargo = root/"bin/cargo"
            cargo.write_text("#!/usr/bin/env python3\nimport json,sys\nfrom pathlib import Path\n"
                "root=Path.cwd()\n"
                "if 'metadata' in sys.argv: print(json.dumps({'target_directory':str(root/'target')}))\n"
                "else:\n"
                " target=sys.argv[sys.argv.index('--target')+1]\n"
                " output=root/'target'/target/'release'; output.mkdir(parents=True,exist_ok=True)\n"
                " (output/'libtinyjuice_module.so').write_bytes(b'compiled-fixture')\n")
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=str(root/"bin")+os.pathsep+os.environ["PATH"])
            env.pop("CARGO_TARGET_DIR", None)
            alias = root/"checkout-link"; alias.symlink_to(root, target_is_directory=True)
            result = subprocess.run(["bash",str(alias/"scripts/build-module.sh"),
                "--crate","tinyjuice-module","--version","1.2.3-beta.1",
                "--target","x86_64-unknown-linux-gnu","--host","ubuntu-22.04-x86_64",
                "--extra-files-json", '["docs/specs/tinybus-module.md"]'],
                cwd=root,env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            archive=root/"release-assets/tinyjuice-module-1.2.3-beta.1-ubuntu-22.04-x86_64.tar.gz"
            with tarfile.open(archive) as package:
                data=package.extractfile("./libtinyjuice_module.so").read()
                manifest=tomllib.loads(package.extractfile("./modules.toml").read().decode())
                self.assertEqual(data,b"compiled-fixture")
                self.assertEqual(manifest,{"libtinyjuice_module.so":hashlib.sha256(data).hexdigest()})
                for name in ["./LICENSE","./README.md","./MODULE.md","./docs/specs/tinybus-module.md"]:
                    self.assertIsNotNone(package.extractfile(name))

if __name__ == "__main__": unittest.main()
