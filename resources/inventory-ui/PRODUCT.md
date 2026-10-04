# Inventory example

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Static HTML, CSS and JavaScript packaged in the existing resource browser. Implementation choices are delegated by the approved platform extension request.

## Users

Skate multiplayer players checking their persistent inventory during play; resource authors learning the browser and authenticated persistence APIs.

## Product Purpose

Show a real inventory and transactional purchases tied to a verified account. Closing returns keyboard and mouse control to skating.

## Capabilities and Constraints

All assets are local. The page has no network access and exchanges JSON only with its resource script. Prices and balances are enforced server-side. No real currency or external service is involved. Keyboard navigation, readable status/error messages and resize support are required.

## Operating Context

A separate native browser window on Linux or Windows. Assumption for this example: a compact desktop inventory with a shop list, owned quantities and a visible balance. It may be resized to a narrow window.

## Evidence on Hand

The browser host, authenticated accounts and asynchronous transactional SQLite APIs are implemented in this repository. The example must use their real results rather than invented successful purchases.
