# 31: Retail menus: menu data, hook registry, mod API (milestone 1) and settings value model (milestone 2)

Branch: `ui/retail-menus` (from `main` b3c96793). Own upstream PR when ready (not part of the audio or living-world
PRs). This milestone adds data, hooks and the mod API only; nothing is drawn yet and the current overlay menus
(`graphics_menu.rs`, `modding/menu.rs`) are unchanged.

## Problem

The engine's pause and settings menus are hand-built (`graphics_menu.rs` sections MAPS / SKATER / GRAPHICS /
MULTIPLAYER / EXTRAS). They do not look or behave like Skate 3's, and they have no structure that features or mods
can plug into: every new feature edits menu code. A real accident showed the cost: the FPS limit was changed by an
unintended press in the overlay. The goal of the menus work is retail menus (layout, navigation, input handling,
sounds) built on the retail data, moddable from the start. Milestone 1 is the foundation: the retail menu
structure as data, a registry with hooks for every entry, and the mod API over it.

## What retail does

Reference: the TU3 build through skate3recomp (by @mchughalex, built on the rexglue SDK) read statically (no game
run), the front-end movies in `data/big/fedata.big` read with upstream's APT tools (`tools/vendor/skate3_ui`, from
chazmm's APT/HUD port 1c768a3e). Addresses are TU3 and only used to verify the extractor; no game code or data is
copied. Full notes: `.claude/notes/fe-menus-re.md` (local).

**The movies ask the game for data.** The pause menu (`screens/main/core_menu.apt`, the "crossbar") and the
settings screen (`screens/options/options.apt`) are AS2 movies; they read the menu contents through host objects
(`CoreMenu`: GetNumMenus / GetOptionName / IsItemEnabled ...) and natives (`GameSettings_GetNumOptions`,
`OnMenuSelect`, `OnMenuGetItemEnabled`, ...; registrar `825A3070`). The contents are C++ tables:

- **Game Settings rows** `0x83026EC8`: 36 x (string id, widget type `option` / `slider` / `selector`); rows 0 and 23
  are blank. **Screens** `0x83026FE8`, right after: 13 x 16 row indices, 0-terminated (1 = main: Difficulty, Audio,
  Video, Controls, Online, Skate Feed; 3 = Audio: SFX / Dialog / Music volume, SFX pack; 4..8 = Video variants; 9
  Controls; 10 Online; 11 Skate Feed; 12 Audio without the SFX pack, unused).
- **Crossbar categories** `0x8302751C`: 12-byte records (name, label id, icon): Multiplayer, Options, SinglePlayer,
  Create, Learn. **Items** `0x83027558`: 20-byte records (name, label id, icon label, sub-option kind or -1, helper
  text id), 49 items from ReplayEditor (id 0) to ExitPartyPlay (id 48). Item ids are positions in this table.
- **Apt key table** `0x83027BF0`: 56 x 20-byte records (input action, name, Apt key code, released flag, kind):
  AptStart 0x12D, AptNext 0x12E, AptPrevious 0x12F, AptUp / AptDown / AptLeft / AptRight 0xE / 0xF / 1 / 2 and the
  `*Released` twins.
- **Per-mode menus** are static data reached through the crossbar mode classes. Each class's vtable (e.g. Career
  `0x82303D54`) holds, in order: destructor, a tab accessor `GetMenu(i)` (`lis; rlwinm r4,2; addi; lwzx; blr`, e.g.
  `825E8368` -> `0x822059E4`), a row accessor `GetItem(tab, slot)` (`825E8380`, rows `0x82205A68`, stride 8 via
  `rlwinm r4,3`; the park variants use `r4*8 + r4` = 9 slots), the item-kind getter, the tab count (`li r3,5`), the
  row counter (skips Project10 until unlocked, `8261F950`), IsItemEnabled, a constant, the mode title getter
  (`8261F940` -> `ID_CROSSBAR_SINGLE_PLAYER_MODE_TITLE`) and the select hook. Nine classes have static tables:
  Career, Career in a park, Free Play, Free Play in a park, a restricted free-skate menu (Main = ChallengeMap only),
  the Skate challenge, two lobby modes and the Party Play menu. Career Main = ChallengeMap, MyCareerTeam,
  SkateProfileOffline, SkateWith, SkateFeed, SoloFreeskate, PartyPlay (matches the retail screenshot row for row).
- **Rules** (from the code): the settings row count stops at the SFX pack row with <= 1 SFX pack
  (`GameSettings_GetNumOptions` 825A6F70) and IsItemEnabled (`8260D648`) greys it, greys Difficulty in online modes
  (GameMode 4 / 5) and Skate Feed settings when unavailable (`8245A2D0`). In a park (`8261FB18`) SkateReel and
  SkateWith are greyed, SavePark needs a park-state flag, RatePark / FlagSkatePark only in a community park. The park
  select hook (`8261F9D8`) asks `ID_SKATEPARK_DIRTY_TITLE` / `_DESC` (Yes / No) before ChallengeMap /
  OnlineChallengeMap leave a dirty park. Settings select (`8260D290`) links rows to screens, Video by front-end
  state, mode and session (screen 8 / 7 / 6 / 5 / 4). Retail's own greyed look is the row state `disabled` in
  core_menu's row sprite.

## Change

### 1. Extraction at setup (`skate_data::menu_tables`)
`skate3rust --extract-menu-tables <default.xex> <menu-tables.json>` (like `--extract-ocean-pca`) unpacks the user's
own executable with `XexImage` and writes `assets/private/menu-tables.json`; setup runs it in the environment stage
(`tools/asset_pipeline/menu_tables.py`, `asset_exports.py`; failures become an optional-content note). Every table is
found by content or code pattern, never by address, so any build works:

- settings rows: the pointer to the string `ID_GAMESETTINGS_SFXVOLUME` whose neighbour points to `slider`; the run of
  (string, widget) pairs around it; the screens follow while every block holds valid row indices;
- crossbar items: the pointer to `ReplayEditor` in a record of the item shape (three strings, a small kind, a
  string); categories are the 12-byte `ID_CROSSBAR_*` records right before;
- Apt keys: the pointer to `AptStart` in a record of the key shape;
- per-mode menus: every pair of adjacent vtable slots where a small evaluator (lis / addi / ori / rlwinm / add /
  mulli / lwzx / lwz / blr) shows the first function reads `table[r4]` and the second `table[r4 * n + r5]`; the
  following slots give the tab count (`li r3,n; blr`) and the title (a two-instruction getter returning a string).
  Item and category ids become names. Mode keys: `Career` / `CareerPark` (single-player title, 8 / 9 slots),
  `FreePlay` / `FreePlayPark` (offline free-skate title; FreePlay has ResumeCareer), `FreeSkateRestricted` (same
  title, no ResumeCareer), others by title (`SKATE_CHALLENGE`, `TEAM_VS_TEAM_LOBBY`, `RANKED_LOBBY`, `PartyPlay`).
  Retail has no class names for these menus, so the keys are ours, derived from the retail title and slot count.

The JSON (`schema` 1) also keeps the source image hash and the table addresses for diagnostics.

**The disc build differs from TU3**, which is why nothing is fixed by address or index: the user's disc
`default.xex` has 47 crossbar items (TU3 added PartyPlay and ExitPartyPlay), 35 settings rows (no
`ID_GAMESETTINGS_SKATEFEED_STATUS_MESSAGE`, so the Skate Feed settings row is index 34, not 35), its categories sit
right after the items instead of before them, and every table is elsewhere (settings rows `0x82FCB168`, items
`0x82FCB7A8`, categories `0x82FCBB54`, keys `0x82FCBE50`, Career vtable `0x822FE764`). Its per-mode menus are the
TU3 ones without the Party Play entries and without the Party Play menu. The registry keys settings rows by string
id, so screens stay correct in both builds.

