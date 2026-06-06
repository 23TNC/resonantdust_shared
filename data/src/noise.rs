//! Deterministic climate noise — the shared source `^biome` draws from.
//!
//! Ported **verbatim** (the `f32` math) from the regions module's
//! `world_gen.rs` so the climate field is byte-identical to what the live
//! server already generates: the same `(q, r, seed)` yields the same lattice,
//! and — because gate, client, and module all link this one crate — every
//! consumer agrees on `^biome` (the determinism `^biome` requires; cf.
//! content/data/SYNTAX.txt). The parity test pins the exact `f32` bit patterns
//! against reference values computed from the legacy code.
//!
//! Two layers:
//!   - the raw `f32 ∈ [0, 1)` samplers (`sample_*`) — the ported field;
//!   - [`climate_ints`] — the D2 contract: each axis scaled to a `0..=99`
//!     integer, the units the `<biome>` envelopes and tile `@init` hooks read
//!     (`*biome.humidity 55 95 within`). The host (`crate::biome`) wraps these
//!     into the `^biome` cell.
//!
//! `rarity` is the one channel with no legacy counterpart: the live worldgen
//! kept rarity as a per-def `placement.rarity` trait, but the DSL tile `@init`
//! reads `*biome.rarity` as a climate-like axis (its `within` tiers define the
//! bands). So it is a fresh decorrelated FBM channel here — its own scale and
//! seed offset, same shape as the four climate axes.

// FBM tuning — dominant low-frequency octave plus a 2× detail octave with a
// seed offset so the detail isn't aligned with the dominant lattice. Sum is
// normalized so output stays in `[0, 1)` regardless of weight tuning.
const FBM_OCTAVE_WEIGHTS: [f32; 2] = [1.0, 0.5];

// Spatial scale (`1 / lattice_period`) per axis — smaller = broader features.
const ELEVATION_BASE_SCALE: f32 = 1.0 / 40.0;
const TEMPERATURE_BASE_SCALE: f32 = 1.0 / 25.0;
const HUMIDITY_BASE_SCALE: f32 = 1.0 / 12.0;
const AETHER_BASE_SCALE: f32 = 1.0 / 15.0;
/// Medium scale, between humidity and aether — rarity clumps are region-ish.
const RARITY_BASE_SCALE: f32 = 1.0 / 18.0;

// Per-axis seed offsets — XORed into the seed so each axis's lattice is
// decorrelated from every other. Distinct splitmix64-of-small-int constants.
const ELEVATION_SEED_OFFSET: u64 = 0xA341_316C_C2C7_3030;
const TEMPERATURE_SEED_OFFSET: u64 = 0x6C25_7BBA_AB47_C0DA;
const HUMIDITY_SEED_OFFSET: u64 = 0x4F49_8C77_E915_4F03;
const AETHER_SEED_OFFSET: u64 = 0x9E1A_C401_BB18_9E5F;
/// Fresh offset for the rarity channel (no legacy lattice to match).
const RARITY_SEED_OFFSET: u64 = 0xD1B5_4A32_D192_ED03;

/// Number of axes in [`climate_ints`]: elevation, temperature, humidity,
/// aether, rarity.
pub const AXIS_COUNT: usize = 5;

