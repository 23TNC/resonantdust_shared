// Pack/unpack helpers for the bit-packed columns on the cards table.
// Single source of truth — the spacetime shard / chat modules
// re-export this module verbatim via `pub use
// resonantdust_content::packed::*;`.
//
// Layouts:
//   valid_at         u64 = [time_ms: u48 | sequence: u16]                   (high | low)
//                          — see `pack_valid_at` below and `sequence.rs`
//                          for how the u16 disambiguator is allocated.
//   macro_zone       u64 = [owner_card_id: u32 | surface: u8 | zone_q: i12 | zone_r: i12]
//                          (high → low). The complete location key.
//                          `owner_card_id` (bits 32-63) is the card_id that
//                          owns the zone (`0` = WORLD; ids 0..1023 reserved).
//                          `surface` is bits 24-31. The payload is always two
//                          signed 12-bit coords (q at 12-23, r at 0-11), read
//                          as (q,r) or (x,y) per surface.
//   micro_location   u32 — TWO INTERPRETATIONS, gated by the `stack` field in
//     `flags` (bits 0-3); `stack == 0` is the loose sentinel:
//     stack != 0 → root card_id. The card is a stack member; its branch is
//                  `stack - 1`, its slot is the `index` field (flags bits 4-7).
//                  Flat chains — root, never parent.
//     stack == 0 → loose coords + offset:
//                   u32 = [local_q: u3 | local_r: u3 | x: i12 | y: i12 | rsvd: u2]
//                           local_q/local_r address a cell in the zone; x/y is
//                           the signed offset within that cell. The loose `kind`
//                           lives in the `index` field.
//   (micro_zone u8 was REMOVED — everything it held now lives in micro_location
//    + flags. The former `micro_is_card` discriminator bit is gone too,
//    subsumed by the `stack == 0` sentinel.)
//   packed_def       u16 = [card_type: u4 | def_id: u12]
//   zone_def         u8  = [card_type: u4 | 0: u4] (lower nibble reserved)
//   tile slot        u16 = [def_id: u12 | stock0: u2 | stock1: u2]
//                          packed 4-per-u64 across 16 u64s per zone — see
//                          docs/TILE_ASPECTS.md. (Per-card row stock lives in the
//                          card's own `stock` u8 column, not here.)
//
// Unified card model: every card is a card. A chain is one root (loose/snapped,
// `stack == 0`) plus N members (`stack != 0`, micro_location == root). The branch
// is `stack - 1`:
//   stack != 0 (stacked): branch 0 = hex, 1 = top, 2 = bottom, 3 = deferred.
//   stack == 0 (loose):   `micro_location` is loose coords + within-cell offset.
// The `index` field (flags bits 4-7) gives the slot within a branch (0..15) when
// stacked, and is UNUSED when loose. (There is no longer a loose "kind": every
// surface is a uniform hex cell, and snapped-vs-free is a pure render concern —
// a snapped card simply carries a zero `(x, y)` offset.) `stack` + `index` live
// in `flags` so they propagate per-row in lockstep with micro_location. The
// "server forcing this position" signal lives in `flags` as the `pos_need`
// (required) / `pos_want` (advisory) pair.

// The `branch` value is stored as `stack - 1` for stack members. The field
// masks/shifts are resolved through `card_model`'s layout (server + client share
// it); packed.rs only owns the `micro_location` bit layout.

/// Stack `branch` value for the deferred branch (only meaningful for a stack
/// member). Resolution runs at mirror time on the client; server-side the
/// deferred follower index keeps `(surface, macro_zone)` in lockstep with the
/// host and clears the anchor on host death.
pub const STACK_STATE_DEFERRED: u8 = 3;

// ---- surface bands ------------------------------------------------------
//
// `Card.surface` and `Zone.surface` are u8, but the values are
// banded into ranges with different semantics. Every band is a
// "container kind" — the `(surface, macro_zone)` tuple identifies
// which container a row belongs to. `macro_zone` means different
// things in different bands:
//
// - INVENTORY_LAYER (1):           `macro_zone` = the owning soul's
//                                  `card_id`. The player's hand /
//                                  bag / inventory grid.
// - PLAYER_INVENTORY_LAYER (2):    `macro_zone` = the owning
//                                  `player_id`. Player-scoped
//                                  inventory shared across that
//                                  player's souls — for permanent
//                                  account-level items, the
//                                  player-wide counterpart of
//                                  per-soul inventory. Same
//                                  bucket convention as soul
//                                  inventory; the only difference
//                                  is what `macro_zone` IS.
// - POCKET_DIMENSION_LAYER (32):   `macro_zone` = the anchor card's
//                                  `card_id`. A private interior
//                                  carried by an anchor card.
// - MINI_ZONE_LAYER (63):          RESERVED. mini_zone functionality was
//                                  stripped (no mini_zones exist yet); the
//                                  band number is held so re-implementation
//                                  doesn't renumber the surface map. When it
//                                  returns it sits just below WORLD_LAYER so
//                                  stack-layout rules apply and world-only
//                                  queries skip its contents.
// - WORLD_LAYER (64) and above:    `macro_zone` = packed
//                                  `(chunkQ:i16, chunkR:i16)`. The
//                                  shared world hex grid.
//
// The split at `< WORLD_LAYER` is what existing code keys "stack
// layout" rules and inventory-like behavior off. The split at
// `>= WORLD_LAYER` is what world-vs-personal queries key off.
pub const INVENTORY_LAYER: u8 = 1;
pub const PLAYER_INVENTORY_LAYER: u8 = 2;
pub const POCKET_DIMENSION_LAYER: u8 = 32;
/// RESERVED band for a future mini_zone (functionality stripped). Kept so the
/// surface map isn't renumbered; currently referenced only by packing tests.
pub const MINI_ZONE_LAYER: u8 = 63;
pub const WORLD_LAYER: u8 = 64;

