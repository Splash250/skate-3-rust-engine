---
name: Inventory example
description: A paper-and-ink shop stock ledger for the resource browser.
colors:
  paper: '#f4f1e9'
  ink: '#172b25'
  quiet: '#52605a'
  line: '#c5cdc0'
  accent: '#1d5842'
  accent-hover: '#103d2c'
  wash: '#e5ecdf'
  error: '#982d24'
  selection: '#c3df8a'
typography:
  display:
    fontFamily: Georgia, serif
    fontSize: 32px
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: -0.02em
  display-narrow:
    fontFamily: Georgia, serif
    fontSize: 27px
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: -0.02em
  balance:
    fontFamily: Georgia, serif
    fontSize: 28px
    fontWeight: 600
  headline:
    fontFamily: system-ui, sans-serif
    fontSize: 20px
    fontWeight: 650
    lineHeight: 1.5
  title:
    fontFamily: system-ui, sans-serif
    fontSize: 17px
    fontWeight: 650
    lineHeight: 1.5
  body:
    fontFamily: system-ui, sans-serif
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.5
  description:
    fontFamily: system-ui, sans-serif
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: system-ui, sans-serif
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.5
  key:
    fontFamily: system-ui, sans-serif
    fontSize: 12px
    fontWeight: 400
rounded:
  control: 6px
spacing:
  '4': 4px
  '10': 10px
  '12': 12px
  '14': 14px
  '16': 16px
  '18': 18px
  '20': 20px
  '24': 24px
  '32': 32px
components:
  button-primary:
    backgroundColor: '{colors.accent}'
    textColor: '{colors.paper}'
    typography: '{typography.body}'
    rounded: '{rounded.control}'
    padding: 10px 16px
  button-primary-hover:
    backgroundColor: '{colors.accent-hover}'
  button-quiet:
    backgroundColor: transparent
    textColor: '{colors.ink}'
    typography: '{typography.body}'
    rounded: '{rounded.control}'
    padding: 10px 16px
  button-quiet-hover:
    backgroundColor: '{colors.wash}'
  shop-row:
    textColor: '{colors.ink}'
    padding: 18px 0
  owned-row:
    textColor: '{colors.ink}'
    padding: 12px 0
  balance:
    textColor: '{colors.ink}'
    typography: '{typography.balance}'
  status:
    textColor: '{colors.quiet}'
    typography: '{typography.label}'
  status-error:
    textColor: '{colors.error}'
---

# Design System: Inventory example

## Overview

**Creative North Star: "Shop stock ledger"**

The shop stock ledger keeps inventory work direct: paper, dark green ink, ruled lists and restrained green actions. Georgia gives the title and coin balance a familiar printed character; system text keeps descriptions and controls readable. The compact desktop layout also supports a narrow native browser window.

This system is scoped to the inventory example. It records the code-led direction already chosen for this surface, with no approved visual comp or formal FORM key. Assets and fonts remain local. The original exploration retained tonal hierarchy, aligned quantities and stable item order; alternative visual worlds were not adopted.

**Key Characteristics:**

- Flat paper surfaces with single ledger rules.
- Distinct serif title and balance; practical system text elsewhere.
- Visible keyboard focus and explicit loading, empty, error and recovery states.

## Colors

Warm paper and subdued green ink establish an indoor, daytime inventory surface. The frontmatter preserves the stylesheet's exact color values.

### Primary

- **Shop green** (`accent`) fills purchase buttons and outlines keyboard focus; `accent-hover` deepens the button on hover.
- **Selection green** (`selection`) marks selected text, paired with ink.

### Secondary

- **Recovery red** (`error`) identifies error status text. The message names the problem and recovery, so color never carries the meaning alone.

### Neutral

- **Paper** (`paper`) is the page and purchase-button text.
- **Ink** (`ink`) is the main text and quiet-button label.
- **Quiet ink** (`quiet`) is descriptive copy, notes, key hints, status text and the scrollbar thumb.
- **Ledger rule** (`line`) divides the header, footer, balance and list rows.
- **Soft wash** (`wash`) appears behind hovered quiet buttons.

## Typography

Georgia with a generic serif fallback is the existing title and balance voice. Body text and controls use the local system sans stack. These are the fonts already chosen for this bounded example; no font files are fetched.

