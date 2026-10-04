import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class StageSteamRelayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="steam staging ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.script = self.root / "scripts" / "stage-steam-relay.py"
        self.script.parent.mkdir()
        source = ROOT / "scripts" / self.script.name
        shutil.copy2(source, self.script)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self._program("rustc", 'printf "host: x86_64-unknown-linux-gnu\\n"')
        self._program("cargo", 'exit_code=${METADATA_EXIT:-0}\n'
                      'if [ "$exit_code" != 0 ]; then exit "$exit_code"; fi\n'
                      'printf "<%s>" "$@" > "$CARGO_LOG"\ncat "$METADATA"')
        self.sdk = self.root / "cargo registry" / "steamworks-sys-0.13.0"
        self.library = self.sdk / "lib/steam/redistributable_bin/linux64/libsteam_api.so"
        self.library.parent.mkdir(parents=True)
        self.library.write_bytes(b"synthetic native SDK library")
        self.library.chmod(0o644)
        (self.sdk / "Cargo.toml").write_text('[package]\nname="steamworks-sys"\nversion="0.13.0"\n')
        self.target = self.root / "custom target"
        self.relay = self.target / "x86_64-unknown-linux-gnu/debug/skate-steam-relay"
        self.relay.parent.mkdir(parents=True)
        self.relay.write_bytes(b"#!/bin/sh\nexit 0\n")
        self.relay.chmod(0o751)
        self.metadata = self.root / "metadata.json"
        self.document = {
            "packages": [{"name": "steamworks-sys", "version": "0.13.0",
                          "manifest_path": str(self.sdk / "Cargo.toml")}],
            "target_directory": str(self.target),
        }
        self._write_metadata()
        self.destination = self.target / "debug/steam-relay"
        self.log = self.root / "cargo.log"

    def _program(self, name, body):
        program = self.bin / name
        program.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
        program.chmod(0o755)

    def _write_metadata(self):
        self.metadata.write_text(json.dumps(self.document))

    def _run(self, *args, **env):
        environment = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                       "METADATA": str(self.metadata), "CARGO_LOG": str(self.log)}
        environment.pop("STEAM_SDK_LOCATION", None)
        environment.update(env)
        return subprocess.run([sys.executable, self.script, *args], cwd=self.root.parent,
                              env=environment, capture_output=True, text=True)

    def test_stages_native_files_using_metadata_target_directory_and_preserves_modes(self):
        result = self._run()
        self.assertEqual(result.returncode, 0, result.stderr)
        relay = self.destination / "skate-steam-relay"
        library = self.destination / "libsteam_api.so"
        self.assertEqual(relay.read_bytes(), b"#!/bin/sh\nexit 0\n")
        self.assertEqual(library.read_bytes(), b"synthetic native SDK library")
        self.assertEqual(stat.S_IMODE(relay.stat().st_mode), 0o751)
        self.assertEqual(stat.S_IMODE(library.stat().st_mode), 0o644)
        self.assertEqual(self.log.read_text(),
                         "<metadata><--format-version><1><--locked><--filter-platform><x86_64-unknown-linux-gnu>")

    def test_metadata_failures_do_not_replace_previous_staging(self):
        self.destination.mkdir(parents=True)
        old = self.destination / "skate-steam-relay"
        old.write_bytes(b"previous helper")
        for document in [None, {}, {"packages": []},
                         {**self.document, "packages": [{"name": "steamworks-sys", "version": "0.12.0"}]},
                         {**self.document, "packages": self.document["packages"] * 2},
                         {**self.document, "packages": [{"name": "steamworks-sys", "version": "0.13.0"}]},
                         {**self.document, "target_directory": None}]:
            with self.subTest(document=document):
                self.metadata.write_text("invalid json" if document is None else json.dumps(document))
                result = self._run()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("metadata", result.stderr.lower())
                self.assertEqual(old.read_bytes(), b"previous helper")
                self.assertFalse((self.destination / "libsteam_api.so").exists())

    def test_failed_metadata_command_reports_failure_without_staging(self):
        result = self._run(METADATA_EXIT="42")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("metadata", result.stderr.lower())
        self.assertIn("42", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_missing_sdk_library_does_not_publish_a_partial_stage(self):
        self.library.unlink()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("libsteam_api.so", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_missing_or_nonexecutable_helper_does_not_publish_a_partial_stage(self):
        self.relay.chmod(0o644)
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("executable", result.stderr)
        self.assertFalse(self.destination.exists())
        self.relay.unlink()
        result = self._run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("skate-steam-relay", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_sdk_override_is_rejected_to_keep_build_and_staged_library_in_sync(self):
        result = self._run("--check-platform", STEAM_SDK_LOCATION="/another SDK")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("STEAM_SDK_LOCATION", result.stderr)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
