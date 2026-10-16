# Church Wing Console

Tools for mapping and documenting our church's Behringer WING console.

- `Libwing/` — a read-only fork of [libwing](https://github.com/dannyfiresnake/libwing)
  (Apache-2.0, see `Libwing/LICENSE`). Changes from upstream: all `set_*` calls return
  `Error::ReadOnly`, `wingschema` exits immediately and `wingprop` refuses `=`, so nothing
  here can change the mixer.
- `Libwing/tools/wingmap.rs` — maps every console setting into `Libwing/maps/`.
- `Libwing/viewer/` — the browser dashboard (`index.html`) and `bundle.py`, which builds a
  shareable one-file copy. Every page except the settings tree has **Export this page: PNG |
  HTML** buttons: PNG saves a 2x light-theme picture of the page as shown, HTML a stand-alone
  copy (no code). Files are named after the page and the map's date, e.g.
  `monitors 2026-10-11.png`. PNG export doesn't work in Safari; use Firefox.
- `Libwing/maps/make_pages.py` — turns the map's Mermaid diagram into Graphviz pages.

Console maps, snapshots and diagrams contain names and network details, so they stay on the
mixing Mac and are excluded by `.gitignore`.
