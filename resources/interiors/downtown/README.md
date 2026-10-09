# Downtown apartment package

This catalog uses the native interior service, shared collision shells and
server-approved travel. It adds two Downtown entrances:

| Entrance | Destinations |
| --- | --- |
| Hotel District Currency Exchange | Floor 2 — Mezzanine Flat; Floor 3 — Mezzanine Studio |
| Aletown Grill storefront | Floor 2 — Office Loft; Floor 3 — Haussmann Apartment |

The source projects are the Sweet Home 3D furnished examples 10, 12, 14 and 17.
Their local runtime folders are `mezzanine-flat`, `office-loft`,
`haussmann-apartment` and `mezzanine-studio`. Each contains `apartment.glb`,
`collision.json`, `preparation.json`, `ATTRIBUTION.txt`, `source-provenance.json`
and the original embedded notices in `source-licenses.json`.

Copy the complete validated package, including this catalog and all four folders,
to the active installation's `maps/locations/downtown/`, or select it with
`--locations DIRECTORY`. A multiplayer host selects the package with the same
flag; clients receive its mandatory native provider through resource admission.
Do not alter Downtown.skate or increase import limits.

The converted surfaces use a 1536-pixel color/alpha atlas and reduced geometry.
Collision retains architectural surfaces and substantial furniture faces;
small decorative details are omitted. Both native client and authority consume
that same shell. Budget and spawn checks do not establish full-room traversal.

See [the native loader and Lua/resource contract](../../../sdk/LOCATIONS.md).
Additional interiors and floors are catalog data; teleport, collision installation,
marker interaction and multiplayer approval remain engine functionality.

Original sources and payloads remain in ignored local storage. Preserve all
creator credits and notices. These examples are installed for local evaluation;
per-model redistribution clearance is not asserted. The previous Modern High-Rise
Apartment by LoneDeveloper is retained in a separate local backup, including its
attribution. Its distorted source JPEGs have not been repaired.
