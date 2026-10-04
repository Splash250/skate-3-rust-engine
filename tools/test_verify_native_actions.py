"""Keep owned-data native tests and standalone input pilots on the same world."""
import json
from pathlib import Path
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from tools import resource_park
from tools.verify_native_actions import scenario_world, snapshot_digest


class NativeActionFixtureTests(unittest.TestCase):
    def test_companion_path_uses_the_native_platform_executable_suffix(self):
        from tools.verify_native_actions import native_executable
        directory = Path('target/debug')
        for platform, filename in [('nt', 'skate3rust.exe'), ('posix', 'skate3rust')]:
            with self.subTest(platform=platform), patch('tools.verify_native_actions.os', SimpleNamespace(name=platform)):
                self.assertEqual(native_executable(directory), directory / filename)

    def test_procedural_sources_match_the_native_test_fixtures(self):
        fixtures = Path(__file__).resolve().parents[1] / 'crates/skate-game/src/tests/fixtures'
        for scenario in ('grab', 'rail'):
            with self.subTest(scenario=scenario):
                source = json.loads((fixtures / f'native-authority-{scenario}.json').read_text())
                expected = (fixtures / f'native-authority-{scenario}.skate').read_bytes()
                self.assertEqual(scenario_world(scenario), source)
                self.assertEqual(resource_park.encode(source), expected)
        self.assertEqual(scenario_world('combo'), scenario_world('grab'),
                         'the two-trick combo must reuse the authored grab world unchanged')

    def test_evidence_digest_detects_state_changes_and_rejects_nan(self):
        snapshot = {'tick': 5, 'root': {'p': [1., 2., 3.]}, 'score': {'awarded': 22.}}
        self.assertEqual(snapshot_digest(snapshot), snapshot_digest(dict(reversed(list(snapshot.items())))))
        changed = dict(snapshot, score={'awarded': 99.})
        self.assertNotEqual(snapshot_digest(snapshot), snapshot_digest(changed))
        with self.assertRaises(ValueError):
            snapshot_digest({'score': float('nan')})