### 2. Registry (`skate_core::menus`, resource `retail_menus::RetailMenus`)
Engine-independent registry over `MenuData` (categories, items, settings rows, screens, modes), built from the JSON
(`MenuTables::to_menu_data`; settings rows keyed by string id, blank rows `GAMESETTINGS_ROW_<n>`). Keys are the retail
internal names (`GameSettings`, `SkateFeed`, `SinglePlayer`, `ID_GAMESETTINGS_SFXVOLUME`).

- Hooks per entry: `visible` and `enabled` (data `Rule`s over published facts and the mode, or native predicates),
  `on_select` / `on_highlight` handlers, select interceptors (confirmation popups), value get/set bindings. Every
  hook list is a stack by owner (`Retail`, `Engine(name)`, `Mod(id)`); the last registered wins and removing an
  owner restores the one below.
- **Greyed until handled:** an entry is enabled only when a handler (or value binding) exists and its enabled rule
  holds; a settings link is enabled when its target screen has an enabled row. So every retail item exists from the
  start, greyed, and a feature enables it by registering a handler, with no menu code changes. Online / Xbox LIVE
  items stay greyed placeholders until the custom online feature registers.
- Retail rules registered as `Owner::Retail` data: SFX pack hidden and greyed at <= 1 pack (fact `sfx_packs`),
  Project10 hidden until `project10_unlocked`, Difficulty greyed when `game_mode` is 4 / 5, Skate Feed settings by
  `skate_feed_available`, park rules (SkateReel / SkateWith off in `CareerPark` / `FreePlayPark`, SavePark by
  `park_saveable`, RatePark / FlagSkatePark by `park_community`), the dirty-park confirmation on ChallengeMap /
  OnlineChallengeMap, and the settings links incl. the Video screen choice (`retail_link`).
- Actions are values with stable ids (`MenuAction::Select / Confirm / Highlight / SetValue`, entry kind + id, plus the
  mode): `dispatch` returns what the host does next (`Confirm(popup)`, `Handled`, `Forward` to a mod,
  `OpenScreen`, `Ignored`, `Declined`), so a menu UI or a remote peer can replay the same actions (multiplayer-ready,
  no networking).
- Structural edits (`Edit::Category / Item / Setting / Hide`) are owner-tagged layers over the retail base: add,
  hide (per mode), reorder, relabel, change icon. Removing an owner rebuilds the structure from the base plus the
  remaining layers, so disabling a mod reverts exactly its changes.

### 3. Mod API (`sdk.menus`, capability `retail_menus` = 1)
`sdk.menus.category / item / setting(id, options)` (add or change: label, retail `icon` label or mod `image`,
help, tab `category`, `modes`, `position`, `widget`, `screen`), `hide(kind, id, modes)`, `handle(kind, id, {select,
highlight, value, enabled, visible, confirm})`, `unhandle`, `set_value`, `value`, `info`. New entries must be named
`<mod id>.<name>`; 64 entries per mod. Handled entries send `on_event {name = "menu_select" | "menu_highlight" |
"menu_value", entry, id, mode, value}`. Rules from Lua: `true` / `false` or `{fact = "sfx_packs", gt = 1}`,
`{modes = {...}}`, `{all = {...}}`, `{any = {...}}`, `{["not"] = ...}`. `sdk.snapshot.menus` holds the mode, whether
the retail tables loaded and every bound value. Everything a mod did is removed when it stops, fails or reloads.
Mod images are a designed field only; drawing them comes with the menus (milestone 5).

## Milestone 2: settings value model (2026-10-08)

### Problem
Milestone 1 had value get/set hooks but no rule for how a row's value changes, and no engine setting was bound, so
every value row was greyed.

### What retail does
All from the TU3 settings input handler `sub_8260C258` (events 14 = Left, 15 = Right), GetIntegerValue
`sub_8260D7E8` and GetStringValue `sub_8260D968`; values live in the settings object `[[0x83067060] + 28]`.
- [code] Volume sliders (rows SFX / Dialog / Music, offsets +128 / +132 / +136, f32):
  `v = clamp(v + dir * 0.1, 0.0, 1.0)` with `fmadds` (step 0.1 at `0x820641A8`, min 0.0 at `0x82165A10`, max 1.0 at
  `0x8231A844`, Left -1.0 at `0x8216DEE0`, Right uses the 1.0 at `0x8231A844`), then `v < 0.05` (`0x82165A00`) -> 0.0.
  The slider shows `floor(v * 10.0 + 0.5)` bars (10.0 at `0x821963E4`, 0.5 at `0x8209975C`).
- [code] Selectors toggle `x = (x == 0)` on either direction; Left and Right only differ in their UI sound. Labels
  (sub_8260D968, strings at `0x82203990 - 668 ..`): Camera Angle +188 0 = `ID_GAMESETTINGS_CAMERA_LOW`, 1 = `_HIGH`
  (other values `#ERROR!!`); offboard Y / X axis +192 / +193 `_AXIS_NORMAL` / `_AXIS_INVERTED`; Subtitles +157,
  Minimap +158, HUD +164 (applied by `sub_824AD2F0` / `sub_825DA2A0`), Camera manual +165, Vibration +163
  (`sub_8260DF40`, plays a test rumble when switched on), Transparency +195, Skate Feed +196..+200:
  `ID_COMMON_OFF` / `_ON`; Units +168 `_UNITS_METRIC` / `_IMPERIAL` (also mirrored to `[0x830CFE34] + 4`).
- [code] Play Mode (+180, `sub_8260DE60`): `v + dir`, above 2 -> 0, below 0 -> 2; applied at once by
  `sub_827D8450` unless `sub_82743FC0` (online session). Labels `sub_825E7648`: 0..4 = `ID_GAMESETTINGS_DIFFICULTY_`
  `EASY` / `NORMAL` / `HARDCORE` / `MOTORIZED` / `TEST`, nothing above 4.
- [code] HOM mode (+184): `v + dir`, 2 or more -> 0, below 0 -> 1 (two values, `ID_COMMON_OFF` / `_ON`).
- [code] Not value rules: SFX pack (`sub_82486870` picks the next pack id), Auto sign-in (`sub_8260E068`, a popup);
  VSync and Sixaxis have no case (Left / Right do nothing). Every change refreshes all rows (`vtbl+20(item, 50)`).