// ---- valid_at ----------------------------------------------------------
//
// PK layout: `(time_ms_u48 << 16) | sequence_u16`.
//
// - High 48 bits: milliseconds since Unix epoch. u48 ms ≈ 8920 years
//   of runway from epoch (covers our lifetime trivially). PK ordering
//   is chronological — a btree scan walks rows in time order, useful
//   for any range queries that need it (though most callers go via
//   `card_id` btree index, where the explicit `max_by_key` ordering
//   is what's load-bearing).
// - Low 16 bits: global sequence number from `sequence::next_sequence`,
//   refreshed per write. Disambiguates two writes that share a
//   millisecond — within one module, even thousands of same-ms writes
//   sit far below the 65k wrap budget. Cross-shard collisions don't
//   exist because shards' PK spaces don't overlap (each shard owns
//   its own rows entirely).

pub fn pack_valid_at(time_ms: u64, sequence: u16) -> u64 {
    (time_ms << 16) | (sequence as u64)
}

pub fn valid_at_time(v: u64) -> u64 {
    v >> 16
}

// ---- card_id -----------------------------------------------------------
//
// A `card_id` is a u32 partitioned `[ db:1 (31) | shard:11 (30-20) | local:20 (19-0) ]`.
// The top bit names the DATABASE the card lives in: `0` = `cards` (owner-sharded
// real cards), `1` = `regions` (position-sharded tile-cards). The next 11 bits
// name the shard WITHIN that database (0..2047); the low 20 bits are a per-shard
// local counter (0..1_048_575, locals 0..1023 reserved — see `cards::FIRST_CARD_ID`).
// The gateway routes a card to its database + shard purely from the id, no index.
// (Adding a third database later — e.g. souls — takes another bit, halving the
// shard counts; we won't approach 2048 shards in practice.)

/// Width of the per-shard local-id field within a `card_id`.
pub const CARD_LOCAL_BITS: u32 = 20;
/// Mask selecting the local-id field (low 20 bits).
pub const CARD_LOCAL_MASK: u32 = (1 << CARD_LOCAL_BITS) - 1;
/// Bit position of the database selector (top bit of the former 12-bit shard field).
pub const CARD_DB_SHIFT: u32 = 31;
/// Largest shard id WITHIN a database (the 11-bit shard field): 2047.
pub const CARD_SHARD_MAX: u16 = ((1u32 << (CARD_DB_SHIFT - CARD_LOCAL_BITS)) - 1) as u16;

/// Database selector: the owner-sharded `cards` database (real cards).
pub const CARD_DB_CARDS: u8 = 0;
/// Database selector: the position-sharded `regions` database (tile-cards).
pub const CARD_DB_REGIONS: u8 = 1;

/// Which database holds `card_id` — [`CARD_DB_CARDS`] (0) or [`CARD_DB_REGIONS`] (1).
pub fn card_db_of(card_id: u32) -> u8 {
    ((card_id >> CARD_DB_SHIFT) & 1) as u8
}

/// The shard id of `card_id` WITHIN its database (the 11-bit shard field, 0..2047).
pub fn card_shard_within_db(card_id: u32) -> u16 {
    ((card_id >> CARD_LOCAL_BITS) & (CARD_SHARD_MAX as u32)) as u16
}

/// The full 12-bit shard field (db bit + shard): `0..2047` = cards, `2048..4095`
/// = regions. Routing uses [`card_db_of`] + [`card_shard_within_db`]; this is the
/// raw field, kept for reference and stable ordering.
pub fn card_shard_of(card_id: u32) -> u16 {
    (card_id >> CARD_LOCAL_BITS) as u16
}

/// The per-shard local id of `card_id` (low 20 bits).
pub fn card_local_of(card_id: u32) -> u32 {
    card_id & CARD_LOCAL_MASK
}

/// Build a `card_id` from a database selector (`CARD_DB_CARDS`/`CARD_DB_REGIONS`),
/// a per-database shard id (0..2047), and a per-shard local id. Fields are masked.
pub fn pack_card_id(db: u8, shard: u16, local: u32) -> u32 {
    (((db as u32) & 1) << CARD_DB_SHIFT)
        | (((shard as u32) & (CARD_SHARD_MAX as u32)) << CARD_LOCAL_BITS)
        | (local & CARD_LOCAL_MASK)
}

// ---- macro_zone --------------------------------------------------------
//
// `macro_zone` is the complete, uniform location key:
//
//   [ owner_card_id:u32 (63-32) | surface:u8 (31-24) | zone_q:i12 | zone_r:i12 ]
//
// The complete location key. `owner_card_id` (bits 32-63) is the card_id that
// owns the zone — `0` is the WORLD sentinel (card ids 0..1023 are reserved;
// see `cards::FIRST_CARD_ID`). `surface` is bits 24-31. The payload is always
// two signed 12-bit coordinates (`zone_q` at bits 12-23, `zone_r` at 0-11) —
// read as `(q, r)` or `(x, y)` per surface; `(0, 0)` for single-chunk surfaces
// like inventory, whose item positions live in `micro_zone` / `micro_location`.

/// Sign-extend the 12-bit field at `shift` of `v` into an `i16`.
fn macro_coord(v: u64, shift: u32) -> i16 {
    let s = ((v >> shift) & 0xFFF) as i16;
    if s & 0x800 != 0 { s - 0x1000 } else { s }
}

/// Pack the **world payload** (chunk coords) into bits 0-23. Combine with
/// [`with_surface`] to produce a full `macro_zone`.
pub fn pack_macro_zone(q: i16, r: i16) -> u64 {
    (((q as u16 as u64) & 0xFFF) << 12) | ((r as u16 as u64) & 0xFFF)
}

pub fn unpack_macro_zone(v: u64) -> (i16, i16) {
    (macro_coord(v, 12), macro_coord(v, 0))
}

