# Third-party notices

## SDL_GameControllerDB

`crates/skate-platform/gamecontrollerdb.txt` is a snapshot of
[SDL_GameControllerDB](https://github.com/mdqinc/SDL_GameControllerDB) at
revision `555ce569a2003b22a4e134882224f1e2bdecc2e3`. The database is used to
map controller-specific Linux inputs to SDL's logical controller layout.

It is redistributed under the zlib license stored at
`crates/skate-platform/gamecontrollerdb.LICENSE.txt`. No locally authored
mapping overrides are included.

## xdvdfs

`skate-xiso` uses `xdvdfs` 0.8.3 for read-only Xbox disc filesystem access.
The crate is distributed under the MIT license recorded by Cargo from its
upstream package at <https://github.com/antangelo/xdvdfs>.

## Lua resource runtime

The client and dedicated server use `mlua` 0.12.1 with Lua 5.4.9 supplied by
`lua-src` 551.0.2. Their MIT notices, including the underlying Lua notice,
are preserved in [LUA-NOTICES.txt](../tools/server-package/LUA-NOTICES.txt)
and included in the standalone Windows server package. The resource host
implements its own Cfx-inspired interfaces; no citizenfx source was copied.