/// Deterministic 64-bit hash from 2D integer coords + seed. Two large odd
/// multipliers (one per axis) plus a splitmix-style finalizer. Not
/// cryptographic. Sign-extends `i32 → u64` so negative coords hash distinctly.
fn hash2(x: i32, y: i32, seed: u64) -> u64 {
  let mut h = seed ^ ((x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
  h = h.wrapping_add((y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
  h ^= h >> 33;
  h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
  h ^= h >> 33;
  h
}

/// Value at a lattice corner, mapped to `[0.0, 1.0)`.
fn lattice_value(x: i32, y: i32, seed: u64) -> f32 {
  let h = hash2(x, y, seed) as u32;
  (h as f32) / 4_294_967_296.0
}

/// Cubic smoothstep `t²(3 - 2t)` — C¹ continuity at lattice corners.
fn smoothstep(t: f32) -> f32 {
  t * t * (3.0 - 2.0 * t)
}

/// Bilinear value-noise sample at a continuous `(x, y)`.
fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
  let xi = x.floor() as i32;
  let yi = y.floor() as i32;
  let xf = x - xi as f32;
  let yf = y - yi as f32;
  let a = lattice_value(xi, yi, seed);
  let b = lattice_value(xi + 1, yi, seed);
  let c = lattice_value(xi, yi + 1, seed);
  let d = lattice_value(xi + 1, yi + 1, seed);
  let u = smoothstep(xf);
  let v = smoothstep(yf);
  let ab = a + u * (b - a);
  let cd = c + u * (d - c);
  ab + v * (cd - ab)
}

/// 2-octave fractional Brownian motion of value noise. Dominant octave at
/// `(x, y)`; detail octave at `(2x, 2y)` with a seed offset so it isn't aligned
/// with the dominant lattice. Normalized to stay in `[0, 1)`.
fn fbm(x: f32, y: f32, seed: u64) -> f32 {
  let [w0, w1] = FBM_OCTAVE_WEIGHTS;
  let o0 = value_noise(x, y, seed);
  let o1 = value_noise(x * 2.0, y * 2.0, seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
  (w0 * o0 + w1 * o1) / (w0 + w1)
}

fn sample_axis(global_q: i32, global_r: i32, seed: u64, scale: f32, offset: u64) -> f32 {
  fbm(global_q as f32 * scale, global_r as f32 * scale, seed ^ offset)
}

/// Sample elevation `∈ [0, 1)`. Very large spatial scale — peaks span tens of
/// cells.
pub fn sample_elevation(global_q: i32, global_r: i32, seed: u64) -> f32 {
  sample_axis(global_q, global_r, seed, ELEVATION_BASE_SCALE, ELEVATION_SEED_OFFSET)
}
/// Sample temperature `∈ [0, 1)`.
pub fn sample_temperature(global_q: i32, global_r: i32, seed: u64) -> f32 {
  sample_axis(global_q, global_r, seed, TEMPERATURE_BASE_SCALE, TEMPERATURE_SEED_OFFSET)
}
/// Sample humidity `∈ [0, 1)` — medium scale, microclimate variation.
pub fn sample_humidity(global_q: i32, global_r: i32, seed: u64) -> f32 {
  sample_axis(global_q, global_r, seed, HUMIDITY_BASE_SCALE, HUMIDITY_SEED_OFFSET)
}
/// Sample aether `∈ [0, 1)`.
pub fn sample_aether(global_q: i32, global_r: i32, seed: u64) -> f32 {
  sample_axis(global_q, global_r, seed, AETHER_BASE_SCALE, AETHER_SEED_OFFSET)
}
/// Sample rarity `∈ [0, 1)` — the DSL-only channel (no legacy lattice).
pub fn sample_rarity(global_q: i32, global_r: i32, seed: u64) -> f32 {
  sample_axis(global_q, global_r, seed, RARITY_BASE_SCALE, RARITY_SEED_OFFSET)
}

/// Scale an `f32 ∈ [0, 1)` sample to the `0..=99` integer the DSL envelopes and
/// `@init` hooks compare against (D2). `floor(v * 100)`, clamped for safety
/// against any `v` that rounds to exactly `1.0`.
fn to_int(v: f32) -> i64 {
  ((v * 100.0).floor() as i64).clamp(0, 99)
}

/// A deterministic per-cell seed for `^seed` — the value tile `@update` /
/// `ring_objects` mix into `random` to vary object placement per hex. Derived
/// from the cell coords + world seed (same lattice hash as the noise), so gate
/// and client agree. Always non-negative (the VM treats `^seed` as an `i64`).
pub fn cell_seed(global_q: i32, global_r: i32, seed: u64) -> i64 {
  (hash2(global_q, global_r, seed) >> 1) as i64
}

/// The five climate axes at `(global_q, global_r)` as `0..=99` integers, in the
/// order `[elevation, temperature, humidity, aether, rarity]`. The unit the
/// `<biome>` envelopes (`70 100 &elevation range`) and tile `@init`
/// (`*biome.humidity 55 95 within`) read; the `^biome` host (`crate::biome`)
/// turns these into the cell fields.
pub fn climate_ints(global_q: i32, global_r: i32, seed: u64) -> [i64; AXIS_COUNT] {
  [
    to_int(sample_elevation(global_q, global_r, seed)),
    to_int(sample_temperature(global_q, global_r, seed)),
    to_int(sample_humidity(global_q, global_r, seed)),
    to_int(sample_aether(global_q, global_r, seed)),
    to_int(sample_rarity(global_q, global_r, seed)),
  ]
}

/// The five climate axes as `0.0..100.0` floats — the float-native unit now that
/// the VM supports floats (`within`/`normalize`/`range` are float-aware). Same
/// axis order as [`climate_ints`]; the unrounded `sample × 100`. The `^biome`
/// host and biome selection use these so a tile sees the true climate value
/// rather than a floored integer.
pub fn climate_floats(global_q: i32, global_r: i32, seed: u64) -> [f64; AXIS_COUNT] {
  [
    sample_elevation(global_q, global_r, seed) as f64 * 100.0,
    sample_temperature(global_q, global_r, seed) as f64 * 100.0,
    sample_humidity(global_q, global_r, seed) as f64 * 100.0,
    sample_aether(global_q, global_r, seed) as f64 * 100.0,
    sample_rarity(global_q, global_r, seed) as f64 * 100.0,
  ]
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
  use super::*;

  /// Reference world seed (`world_gen::WORLD_SEED`), used by the parity lock.
  const WORLD_SEED: u64 = 0x27;

  #[test]
  fn deterministic_and_seed_sensitive() {
    assert_eq!(value_noise(1.7, 2.3, 42), value_noise(1.7, 2.3, 42));
    assert_ne!(value_noise(1.7, 2.3, 42), value_noise(1.7, 2.3, 43));
  }

  #[test]
  fn samples_in_unit_range() {
    for q in -50..50 {
      for r in -50..50 {
        for v in climate_ints(q, r, WORLD_SEED) {
          assert!((0..=99).contains(&v), "axis out of range at ({q},{r}): {v}");
        }
      }
    }
  }

  /// Parity lock: the ported `f32` math must reproduce, bit-for-bit, the values
  /// computed from the legacy `world_gen.rs` noise (see /tmp/noise_ref.rs run).
  /// If this breaks, the climate field has drifted from the live server's — a
  /// gate↔client / new↔legacy divergence, not a cosmetic change.
  #[test]
  fn matches_legacy_noise_bit_for_bit() {
    assert_eq!(value_noise(1.7, 2.3, 42).to_bits(), 0x3F3A_3729);
    // [elevation, temperature, humidity, aether] f32 bits at WORLD_SEED.
    // (rarity is DSL-only — no legacy reference, so not pinned here.)
    let cases: [(i32, i32, [u32; 4]); 5] = [
      (0, 0, [0x3F16_282F, 0x3F10_E401, 0x3F40_0A1D, 0x3E21_7E5D]),
      (3, -2, [0x3F17_A28C, 0x3F0F_2D23, 0x3F3F_F39B, 0x3E4A_E0F7]),
      (12, 7, [0x3F12_CA93, 0x3F22_968B, 0x3ED4_B28F, 0x3F0C_D997]),
      (-5, 9, [0x3F06_0757, 0x3F25_F10E, 0x3EEE_24D5, 0x3EFE_A9B7]),
      (40, 40, [0x3F1D_8603, 0x3F2E_0A9F, 0x3ECE_D2F3, 0x3F26_2CE8]),
    ];
    for (q, r, bits) in cases {
      assert_eq!(sample_elevation(q, r, WORLD_SEED).to_bits(), bits[0], "elev ({q},{r})");
      assert_eq!(sample_temperature(q, r, WORLD_SEED).to_bits(), bits[1], "temp ({q},{r})");
      assert_eq!(sample_humidity(q, r, WORLD_SEED).to_bits(), bits[2], "humid ({q},{r})");
      assert_eq!(sample_aether(q, r, WORLD_SEED).to_bits(), bits[3], "aether ({q},{r})");
    }
  }

  #[test]
  fn origin_lands_in_forest_envelope() {
    // WORLD_SEED is chosen so the spawn origin sits inside the forest biome
    // (elevation 30-75, temperature 20-70, humidity 55-95). Guards the seed +
    // the int scaling together.
    let [elev, temp, humid, _aeth, _rar] = climate_ints(0, 0, WORLD_SEED);
    assert_eq!([elev, temp, humid], [58, 56, 75]);
    assert!((30..=75).contains(&elev) && (20..=70).contains(&temp) && (55..=95).contains(&humid));
  }
}