- [code] Volume routing: the MixMap Master controller's step `sub_824D5160` (runs every audio pass, so a change is
  heard on the next pass) reads the three floats from the settings object and writes its own inputs through
  `[controller + 12]->vtbl+8(input, value)` with `value = trunc(v * 32767.0)` (`fmuls` by 32767.0 at `0x821747FC`,
  `fctiwz`), clamped to 0..32767: Master.in1 = Music (+136), in2 = SFX (+128), in3 = Dialog (+132). The MixMap data
  then scales the buses (mixmap-spec section 6.5: in1 -> Music, in2 -> nearly every SFX sum, in3 -> Announcer, NIS
  and CameraMan speech). Ambience and emitters hang on Master.in4, which is not an option volume (same function, from
  a game object's float, 32767 in free skate). Music +136 is also a gate elsewhere: `sub_8249A7E8` and
  `sub_824CF350` only start / advance a music track when it is above 0.0, `sub_8249C078` tests it for 0.0;
  `sub_825AD1F8` writes +136 from a system call result (`sub_82E5F430`).

### Change
- `skate_core::menu_values` (new): `ValueRule` {Slider, Toggle, Cycle} with the arithmetic above (f32,
  `mul_add` like `fmadds`, deterministic), `Display` (bars or a label id), `Direction` (serialisable by name), and
  `retail_rule(id)`: the retail rules keyed by string id (disc and TU3 tables agree), so ranges and steps are data.
- Registry: retail rules installed as `Owner::Retail`; `set_rule` stacks an owner's rule over it (last wins);
  `MenuAction::Step { entry, direction }` turns Left / Right into a `SetValue` through the rule in force;
  `rule`, `display`, `owner_value`, `set_owner_value`. Removing an owner drops its rules too.
- Engine bindings (`retail_menus.rs`, `Owner::Engine`), which enable these rows: SFX / Dialog / Music volume <->
  `game_audio::RetailVolumes` (0..1 floats, default 1.0, saved in `settings/audio.json` as `retail_volumes`), which
  the native pass (`native::mixmap_frame`) writes to Master.in2 / in3 / in1 exactly as `sub_824D5160` (it wrote
  32767 to all four before); our own Master / Ambience / Effects percent settings are separate engine settings and
  no longer tied to a retail row. Camera Angle <-> `CameraAngleSettings.selected` (0 Low / 1 High, saved),
  Play Mode <-> difficulty (physics mode, `Config`, `settings/gameplay.json`). A sync system applies a row change to
  the engine and saves it, and takes changes made elsewhere (current menu, settings files) into the row without an
  event. Motorized and Custom stay settings-file values (shown as their index label; the retail rule steps back into
  Easy..Hardcore). Engine-only settings stay in the settings files (decision above).
- Mods: `sdk.menus.handle('setting', id, {value = ..., rule = {kind = 'slider' | 'toggle' | 'cycle', ...}})` binds a
  value and / or overrides the rule over the engine binding; `set_value` and Left / Right reach the mod layer
  (`menu_value` event); disable / reload / failure removes both and the engine value shows again.
- Mod `set_value` never writes the user's real setting (follow-up 2026-10-08). On a row whose value in force is not
  the mod's own (an engine binding, another mod's value) the value goes into the mod's own value layer on top of the
  owner stack, exactly as `handle('setting', id, {value = ...})` does (`MenuRegistry::value_owner` tells which layer
  is in force). Last writer wins: a mod writing again moves its layer back on top. While a mod layer is on top of an
  engine-bound row, the sync system applies its value at runtime without saving: SFX / Dialog / Music through
  `AudioSettings` runtime volume overrides on the Master inputs (`settings/audio.json` keeps the user's volumes),
  Camera Angle as the camera's forced angle (owner `menus`; the player's `selected` angle is untouched), Play Mode
  as the physics mode and `Config` difficulty without writing `gameplay.json`. When the layer goes (disable, reload,
  failure) the next layer down is applied again, ending with the user's own setting. A mod's `handle` value
  override on an engine row is applied the same way (before, it was only shown). On a row whose top layer is the
  mod's own, `set_value` acts like a menu change (`menu_value` event to the mod), as before.

### Verification
- `cargo test -p skate-core menu_values`: slider steps, clamp, snap below 0.05, exact zero from 0.1, bars rounding;
  toggles on both directions and any non-zero; Play Mode and HOM wrap, labels incl. out-of-range; undecoded rows have
  no rule.
- `cargo test -p skate-core menus`: Step through the retail rule on an engine binding (0.75 -> 0.85, Hardcore ->
  Easy), engine-side change without an event, mod rule + value override and exact revert to the retail rule and the
  engine value.
- `cargo test -p skate-mods menus`: rule specs parse and validate.
- `cargo test -p skate-game mod_set_value_layers_over_engine_rows_and_reverts`: a mod's `set_value` on the SFX volume
  and Camera Angle rows is shown and applied (Master.in2, forced camera angle) while the user's volume and camera
  choice stay; a second mod on top wins, the first writing again wins back; removing each mod reverts only its own
  layer; after both, the engine value is back exactly, `audio.json` is byte-identical and `camera.json` was never
  written.
- `cargo test -p skate-game retail_volumes volume_rows`: default 1.0 -> 32767 on in1..3; 0.9 / 0.5 / 0.0 -> 29490 /
  16383 / 0 (truncation), clamp and NaN, clamping on load; each row stepped Left reaches only its own Master input
  (SFX in2, Dialog in3, Music in1), is saved, and an engine-side change shows in the row.

### Open
- Subtitles, Minimap, HUD, offboard axes, Camera manual, Vibration, Units, Transparency, HOM
  mode, Skate Feed rows: rules ported, no engine setting yet, so they stay greyed until a feature binds them.
- Music volume as a gate (no track started at 0.0, `sub_8249A7E8` / `sub_824CF350`): needs the music player.
- Retail's default volumes (settings constructor, inside the profile save) not read; 1.0 matches the recomp's
  free-skate capture (Master.in1..3 = 32767).
- SFX pack (`sub_82486870`) and Auto sign-in (`sub_8260E068`) rules not decoded.
- Retail defaults of the settings object (its constructor) not read; our bound settings keep their own defaults.
- The current overlay (`graphics_menu.rs`) keeps its own difficulty copy; it is replaced in milestone 5.
- While a mod holds Play Mode, a difficulty change saved from the current overlay saves that overlay's choice; the
  mod's value itself is never saved. A mod already holding the camera through `sdk.camera` keeps it (the menus
  hold is refused with a warning).

## Milestone 3a: APT VM opcodes for the menu movies (2026-10-08)

### Problem
The 16 retail front-end movies (core_menu, options, highlight_selector / option / slider, screenmanager,
screentransitionmanager, menu_picker, tabs, button_item2, menu_part_text_hilite, panel, scrollbar, dimmer, FE_root,
main3dhud) use 6 action opcodes our APT VM (`crates/skate-game/src/apt_vm.rs`, HUD-only until now) rejected with
"Unsupported APT opcode": 0x26, 0x30, 0x42, 0x43, 0x4A, 0xA5.

### What retail does
Retail dispatches actions through a table of 256 handler pointers at `0x82FC9BF0` (TU3 image; entry 0x8E =
DefineFunction2 `82E73010`, 0x9B = DefineFunction `82E72EC0`, which confirms the base). Handlers:
- **0x26 trace** `82E6D958`: pops one value, converts it to a string and prints `AptTrace: %s` (`0x82063488`) to the
  debug output.
- **0x30 random** `82E6DDD0`: n = ToInteger(top); result = rand() % n (unsigned `divwu`; `twllei` traps on n = 0);
  pops and pushes the integer. rand is `82E82528`, a Mersenne-Twister-style generator (tempering, refill `82E823A8`).
- **0x42 initArray** `82E6E9C0`: n = ToInteger(pop); creates an array; element i = the i-th value from the top;
  length n; pops n, pushes the array (n <= 0: empty).
- **0x43 initObject** `82E6EB00`: n = ToInteger(pop); creates an object; for i = 0..n the pair is value = top, name =
  the value below it; pairs are set from the top pair down (so for a repeated name the pair deepest in the stack,
  i.e. written first in the source, wins); pops 2n, pushes the object.
- **0x4A toNumber** `82E702F0`: numbers stay; values the NaN test `82E67400` rejects become NaN; otherwise a string
  without '.' (search for `"."` at `0x820634A4`) becomes an integer via ToInteger, one with '.' a float.