/// Bit offset of the `surface` byte within `macro_zone`.
pub const MACRO_SURFACE_SHIFT: u32 = 24;

/// Read the `surface` band (bits 24-31) out of a `macro_zone`.
pub fn surface_of(v: u64) -> u8 {
    ((v >> MACRO_SURFACE_SHIFT) & 0xFF) as u8
}

/// Set the `surface` band (bits 24-31), preserving the payload (bits 0-23)
/// and the reserved u32 (bits 32-63). The single combiner for building a
/// `macro_zone`: `with_surface(pack_macro_zone(q, r), WORLD_LAYER)` for world,
/// `with_surface(id as u64, INVENTORY_LAYER)` for a container.
pub fn with_surface(v: u64, surface: u8) -> u64 {
    (v & !(0xFFu64 << MACRO_SURFACE_SHIFT)) | ((surface as u64) << MACRO_SURFACE_SHIFT)
}

/// Read the 24-bit coordinate payload (bits 0-23) — the raw packed `(q, r)`.
pub fn macro_payload(v: u64) -> u32 {
    (v & 0x00FF_FFFF) as u32
}

/// Bit offset of the `owner_card_id` field within `macro_zone`.
pub const MACRO_OWNER_SHIFT: u32 = 32;

/// Read the `owner_card_id` band (bits 32-63) — the card_id that owns the
/// zone; `0` is the WORLD sentinel.
pub fn owner_of(v: u64) -> u32 {
    (v >> MACRO_OWNER_SHIFT) as u32
}

/// Set the `owner_card_id` band (bits 32-63), preserving the low 32 bits
/// (surface + coords).
pub fn with_owner(v: u64, owner: u32) -> u64 {
    (v & 0x0000_0000_FFFF_FFFF) | ((owner as u64) << MACRO_OWNER_SHIFT)
}

/// Build a complete `macro_zone` from its fields. The single server-side
/// combiner: `owner_card_id` in bits 32-63, `surface` in 24-31, and the two
/// signed 12-bit coords in the low 24. World uses `owner = 0`; container
/// surfaces (inventory / pocket) pass the soul / anchor card_id as
/// the owner with `(q, r) = (0, 0)`.
pub fn pack_macro_zone_full(owner: u32, surface: u8, q: i16, r: i16) -> u64 {
    with_owner(with_surface(pack_macro_zone(q, r), surface), owner)
}

// ---- macro_region ------------------------------------------------------
//
// A `Region` covers a `REGION_SIZE × REGION_SIZE` block of zones (8×8 = 64),
// tracking per-zone spawn presence/availability as two u64 bitfields. Its
// `macro_region` key is bit-identical to `macro_zone` — same `[card_id:u32 |
// surface:u8 | region_q:i12 | region_r:i12]` layout — only the coordinate
// scale differs (region units, where 1 region = 8 zones). So it reuses the
// `macro_zone` combiners verbatim.

/// Zones per region edge (region is `REGION_SIZE × REGION_SIZE` zones).
pub const REGION_SIZE: i16 = 8;

/// Build a `macro_region` from its fields. Same layout as `macro_zone`;
/// `(region_q, region_r)` are in region units. World uses `card_id = 0`.
pub fn pack_macro_region(card_id: u32, surface: u8, region_q: i16, region_r: i16) -> u64 {
    pack_macro_zone_full(card_id, surface, region_q, region_r)
}

/// Map a `macro_zone` to its containing `(macro_region, bit)`, where `bit`
/// (`0..63`) indexes the zone's slot inside the region's 64-bit
/// presence/availability fields. Bit layout is row-major over the region's
/// 8×8 zones: `bit = local_r * REGION_SIZE + local_q`, so bit `i` ↔ the zone
/// at `(region_q*8 + i % 8, region_r*8 + i / 8)`. `card_id` and `surface` are
/// carried through unchanged, so a non-world zone maps to its own owner's /
/// surface's region.
pub fn region_of_zone(macro_zone: u64) -> (u64, u8) {
    let (zq, zr) = unpack_macro_zone(macro_zone);
    let card_id = owner_of(macro_zone);
    let surface = surface_of(macro_zone);
    // Euclidean div/rem so negative coords floor toward -inf and locals stay 0..7.
    let region_q = zq.div_euclid(REGION_SIZE);
    let region_r = zr.div_euclid(REGION_SIZE);
    let local_q = zq.rem_euclid(REGION_SIZE);
    let local_r = zr.rem_euclid(REGION_SIZE);
    let bit = (local_r * REGION_SIZE + local_q) as u8;
    (pack_macro_region(card_id, surface, region_q, region_r), bit)
}

// ---- stack branch directions -------------------------------------------
//
// `branch` values for a stack member (stored as `stack - 1`). They match the
// recipe-grammar **branch number** convention: a card placed in `slot.<N>.<index>`
// of its chain root carries `branch == N` (and is stored with `stack == N + 1`).
//
// - `0` = `STACK_DIR_HEX` — the tile branch (visually beneath root; `slot.0.X`).
// - `1` = `STACK_DIR_UP` — top branch (`slot.1.X`).
// - `2` = `STACK_DIR_DOWN` — bottom branch (`slot.2.X`).
//
// The branches are structurally identical (flat root + index chains). The value
// only tells renderers which side of root to attach the chain to. Branch value 3
// is `STACK_STATE_DEFERRED`.
pub const STACK_DIR_HEX: u8 = 0;
pub const STACK_DIR_UP: u8 = 1;
pub const STACK_DIR_DOWN: u8 = 2;

// ---- micro_location ----------------------------------------------------
//
// When `stack == 0` (loose), `micro_location` is a within-cell offset + cell:
//   [ x:i12 (20-31) | y:i12 (8-19) | local_q:u3 (5-7) | local_r:u3 (2-4) | rsvd:u2 (0-1) ]
// The low byte is `[local_q:u3 | local_r:u3 | rsvd:u2]` — the cell address inside
// the zone; the upper 24 bits are the signed within-cell `(x, y)` offset. A
// snapped (centered) card just carries a zero offset. When `stack != 0` (stack
// member), `micro_location` is the root card_id (identity).

