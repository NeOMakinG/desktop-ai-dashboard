"""Offline packaging invariants; no Hermes/provider/controller execution."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("bundle.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class BundleTests(unittest.TestCase):
    def test_inventory_is_relative_and_hashed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "module.py").write_text("pass\n", encoding="utf-8")
            self.assertEqual(bundle.inventory(root), [{"path": "module.py", "sha256":
                "9f56e761d79bfdb34304a012586cb04d16b435ef6130091a97702e559260a2f2"}])

    def test_inventory_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "target").write_bytes(b"owned")
            (root / "link").symlink_to("target")
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                bundle.inventory(root)

    def test_materialize_keeps_owned_executable_inside(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "python").write_bytes(b"owned")
            (root / "python").chmod(0o755)
            (root / "python3").symlink_to("python")
            bundle.materialize(root)
            self.assertFalse((root / "python3").is_symlink())
            self.assertEqual((root / "python3").read_bytes(), b"owned")
            self.assertTrue((root / "python3").stat().st_mode & 0o111)

    def test_external_archive_link_is_not_copied(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "bundle"
            root.mkdir()
            (parent / "outside").write_bytes(b"must not copy")
            (root / "python").symlink_to("../outside")
            with self.assertRaisesRegex(RuntimeError, "escaped"):
                bundle.materialize(root)
            self.assertEqual((parent / "outside").read_bytes(), b"must not copy")

    def test_macho_identity_is_not_a_loaded_dependency(self):
        rpaths, dependencies = bundle.macho_load_commands([
            "cmd LC_ID_DYLIB", "cmdsize 56", "name libthread.dylib (offset 24)",
            "cmd LC_RPATH", "cmdsize 40", "path @loader_path/../lib (offset 12)",
            "cmd LC_LOAD_DYLIB", "cmdsize 72", "name @rpath/libpython3.12.dylib (offset 24)",
            "cmd LC_LOAD_WEAK_DYLIB", "cmdsize 72", "name /usr/lib/libSystem.B.dylib (offset 24)",
        ])
        self.assertEqual(rpaths, ["@loader_path/../lib"])
        self.assertEqual(dependencies, ["@rpath/libpython3.12.dylib", "/usr/lib/libSystem.B.dylib"])

    def test_only_exact_inert_rpath_image_is_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image = root / "library.dylib"
            image.write_bytes(b"\xcf\xfa\xed\xfe" + b"fixture")
            inert = {"file": "library.dylib", "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
                     "path": "/upstream/build", "loads": ["/usr/lib/libSystem.B.dylib"]}
            (root / "build-pins.json").write_text(json.dumps({"platforms": {"darwin-arm64": {
                "inertRpaths": [inert]}}}), encoding="utf-8")
            commands = "cmd LC_RPATH\ncmdsize 40\npath /upstream/build (offset 12)\n" + \
                "cmd LC_LOAD_DYLIB\ncmdsize 72\nname /usr/lib/libSystem.B.dylib (offset 24)\n"
            with patch.object(bundle.subprocess, "check_output", return_value=commands):
                self.assertEqual(bundle.macho_dependencies(root), (1, [inert]))
            active = commands.replace("/usr/lib/libSystem.B.dylib", "@rpath/libpython3.12.dylib")
            with patch.object(bundle.subprocess, "check_output", return_value=active):
                with self.assertRaisesRegex(RuntimeError, "became active"):
                    bundle.macho_dependencies(root)
            image.write_bytes(image.read_bytes() + b"changed")
            with patch.object(bundle.subprocess, "check_output", return_value=commands):
                with self.assertRaisesRegex(RuntimeError, "image changed"):
                    bundle.macho_dependencies(root)

    def test_pip_devnull_blocks_global_and_site_configuration(self):
        # Use the pristine bootstrap interpreter (includes pip), not final app
        # Python (deliberately excludes installers). No host config is opened.
        from pip._internal import configuration
        fixture = {configuration.kinds.GLOBAL: ["/unowned/global/pip.conf"],
                   configuration.kinds.SITE: ["/unowned/site/pip.conf"], configuration.kinds.USER: []}
        with patch.dict(os.environ, {"PIP_CONFIG_FILE": os.devnull}, clear=True):
            with patch.object(configuration, "get_configuration_files", return_value=fixture):
                with patch.object(configuration.Configuration, "_load_file") as read_config:
                    configuration.Configuration(isolated=True).load()
                    read_config.assert_not_called()

    def test_import_path_cannot_escape_to_sibling(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "bundle"
            root.mkdir()
            with self.assertRaisesRegex(RuntimeError, "escaped"):
                bundle.inside(parent / "bundle-other/lib", root)


if __name__ == "__main__":
    unittest.main()
