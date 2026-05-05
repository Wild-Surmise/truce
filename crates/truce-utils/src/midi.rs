//! MIDI value-domain helpers: normalize / denormalize between
//! wire-native integers and `f32` ranges.
//!
//! truce's `EventBody` carries MIDI events as wire-native integers
//! (7-bit `u8`, 14-bit `u16`, 16-bit `u16`, 32-bit `u32`) so the
//! framework's representation round-trips exactly with the wire.
//! Plugin code that wants to multiply by a parameter, accumulate
//! into a phase, or otherwise use the value as a float reaches for
//! the helpers below.
//!
//! Each pair (`norm_*` / `denorm_*`) round-trips for every
//! representable wire input. See the per-helper docs for endpoint
//! semantics — pitch-bend is asymmetric on both MIDI 1.0 and MIDI
//! 2.0 because the spec's center value sits one code closer to the
//! negative end than the positive.
//!
//! Lints: the helpers do `as`-casts at well-defined widening or
//! lossless points (`u8 → f32`, `u16 → f32`, `f64 → f32` after
//! a clamped multiply), so the `cast_*` lints are allowed at the
//! module level rather than per call.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

// ---------------------------------------------------------------------------
// 7-bit (MIDI 1.0 velocity / CC / aftertouch / channel pressure / program)
// ---------------------------------------------------------------------------

/// MIDI 1.0 7-bit unsigned (`0..=127`) → `f32 ∈ [0.0, 1.0]`.
///
/// `norm_7bit(0) == 0.0`, `norm_7bit(127) == 1.0`. Inputs above 127
/// debug-assert: the high bit is reserved as the MIDI status flag,
/// so a value here is a sign of caller bug (the wrapper-level demux
/// already strips the status bit).
#[inline]
#[must_use]
pub fn norm_7bit(v: u8) -> f32 {
    debug_assert!(
        v <= 127,
        "norm_7bit: {v} > 127 (high bit is the MIDI status flag)",
    );
    f32::from(v) / 127.0
}

/// `f32 ∈ [0.0, 1.0]` → MIDI 1.0 7-bit unsigned (`0..=127`).
///
/// Clamps and rounds half-to-even. Negative inputs land on `0`;
/// inputs ≥ 1.0 land on `127`. NaN debug-asserts; release builds
/// land on `0` (clamp returns the lower bound for unordered input).
#[inline]
#[must_use]
pub fn denorm_7bit(v: f32) -> u8 {
    debug_assert!(
        !v.is_nan(),
        "denorm_7bit: NaN input — caller's normalized value is uninitialized?",
    );
    (v.clamp(0.0, 1.0) * 127.0).round() as u8
}

// ---------------------------------------------------------------------------
// 14-bit pitch bend (MIDI 1.0)
// ---------------------------------------------------------------------------

/// MIDI 1.0 14-bit pitch-bend code (`0..=16383`) → `f32 ∈ [-1.0,
/// ~0.99987]`.
///
/// Center is `8192`. The mapping is asymmetric (8192 negative
/// codes, 8191 positive codes) because that is the MIDI 1.0
/// convention: `0` decodes to exactly `-1.0`, but the positive
/// endpoint stops at `8191/8192`. Inputs above 16383 debug-assert.
///
/// Round-trips exactly with [`denorm_pitch_bend`] for every
/// `raw ∈ [0, 16383]`.
#[inline]
#[must_use]
pub fn norm_pitch_bend(raw: u16) -> f32 {
    debug_assert!(
        raw <= 16383,
        "norm_pitch_bend: raw {raw} > 16383 — caller didn't mask LSB|MSB<<7?",
    );
    (f32::from(raw) - 8192.0) / 8192.0
}

