import os
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT=Path(__file__).resolve().parents[1]


class LinuxScriptTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='skate scripts ')
        self.root=Path(self.temp.name)
        shutil.copy2(ROOT/'BUILD.sh',self.root/'BUILD.sh')
        shutil.copy2(ROOT/'PLAY.sh',self.root/'PLAY.sh')
        (self.root/'scripts').mkdir()
        helper=ROOT/'scripts'/'stage-steam-relay.py'
        shutil.copy2(helper,self.root/'scripts'/helper.name)
        (self.root/'bin').mkdir()
        self.log=self.root/'commands.log'
        self._program('cargo','printf \'RUSTFLAGS=%s ARGS=\' "$RUSTFLAGS" >> "$SCRIPT_LOG"; printf \' <%s>\' "$@" >> "$SCRIPT_LOG"; printf \'\\n\' >> "$SCRIPT_LOG"\nif [ "$1" = metadata ]; then cat "$FAKE_METADATA"; fi')
        self._program('ldd','printf \'%s\\n\' "${FAKE_LDD:-glibc}"')
        self._program('rustc','if [ "$1" = -vV ]; then printf \'host: %s\\n\' "${FAKE_RUST_HOST:-x86_64-unknown-linux-gnu}"; else printf \'%s\\n\' "$FAKE_RUST_LIB"; fi')

    def tearDown(self):
        self.temp.cleanup()

    def _program(self,name,body):
        path=self.root/'bin'/name
        path.write_text('#!/bin/sh\nset -eu\n'+body+'\n')
        path.chmod(0o755)
        return path

    def _env(self,**extra):
        return {**os.environ,'PATH':str(self.root/'bin')+os.pathsep+os.environ['PATH'],
                'SCRIPT_LOG':str(self.log),'FAKE_RUST_LIB':str(self.root/'rustlib'),**extra}

    def test_build_profiles_and_never_selects_relay(self):
        subprocess.run([self.root/'BUILD.sh'],check=True,env=self._env(RUSTFLAGS=''))
        subprocess.run([self.root/'BUILD.sh','--release'],check=True,env=self._env(RUSTFLAGS=''))
        lines=self.log.read_text().splitlines()
        self.assertIn('<-p> <skate-game> <--bin> <skate3rust>',lines[0])
        self.assertIn('<-p> <skate-xiso>',lines[1])
        self.assertTrue(all('<--release>' in line for line in lines[2:]))
        self.assertNotIn('skate-steam-relay',self.log.read_text())

    def test_musl_appends_existing_rustflags(self):
        subprocess.run([self.root/'BUILD.sh'],check=True,
                       env=self._env(RUSTFLAGS='-C debuginfo=1',FAKE_LDD='musl libc'))
        self.assertIn('RUSTFLAGS=-C debuginfo=1 -C target-feature=-crt-static',self.log.read_text())

    def test_browser_companion_uses_matching_build_profile(self):
        for arguments in [('--browser',), ('--release', '--browser')]:
            with self.subTest(arguments=arguments):
                self.log.unlink(missing_ok=True)
                subprocess.run([self.root/'BUILD.sh', *arguments],check=True,env=self._env(RUSTFLAGS=''))
                builds=self.log.read_text().splitlines()
                self.assertEqual(len(builds),3)
                self.assertIn('<-p> <skate-browser> <--features> <host>',builds[-1])
                self.assertTrue(all(('<--release>' in line)==('--release' in arguments) for line in builds))

    def test_steam_option_builds_and_stages_each_profile_with_either_flag_order(self):
        sdk=self.root/'cargo registry'/'steamworks-sys-0.13.0'
        library=sdk/'lib'/'steam'/'redistributable_bin'/'linux64'/'libsteam_api.so'
        library.parent.mkdir(parents=True)
        library.write_bytes(b'native Steam library fixture')
        (sdk/'Cargo.toml').write_text('[package]\nname="steamworks-sys"\nversion="0.13.0"\n')
        metadata=self.root/'metadata.json'
        metadata.write_text(json.dumps({'packages':[{'name':'steamworks-sys','version':'0.13.0',
                                                     'manifest_path':str(sdk/'Cargo.toml')}],
                                        'target_directory':str(self.root/'target')}))
        for args,profile in [(('--steam',),'debug'),
                             (('--release','--steam'),'release'),
                             (('--steam','--release'),'release')]:
            with self.subTest(args=args):
                relay=self.root/'target'/'x86_64-unknown-linux-gnu'/profile/'skate-steam-relay'
                relay.parent.mkdir(parents=True,exist_ok=True)
                relay.write_bytes(b'#!/bin/sh\nexit 0\n');relay.chmod(0o755)
                self.log.unlink(missing_ok=True)
                result=subprocess.run([self.root/'BUILD.sh',*args],text=True,capture_output=True,
                                      env=self._env(RUSTFLAGS='',FAKE_METADATA=str(metadata)))
                self.assertEqual(result.returncode,0,result.stderr)
                lines=self.log.read_text().splitlines()
                builds=[line for line in lines if '<build>' in line]
                self.assertEqual(len(builds),3)
                self.assertIn('<-p> <skate-steam-relay>',builds[2])
                self.assertIn('<--target> <x86_64-unknown-linux-gnu>',builds[2])
                self.assertTrue(all(('<--release>' in line)==(profile=='release') for line in builds))
                staged=self.root/'target'/profile/'steam-relay'
                self.assertEqual((staged/'skate-steam-relay').read_bytes(),relay.read_bytes())
                self.assertTrue(os.access(staged/'skate-steam-relay',os.X_OK))
                self.assertEqual((staged/'libsteam_api.so').read_bytes(),library.read_bytes())

    def test_invalid_build_arguments_fail_before_building(self):
        for args in [('--unknown',),('--release','--release'),('--steam','--steam'),('--browser','--browser'),
                     ('--release','unexpected')]:
            with self.subTest(args=args):
                result=subprocess.run([self.root/'BUILD.sh',*args],text=True,capture_output=True,env=self._env())
                self.assertEqual(result.returncode,2)
                self.assertIn('Usage:',result.stderr)
                self.assertFalse(self.log.exists())

    def test_unsupported_steam_hosts_fail_before_building(self):
        for host in ['x86_64-unknown-linux-musl','aarch64-unknown-linux-gnu','x86_64-pc-windows-msvc']:
            with self.subTest(host=host):
                result=subprocess.run([self.root/'BUILD.sh','--steam'],text=True,capture_output=True,
                                      env=self._env(FAKE_RUST_HOST=host))
                self.assertNotEqual(result.returncode,0)
                self.assertIn('x86_64-unknown-linux-gnu',result.stderr)
                self.assertFalse(self.log.exists())

    def test_steam_rejects_cargo_build_target_before_building(self):
        result=subprocess.run([self.root/'BUILD.sh','--steam'],text=True,capture_output=True,
                              env=self._env(RUSTFLAGS='',CARGO_BUILD_TARGET='x86_64-unknown-linux-gnu'))
        self.assertFalse(self.log.exists())
        self.assertNotEqual(result.returncode,0)
        self.assertIn('CARGO_BUILD_TARGET',result.stderr)

    def _game(self,profile='debug'):
        game=self.root/'target'/profile/'skate3rust';game.parent.mkdir(parents=True)
        game.write_text('#!/bin/sh\nprintf \'MODS=%s\\nLIBS=%s\\n\' "$SKATE3_MODS" "$LD_LIBRARY_PATH" > "$SCRIPT_LOG"\nprintf \'<%s>\' "$@" >> "$SCRIPT_LOG"\n')
        game.chmod(0o755)

    def test_play_uses_absolute_assets_default_mods_and_forwards_arguments(self):
        self._game();(self.root/'assets').mkdir()
        subprocess.run([self.root/'PLAY.sh','--map','a park.skate'],check=True,env=self._env())
        text=self.log.read_text()
        self.assertIn('MODS='+str(self.root/'mods'),text)
        self.assertEqual(text.splitlines()[1],f'LIBS={self.root}/target/debug/deps:{self.root}/rustlib')
        self.assertEqual(text.splitlines()[2],f'<--assets><{self.root}/assets><--map><a park.skate>')

    def test_play_release_preserves_mod_override_and_adjacent_data_fallback(self):
        self._game('release')
        subprocess.run([self.root/'PLAY.sh','--check-assets'],check=True,
                       env=self._env(SKATE_RELEASE='1',SKATE3_MODS='/custom mods'))
        self.assertEqual(self.log.read_text().splitlines(),
                         ['MODS=/custom mods',f'LIBS={self.root}/target/release/deps:{self.root}/rustlib','<--check-assets>'])

    def test_missing_game_reports_matching_build(self):
        result=subprocess.run([self.root/'PLAY.sh'],text=True,capture_output=True,env=self._env(SKATE_RELEASE='1'))
        self.assertEqual(result.returncode,2)
        self.assertIn('./BUILD.sh --release',result.stderr)


if __name__=='__main__':unittest.main()