- **0xA5** `82E73F50`: pushes its string operand, then tail-calls the getMember handler `82E705C0` (0x4E): pop name,
  pop object, push object[name] (EA's string-operand GetMember, like 0xA4 / 0xAF for variables / constants).
- **ToInteger** `82E5F2A8` (used by random, initArray, initObject, toNumber): int as is; float clamped to i32 and
  truncated; bool 0 / 1; string: "0x..." (length > 2) via strtol base 16 `82F53468`, else atoi `82F4DEC0`; values
  without the valid flag give 0; other types compare against the global value `[0x8307F3A8]` (0 if equal, else 1).
These match standard SWF / AVM1 Trace, RandomNumber, InitArray, InitObject, ToNumber and GetMember; retail's
handlers decide the details above (integer results, the '.' rule, atoi-style string parsing).

### Change
- `apt_vm.rs`: the six opcodes; `pub fn to_integer` (82E5F2A8) and `pub fn to_number` (82E702F0); `Host::trace`
  (default no-op, so the HUD host is unchanged) receives trace output; `Vm::seed_random` seeds the script random
  generator.
- Safety on untrusted (mod) movies: initializer count capped at 256 and checked against the stack before anything is
  popped; random with n = 0 gives 0 where retail traps; 0xA5 without a string operand is an error, not a panic;
  all errors go through the existing `Result` path and execution budget.
- Determinism: random uses a seeded xorshift64* owned by the VM (fixed default seed), so replays and a future
  multiplayer session get the same values from the same seed. Only the `rand % n` contract is retail's; retail's
  own generator state is runtime data and not reproduced.

### Verification
- `cargo test -p skate-game --bins -- apt hud`: 19 passed, 4 ignored (data-gated), including per-opcode unit tests
  (`apt_vm::tests`: trace pop + host output, random range / determinism / n = 0 / underflow, toNumber and ToInteger
  rules, initArray order / underflow / limit / negative count, initObject pairs and duplicates, 0xA5 member read)
  and the existing HUD tests unchanged.
- `apt_vm::tests::retail_menu_movies_use_no_unknown_opcode` (skipped when no local data): runs every action stream
  of the 16 decoded movies (183 streams) with a stub host and executes every opcode they contain in isolation; no
  "Unsupported APT opcode". Data: `python .claude/skills/recomp-research/tools/apt_actions_json.py
  <fedata.big> .local/research/fe-menus/actions <movie paths>` (local only, never committed); the test reads
  `SKATE3_FE_ACTIONS` or `.local/research/fe-menus/actions`.

### Open
- The exact string grammar of the retail NaN test `82E67400` (leading spaces, signs, exponents) is only partly read;
  ours accepts what Rust's float parser accepts plus "0x" hex.
- Frame labels: the decoded movies list 151 labels; reaching them needs the display side (imports, screen manager),
  milestone 3 later parts.

## Milestone 3b: cross-movie imports (2026-10-08)

### Problem
The menu movies are built from shared library movies: core_menu imports `Button Item` (button_item2),
`HighlightText` (menu_part_text_hilite), the picker (menu_picker) and `Image_Renderer_Component` (image_renderer);
options imports panel and highlight_option / selector / slider; the libraries import further libraries
(menu_picker -> menu_part_text_hilite -> menu_part_glow, panel -> button_menu -> button_item2). An imported character
id has no local definition, so our player (`apt_movie.rs`, HUD-only, whose movie is self-contained) fails with
"APT unknown character" as soon as a menu movie places one.

### What retail does
[data] The root character of a movie (type 9) has an import table (count / offset at body +32 / +36, 16-byte records:
source path, symbol name, local character id, runtime slot) and an export table (+40 / +44, 8-byte records: name,
character id). Source paths are relative to `data/fe/` (`source/controls/button_item2`).

[code, TU3]
- **When the source loads:** the APT load queue `sub_82E7E9F8` moves an entry from requested to loading by calling
  the host load callback stored in AptInitParms +36 (parms block `0x830CE9C8`, filled by `sub_825D3840`): that is
  `sub_82CA4138`, which prefixes `data\fe\` (`0x820AED74`) and opens the movie with `sub_82CA3428` (`/` turned into
  `\`, `big:%s`, `.apt` and `.const`). When a movie has loaded, `sub_82E7E8C0` walks its import list and asks
  `sub_82E7E6E0` for each source movie by name; the movie is linked only when every source is there, otherwise it
  stays waiting. Sources are therefore loaded on demand, before the importer is linked, and are themselves already
  linked (chained imports resolved) at that point.
- **Linking** `sub_82E76070` (argument: the movie body): for each import record in table order, scan the source
  movie's export list from the start and take the first export whose name equals the record's name byte for byte
  (case-sensitive compare loop at `82E760CC`); write the source movie's character-table entry for that export's id
  into this movie's character table at the record's character id, set the character's owner reference to the source
  movie (`sub_82CA44A0`, record +12) and count a use (16-bit, saturating). No export with that name: the slot is set to
  null. The second loop of the same function only turns local sprite / button child indices into pointers.
- **Use:** placements look the character up in the movie's character table (`sub_82E810B8`), so an imported id
  behaves like a local one from then on.

### Change
- New `crates/skate-game/src/apt_imports.rs`, engine-independent: `Import`, `Export`, `MovieDecl` (from the decoded
  JSON or the pipeline's movie JSON), `library_key` (stable key: `/` separators, no `data/fe/`, no `.apt`, like
  retail's prefix and extension handling), `MovieSource` (where movies come from: the user's assets, test data),
  `resolve(root, movie, source)`: loads each source movie once, on demand, and binds every import of the root and
  of every library it reaches with retail's rule (first export of exactly that name; a chained import is followed to
  the defining movie, which is what retail's pre-linked source table gives). Output: sorted `Binding`s
  (movie, character id -> defining movie, character id) and `Unresolved` entries with a `Failure`; serialisable and
  deterministic (BTreeMaps, sorted output).
- `Movie::link_imports(key, resolution, libraries)` in `apt_movie.rs`: gives each bound id a copy of the defining
  library's character plus everything it places, its action streams and its fonts, under free ids / action keys
  from a deterministic allocator; unresolved ids stay absent (retail's null slot). The HUD path does not call it,
  so the HUD is unchanged.
- **Moddable:** `OverrideTable` holds library movies supplied by mods, keyed by library key, per owner (newest owner
  wins); `table.over(&mut base)` is a source that consults the overrides before the user's assets, so a mod can
  replace a retail library (`source/controls/button_item2`) or add a new one that its own movies import;
  `clear_owner(id)` reverts it when the mod is disabled.
- **Safety on mod movies:** missing source movie, missing export, export of an undefined id, import cycles, chains
  longer than 16 and more than 256 movies are reported as failures, never panics or loops; the copy step has its
  own depth (64) and size (65 536 characters) limits and returns errors.
- **NOT RETAIL (layout):** retail shares the source character by pointer and keeps the source movie for its
  constants; ours copies, which behaves the same because character data is immutable and our action streams are
  already decoded (no constant-pool lookups at run time).
- **NOT RETAIL YET (not decoded):** what retail does when a source movie file does not exist (the queue entry never
  reaches the loaded state; whether it waits forever or gives up was not traced) and with import cycles (each movie
  would wait for the other). Ours reports both and links nothing for them; no retail movie has either. The limits
  above are ours (safety).
- Tool: `.claude/skills/recomp-research/tools/apt_actions_json.py` now also writes `imports`, `exports` and the
  local character ids; the 4 libraries reached only through imports (image_renderer, menu_part_arrow,
  menu_part_glow, button_menu) were decoded as well (20 movies, local only, never committed).

### Verification
- `cargo test -p skate-game --bins -- apt hud`: unit tests `apt_imports::tests` (chain resolve, `.apt` / `data/fe/`
  key forms, missing movie / export / undefined export, cycle and chain limit, determinism, mod override and revert)
  and `apt_movie::import_tests` (library subtree copied, child ids and action streams remapped, the imported clip
  instantiates, errors instead of panics); the existing HUD and VM tests unchanged.
- `apt_imports::tests::retail_menu_movies_resolve_every_import` (data-gated, `SKATE3_FE_ACTIONS` or
  `.local/research/fe-menus/actions`): loads the 20 decoded movies and resolves every import of every movie:
  "20 movies, 19 of 19 imports resolved", none unresolved. Run 2026-10-08: 25 passed, 4 ignored.

### Open
- Lua entry point: the `OverrideTable` is the mod-facing layer, but no `sdk` command feeds it yet (needs the menu
  movies in the user's assets first, part of the screen-manager step).
- Running the linked menu movies end to end needs the pipeline to export menu movies in the player's JSON format
  (characters, fonts, language) like the HUD prepare step; then `link_imports` is called after `Movie::load`.
- The missing-file and cycle behaviour of retail (above).

## Milestone 3c: menu movies exported at setup, loaded and linked (2026-10-08)

### Problem
The APT player and the import linker (3a, 3b) only ran on research decodes in `.local/`. The engine had no menu
movies from the user's disc, and mods had no way to reach the import override layer.

### Change
- **Setup export** `tools/prepare_menu_movies.py`, run inside the HUD stage (`prepare_runtime_huds.py`, same disc
  archives and fonts as the scoring HUD; `versions.py` fingerprints it in the `hud` group, so a re-run refreshes the
  set). Starts from 12 menu screens (`MENU_ROOTS`: FE_root, core_menu, dimmer, freeskate_options, options, main3dhud,
  screenmanager, screentransitionmanager, tabs, scrollbar, sk8popup, small_popup) and follows every import, as
  retail's load queue requests each import source by name before linking (sub_82E7E8C0, milestone 3b). Writes
  `assets/private/menu-movies/` (never committed): `manifest.json` (roots, movies, hashes), `shared.json` (language
  table, fonts, native font layout, once for all movies) and `movies/<key>.json` per movie in the HUD player format
  plus `imports` / `exports`; keys are the retail import paths (`apt_imports::library_key`). Install verifies every
  hash into a staging folder and swaps it in; a failed export keeps the previous set and writes
  `menu-movies-availability.json` like the HUD. On the user's disc: 23 movies (12 roots + 11 libraries), none
  missing, every font resolved, about 9 MB.
- **Engine** `crates/skate-game/src/retail_menu_movies.rs`: resource `RetailMenuMovies`, loaded at startup next to the
  menu tables (`retail_menus.rs` plugin). Each movie: `MovieDecl::from_json` + `apt_imports::resolve` (import
  tables resolve even for movies the player cannot load yet), `Movie::load`, `Movie::link_imports`. Log
  `RETAIL_MENUS movies=<linked> imports=<resolved>/<total>`, one warning per movie that failed with the reason.
  Nothing is drawn.
- **Moddable** `sdk.menus.movie(name, path)` (command `menu_movie`): a mod supplies or replaces a menu or library
  movie by its import name with a player-format JSON file from its own folder (path contained in the package,
  16 MB cap). It goes through `OverrideTable` (newest owner wins) and every movie is relinked, so menus importing a
  replaced library pick up the mod's version (a missing export leaves the slot empty, as retail's linker does).
  Removed on mod stop / disable (`retail_menus::clear_owner` / `clear_mods`). `sdk.menus.info().movies` lists linked
  movies, import counts, failures and each mod's movies. Multiplayer: keys are stable strings, state is per-owner and
  rebuilt deterministically (sorted keys).
- **Data-gated tests from a worktree**: `apt_imports::main_checkout()` follows a worktree's `.git` file back to the
  main checkout (override `SKATE3_MAIN_CHECKOUT`), so the 3a / 3b tests now find `.local/research/fe-menus/actions`
  from `.local/wt-menus` too. The Python test finds the disc the same way through `git rev-parse --git-common-dir`
  (overrides `SKATE3_DISC`, `SKATE3_COLLECTIONS`).
- NOT RETAIL (layout): the shared language / font file and the JSON format are ours; retail loads `.apt` / `.const`
  through `data\fe\` at run time. Same content.

### Verification
- `python -m unittest test_prepare_menu_movies` (in `tools/`): key rule matches the engine, hash-verified install
  replaces the old set and keeps it on failure, contained paths; data-gated export from the owned disc into a
  temporary folder: every root exported, none missing, no unresolved font, every import's export present in its
  source movie. 4 passed (13 s).
- `cargo test -p skate-game --bin skate3rust -- apt menu` with `SKATE3_MENU_MOVIES` at a scratch export: 42 passed,
  3 ignored. `retail_menu_movies::tests`: link + mod replace + revert, missing set; data-gated
  `retail_menu_movie_set_loads_links_and_runs`: `RETAIL_MENUS movies=13 of 23 imports=25/25`, no unknown opcode in
  any stream of the 23 movies. `cargo test -p skate-mods --lib -- menu`: 4 passed (Lua `menu_movie` command).

### Open (player gaps found by loading the real set)
- 5 movies use placement clip actions (place flag 0x80, `onClipEvent` handlers): `apt_display.rs` refuses them
  (button_item2, button_menu, image_renderer, tabs, core_menu). 4 more fail only because they import button_menu
  (panel, freeskate_options, options, sk8popup). Done in Milestone 3d.
- small_popup: the Futura Shadow foreground-font rule in `apt_text.rs` is HUD-specific (looks for a `futuraheavy`
  font in the same movie); retail's pairing rule for other movies is not decoded.
- The root list is ours (the menu screens the engine will use); more screens can be added without code changes in
  the engine.

## Milestone 3d: placement clip actions (2026-10-08)

### Problem
9 of the 23 exported movies did not load: 5 place clips with clip actions (place flag 0x80, `onClipEvent`
handlers), which `apt_display.rs` refused, and 4 import one of them (button_menu).

### What retail does [code, TU3]
- **Record.** Placement +60 (sub_82E7B340 reads it at record+4+56 when flag 0x80 is set; on a new placement and on
  a move) points at `{u32 count, u32 events}`; each event is 12 bytes `{u32 flags, u32 key_code, u32 actions}`
  (sub_82E5B158 walks it with stride 12 and queues record+8). sub_82E7B090 stores the table pointer at display
  object +24 only when one is given, so a move without 0x80 keeps the old table.
- **Flags** are the SWF clip-event values: Load 0x1, EnterFrame 0x2, Unload 0x4, MouseMove 0x8, MouseDown 0x10,
  MouseUp 0x20, KeyDown 0x40, KeyUp 0x80, Data 0x100, Initialize 0x200, Press 0x400, Release 0x800,
  ReleaseOutside 0x1000, RollOver 0x2000, RollOut 0x4000, DragOver 0x8000, DragOut 0x10000, KeyPress 0x20000,
  Construct 0x40000. Evidence: the dispatch sites below, and the dynamic-handler table 0x82FC9A3C (6 pairs of mask
  and name id) mapping 2 / 0x40 / 0x80 / 0x100 to onEnterFrame / onKeyDown / onKeyUp / onData (name table
  0x83044C60). The ids for 1 and 4 sit 10 apart like onLoad / onUnload (0xCF / 0xD9) but one entry off from the
  others, so that pair is medium confidence; the dispatch sites confirm Load = 1 and Unload = 4 by use.
- **Placement** sub_82E7AF18 (sprites only): ORs every event's flags << 8 into display flags +20 for the masks
  0x201C7 (load, enterFrame, unload, key down / up, data, keyPress), registers the clip as a key listener
  (sub_82E88050) when it has keyDown / keyUp / keyPress, then dispatches **Initialize (0x200) and Construct (0x40000)
  at once** (sub_82E5B158: an action object of priority 60 / 70 run synchronously through sub_82E6A208).
- **Tick** sub_82E5B640 (every clip, playing or not): frame advance, frame controls (sub_82E810B8: init actions,
  placements), frame do_actions queued at the back (sub_82E81438 -> sub_82E78168); then **EnterFrame** (2), except
  on the clip's first tick, pushed at the **front** of the action queue (sub_82E78240); on the first tick
  (flag 0x80 in +20) **Load** (1) at the back instead, then that flag is cleared; then the children
  (sub_82E7C658). So children's enterFrame handlers run before their parent's, and all of them before that tick's
  frame actions.
- **Unload** (4): removal sub_82E55DA8 queues it at the back before tearing down the children; it runs on the
  removed clip.
- **Scope**: the queue entry's target is the clip that owns the placement (sub_82E78168 stores r5 = the clip), so
  `this` is the placed clip, as in Flash.
- **Input events** (press / release / rollOver / key*) come from the host's input through the same dispatcher;
  no movie in the set uses them (all 9 tables hold only Construct). Out of scope here.
- **0xA6** (handler 82E74028): push string operand, then setVariable (0x1D, 82E6D248). Every construct handler in
  the set is `_type = "CustomButton"` / `mButtonImage = "A"` style component parameters written with it.

### Change
- `tools/prepare_menu_movies.py`: decodes the table into `clip_actions` on the placement (bounds-checked, at most
  64 events) and exports the handler streams; menu set version 2 (the engine refuses version 1, setup re-exports).
- `apt_display.rs`: `ClipAction`, `clip_event` flag constants, `Placement::clip_actions` (kept on a move without
  0x80, as retail); a 0x80 placement without a decoded table (old export) and more than 64 events are errors.
- `apt_movie.rs`: `Pending { object, offset, event }` queue; `create` queues Initialize then Construct in
  `immediate` (run before anything queued) before the clip's first frame, Load after its first frame's actions;
  `advance` pushes EnterFrame of every clip that existed before the tick to the queue front, in creation order (so
  children first); `remove` queues Unload; `next_action()` drains immediate first and skips handlers of removed
  clips except Unload. `link_imports` copies and renumbers clip-action streams. `hud_runtime.rs` drains through
  `next_action()` (HUD movies have no clip actions: unchanged).
- `apt_vm.rs`: opcode 0xA6.
- Mod movies use the same path (same JSON, same limits).
- NOT RETAIL (timing): retail runs Initialize / Construct inside the placing action; ours right after the running
  action, still before every other queued action. NOT RETAIL YET (order): retail queues a parent's frame actions
  before its new children tick (Load); our seek places children first (existing frame-action order).

### Verification
- `cargo test -p skate-game --bin skate3rust -- apt hud menu` with `SKATE3_MENU_MOVIES` at a scratch export:
  47 passed, 4 ignored (unrelated data-gated tests matched by the filter). New `apt_movie::clip_action_tests`:
  dispatch order (initialize, construct, the clip's frame action, load; next tick enterFrame; unload runs on the
  removed clip; nothing after removal), scope (construct sets `_type` on the placed clip, not the parent, through
  0xA6), unsafe tables (old export without a decoded table, more than 64 events) are errors and a move keeps the
  table. Data-gated `retail_menu_movie_set_loads_links_and_runs`: **`RETAIL_MENUS movies=22 of 23
  imports=25/25`** (was 13), no unknown opcode in any stream including the clip-action streams.
- HUD: `scoring_runtime::tests::landed_multiplier_reaches_hud_across_repeated_tricks` (with `SKATE3_ASSET_ROOT`)
  passes; the HUD drain only changed to `next_action()`.
- `python -m unittest test_prepare_menu_movies`: 5 passed (new: table layout and bounds); the real export decodes
  9 tables, all Construct only.

### Open
- small_popup (the last of the 23): the Futura Shadow foreground-font rule (done in Milestone 3e).
- Input-driven clip events (press, release, rollOver, key*) wait for the menu input milestone; the dispatcher
  shape (`queue_events`, flag constants) is ready for them.
- The onLoad / onUnload name ids in table 0x82FC9A3C are one entry off from the others (see above); behaviour
  does not depend on it.

## Milestone 3e: the Futura Shadow secondary font in every movie (2026-10-08)

### Problem
small_popup, the last of the 23 movies, failed with "Missing native Futura Shadow foreground font". Our rule
(apt_text.rs, written for the HUD) looked for the foreground font among the movie's own APT font characters;
small_popup places "Futura Shadow" but no character that uses futuraheavy.

### What retail does [code, TU3]
- Native font table: FontManager sub_82808AE8 builds up to 12 rows of 36 bytes at 0x830680D0 from the font records
  (class 0xFECFBCAF356518C4, the list picked by language mode): row +0 file name, +4 APT name, scale / offset
  floats, +32 the font (loaded lazily by sub_82809308 from `data\fe/fonts/<file name>.bmpFont`, sub_82807870).
- Lookup sub_82809208(name): case-insensitive compare (sub_82AE8A10) of `name` against each row's +4 name; on a
  miss, the row whose +0 name is "debug" (0x821A06E0); else row 0. It never fails.
- Text font setup sub_825D6B68: primary = lookup(APT font name); if that name equals "Futura Shadow" (0x8220BD44,
  sub_82AE89B0) the text also gets a secondary font = lookup("futuraheavy") (0x8220BD54) with X adjustment 1 and
  the other 0. sub_82CA1FD8 draws the primary black, then the secondary in the text colour. The secondary comes
  from the global native table: it does not depend on the movie using that font.

### Change
- `apt_text.rs`: `Font::foreground` is now the secondary native font itself (`Option<Box<Font>>`), not an APT
  character id. `RETAIL_FONT_PAIRS` holds retail's one pair; data may replace it with `font_pairs` (APT name ->
  native file name; the menu export writes retail's table into shared.json, an empty map turns pairs off). The
  partner is found in the whole native table (`font_mappings` by file name), case-insensitive. A missing partner
  (old export, a mod's data) leaves the text with its primary pass: no error, no panic.
- `apt_movie.rs` (import linking) copies the secondary with the font; `apt_scene.rs` draws it directly;
  `scoring_hud.rs` loads its texture too. HUD output unchanged (same fonts, same passes).
- `tools/prepare_menu_movies.py`: `FONT_PAIRS`; the export adds each pair's partner font to shared.json even when
  no movie places it, and writes `font_pairs`. Older installs still load (retail table default; without the
  partner asset the shadow text draws its primary pass only): re-running setup group `hud` adds it.
- Moddability: a mod's movie (same JSON path) gets the same rule; a mod font missing its partner falls back
  without error.
- NOT RETAIL (fallback): retail's lookup falls back to the "debug" row; we have no debug bank and draw the
  primary pass only.
- OPEN QUESTION (match field): the static data says row +4 is the APT name (the debug row's is "NOT FOR USE IN
  APT", the futuraheavy row's is "Futura Std Medium"), so lookup("futuraheavy") would miss and fall back to the
  debug font. We keep the HUD's established match by file name (futuraheavy), which the HUD port matched
  visually. A recomp trace of sub_82809208's return for "futuraheavy" would settle it.

### Verification
- `cargo test -p skate-game --bin skate3rust -- apt hud menu font` with `SKATE3_MENU_MOVIES` at a scratch export:
  51 passed, 4 ignored. New `apt_text::font_pair_tests` (4): Futura Shadow gets futuraheavy from the native table
  without a character using it; missing partner asset or table row gives the primary pass only; `font_pairs` data
  replaces the retail table (case-insensitive, empty map turns pairs off, unknown file falls back); other fonts get
  no partner. Data-gated `retail_menu_movie_set_loads_links_and_runs`: **`RETAIL_MENUS movies=23 of 23
  imports=25/25`**; the test now fails on any load failure.
- HUD unchanged: `scoring_runtime::tests::landed_multiplier_reaches_hud_across_repeated_tricks` (with
  `SKATE3_ASSET_ROOT`), `scoring_hud::tests` and the other scoring tests pass.
- `python -m unittest test_prepare_menu_movies`: 5 passed; the scratch export holds 23 movies, no missing movie,
  no unresolved font, `Futura Std Medium` in shared.json.

## Milestone 3f: placement clip depth (2026-10-08)

### Problem
A SWF placement can carry a clip depth: the placed shape is then a mask for the depths above it up to the clip
depth. Our display list parsed the field (`apt_display.rs`) but the drawing path (`apt_scene.rs`) ignored it, and it
was unknown whether retail APT draws such masks (stencil, scissor) and whether the menu movies use them.

### What retail does [code, TU3]
- **Read.** `sub_82E7B340` (placement record at r31 = control + 4: +0 flags, +4 depth, +8 character, +12 matrix,
  +36 colour transform, +44 ratio, +48 name, +52 clip depth, +56 clip-event table) loads +52 and passes it to
  `sub_82E7AE98` -> `sub_82E7B090` on every path that creates a display object: flag 2 (new or replaced character)
  and a move (flag 1) onto an empty depth. It does not test flag 0x40. A move of an existing object
  (`loc_82E7B5F8`) passes -1, and `sub_82E7B090` only hands the value to the create call `sub_82E7A950`, so a move
  never changes a stored clip depth.
- **Store.** `sub_82E7A950` writes it as a halfword: `sth r23,22(props)` (props = display object +32 -> +4; +20 is the
  depth halfword). Clones copy it (`sub_82E832C8`, `sub_82E83BC0`).
- **Use.** Over the whole recompiled image only two code sites read props +22: the getter `sub_82E48930` (and
  `sub_82E461A0` on props), which no code calls and no data table points at, and the bounds walk `sub_82E59C28`
  (called by `sub_82E59DA8`, the local-bounds function behind sizes and hit areas), which leaves out every child whose
  clip depth halfword is below 0x8000 (a mask). No other reader exists, the APT renderer (game side, 82CA range) has
  none either, and no load of the word at +20 extracts the low half. So **retail draws a clip-depth shape like any
  other shape and masks nothing**; the only effect is that masks do not count towards a clip's bounds. Script masks
  (`setMask`, warning string 0x82062250) are a separate mechanism, not used by these movies.
- **Data.** All 23 retail menu movies: 1434 placements, every clip depth -1, flag 0x40 never set. Both HUD movies:
  1233 placements, all -1. No retail movie we ship uses a clip depth.

### Change
- `apt_display.rs`: clip depth follows retail: taken from the record whenever a placement creates the object
  (flag 2) regardless of flag 0x40, stored as 16 bits (`as i16`), missing value = -1; a move keeps the stored
  value (flag 0x40 on a move no longer errors or changes it). Doc comment on `Placement::clip_depth`.
- Drawing (`apt_scene.rs`) unchanged on purpose: retail draws the shape and masks nothing, so no stencil or
  scissor. Bounds: our player has no sprite bounds yet (`_width` / `_height` exist only for text,
  `apt_movie.rs` text setup); when sprite bounds land they must skip children with clip depth >= 0
  (`sub_82E59C28`).
- Mods: a mod movie with clip depths draws exactly as retail would (no mask); values are clamped to 16 bits like
  retail, no panic path.

### Verification
- `cargo test -p skate-game --bin skate3rust -- apt hud menu font` with `SKATE3_MENU_MOVIES` at a scratch export:
  52 passed, 4 ignored. New `apt_movie::clip_action_tests::apt_clip_depth_follows_retail_placement` (stored without
  flag 0x40, kept on a move with flag 0x40, replaced with the character, 16-bit storage, missing = -1, removed with
  the object). The data-gated set test now counts clip depths: **`RETAIL_MENUS movies=23 of 23 imports=25/25
  placements=1434 clip_depth=0`** and fails if a retail menu movie ever carries one.
- HUD unchanged (all its clip depths are -1 and drawing does not read the field):
  `scoring_runtime::tests::landed_multiplier_reaches_hud_across_repeated_tricks` (with `SKATE3_ASSET_ROOT`) passes.

### Open
- Retail creates an object when a move (flag 1 only) targets an empty depth; our display list still refuses that
  ("APT move references empty depth"). Not hit by any retail movie.
- A recomp trace could confirm at run time that nothing reads props +22 while a menu draws (static evidence only).

## Milestone 4: bitmaps and textured fills (2026-10-08)

### Problem
The menu movies are mostly textured shapes (panels, glows, arrows, tab icons), but the player could only describe
shapes that always have a texture (the HUD case): solid units did not load, imported shapes lost their geometry
(`link_imports` gives them a new id in the importing movie) and nothing named a bitmap, so a mod could not replace
one.

### What retail does
- [data, the user's disc, 23 menu movies] 154 bitmap characters (type 7, `{texture_id}` only). None is ever placed:
  placements reference sprites (204), shapes (193) and text (25) only. Every bitmap is drawn as the texture of a
  shape unit. Shapes come pre-tessellated in each movie's `.geo` (no gradients or curves at run time): 231 units,
  207 `texture_clamped` (render type 2) and 24 `solid` (type 1); no `texture_wrapped` (3) or `line` (0). Every unit
  resolves; the 207 textured units use all 154 bitmaps (30 are used by more than one unit). The 2 HUD movies
  (trickdisplay and its fonts) have only textured units.
- [data] GEO unit layout (`tools/vendor/skate3_ui/geo.py`): +0x00 render type, +0x04 RGBA colour (floats), +0x14
  bitmap character id, +0x18 UV matrix (6 floats, applied to the vertex position, in texels), +0x30 triangle count,
  +0x34 triangles. The bitmap character's `texture_id` names the texture `<movie>_textures\<id>.Texture` in the
  movie's texture package (fetexture.big / fedata.big RX2), decoded to RGBA8 at setup (158 payloads incl. fonts).
- [code, TU3] The movie loader `sub_82CA3428` (AptInitParms +36 callback chain, doc "Milestone 3b") opens the three
  files of a movie from the `data\fe\` table at `0x820AED00` (`.apt`, `.const`, `.geo` at `0x820AECF8`) into
  {pointer, size} slots at record +532 / +540 / +548: the GEO blob is kept as authored. The texture name format
  `%s_textures\%i.Texture` is at `0x820AED9C`, with a fallback `missingtexture.Texture` at `0x820AEDB4`.
- [code, TU3] Draw chain (details in `.claude/notes/fe-menus-re.md` "APT shape renderer"): `sub_82CA4020` builds
  the movie resource, `sub_82CA30B8` relocates the GEO in place, `sub_82CA2A40` makes one record per shape and
  `sub_825D5DC8` one draw object per unit: type 1 -> solid object `sub_825D5800` (draw `825D5AE0`), types 2 / 3 ->
  textured object `sub_825D4FC0` (draw `825D5580`), any other type (line 0) -> nothing. The textured draw computes
  one colour = unit colour * colour-transform multiply + add, draws nothing when its alpha <= 0, and hands 6
  vertices (2 triangles), the UV rows from unit +24..+44, the texture and that colour to `TheSimpleDraw`
  (`sub_82805368`, shaders `simpledraw_*` in shaders_final.big).
- Not decoded yet: the sampler (filter, and which of types 2 / 3 clamps); see Open.

### Change
- **Setup** (`tools/prepare_menu_movies.py`, set version 3): each textured unit also carries `bitmap`, the GEO unit
  +0x14 id it was resolved through (`_with_bitmap`). Re-export on the next setup run (the `hud` group fingerprint
  covers the tool). No new files: the RGBA payloads were already exported.
- **Scene** (`crates/skate-game/src/apt_scene.rs`): `Fill` (GEO render type), `Shape.texture` optional (solid and
  line units), `Shape.bitmap`, `Vertex.uv` optional; `Draw.fill` tells the renderer clamp or wrap addressing (text
  draws are clamped); a solid draw has texture `""`. Line units are not drawn (retail `sub_825D5DC8`). Shapes come
  from a `ShapeSource`; for menus (`retail_unit_colour`) a unit draws with retail's single colour
  `unit colour * multiply + add` and is skipped at alpha <= 0 (`825D5580`). The HUD keeps its `Shapes` table and
  its earlier `texel * multiply + add` (output unchanged; see Open).
- **Imports** (`apt_movie.rs`): `Movie::shape_origins` records, for every shape `link_imports` copies, the library
  and id that define it (following nested imports to the defining movie).
- **Menu set** (`retail_menu_movies.rs`): GEO units per movie key (validated: textured units need a bitmap,
  finite values; a bad movie is a failure, not a panic); `scene(key, movie)` gives the `ShapeSource` for a running
  instance; `image(texture)` returns the retail payload (path in the set + size) or a mod image.
- **Stable id and mod entry point**: a bitmap is `<movie key>#<bitmap character id>` in the movie that defines the
  shape. `sdk.menus.bitmap(movie, bitmap, path)` (command `menu_bitmap`) replaces it with a PNG from the mod's
  folder (4 MB file, up to 2048 x 2048, 64 per mod), decoded to RGBA8; the image covers the same fill (same UVs,
  normalised), newest mod wins, removed on mod stop / disable with the rest of the mod's menu content
  (`clear_owner` / `clear_mods`). `sdk.menus.info().movies.bitmaps` lists every id (`ids`) and each mod's
  replacements (`mods`). A whole movie can still be replaced with `sdk.menus.movie`.
- **Fixed**: `menu_movie` validated its path as a `.glb` asset, so every Lua `sdk.menus.movie(..., 'x.json')` was
  rejected at submit; it now takes a `.json` content path.
- Deterministic and local: draws follow the display list order; ids are strings sorted in `BTreeMap`s.

### Verification
- `python -m unittest test_prepare_menu_movies` (in `tools/`): new `test_textured_units_get_their_bitmap_id`; the
  data-gated export checks that every shape resolves and every textured unit names a bitmap character whose texture
  id is the payload's. 6 passed.
- `retail_menu_movies::tests::menu_bitmaps_draw_through_imports_and_mods_replace_then_revert`: a menu importing a
  library sprite draws the library's solid and textured units (colour, UVs, placement), the retail payload resolves,
  a mod bitmap replaces it by stable id (invalid ids, sizes and movies refused), newest owner wins, stop reverts.
  `menu_units_follow_the_retail_colour_rule` (line unit not drawn, alpha 0 skipped, add folded into the colour),
  `menu_shapes_reject_textured_units_without_bitmap`. skate-mods: `menu_movie` / `menu_bitmap` validation.
- Data-gated `retail_menu_movie_set_loads_links_and_runs` now also walks every shape of every linked movie
  (local and imported) and draws each movie's first frame: **`RETAIL_MENUS movies=23 of 23 imports=25/25
  placements=1434 clip_depth=0 shape_units=528 textured=481 solid=47 bitmaps_used=154 bitmap_ids=154
  frame0_draws=863 frame0_bitmap_draws=835`** (unit counts are per linked movie, so library shapes count once per
  importer; every one of the 154 bitmaps is reached and its payload has the exported size).

### Open
- Sampler: the tfetch filter / address bits of the `simpledraw_*` pixel shaders are not decoded (disassembler fails
  on constant-less shaders; notes "APT shape renderer" OPEN), nor which of types 2 / 3 clamps. Ours names the
  addressing from the render type (`Draw.fill`); the screen-manager renderer must take filtering from that decode.
- The HUD movie has 233 placements with colour-transform add terms; retail folds add into the unit colour
  (`825D5580`), ours still adds after the texel for the HUD. Switching the HUD to `retail_unit_colour` is a one-line
  change that changes its look, so it waits for an in-game check. The solid draw `825D5AE0` is assumed to use the
  same colour rule (to read).
- `missingtexture.Texture` (`0x820AEDB4`): retail's fallback for a missing payload; ours fails the movie at setup
  instead (no retail movie misses one).
- Drawing the menus on screen comes with the screen manager milestone (the renderer maps `Draw.texture` through
  `RetailMenuMovies::image`).

## Files
- `crates/skate-data/src/menu_tables.rs` (extractor, JSON, conversion), `crates/skate-data/tests/menu_tables.rs`
- `crates/skate-core/src/menus.rs`, `menus_tests.rs` (registry), `menu_values.rs` (value rules, milestone 2)
- `crates/skate-game/src/apt_vm.rs` (milestone 3a opcodes, tests)
- `crates/skate-game/src/apt_imports.rs` (milestone 3b import resolver, mod overrides, tests), `apt_movie.rs`
  (`link_imports`), `main.rs` (module)
- `crates/skate-game/src/apt_imports.rs` (milestone 3b import resolver, mod overrides, tests), `apt_movie.rs`
  (`link_imports`), `main.rs` (module)
- `crates/skate-game/src/retail_menus.rs` (resource, loader, mod commands, events, snapshot, engine value bindings),
  `game_audio/mod.rs` (`RetailVolumes`, saved), `game_audio/native.rs` (Master.in1..3), `main.rs`
  (`--extract-menu-tables`), `app.rs`, `modding/mod.rs` (commands, cleanup, events, snapshot)
- `crates/skate-mods/src/menus.rs`, `vm.rs` (commands, capability), `api.lua`; `sdk/skate.lua`
- `tools/asset_pipeline/menu_tables.py`, `test_menu_tables.py`, `asset_exports.py`, `versions.py`
- Milestone 3c: `tools/prepare_menu_movies.py`, `tools/test_prepare_menu_movies.py`, `tools/prepare_runtime_huds.py`;
  `crates/skate-game/src/retail_menu_movies.rs`, `retail_menus.rs` (startup, cleanup, snapshot), `modding/mod.rs`
  (`menu_movie`), `apt_imports.rs` (`main_checkout`, `OverrideTable::position`), `apt_vm.rs` (test path)
- Milestone 3d: `crates/skate-game/src/apt_display.rs` (`ClipAction`, `clip_event`), `apt_movie.rs` (`Pending`,
  dispatch, `next_action`, link), `hud_runtime.rs` (drain), `apt_vm.rs` (0xA6), `retail_menu_movies.rs` (set
  version 2, test); `tools/prepare_menu_movies.py` (`clip_actions`), `tools/test_prepare_menu_movies.py`
- Milestone 3f: `crates/skate-game/src/apt_display.rs` (clip-depth storage rule), `apt_movie.rs` (test),
  `retail_menu_movies.rs` (clip-depth count in the set test)
- Milestone 4: `crates/skate-game/src/apt_scene.rs` (`Fill`, `ShapeSource`), `apt_movie.rs` (`shape_origins`),
  `retail_menu_movies.rs` (shapes, `MenuScene`, `image`, mod bitmaps, tests), `scoring_hud.rs` (optional texture),
  `modding/mod.rs` (`menu_bitmap`); `crates/skate-mods/src/vm.rs`, `api.lua`; `sdk/skate.lua`;
  `tools/prepare_menu_movies.py` (set version 3), `tools/test_prepare_menu_movies.py`

## Verification
- `cargo test -p skate-data --test menu_tables -- --ignored` (local files, skipped when absent): the TU3 image gives
  exactly the documented tables and addresses (36 rows, 13 screens, 5 categories, 49 items, 56 keys, 9 modes,
  Career Main, park rows 9 slots, Free Play 4 tabs); the user's disc `default.xex` (a different build) gives the same
  content at its own addresses, minus TU3's additions (above), compared entry by entry against TU3.
- `cargo test -p skate-core menus`: greyed by default, a handler enables, dirty-park confirmation intercepts, hidden
  rules (Project10, SFX pack), links and the Video choice, mod add / hide / reorder / relabel and exact revert,
  owner stacking.
- `cargo test -p skate-mods menus`, `cargo test -p skate-game retail_menus`: options and rule validation, mod
  commands end to end, events JSON, revert.
- `python -m unittest tools.asset_pipeline.test_menu_tables`: the setup step through the real spawn path.

## Open questions
- Career IsItemEnabled (`8261F850`) greys SkateWith / SkateReel when `82511168` fails and EnterSkatePark / PartyPlay by
  other checks; those conditions are not decoded yet, so Career uses the default rule (enabled once handled).
- The Free Play park twin of IsItemEnabled (`826205C8`) is assumed to match the Career park one; to read.
- Mode keys are ours (retail has no names); the online / lobby menus are extracted but not decoded further (online
  is a custom implementation later).
- Retail stops the settings row count at a hidden row; the registry skips hidden rows instead (identical for the only
  retail case, the SFX pack row, which is last on its screen).
- Next milestones (settings model done, above): APT player gaps, host side (Game / CoreMenu objects, fe
  sounds, input repeat), then swapping in the retail menus with the current features as hooks.