The frontmatter records the full observed type ramp: `display` for the page title, `display-narrow` at the narrow breakpoint, `balance` for available coins, `headline` for section titles, `title` for item names, `body` for controls and regular text, `description` for item copy, `label` for notes and footer, and `key` for the Escape hint. The balance and key shorthand use the browser's normal line height. Body and button line height is inherited by other text roles unless explicitly reset.

**The Aligned Numbers Rule.** Use tabular numerals for coin balances, purchase prices and owned quantities.

## Layout

The header places the title beside Back to park. The main area uses two tracks (`minmax(230px, 1fr)` and `minmax(340px, 1.55fr)`), a 42px gap, 32px padding and a 390px minimum height. The bag occupies the first track; the shop occupies the wider second track. The balance follows the bag contents, pushed to the bottom when space permits. The footer pairs live status with the reopen hint.

At widths of 680px or less, the main area becomes one column with a 32px gap and 24px by 20px padding. The shop moves visually above the bag; document reading order remains bag then shop. The balance loses its automatic top margin. Header padding becomes 20px, the footer stacks with an 8px gap, and the Escape key hint is hidden while the Back to park control remains. Ledger rows retain their text-and-action columns.

Spacing tokens capture the repeated pixel steps; header padding (26px by 32px) and the main grid gap are layout-specific values. Empty copy is limited to 27ch, the account note to 32ch on wide windows, and footer status to 65ch. Notes use the available width on narrow windows.

## Elevation & Depth

No shadows, gradients or elevated cards are present. Contrast, open space and one-pixel rules establish hierarchy. There are no authored animations or transitions; feedback changes immediately with request state.

**The Ledger Rule.** Separate entries with single rules and space; keep the item name, price and action together.

## Shapes

Buttons have gently rounded control corners, no border and a minimum height of 44px. Purchase buttons also have a minimum width of 110px. List rows and page regions remain square and open. Focus is a 3px solid accent outline offset by 4px, outside the control shape.

## Components

### Buttons and keyboard navigation

Purchase buttons show the price in coins; their accessible name also names the item and Buy action. They use paper text on the accent fill, the deeper hover fill and the shared control radius and padding. Back to park and Refresh are quiet buttons with ink text, a transparent background and a soft-wash hover. Disabled controls use 0.55 opacity and a default cursor. All keyboard-focused controls receive the shared outline.

Back to park and Escape post the close action. Refresh requests current inventory. A purchase response restores focus to the same item when it remains enabled; otherwise it falls back to Refresh. Restoration occurs only when focus was in the catalog or fell to the document body, preserving focus moved elsewhere by the user. A timed-out purchase with lost button focus moves focus to Refresh.

### Ruled stock and bag lists

Each shop row groups a title and quieter description beside the purchase button, with 18px vertical padding, a 16px gap and a top rule. Owned rows pair a name with its quantity, use 12px vertical padding and end with a rule. The bag's empty message disappears when an owned row exists. The coin balance stays labelled Available coins and uses a top rule; the account note follows it.

### Status and purchase recovery

The footer status is a polite live region. Initial loading and unavailable states occupy the catalog itself. The catalog exposes its pending state with `aria-busy`. While a request is pending, Refresh and all purchase buttons are disabled. Insufficient balance disables the affected purchase button.

Requests time out after 10 seconds. A purchase timeout marks the result uncertain, shows recovery instructions in error color, enables Refresh and keeps every purchase disabled. An error reply after that timeout preserves this lock; a valid successful inventory response clears it. The initial connection timeout offers Refresh. Existing inventory remains visible on later errors. Balances and ownership are updated only from accepted server data; no optimistic success is displayed.

## Do's and Don'ts

### Do:

- Do keep the bag and available coins together and preserve server-provided item order.
- Do retain visible keyboard focus, descriptive purchase names and textual recovery instructions.
- Do show balances and quantities from server responses, with an unresolved balance shown as a dash.

### Don't:

- Don't replace ruled lists with decorative cards, textures or imagery for this scoped ledger.
- Don't introduce remote assets or fonts.
- Don't show an optimistic purchase success or unlock buying after an uncertain purchase until a valid inventory response arrives.
