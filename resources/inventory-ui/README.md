# Authenticated inventory and browser example

This resource gives each verified account 100 example coins on its first visit. Purchases use parameterized transactions in a resource-owned SQLite database. Three reusable consumable inventory entries demonstrate persistent quantities; buying an entry does not change character appearance. Prices, starting coins and the catalog are game-mode policy in `server.lua`.

Build `skate-browser-host` with `cargo build --locked -p skate-browser --features host` and keep it beside `skate3rust`. Linux builds require WebKitGTK 4.1 development packages; runtime requires WebKitGTK 4.1. Windows uses the installed WebView2 runtime. See the browser documentation for the permission boundary and platform verification status.

Enable `inventory-ui` from `platform-examples.json` on an account-enabled server. Follow [account setup](../../docs/multiplayer/accounts-and-administration.md), then connect the game with `--connect SERVER:PORT --account-config credentials.json`. The page opens on activation. Escape or **Back to park** closes it and restores input; press **I** to reopen. Anonymous servers display a sign-in explanation. No password, session token or account selector enters resource or page messages.

The client and its HTML/CSS/JavaScript download as ordinary verified resource files. `server.lua` remains server-only. The page can send `inspect`, `buy` and `close` requests; the script only forwards known operations. The server checks the authenticated sender, validates catalog entries, allows one pending operation per player and limits total pending operations to 64. SQL constraints reject negative balances and quantities above 9999, rolling back the whole purchase. A revoked or disconnected session receives no late database result. Resource restarts close the page and cancel generation-owned service work; committed data survives.

The page has no network or native API access. All displayed items and balances come from server results, with errors and timeouts leaving Refresh available. Asset loading, browser readiness and server database readiness are separate stages.
