# React-import acceptance fixture: BGCH Hub

This fixture prevents a fragment-only JSX converter from being represented as a
React page importer.  The entry file is the real, untouched Hub
`AppDirectory.jsx`; the fixed super-admin snapshot selects the branch that
contains the complete seven-tile application directory.

Run the fixture through the MCP tool in the `request` object, then inspect the
created subtree (and a raster export) against `assertions`.

The two SHA-256 values deliberately pin the entry component and its canonical
`SUITE_APPS` catalog.  A runner must fail with an explicit stale-source
diagnostic if either file differs.  This is not permission to execute either
file: the importer may only parse the bounded static syntax required by this
fixture and resolve modules underneath `module_roots`.

The current fragment-only `jsx` parameter cannot satisfy this fixture.  A
passing API must accept `source_path`, `export_name`, a JSON-only `props`
snapshot, and bounded import resolution.  The import needs to preserve text,
image references, tile URLs, and a 3-column desktop layout as editable
Photonic nodes.  It must not rasterize the page or silently discard unsupported
content.

Minimum verification sequence:

1. Use `dry_run: true`; expect a complete plan and zero error diagnostics.
2. Run the exact non-dry request; expect one undoable grouped import.
3. Query document state and assert seven link/tile groups, seven image/icon
   references, and fifteen editable text nodes.
4. Export a 1120 x 720 PNG and visually verify a 3 / 3 / 1 card grid.
5. Undo once and assert that the imported root and every descendant are gone.

Also test the source-security boundary separately: a source outside
`module_roots`, a changed hash, dynamic `onClick`, or an unresolved imported
component must return structured diagnostics before document mutation.

## Metamorphic (anti-template) suite

`hub_app_directory_metamorphic.json` is an additional required acceptance
suite. Its runner copies the actual component and canonical catalog to a fresh
temporary directory, uses that directory as the sole allowed root, and makes
one source-only change per case. The importer binary is not rebuilt or edited
between cases. In every successful dry run, the returned plan must include a
machine-readable `semantic_tree` (or equivalent source-mapped node list) so a
test can prove that the changed value is present before committing artwork.

The final case deliberately introduces unsupported syntax. It must produce a
specific structured diagnostic and leave the pre-import document state byte-for-
byte equivalent. A generic successful rendering or an image alone does not
pass this suite: it would permit an implementation that recognizes the Hub
filename and draws a hard-coded seven-tile template.

## Second-app gate: CHECK-IN ModeSelector

`checkin_mode_selector.json` and its metamorphic companion extend acceptance to
an unrelated app and component shape. The untouched entry must resolve
`KioskLayout`, `Button`, Lucide icons, and CHECK-IN theme tokens under a bounded
local root. Interactions use the explicit `strip` policy; dynamic backgrounds
and inactivity behavior are absent from the snapshot.

The metamorphic cases copy sources to a fresh temporary root, then alter text,
section order, Tailwind spacing/radius, component classes, and CSS variables.
The plan and editable document must follow every change without rebuilding the
importer. `strict` interaction handling, rendered unknown expressions, missing
imports, and outside-root imports must diagnose before document/history
mutation. This second-app gate is mandatory specifically to catch a
component-name switch that returns prebuilt ModeSelector artwork.