const MICRO_LOOSE_X_SHIFT: u32 = 20;
const MICRO_LOOSE_Y_SHIFT: u32 = 8;
const MICRO_LOOSE_LQ_SHIFT: u32 = 5;
const MICRO_LOOSE_LR_SHIFT: u32 = 2;

/// Sign-extend the 12-bit field at `shift` of `v` (a u32) into an `i16`.
fn micro_coord(v: u32, shift: u32) -> i16 {
    let s = ((v >> shift) & 0xFFF) as i16;
    if s & 0x800 != 0 { s - 0x1000 } else { s }
}

/// Pack loose coords + within-cell offset. `local_q`/`local_r` are the 0..7
/// cell address inside the zone; `x`/`y` are the signed offset within the cell
/// (±2047). Used when `micro_is_card` is clear.
pub fn pack_micro_loose(local_q: u8, local_r: u8, x: i16, y: i16) -> u32 {
    (((local_q & 0b111) as u32) << MICRO_LOOSE_LQ_SHIFT)
        | (((local_r & 0b111) as u32) << MICRO_LOOSE_LR_SHIFT)
        | (((x as u16 as u32) & 0xFFF) << MICRO_LOOSE_X_SHIFT)
        | (((y as u16 as u32) & 0xFFF) << MICRO_LOOSE_Y_SHIFT)
}

/// Inverse of [`pack_micro_loose`]. Returns `(local_q, local_r, x, y)`.
pub fn unpack_micro_loose(v: u32) -> (u8, u8, i16, i16) {
    (
        ((v >> MICRO_LOOSE_LQ_SHIFT) & 0b111) as u8,
        ((v >> MICRO_LOOSE_LR_SHIFT) & 0b111) as u8,
        micro_coord(v, MICRO_LOOSE_X_SHIFT),
        micro_coord(v, MICRO_LOOSE_Y_SHIFT),
    )
}

/// Read just the cell address `(local_q, local_r)` from a loose `micro_location`.
pub fn micro_loose_cell(v: u32) -> (u8, u8) {
    (
        ((v >> MICRO_LOOSE_LQ_SHIFT) & 0b111) as u8,
        ((v >> MICRO_LOOSE_LR_SHIFT) & 0b111) as u8,
    )
}

/// Build a snapped (centered) loose `micro_location` — cell with zero offset.
pub fn pack_micro_snap(local_q: u8, local_r: u8) -> u32 {
    pack_micro_loose(local_q, local_r, 0, 0)
}

/// `micro_location` as a root card_id (identity; used when `micro_is_card` set).
pub fn pack_micro_location_card_id(card_id: u32) -> u32 {
    card_id
}

pub fn unpack_micro_location_card_id(v: u32) -> u32 {
    v
}

// ---- packed_definition -------------------------------------------------
//
// u16 layout: `[ card_type: u4 | def_id: u12 ]`. The `card_category`
// dimension was retired (see
// docs/CATEGORY_RETIRE_AND_TILE_EXPAND.md) — `category` had never
// been populated outside of the single `default = 0` value, so its
// 4-bit slot collapsed into `def_id` to give 4095 distinct defs per
// type. Subscription mask `packed_definition < 0x4000` (public types
// 0..=3) still works because the top 4 bits are still `card_type`.

/// Max `def_id` value that fits in `packed_definition`'s low 12 bits.
pub const DEF_ID_MAX: u16 = 0x0FFF;

/// Bit mask isolating the `def_id` field of a `packed_definition`.
pub const DEF_ID_MASK: u16 = 0x0FFF;

/// Bit mask isolating the `card_type` field of a `packed_definition`.
pub const CARD_TYPE_MASK: u16 = 0xF000;

pub fn pack_definition(card_type: u8, def_id: u16) -> u16 {
    (((card_type & 0xF) as u16) << 12) | (def_id & DEF_ID_MASK)
}

pub fn unpack_definition(v: u16) -> (u8, u16) {
    (((v >> 12) & 0xF) as u8, v & DEF_ID_MASK)
}

/// `card_type` nibble for souls (the high 4 bits of `packed_definition`). All
/// souls — including the player_soul — live in this type, so `packed_definition
/// >= SOUL_CARD_TYPE << 12` (`>= 0xF000`) is "any soul".
pub const SOUL_CARD_TYPE: u8 = 0xF;

/// Lowest `packed_definition` reserved for **player-soul** cards: the top 16
/// def_ids of the soul type (`0xFFF0..=0xFFFF`). A card in this range IS a
/// player_soul — the owner-walk terminus, the player's in-world representative —
/// identified by **definition alone** (no flag), so it's queryable with a plain
/// `packed_definition >= PLAYER_SOUL_PACKED_MIN`. The canonical player_soul is
/// [`PLAYER_SOUL_PACKED`] (`0xFFFF`); the other 15 slots are reserved for future
/// player-soul variants. Pinned in content via an explicit DSL `def_id`.
pub const PLAYER_SOUL_PACKED_MIN: u16 = 0xFFF0;

/// The canonical player_soul `packed_definition` (`0xFFFF` — soul type, top def).
pub const PLAYER_SOUL_PACKED: u16 = 0xFFFF;

/// Is this packed definition a player-soul card (the reserved top-16 range)?
/// Replaces the old `player_owned` flag as the player-soul boundary marker.
pub fn is_player_soul(packed_definition: u16) -> bool {
    packed_definition >= PLAYER_SOUL_PACKED_MIN
}

// ---- zone_definition (u8 = u4 card_type | u4 0) -----------------------
//
// Lower nibble is reserved (formerly `card_category`). Kept u8 for
// schema stability rather than narrowing the `Zone.packed_definition`
// column to u4 — the byte is on the wire either way.

