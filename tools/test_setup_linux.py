import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.asset_pipeline import install
from tools.asset_pipeline import customiser_setup
from tools import setup


class NativeExtractorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.base = self.root/'base'
        self.game = self.root/'bin'/'skate3'
        self.game.parent.mkdir()
        self.game.write_bytes(b'')

    def tearDown(self):
        self.temp.cleanup()

    def test_override_has_precedence(self):
        override = self.root/'custom-xiso'
        override.write_bytes(b'')
        adjacent = self.game.parent/'skate-xiso'
        adjacent.write_bytes(b'')
        with mock.patch.dict(os.environ, {'SKATE_XISO': str(override)}):
            self.assertEqual(install.xiso_extractor(self.base, self.game, print), override)

    def test_missing_override_is_an_error(self):
        with mock.patch.dict(os.environ, {'SKATE_XISO': str(self.root/'missing')}):
            with self.assertRaisesRegex(RuntimeError, 'SKATE_XISO does not name an existing file'):
                install.xiso_extractor(self.base, self.game, print)


class HeadlessSetupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.base = self.root/'base'
        self.game = self.root/'bin'/'skate3'
        self.game.parent.mkdir()
        self.game.write_bytes(b'')

    def tearDown(self):
        self.temp.cleanup()

    def test_source_dispatch_succeeds_without_tk(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);base=root/'base';source=root/'default.xex';source.write_bytes(b'')
            with mock.patch('tools.asset_pipeline.customiser_setup.install', return_value=root/'installed') as operation, \
                 mock.patch('tools.asset_pipeline.optional_content.summary', return_value=[]), \
                 mock.patch.dict('sys.modules', {'tkinter': None}):
                status=setup.main(['--base',str(base),'--game-exe',str(root/'game'),'--source',str(source)])
            self.assertEqual(status,0)
            operation.assert_called_once()

    def test_source_dispatch_records_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);base=root/'base'
            with mock.patch('tools.asset_pipeline.customiser_setup.install', side_effect=RuntimeError('broken')):
                status=setup.main(['--base',str(base),'--game-exe',str(root/'game'),'--source',str(root/'disc')])
            self.assertEqual(status,2)
            self.assertIn('broken',(base/'setup-error.log').read_text())

    def test_non_windows_gui_skips_iconbitmap(self):
        icon=self.root/'icon.ico';icon.write_bytes(b'icon')
        window=mock.Mock()
        with mock.patch.object(setup.os,'name','posix'):
            setup.set_window_icon(window,icon)
        window.iconbitmap.assert_not_called()

    def test_extracted_directory_and_default_xex_resolve_to_same_root(self):
        from tools.asset_pipeline.setup_state import source_directory
        game=self.root/'disc';(game/'data/big').mkdir(parents=True)
        (game/'data/content').mkdir(parents=True)
        for relative in ('default.xex','data/big/miscload.big','data/big/miscboot.big',
                         'data/big/db.big','data/content/createacharacter.big'):
            (game/relative).write_bytes(b'owned fixture')
        self.assertEqual(source_directory(game),game.resolve())
        self.assertEqual(source_directory(game/'default.xex'),game.resolve())

    def test_iso_invocation_puts_options_before_operands(self):
        image=self.root/'disc.iso';image.write_bytes(b'fixture')
        extractor=self.root/'skate-xiso';extractor.write_bytes(b'')
        calls=[]
        def capture(args,log,report):calls.append(args)
        with mock.patch.object(install,'xiso_extractor',return_value=extractor), \
             mock.patch.object(install,'run',side_effect=capture), \
             mock.patch.object(install,'_install',return_value=self.root/'stage'):
            customiser_setup.install(image,self.base,self.game,lambda _:None)
        self.assertEqual(calls[0][0],extractor)
        self.assertEqual(calls[0][1],'-d')
        self.assertEqual(calls[0][3:],['-x',image.resolve()])

    def test_adjacent_native_extractor(self):
        adjacent = self.game.parent/'skate-xiso'
        adjacent.write_bytes(b'')
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(install.xiso_extractor(self.base, self.game, print), adjacent)

    def test_checkout_prefers_release_then_debug(self):
        with tempfile.TemporaryDirectory() as checkout:
            tools = Path(checkout)/'tools'
            release = Path(checkout)/'target/release/skate-xiso'
            debug = Path(checkout)/'target/debug/skate-xiso'
            release.parent.mkdir(parents=True);release.write_bytes(b'')
            debug.parent.mkdir(parents=True);debug.write_bytes(b'')
            with mock.patch.object(install, 'TOOLS', tools), mock.patch.dict(os.environ, {}, clear=True):
                self.assertEqual(install.xiso_extractor(self.base, self.game, print), release)
                release.unlink()
                self.assertEqual(install.xiso_extractor(self.base, self.game, print), debug)

    def test_windows_retains_pinned_fallback(self):
        fallback = self.root/'extract-xiso.exe'
        with mock.patch.object(install, 'WINDOWS', True), mock.patch.dict(os.environ, {}, clear=True), \
             mock.patch.object(install, 'dependency', return_value=fallback) as dependency:
            self.assertEqual(install.xiso_extractor(self.base, self.game, print), fallback)
            dependency.assert_called_once_with(self.base/'tools', 'extract-xiso', install.XISO_URL, install.XISO_SHA, print)

    def test_linux_error_is_actionable(self):
        with tempfile.TemporaryDirectory() as checkout, \
             mock.patch.object(install, 'TOOLS', Path(checkout)/'tools'), \
             mock.patch.dict(os.environ, {}, clear=True), \
             mock.patch.object(install, 'WINDOWS', False):
            with self.assertRaisesRegex(RuntimeError, r'cargo build --locked -p skate-xiso'):
                install.xiso_extractor(self.base, self.game, print)


if __name__ == '__main__':
    unittest.main()
