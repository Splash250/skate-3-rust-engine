"""Export the original front-end menu movies for the engine's APT player.

Setup runs this inside the HUD stage (prepare_runtime_huds.py). It reads the
owned disc's fedata.big / fetexture.big / miscboot.big, starts from the menu
screens in MENU_ROOTS and follows every cross-movie import (retail load queue
sub_82E7E8C0 requests each import source by name before linking), so the export
holds the menu movies plus every library movie they import.

Output (assets/private/menu-movies/, never committed):
  manifest.json            format, roots, movie key -> file, shared language/fonts, file hashes
  shared.json              language table, font definitions and native font layout (shared by all movies)
  movies/<key>.json        one movie in the HUD player format (characters, actions, shapes)
                           plus imports / exports / character ids for apt_imports.rs
  <cache paths>            the RGBA payloads the shapes and fonts reference

Movie keys are the retail import paths relative to data/fe/ without extension
(apt_imports.rs library_key), e.g. source/screens/main/core_menu.
No game content is embedded in this tool: only screen names.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil

from vendor.skate3_ui.project import extract_project
from vendor.skate3_ui.scene_graph import AssetCache, SceneFlattener
from vendor.skate3_ui.actions import Actions

FORMAT = 'skate3-menu-movies'
VERSION = 3  # 2: placement clip actions (Milestone 3d); 3: unit bitmap ids (Milestone 4)
# Menu screens the engine's retail menus start from (pause menu, Game Settings,
# Freeskate options, the screen manager and its helpers). Library movies are
# found through imports, not listed.
# APT font name -> paired native font file (825D6B68; apt_text.rs RETAIL_FONT_PAIRS).
FONT_PAIRS = {'Futura Shadow': 'futuraheavy'}

MENU_ROOTS = (
    'source/screens/main/FE_root',
    'source/screens/main/core_menu',
    'source/screens/main/dimmer',
    'source/screens/main/freeskate_options',
    'source/screens/options/options',
    'source/screens/main3dhud',
    'source/helper/screenmanager',
    'source/helper/screentransitionmanager',
    'source/controls/tabs',
    'source/controls/scrollbar',
    'source/screens/popup/sk8popup',
    'source/screens/popup/small_popup',
)
MAX_MOVIES = 256  # apt_imports.rs MAX_MOVIES


def library_key(file):
    """Same rule as apt_imports.rs library_key."""
    path = file.replace('\\', '/').lstrip('/')
    if path.lower().startswith('data/fe/'):
        path = path[len('data/fe/'):]
    return path[:-4] if path.endswith('.apt') else path


def _digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _contained(root, relative):
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()) or path == root.resolve():
        raise ValueError(f'Menu movie path escapes its folder: {relative}')
    return path


PLACE_HAS_CLIP_ACTIONS = 0x80
MAX_CLIP_ACTIONS = 64  # apt_display.rs MAX_CLIP_ACTIONS


def clip_actions(apt, pointer):
    """Clip-event table of a placement (doc 31, Milestone 3d; retail sub_82E7B340 passes
    placement+60, sub_82E5B158 walks it): u32 count, u32 events; 12-byte events
    {u32 flags, u32 key_code, u32 actions}. Big-endian, offsets relative to the .apt."""
    def u32(offset):
        if offset < 0 or offset + 4 > len(apt):
            raise ValueError(f'clip action table outside the movie at {offset:#x}')
        return int.from_bytes(apt[offset:offset + 4], 'big')
    count, events = u32(pointer), u32(pointer + 4)
    if count > MAX_CLIP_ACTIONS:
        raise ValueError(f'{count} clip actions at {pointer:#x}')
    out = []
    for i in range(count):
        base = events + i * 12
        out.append({'flags': u32(base), 'key_code': u32(base + 4), 'actions_offset': u32(base + 8)})
    return out


def _with_bitmap(bundle, primitive):
    """Adds the bitmap character a textured GEO unit samples (unit +0x14, the id the flattener
    resolved the texture through), so the engine can name the bitmap by a stable id
    (movie key + bitmap character id) and a mod can replace it (doc 31, Milestone 4)."""
    if primitive.get('texture'):
        unit = bundle['shapes'][primitive['character_id']]['units'][primitive['unit']]
        primitive['bitmap'] = unit['texture_id']
    return primitive


def export_movie(cache, raw, key):
    """One movie in the HUD player JSON format plus its import tables."""
    name = 'data/fe/' + key
    bundle = cache.load_bundle(name)
    apt_path = raw / (name + '.apt')
    const_path = apt_path.with_suffix('.const')
    apt_bytes = apt_path.read_bytes()
    actions = Actions(apt_bytes, const_path.read_bytes())
    blocks, shapes, unresolved_shapes, families = {}, {}, {}, set()
    characters = list(bundle['characters'].values())
    for c in characters:
        for f in c.get('frames', []):
            for control in f['controls']:
                if control['type_name'] in ('do_action', 'do_init_action'):
                    offset = control.get('actions_offset', 0)
                    if offset:
                        blocks[str(offset)] = actions.stream(offset)
                elif control['type_name'] in ('place_object2', 'place_object3') \
                        and control.get('flags', 0) & PLACE_HAS_CLIP_ACTIONS and control.get('actions_offset', 0):
                    control['clip_actions'] = clip_actions(apt_bytes, control['actions_offset'])
                    for event in control['clip_actions']:
                        blocks[str(event['actions_offset'])] = actions.stream(event['actions_offset'])
        if c['type_name'] == 'shape':
            try:
                scene = SceneFlattener(cache, lambda *_: {}).flatten(name, c['id'])
            except Exception as error:  # noqa: BLE001 - drawing is a later milestone
                unresolved_shapes[str(c['id'])] = str(error)
                continue
            if scene['unresolved']:
                unresolved_shapes[str(c['id'])] = scene['unresolved']
            else:
                shapes[str(c['id'])] = [_with_bitmap(bundle, p) for p in scene['primitives']]
        elif c['type_name'] == 'font':
            families.add(c['font']['name'])
    apt = bundle['apt']
    return {
        'format': FORMAT, 'version': VERSION, 'movie': key,
        'source': {'bundle': name,
                   'apt_sha256': hashlib.sha256(apt_path.read_bytes()).hexdigest(),
                   'const_sha256': hashlib.sha256(const_path.read_bytes()).hexdigest()},
        'characters': characters,
        'shapes': shapes, 'unresolved_shapes': unresolved_shapes, 'actions': blocks,
        'imports': [{k: i[k] for k in ('file', 'name', 'character_id')} for i in apt.get('imports', [])],
        'exports': [{k: e[k] for k in ('name', 'character_id')} for e in apt.get('exports', [])],
    }, families


def prepare(game, output, collections, roots=MENU_ROOTS):
    """Builds the runtime set in `output` (a work folder) and returns its manifest."""
    from prepare_hud import font_mapping
    cache_root = output / 'cache'
    extract_project(game, cache_root, prefixes=('data/fe/source/',), update=True)
    cache = AssetCache(cache_root)
    mappings = font_mapping(collections, cache)
    runtime = output / 'runtime'
    if runtime.exists():
        shutil.rmtree(runtime)
    (runtime / 'movies').mkdir(parents=True)
    queue, movies, families, missing = list(roots), {}, set(), []
    while queue:
        key = library_key(queue.pop(0))
        if key in movies or key in missing:
            continue
        if len(movies) >= MAX_MOVIES:
            raise ValueError('Too many menu movies')
        if not (cache_root / 'raw' / ('data/fe/' + key + '.apt')).is_file():
            missing.append(key)  # reported; apt_imports.rs reports the import as MissingMovie
            continue
        movie, used = export_movie(cache, cache_root / 'raw', key)
        families |= used
        target = runtime / 'movies' / (key + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(movie, separators=(',', ':')) + '\n', encoding='utf-8')
        movies[key] = 'movies/' + key + '.json'
        queue.extend(i['file'] for i in movie['imports'])
    # Retail's secondary font (825D6B68: "Futura Shadow" -> native "futuraheavy", looked up in the whole
    # native font table, 82809208) is needed even when no movie in the set places that font itself.
    for family in sorted(families):
        partner = next((v for k, v in FONT_PAIRS.items() if k.lower() == family.lower()), None)
        if partner:
            families |= {name for name, row in mappings.items() if row['file_name'].lower() == partner.lower()}
    fonts = {family: cache.font_asset(family) for family in sorted(families)}
    language = {row['label'].strip(): row['value'] for row in json.loads(
        (cache_root / 'metadata/languages/english_global.json').read_text(encoding='utf-8'))['entries']}
    shared = {'language': language, 'fonts': fonts, 'font_mappings': mappings, 'font_pairs': FONT_PAIRS}
    (runtime / 'shared.json').write_text(json.dumps(shared, separators=(',', ':')) + '\n', encoding='utf-8')
    # RGBA payloads referenced by shapes and fonts, copied from the cache.
    payloads = set()
    for file in movies.values():
        for shape in json.loads((runtime / file).read_text(encoding='utf-8'))['shapes'].values():
            payloads.update(p['texture']['rgba'] for p in shape if p.get('texture'))
    payloads.update(f['texture'] for f in fonts.values() if f)
    for relative in sorted(payloads):
        source = _contained(cache_root, relative)
        target = _contained(runtime, relative)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    files = {p.relative_to(runtime).as_posix(): _digest(p) for p in sorted(runtime.rglob('*')) if p.is_file()}
    manifest = {'format': FORMAT, 'version': VERSION, 'roots': list(roots), 'movies': movies,
                'missing_movies': missing, 'shared': 'shared.json',
                'unresolved_fonts': [f for f, a in fonts.items() if a is None], 'files': files}
    (runtime / 'manifest.json').write_text(json.dumps(manifest, indent=1) + '\n', encoding='utf-8')
    print(f'Prepared {len(movies)} original menu movies ({len(payloads)} payloads): {runtime}', flush=True)
    return manifest


def runtime_files(root):
    """Verified {relative: sha256} of a prepared set (manifest last)."""
    manifest = json.loads((root / 'manifest.json').read_text(encoding='utf-8'))
    if manifest.get('format') != FORMAT or manifest.get('version') != VERSION or not manifest.get('movies'):
        raise ValueError(f'Invalid menu movie set: {root}')
    files = {}
    for relative, sha256 in manifest['files'].items():
        path = _contained(root, relative)
        if _digest(path) != sha256:
            raise ValueError(f'Menu movie file hash mismatch: {path}')
        files[relative] = sha256
    files['manifest.json'] = _digest(root / 'manifest.json')
    return files


def install(assets, runtime):
    """Replaces assets/private/menu-movies with the verified set."""
    files = runtime_files(runtime)
    destination = assets / 'private' / 'menu-movies'
    staging = destination.with_name('menu-movies.new')
    if staging.exists():
        shutil.rmtree(staging)
    for relative in files:
        target = _contained(staging, relative)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(_contained(runtime, relative), target)
    runtime_files(staging)
    if destination.exists():
        shutil.rmtree(destination)
    staging.rename(destination)
    return {f'private/menu-movies/{relative}': sha for relative, sha in files.items()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='work folder (cache + runtime)')
    parser.add_argument('--collections', type=Path, required=True,
                        help='Owned private/stock/skater-collections.json')
    args = parser.parse_args()
    prepare(args.game.resolve(), args.output.resolve(), args.collections.resolve())