/// `f32 ∈ [-1.0, 1.0]` → MIDI 1.0 14-bit pitch-bend code
/// (`0..=16383`).
///
/// Inverse of [`norm_pitch_bend`]. `-1.0` → `0`, `0.0` → `8192`,
/// `1.0` → `16383` (clamped — the perfectly symmetric `+1.0`
/// would be `16384`). NaN debug-asserts.
#[inline]
#[must_use]
pub fn denorm_pitch_bend(v: f32) -> u16 {
    debug_assert!(
        !v.is_nan(),
        "denorm_pitch_bend: NaN input — caller's normalized value is uninitialized?",
    );
    let raw = (v.clamp(-1.0, 1.0) * 8192.0 + 8192.0).round();
    (raw as u16).min(16383)
}

/// Split a 14-bit pitch-bend code into the (LSB, MSB) byte pair the
/// wire format carries. Each output byte has the high bit clear.
///
/// Used by every format wrapper's MIDI 1.0 output path. Unifies the
/// `(raw & 0x7F) as u8` / `((raw >> 7) & 0x7F) as u8` magic-constant
/// split that previously lived in six places.
#[inline]
#[must_use]
pub fn pitch_bend_to_bytes(raw: u16) -> (u8, u8) {
    debug_assert!(raw <= 16383, "pitch_bend_to_bytes: raw {raw} > 16383");
    let lsb = (raw & 0x7F) as u8;
    let msb = ((raw >> 7) & 0x7F) as u8;
    (lsb, msb)
}

/// Combine two MIDI bytes (LSB first) into a 14-bit pitch-bend code.
/// Each input byte's high bit is masked off before combining.
///
/// Inverse of [`pitch_bend_to_bytes`]. The masking matters: a
/// running-status parser may hand bytes that include the status
/// flag, and `(msb << 7) | lsb` without masking would corrupt the
/// result on out-of-domain input.
#[inline]
#[must_use]
pub fn pitch_bend_from_bytes(lsb: u8, msb: u8) -> u16 {
    (u16::from(msb & 0x7F) << 7) | u16::from(lsb & 0x7F)
}

// ---------------------------------------------------------------------------
// 16-bit (MIDI 2.0 velocity)
// ---------------------------------------------------------------------------

/// MIDI 2.0 16-bit unsigned (`NoteOn2.velocity`) → `f32 ∈ [0.0,
/// 1.0]`.
///
/// `0` → `0.0`, `65535` → `1.0`. Linear scale. The MIDI 2.0 spec
/// reserves `velocity == 0` as the legacy "treat as note-off"
/// signaler only when bridging *from* MIDI 1.0; native MIDI 2.0
/// note-off uses the dedicated message and zero velocity is a
/// genuine zero.
#[inline]
#[must_use]
pub fn norm_16bit(v: u16) -> f32 {
    f32::from(v) / 65535.0
}

