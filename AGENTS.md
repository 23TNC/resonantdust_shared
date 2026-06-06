# wasm/

The shared Rust — the definition-language toolchain, the VM that runs it, the
content loader, the storage bridge, and the client bindings. It's called "wasm"
because this code **also** compiles to wasm for the pixijs client; it is *not*
client-only. The **gate links it as an rlib**; the **client gets it as a wasm
bundle**. Same logic, both sides — that single shared evaluation is the point of
the rewrite (it replaces the old JSON crate whose logic was mirrored in TS).

## Crates

- **`data/` — `resonantdust-data` (rlib).** The shared logic; no JS bindings, so
  the gate links it directly. Modules:
  - `parser.rs` — lexer + block-tree for the `.rd` DSL (`<bucket>` / `::def` /
    `:facet` / `@hook` headers; `$ & * : ^ #` reference sigils).
  - `validate.rs` — per-file checks: stack-neutrality, local labels, the op table.
  - `resolve.rs` — whole-corpus symbol table + `$`-ref resolution, and the
    `aspect.<name>` member lint.
  - `vm.rs` — the interpreter: data hooks, recipe `match_recipe` / `plan_recipe`
    (holds + effects), `^` system calls, the `Cell` store, and the `Catalog`
    (asset / manifest / aspect `@define` records).
  - `loader.rs` — `load(&[(name, source)]) -> Bundle`: parse all `.rd` → symbol
    table + catalog + functions + card/recipe defs indexed by **content-derived
    `def_id`** (sorted names; no `id.json`). Runs the corpus acceptance pass.
  - `bridge.rs` — storage bridge: `card_view` (a stored card → the VM's
    operating-set `Cell`, with the satisfies fold), `operating_set` (assemble a
    frame), `pack_stock`/`unpack_stock`.
  - `bits.rs` — `get_field`/`set_field` bit primitives.
- **root — `resonantdust-wasm` (cdylib).** Thin `wasm-bindgen` JSON wrappers over
  `data` for the browser — the `Content` handle (`load` / `cardView` /
  `matchRecipe` / `planRecipe`), gated on the `js` feature. The plain logic is
  feature-independent so `cargo test`/`check` exercise it natively.

## DSL spec

The language is specified in the content submodule, next to the content it
governs: [`content/data/SYNTAX.txt`](../content/data/SYNTAX.txt) (sigils + ops)
and [`content/data/CONVENTIONS.txt`](../content/data/CONVENTIONS.txt) (the
slot / aspect / chain model).

## Build & test (`bin/wasm`)

All dockerized on the `clockworklabs/spacetime` image (host `cargo` is not used):

| Command | Action |
| --- | --- |
| `bin/wasm test` | `cargo test --workspace` |
| `bin/wasm corpus` | load + validate + resolve every `content/data/*.rd` (the load-bearing cross-file lint) |
| `bin/wasm check` | `cargo check --workspace --all-targets` |
| `bin/wasm build` | the wasm bundle (`cargo build --target wasm32 --features js` + `wasm-bindgen`) → `pkg/` |

## Migration status

**Done + tested** — the whole pure data + wasm layer: parser → validate →
resolve → VM (incl. recipe match/plan, `^` system calls, the aspect satisfies
fold) → loader → bridge → wasm bindings.

**Live integration** (in the consuming codebases, not here):
- gate: `gather` (walk card rows → an `operating_set` frame, `slot.a.b`) +
  `apply` (a validated `Plan` → the coarse `apply_action` / `apply_action_tile`
  reducers — one transaction/commit per shard); the `^biome`/`^seed` system-call impls.
- client: load the wasm bundle, render via `cardView`, match locally via
  `matchRecipe`.
- modules: the data modules link `resonantdust-data` (the retired
  `resonantdust-content` crate is gone); `cards`+`regions` are now the unified
  [`shard`](../spacetime/server/modules/shard/AGENTS.md) module, `players` stays
  separate.

## Planned crate decomposition (by load-side)

`wasm/` is the home for *all* shared Rust, split so neither runtime loads what it
doesn't use (Cargo's dependency graph enforces it):

- **SHARED** (gate + client): `core` (today's `data`) · `biome` (terrain gen,
  the impl behind `^biome` — the client needs it too).
- **CLIENT-ONLY**: `locales` (strings) · `render` (texture/sprite helpers). Ported
  from the legacy `locales_core` / `texture_core`.
- **SERVER-ONLY**: `apply` (the pure `Plan` → mutation-descriptor half; the gate
  keeps the IO).

The wasm cdylib links shared + client crates; the gate links shared + server
crates. (Not yet carved — `data` is the seed of `core`.)