pub fn pack_zone_definition(card_type: u8) -> u8 {
    (card_type & 0xF) << 4
}

pub fn unpack_zone_definition(v: u8) -> u8 {
    (v >> 4) & 0xF
}

// ---- nibble-pair (u8 = [count: u4 | max: u4]) ------------------------
//
// Compact pair-of-counts encoding. Used by `PlayerProfile` for
// `blueprint_info` (current placed blueprint count vs cap) and
// `soul_info` (current soul-card count vs cap). Both nibbles are
// in 0..=15; values above 15 saturate when packing.
//
// Layout: `count` in the high nibble, `max` in the low nibble. The
// "current" count goes on top so a `(0, 5)` initial state reads as
// `0x05` rather than `0x50` — easier to eyeball in a hex dump.

/// Pack `(count, max)` into a `u8`. Values above 15 saturate to 15.
pub fn pack_nibbles(count: u8, max: u8) -> u8 {
    let c = count.min(0xF);
    let m = max.min(0xF);
    (c << 4) | m
}

/// Inverse of [`pack_nibbles`]. Returns `(count, max)`.
pub fn unpack_nibbles(v: u8) -> (u8, u8) {
    ((v >> 4) & 0xF, v & 0xF)
}

/// Read just the `count` (high) nibble.
pub fn nibble_count(v: u8) -> u8 {
    (v >> 4) & 0xF
}

/// Read just the `max` (low) nibble.
pub fn nibble_max(v: u8) -> u8 {
    v & 0xF
}

/// Replace the `count` nibble, preserving the `max` nibble.
/// Values above 15 saturate.
pub fn with_nibble_count(v: u8, count: u8) -> u8 {
    pack_nibbles(count, nibble_max(v))
}

/// Replace the `max` nibble, preserving the `count` nibble.
/// Values above 15 saturate.
pub fn with_nibble_max(v: u8, max: u8) -> u8 {
    pack_nibbles(nibble_count(v), max)
}

// ---- recipe (u16 = u3 type | u3 category | u10 id) -------------------

pub const RECIPE_TYPE_OR_CATEGORY_MASK: u16 = 0x7;
pub const RECIPE_ID_MASK: u16 = 0x3FF;

pub fn pack_recipe(recipe_type: u8, recipe_category: u8, recipe_id: u16) -> u16 {
    (((recipe_type as u16) & RECIPE_TYPE_OR_CATEGORY_MASK) << 13)
        | (((recipe_category as u16) & RECIPE_TYPE_OR_CATEGORY_MASK) << 10)
        | (recipe_id & RECIPE_ID_MASK)
}

pub fn unpack_recipe(v: u16) -> (u8, u8, u16) {
    (
        ((v >> 13) & RECIPE_TYPE_OR_CATEGORY_MASK) as u8,
        ((v >> 10) & RECIPE_TYPE_OR_CATEGORY_MASK) as u8,
        v & RECIPE_ID_MASK,
    )
}

// ---- zone tile storage (16 u64 holding 64 u16 tile slots) ------------
//
// Each zone has 64 tiles (8 × 8 grid). Each tile slot is u16 wide:
//
//     [ def_id:u12 | stock0:u2 | stock1:u2 ]
//
//   - `def_id` (low 12 bits): the tile's `CardDefinition` packed_id
//     payload — same value the per-card `packed_definition` carries.
//   - `stock0` / `stock1` (bits 12-13, 14-15): u2 values for the
//     def's declared row-mutable aspect slots (see
//     `CardDefinition.stock`). The def maps slot index → aspect.
//
// 64 tiles × 16 bits = 1024 bits = exactly 16 u64. 8 tiles per row =
// 128 bits = 2 u64 per row, no boundary straddling — unlike the u12
// layout this replaces. See docs/TILE_ASPECTS.md.

/// Number of u64 fields in the zone tile-data packing.
pub const ZONE_TILE_U64_COUNT: usize = 16;

/// Number of tiles per zone (8 × 8 grid).
pub const ZONE_TILE_COUNT: usize = 64;

/// Bit width of a single tile slot (def_id + two stocks).
pub const ZONE_TILE_BITS: usize = 16;

/// Number of stock slots per tile. Matches `MAX_STOCK_SLOTS` on the
/// def side. Slot 0 lives at bits 12-13, slot 1 at bits 14-15.
pub const ZONE_TILE_STOCK_SLOTS: usize = 2;

/// Max value a stock slot can store (u2).
pub const ZONE_TILE_STOCK_MAX: u8 = 0x3;

/// Bit mask isolating one tile's u16 within its u64 (after shifting
/// to the tile's bit offset).
const TILE_MASK: u64 = 0xFFFF;

/// Read tile `idx` (0..64) — returns `(def_id, stock0, stock1)`.
pub fn tile_full(packed: &[u64; ZONE_TILE_U64_COUNT], idx: usize) -> (u16, u8, u8) {
    debug_assert!(idx < ZONE_TILE_COUNT, "tile index {} out of range", idx);
    let u64_idx = idx / 4; // 4 tiles per u64
    let bit_offset = (idx % 4) * 16;
    let slot = (packed[u64_idx] >> bit_offset) & TILE_MASK;
    let def_id = (slot & 0x0FFF) as u16;
    let stock0 = ((slot >> 12) & 0x3) as u8;
    let stock1 = ((slot >> 14) & 0x3) as u8;
    (def_id, stock0, stock1)
}

/// Read just the def_id (low 12 bits) of tile `idx`.
pub fn tile_def_id(packed: &[u64; ZONE_TILE_U64_COUNT], idx: usize) -> u16 {
    debug_assert!(idx < ZONE_TILE_COUNT, "tile index {} out of range", idx);
    let u64_idx = idx / 4;
    let bit_offset = (idx % 4) * 16;
    ((packed[u64_idx] >> bit_offset) & 0x0FFF) as u16
}