/// `f32 ∈ [0.0, 1.0]` → MIDI 2.0 16-bit unsigned (`0..=65535`).
///
/// Clamps and rounds half-to-even. NaN debug-asserts. Inverse of
/// [`norm_16bit`]; round-trips for every representable `u16`.
#[inline]
#[must_use]
pub fn denorm_16bit(v: f32) -> u16 {
    debug_assert!(
        !v.is_nan(),
        "denorm_16bit: NaN input — caller's normalized value is uninitialized?",
    );
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

// ---------------------------------------------------------------------------
// 32-bit unipolar (MIDI 2.0 channel CC, per-note CC, channel pressure,
//                   poly pressure, registered/assignable controllers)
// ---------------------------------------------------------------------------

/// MIDI 2.0 32-bit unsigned → `f32 ∈ [0.0, 1.0]`.
///
/// The intermediate is `f64` so a partially-rounded `f32` numerator
/// can't bias the divide. `f32`'s 24-bit mantissa truncates past
/// `2^24` regardless, so callers needing full 32-bit fidelity should
/// keep the value as `u32` or normalize through `f64` themselves.
#[inline]
#[must_use]
pub fn norm_32bit(v: u32) -> f32 {
    (f64::from(v) / f64::from(u32::MAX)) as f32
}

/// `f32 ∈ [0.0, 1.0]` → MIDI 2.0 32-bit unsigned. Clamps and
/// rounds half-to-even. NaN debug-asserts.
#[inline]
#[must_use]
pub fn denorm_32bit(v: f32) -> u32 {
    debug_assert!(
        !v.is_nan(),
        "denorm_32bit: NaN input — caller's normalized value is uninitialized?",
    );
    (f64::from(v.clamp(0.0, 1.0)) * f64::from(u32::MAX)).round() as u32
}

// ---------------------------------------------------------------------------
// 32-bit pitch bend (MIDI 2.0 channel + per-note pitch bend)
// ---------------------------------------------------------------------------

/// MIDI 2.0 32-bit pitch-bend code → `f32 ∈ [-1.0, 1.0]`.
///
/// Center is `0x8000_0000`. `0` → `-1.0`, `0x8000_0000` → `0.0`,
/// `0xFFFF_FFFF` → ~`0.99999999953` (asymmetric, same way MIDI 1.0
/// is — the negative side has one more representable code).
#[inline]
#[must_use]
pub fn norm_pitch_bend_32(raw: u32) -> f32 {
    let signed = i64::from(raw) - i64::from(0x8000_0000_u32);
    (signed as f64 / f64::from(0x8000_0000_u32)) as f32
}

/// `f32 ∈ [-1.0, 1.0]` → MIDI 2.0 32-bit pitch-bend code. Inverse
/// of [`norm_pitch_bend_32`]; clamps to the asymmetric range so the
/// positive endpoint lands on `0xFFFF_FFFF`. NaN debug-asserts.
#[inline]
#[must_use]
pub fn denorm_pitch_bend_32(v: f32) -> u32 {
    debug_assert!(
        !v.is_nan(),
        "denorm_pitch_bend_32: NaN input — caller's normalized value is uninitialized?",
    );
    let scaled =
        (f64::from(v.clamp(-1.0, 1.0)) * f64::from(0x8000_0000_u32)) + f64::from(0x8000_0000_u32);
    scaled.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

// ---------------------------------------------------------------------------
// MIDI 1.0 ↔ MIDI 2.0 bridge (per the MIDI 2.0 Core Spec, M2-100)
// ---------------------------------------------------------------------------
//
// The spec defines exact algorithms for upconverting 7/14-bit values
// to 16/32-bit so a host bridging a MIDI 1.0 device into a MIDI 2.0
// pipeline reproduces the same effective ratio. truce uses these
// when a wrapper only delivers MIDI 1.0 but the plugin author wants
// to consume the unified MIDI 2.0 path.

/// MIDI 2.0 spec's "min-center-max scale and bit-replicate"
/// upconvert. Below or at center: pure left-shift (preserves the
/// "center → center" invariant). Above center: replicates the
/// `src_bits - 1` low bits of the source as many times as fits in
/// the trailing zero region, so max input maps to max output.
///
/// See M2-100-U §4.7 ("scaleUp" pseudocode).
#[inline]
fn upconvert(src_val: u32, src_bits: u32, dst_bits: u32) -> u32 {
    debug_assert!(src_bits < dst_bits);
    let scale_bits = dst_bits - src_bits;
    let mut bit_shifted = src_val << scale_bits;
    let center = 1u32 << (src_bits - 1);
    if src_val <= center {
        return bit_shifted;
    }
    let repeat_bits = src_bits - 1;
    let repeat_mask = (1u32 << repeat_bits) - 1;
    let repeat_value = src_val & repeat_mask;
    let mut remaining = scale_bits;
    while remaining > 0 {
        if remaining >= repeat_bits {
            remaining -= repeat_bits;
            bit_shifted |= repeat_value << remaining;
        } else {
            bit_shifted |= repeat_value >> (repeat_bits - remaining);
            break;
        }
    }
    bit_shifted
}

/// 7-bit MIDI 1.0 value (`0..=127`) → MIDI 2.0 16-bit value.
///
/// Min/center/max preserved: `0 → 0`, `64 → 0x8000`, `127 →
/// 0xFFFF`. Inverse of [`downconvert_16_to_7`].
#[inline]
#[must_use]
pub fn upconvert_7_to_16(v: u8) -> u16 {
    debug_assert!(v <= 127, "upconvert_7_to_16: v {v} > 127");
    upconvert(u32::from(v), 7, 16) as u16
}

/// 7-bit MIDI 1.0 value → MIDI 2.0 32-bit value. Min/center/max
/// preserved.
#[inline]
#[must_use]
pub fn upconvert_7_to_32(v: u8) -> u32 {
    debug_assert!(v <= 127, "upconvert_7_to_32: v {v} > 127");
    upconvert(u32::from(v), 7, 32)
}

/// 14-bit MIDI 1.0 value (`0..=16383`) → MIDI 2.0 32-bit value.
/// Min/center/max preserved: `0 → 0`, `8192 → 0x8000_0000`,
/// `16383 → 0xFFFF_FFFF`.
#[inline]
#[must_use]
pub fn upconvert_14_to_32(v: u16) -> u32 {
    debug_assert!(v <= 16383, "upconvert_14_to_32: v {v} > 16383");
    upconvert(u32::from(v), 14, 32)
}

/// MIDI 2.0 16-bit value → 7-bit MIDI 1.0 value, per the spec's
/// downconvert (truncate top 7 bits).
#[inline]
#[must_use]
pub fn downconvert_16_to_7(v: u16) -> u8 {
    (v >> 9) as u8
}

/// MIDI 2.0 32-bit value → 7-bit MIDI 1.0 value, per the spec's
/// downconvert.
#[inline]
#[must_use]
pub fn downconvert_32_to_7(v: u32) -> u8 {
    (v >> 25) as u8
}

/// MIDI 2.0 32-bit value → 14-bit MIDI 1.0 value, per the spec's
/// downconvert.
#[inline]
#[must_use]
pub fn downconvert_32_to_14(v: u32) -> u16 {
    (v >> 18) as u16
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // ---------- 7-bit ----------

    #[test]
    fn norm_7bit_endpoints() {
        assert_eq!(norm_7bit(0), 0.0);
        assert_eq!(norm_7bit(127), 1.0);
        assert!((norm_7bit(64) - (64.0 / 127.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn denorm_7bit_endpoints() {
        assert_eq!(denorm_7bit(0.0), 0);
        assert_eq!(denorm_7bit(1.0), 127);
        assert_eq!(denorm_7bit(0.5), 64); // round-half-to-even via .round()
    }

    #[test]
    fn denorm_7bit_clamps() {
        assert_eq!(denorm_7bit(-0.5), 0);
        assert_eq!(denorm_7bit(2.0), 127);
        assert_eq!(denorm_7bit(f32::INFINITY), 127);
        assert_eq!(denorm_7bit(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn round_trip_7bit_all_codes() {
        // Every representable 7-bit value normalizes and denormalizes
        // back to itself.
        for raw in 0u8..=127 {
            assert_eq!(denorm_7bit(norm_7bit(raw)), raw);
        }
    }

    // ---------- 14-bit pitch bend ----------

    #[test]
    fn norm_pitch_bend_endpoints() {
        assert_eq!(norm_pitch_bend(0), -1.0);
        assert_eq!(norm_pitch_bend(8192), 0.0);
        // Asymmetric positive endpoint: 8191 / 8192 ≈ 0.99987.
        let max_pos = norm_pitch_bend(16383);
        assert!((max_pos - 8191.0_f32 / 8192.0_f32).abs() < f32::EPSILON);
    }

    #[test]
    fn denorm_pitch_bend_endpoints() {
        assert_eq!(denorm_pitch_bend(-1.0), 0);
        assert_eq!(denorm_pitch_bend(0.0), 8192);
        assert_eq!(denorm_pitch_bend(1.0), 16383);
    }

    #[test]
    fn round_trip_pitch_bend_all_codes() {
        for raw in 0u16..=16383 {
            let v = norm_pitch_bend(raw);
            let back = denorm_pitch_bend(v);
            assert_eq!(back, raw, "raw={raw}, v={v}");
        }
    }

    #[test]
    fn pitch_bend_byte_split_round_trip() {
        for raw in 0u16..=16383 {
            let (lsb, msb) = pitch_bend_to_bytes(raw);
            assert!(lsb < 128 && msb < 128);
            assert_eq!(pitch_bend_from_bytes(lsb, msb), raw);
        }
    }

    #[test]
    fn pitch_bend_from_bytes_masks_high_bit() {
        // Status-flag bits in either byte must not corrupt the result.
        assert_eq!(pitch_bend_from_bytes(0xFF, 0xFF), 16383);
        assert_eq!(pitch_bend_from_bytes(0x80, 0x80), 0);
    }

    // ---------- 16-bit ----------

    #[test]
    fn norm_16bit_endpoints() {
        assert_eq!(norm_16bit(0), 0.0);
        assert_eq!(norm_16bit(65535), 1.0);
    }

    #[test]
    fn denorm_16bit_endpoints() {
        assert_eq!(denorm_16bit(0.0), 0);
        assert_eq!(denorm_16bit(1.0), 65535);
    }

    #[test]
    fn round_trip_16bit_endpoints_and_centers() {
        for raw in [0u16, 1, 32_767, 32_768, 65_534, 65_535] {
            assert_eq!(denorm_16bit(norm_16bit(raw)), raw);
        }
    }

    // ---------- 32-bit unipolar ----------

    #[test]
    fn norm_32bit_endpoints() {
        assert_eq!(norm_32bit(0), 0.0);
        assert_eq!(norm_32bit(u32::MAX), 1.0);
    }

    #[test]
    fn denorm_32bit_endpoints() {
        assert_eq!(denorm_32bit(0.0), 0);
        assert_eq!(denorm_32bit(1.0), u32::MAX);
    }

    // ---------- 32-bit pitch bend ----------

    #[test]
    fn norm_pitch_bend_32_endpoints() {
        assert_eq!(norm_pitch_bend_32(0), -1.0);
        assert_eq!(norm_pitch_bend_32(0x8000_0000), 0.0);
        // ~0.99999999953 — close enough to 1.0 in f32.
        let max_pos = norm_pitch_bend_32(0xFFFF_FFFF);
        assert!((max_pos - 1.0).abs() < 1e-6);
    }

    #[test]
    fn denorm_pitch_bend_32_endpoints() {
        assert_eq!(denorm_pitch_bend_32(-1.0), 0);
        assert_eq!(denorm_pitch_bend_32(0.0), 0x8000_0000);
        assert_eq!(denorm_pitch_bend_32(1.0), u32::MAX);
    }

    // ---------- spec bridge ----------

    #[test]
    fn upconvert_7_to_16_endpoints() {
        // The spec algorithm preserves: 0 → 0, 64 → 0x8000 (center
        // → center), 127 → 0xFFFF (max → max).
        assert_eq!(upconvert_7_to_16(0), 0);
        assert_eq!(upconvert_7_to_16(64), 0x8000);
        assert_eq!(upconvert_7_to_16(127), 0xFFFF);
    }

    #[test]
    fn upconvert_14_to_32_endpoints() {
        assert_eq!(upconvert_14_to_32(0), 0);
        assert_eq!(upconvert_14_to_32(8192), 0x8000_0000);
        assert_eq!(upconvert_14_to_32(16383), 0xFFFF_FFFF);
    }

    #[test]
    fn downconvert_round_trips_endpoints() {
        for v in [0u8, 64, 127] {
            assert_eq!(downconvert_16_to_7(upconvert_7_to_16(v)), v);
            assert_eq!(downconvert_32_to_7(upconvert_7_to_32(v)), v);
        }
        for v in [0u16, 8192, 16383] {
            assert_eq!(downconvert_32_to_14(upconvert_14_to_32(v)), v);
        }
    }
}
