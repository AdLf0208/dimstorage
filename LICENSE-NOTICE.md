# Licence notice — DimStorage

This port carries the original mod's assets and data and rewrites its behaviour
in Rust. The original licence and copyright notices are reproduced verbatim in
[LICENSE](LICENSE).

## Where the licence was read

| | |
|---|---|
| Pack jar | `DimStorage-1.21.1-9.1.1.jar` (179,454 B, SHA-256 `430e34540916cae401f8c18fe92c59d4dfa96f8e86cea3e14346bb1d2f9191db`) |
| Declared licence | `GNU AGPLv3` in `META-INF/neoforge.mods.toml` |
| Licence text | the upstream repository's `LICENSE` (the full AGPL-3.0 text, already carried in the port); the jar bundles no licence file and only names the licence in `META-INF/neoforge.mods.toml` (34,523 B) |
| SPDX identifier | `AGPL-3.0-only` |
| Copyright line | `Copyright (c) Edivad99 and DimStorage contributors` |

The jar ships no separate licence file. The jar names the author(s) `Edivad99` but
carries no separate project copyright line, so the copyright line above uses
that author list; replace it with the upstream `LICENSE` copyright statement if
one is added later.

## The port

- Original: DimStorage 9.1.1 by
  Edivad99 — AGPL-3.0-only
  (https://github.com/Edivad99/DimStorage).
- Port to Horizon by AdLf0208: the Rust runtime, the Horizon declarations and the
  build/verification tooling in this repository.
- This repository is the complete corresponding source: the licence requires
  that it travels with every copy and with every modified version.
- No Mojang file is included. The carried files are the mod's own assets and
  data; `data/minecraft/tags/…` files only add this mod's ids to vanilla tags.