/// Read tile `idx`'s stock slot `slot` (0 or 1). Returns 0..=3.
pub fn tile_stock(packed: &[u64; ZONE_TILE_U64_COUNT], idx: usize, slot: usize) -> u8 {
    debug_assert!(idx < ZONE_TILE_COUNT, "tile index {} out of range", idx);
    debug_assert!(slot < ZONE_TILE_STOCK_SLOTS, "stock slot {} out of range", slot);
    let u64_idx = idx / 4;
    let bit_offset = (idx % 4) * 16 + 12 + (slot * 2);
    ((packed[u64_idx] >> bit_offset) & 0x3) as u8
}

/// Write tile `idx`'s full u16 slot. Each field is masked to its
/// declared width — excess bits are silently dropped, not panicked
/// on.
pub fn set_tile_full(
    packed: &mut [u64; ZONE_TILE_U64_COUNT],
    idx: usize,
    def_id: u16,
    stock0: u8,
    stock1: u8,
) {
    debug_assert!(idx < ZONE_TILE_COUNT, "tile index {} out of range", idx);
    let u64_idx = idx / 4;
    let bit_offset = (idx % 4) * 16;
    let mask = TILE_MASK << bit_offset;
    let value = (def_id as u64 & 0x0FFF)
        | ((stock0 as u64 & 0x3) << 12)
        | ((stock1 as u64 & 0x3) << 14);
    packed[u64_idx] = (packed[u64_idx] & !mask) | (value << bit_offset);
}

/// Write a single stock slot on tile `idx`. Other bits in the u16
/// slot (the def_id and the other stock) are left untouched.
pub fn set_tile_stock(
    packed: &mut [u64; ZONE_TILE_U64_COUNT],
    idx: usize,
    slot: usize,
    value: u8,
) {
    debug_assert!(idx < ZONE_TILE_COUNT, "tile index {} out of range", idx);
    debug_assert!(slot < ZONE_TILE_STOCK_SLOTS, "stock slot {} out of range", slot);
    let u64_idx = idx / 4;
    let bit_offset = (idx % 4) * 16 + 12 + (slot * 2);
    let mask = 0x3u64 << bit_offset;
    let v = (value as u64 & 0x3) << bit_offset;
    packed[u64_idx] = (packed[u64_idx] & !mask) | v;
}

/// Decode one row of 8 tiles. Returns `(def_id, stock0, stock1)` per
/// column, row-major (row 0 = tile indices 0..=7, row 1 = 8..=15,
/// etc.).
pub fn tile_row(packed: &[u64; ZONE_TILE_U64_COUNT], row: usize) -> [(u16, u8, u8); 8] {
    let mut out = [(0u16, 0u8, 0u8); 8];
    let base = row * 8;
    for col in 0..8 {
        out[col] = tile_full(packed, base + col);
    }
    out
}

pub fn set_tile_row(
    packed: &mut [u64; ZONE_TILE_U64_COUNT],
    row: usize,
    slots: &[(u16, u8, u8); 8],
) {
    debug_assert!(row < 8, "row {} out of range", row);
    let base = row * 8;
    for (col, (def_id, stock0, stock1)) in slots.iter().enumerate() {
        set_tile_full(packed, base + col, *def_id, *stock0, *stock1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_at_roundtrip() {
        let v = pack_valid_at(0x0000_DEAD_BEEF_1234, 0x5678);
        assert_eq!(valid_at_time(v), 0x0000_DEAD_BEEF_1234);
        assert_eq!(v & 0xFFFF, 0x5678);
    }

    #[test]
    fn macro_zone_signed_roundtrip() {
        let v = pack_macro_zone(-1, 1);
        assert_eq!(unpack_macro_zone(v), (-1, 1));
        // i12 extremes: -2048 (0x800) .. 2047 (0x7FF).
        let v = pack_macro_zone(-2048, 2047);
        assert_eq!(unpack_macro_zone(v), (-2048, 2047));
        let v = pack_macro_zone(2047, -2048);
        assert_eq!(unpack_macro_zone(v), (2047, -2048));
        // The payload occupies only bits 0-23 — surface (24-31) and the
        // reserved u32 (32-63) stay zero before folding, even at the extremes.
        assert_eq!(pack_macro_zone(-2048, -2048) >> 24, 0);
        assert_eq!(pack_macro_zone(2047, 2047) >> 24, 0);
    }

    #[test]
    fn macro_zone_surface_roundtrip() {
        // World: fold surface over the coord payload; both round-trip and
        // the reserved u32 band stays zero.
        let v = with_surface(pack_macro_zone(-2048, 2047), WORLD_LAYER);
        assert_eq!(surface_of(v), WORLD_LAYER);
        assert_eq!(unpack_macro_zone(v), (-2048, 2047));
        assert_eq!(v >> 32, 0);
        // Non-world: a container id in the 24-bit payload + a surface band.
        let v = with_surface(0x00AB_CDEF, MINI_ZONE_LAYER);
        assert_eq!(surface_of(v), MINI_ZONE_LAYER);
        assert_eq!(macro_payload(v), 0x00AB_CDEF);
        assert_eq!(v >> 32, 0);
        // with_surface replaces the band without touching the payload.
        let v = with_surface(with_surface(0x00123456, INVENTORY_LAYER), WORLD_LAYER);
        assert_eq!(surface_of(v), WORLD_LAYER);
        assert_eq!(macro_payload(v), 0x00123456);
    }

    #[test]
    fn macro_zone_owner_roundtrip() {
        // Full address: owner card_id + surface + signed coords. Exercise a
        // high owner id (top bit of the u32 set) and the coord extremes.
        let owner: u32 = 0xC000_0001;
        let v = pack_macro_zone_full(owner, INVENTORY_LAYER, 0, 0);
        assert_eq!(owner_of(v), owner);
        assert_eq!(surface_of(v), INVENTORY_LAYER);
        assert_eq!(unpack_macro_zone(v), (0, 0));

        let v = pack_macro_zone_full(0, WORLD_LAYER, -2048, 2047);
        assert_eq!(owner_of(v), 0); // WORLD sentinel
        assert_eq!(surface_of(v), WORLD_LAYER);
        assert_eq!(unpack_macro_zone(v), (-2048, 2047));

        // with_owner replaces only the owner band, preserving surface + coords.
        let base = pack_macro_zone_full(1024, MINI_ZONE_LAYER, 5, -3);
        let re = with_owner(base, 2048);
        assert_eq!(owner_of(re), 2048);
        assert_eq!(surface_of(re), MINI_ZONE_LAYER);
        assert_eq!(unpack_macro_zone(re), (5, -3));
    }

    #[test]
    fn region_of_zone_origin_block() {
        // The origin 8×8 block (world chunks q,r ∈ 0..7) all maps to region
        // (0,0); corners hit bit 0 and bit 63.
        let region00 = pack_macro_region(0, WORLD_LAYER, 0, 0);
        let (mr, bit) = region_of_zone(pack_macro_zone_full(0, WORLD_LAYER, 0, 0));
        assert_eq!(mr, region00);
        assert_eq!(bit, 0);
        let (mr, bit) = region_of_zone(pack_macro_zone_full(0, WORLD_LAYER, 7, 7));
        assert_eq!(mr, region00);
        assert_eq!(bit, 63);
        // Row-major: (local_q=3, local_r=1) → bit 1*8 + 3 = 11.
        let (mr, bit) = region_of_zone(pack_macro_zone_full(0, WORLD_LAYER, 3, 1));
        assert_eq!(mr, region00);
        assert_eq!(bit, 11);
    }

    #[test]
    fn region_of_zone_boundary_and_negative() {
        // zq=8 crosses into region (1,0), local bit 0.
        let (mr, bit) = region_of_zone(pack_macro_zone_full(0, WORLD_LAYER, 8, 0));
        assert_eq!(mr, pack_macro_region(0, WORLD_LAYER, 1, 0));
        assert_eq!(bit, 0);
        // Negative coords floor toward -inf: zq=-1 → region (-1), local 7.
        let (mr, bit) = region_of_zone(pack_macro_zone_full(0, WORLD_LAYER, -1, -1));
        assert_eq!(mr, pack_macro_region(0, WORLD_LAYER, -1, -1));
        assert_eq!(bit, 7 * REGION_SIZE as u8 + 7); // (local_q=7, local_r=7) → 63
        // card_id + surface carry through to the region key.
        let (mr, _) = region_of_zone(pack_macro_zone_full(1024, MINI_ZONE_LAYER, 9, 2));
        assert_eq!(mr, pack_macro_region(1024, MINI_ZONE_LAYER, 1, 0));
    }

    #[test]
    fn micro_loose_roundtrip() {
        // Full field exercise: cell 0..7 each, signed offsets.
        for lq in 0u8..8 {
            for lr in 0u8..8 {
                let v = pack_micro_loose(lq, lr, -100, 200);
                assert_eq!(unpack_micro_loose(v), (lq, lr, -100, 200));
                assert_eq!(micro_loose_cell(v), (lq, lr));
            }
        }
        // Signed-offset extremes (i12 range ±2047).
        let v = pack_micro_loose(5, 3, 2047, -2048);
        assert_eq!(unpack_micro_loose(v), (5, 3, 2047, -2048));
        // Snap = zero offset.
        let v = pack_micro_snap(6, 1);
        assert_eq!(unpack_micro_loose(v), (6, 1, 0, 0));
    }

    #[test]
    fn micro_location_card_id_identity() {
        assert_eq!(unpack_micro_location_card_id(pack_micro_location_card_id(4242)), 4242);
    }

    #[test]
    fn definition_roundtrip() {
        // type=0xA, def_id=0xABC.
        let v = pack_definition(0xA, 0xABC);
        assert_eq!(unpack_definition(v), (0xA, 0xABC));
        // Saturate def_id at the u12 max.
        let v = pack_definition(0x7, 0xFFF);
        assert_eq!(unpack_definition(v), (0x7, 0xFFF));
        // Excess bits in def_id are masked off, not panicked on.
        let v = pack_definition(0x3, 0x1234);
        assert_eq!(unpack_definition(v), (0x3, 0x234));
        // Public-type subscription mask: type < 4 ⇔ packed < 0x4000.
        assert!(pack_definition(0x3, 0xFFF) < 0x4000);
        assert!(pack_definition(0x4, 0x000) >= 0x4000);
    }

    #[test]
    fn recipe_roundtrip() {
        let v = pack_recipe(0b101, 0b011, 0x2F4);
        assert_eq!(unpack_recipe(v), (0b101, 0b011, 0x2F4));
        // Saturate each field at its mask.
        let v = pack_recipe(0x7, 0x7, RECIPE_ID_MASK);
        assert_eq!(unpack_recipe(v), (0x7, 0x7, RECIPE_ID_MASK));
        // Overflow bits in inputs should be masked off cleanly.
        let v = pack_recipe(0xFF, 0xFF, 0xFFFF);
        assert_eq!(unpack_recipe(v), (0x7, 0x7, RECIPE_ID_MASK));
    }

    #[test]
    fn zone_definition_roundtrip() {
        let v = pack_zone_definition(0xC);
        assert_eq!(unpack_zone_definition(v), 0xC);
        // Lower nibble unused after the category retire — always 0.
        assert_eq!(v & 0xF, 0x0);
    }

    #[test]
    fn tile_full_roundtrip() {
        let mut packed = [0u64; ZONE_TILE_U64_COUNT];
        // Read-empty: every slot zero.
        for i in 0..ZONE_TILE_COUNT {
            assert_eq!(tile_full(&packed, i), (0, 0, 0));
        }
        // Write each tile to a distinct (def_id, stock0, stock1) and
        // read it back. Defs use the full u12; stocks cycle through
        // 0..=3 so we exercise all u2 values.
        for i in 0..ZONE_TILE_COUNT {
            set_tile_full(
                &mut packed,
                i,
                (i + 1) as u16,
                (i % 4) as u8,
                ((i + 1) % 4) as u8,
            );
        }
        for i in 0..ZONE_TILE_COUNT {
            assert_eq!(
                tile_full(&packed, i),
                ((i + 1) as u16, (i % 4) as u8, ((i + 1) % 4) as u8),
            );
        }
    }

    #[test]
    fn tile_field_masking() {
        // Each field is masked to its declared width; excess bits
        // are silently dropped, not panicked on. Confirms the high
        // bits don't bleed into neighbour fields.
        let mut packed = [0u64; ZONE_TILE_U64_COUNT];
        // def_id u12 — 0xFFFF gets masked to 0xFFF.
        set_tile_full(&mut packed, 0, 0xFFFF, 0, 0);
        assert_eq!(tile_full(&packed, 0), (0xFFF, 0, 0));
        // stock u2 — 0xFF gets masked to 0x3.
        set_tile_full(&mut packed, 0, 0, 0xFF, 0xFF);
        assert_eq!(tile_full(&packed, 0), (0, 3, 3));
        // Neighbours stay zero across all the masking.
        assert_eq!(tile_full(&packed, 1), (0, 0, 0));
    }

    #[test]
    fn tile_stock_isolated_writes() {
        // `set_tile_stock` mutates one stock without touching the
        // def or the other stock.
        let mut packed = [0u64; ZONE_TILE_U64_COUNT];
        set_tile_full(&mut packed, 7, 0xABC, 1, 2);
        set_tile_stock(&mut packed, 7, 0, 3);
        assert_eq!(tile_full(&packed, 7), (0xABC, 3, 2));
        set_tile_stock(&mut packed, 7, 1, 0);
        assert_eq!(tile_full(&packed, 7), (0xABC, 3, 0));
        // tile_stock reads the same value
        assert_eq!(tile_stock(&packed, 7, 0), 3);
        assert_eq!(tile_stock(&packed, 7, 1), 0);
    }

    #[test]
    fn tile_neighbour_independence() {
        // 4 tiles share a u64. Writing one tile must not corrupt
        // the other three. Pin tile 5's bits in the middle of u64[1]
        // (which holds tiles 4..=7) and confirm the surrounding
        // tiles stay zero, then mutate tile 5's stock and confirm
        // 4 / 6 / 7 still report zero.
        let mut packed = [0u64; ZONE_TILE_U64_COUNT];
        set_tile_full(&mut packed, 5, 0xABC, 2, 1);
        for &idx in &[0, 4, 6, 7, 8, 63] {
            assert_eq!(tile_full(&packed, idx), (0, 0, 0));
        }
        set_tile_stock(&mut packed, 5, 0, 0);
        for &idx in &[0, 4, 6, 7, 8, 63] {
            assert_eq!(tile_full(&packed, idx), (0, 0, 0));
        }
        // tile 5 retained its def + slot 1 stock.
        assert_eq!(tile_full(&packed, 5), (0xABC, 0, 1));
    }

    #[test]
    fn card_id_db_shard_roundtrip() {
        // db (top bit), shard (11 bits), local (low 20).
        let id = pack_card_id(CARD_DB_CARDS, 0, 1024);
        assert_eq!(card_db_of(id), CARD_DB_CARDS);
        assert_eq!(card_shard_within_db(id), 0);
        assert_eq!(card_shard_of(id), 0);
        assert_eq!(card_local_of(id), 1024);

        // cards shard 7.
        let id = pack_card_id(CARD_DB_CARDS, 7, 1_048_575);
        assert_eq!(card_db_of(id), CARD_DB_CARDS);
        assert_eq!(card_shard_within_db(id), 7);
        assert_eq!(card_shard_of(id), 7);
        assert_eq!(card_local_of(id), 1_048_575);

        // regions tile-card: db bit set → the 12-bit field is 2048 + shard.
        let id = pack_card_id(CARD_DB_REGIONS, 3, 42);
        assert_eq!(card_db_of(id), CARD_DB_REGIONS);
        assert_eq!(card_shard_within_db(id), 3);
        assert_eq!(card_shard_of(id), 2048 + 3);
        assert_eq!(card_local_of(id), 42);

        // Max fields → u32::MAX (db=1, shard=2047, local=0xFFFFF).
        let id = pack_card_id(CARD_DB_REGIONS, CARD_SHARD_MAX, CARD_LOCAL_MASK);
        assert_eq!(id, u32::MAX);
        assert_eq!(card_shard_within_db(id), CARD_SHARD_MAX);
        assert_eq!(CARD_SHARD_MAX, 2047);

        // The WORLD sentinel (0) decodes to the cards DB, shard 0, local 0.
        assert_eq!(card_db_of(0), CARD_DB_CARDS);
        assert_eq!(card_shard_of(0), 0);
        assert_eq!(card_local_of(0), 0);
    }

    #[test]
    fn tile_row_decode() {
        let mut packed = [0u64; ZONE_TILE_U64_COUNT];
        for col in 0..8 {
            set_tile_full(&mut packed, col, 100 + col as u16, 1, 2);
        }
        let row0 = tile_row(&packed, 0);
        for (col, entry) in row0.iter().enumerate() {
            assert_eq!(*entry, (100 + col as u16, 1, 2));
        }
        // Row 1 is still empty.
        let row1 = tile_row(&packed, 1);
        assert_eq!(row1, [(0, 0, 0); 8]);
    }
}
