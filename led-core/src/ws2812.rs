//! WS2812 (NeoPixel) wire-protocol encoding.
//!
//! ⚠ **The legacy model in the first half of this module (items before the
//! `V2` marker near the bottom) is NON-COMPLIANT with the WS2812 datasheet
//! and produces garbled colors on real hardware — this has been confirmed
//! visually on the board. Do not wire it to a physical LED. Use the
//! nanosecond-resolution, two-phases-per-bit model instead: [`encode_rgb_v2`]
//! / [`encode_grb_v2`] / [`replay_frame_v2`] / [`PinPulseNs`], with the
//! timing constants [`T0H_NS`], [`T0L_NS`], [`T1H_NS`], [`T1L_NS`],
//! [`RESET_NS`].**
//!
//! The protocol is timing-critical and is driven on hardware by GPIO
//! bit-banging. To make the *encoding* logic testable on the host, we model
//! it as a stream of timed [`Ws2812Event`]s: each event sets the pin to a
//! level and holds it for `us` microseconds. The hardware layer replays the
//! events with [`replay_frame`] — the exact GPIO sequence the firmware
//! performs, so what is tested here is what runs on the device.
//!
//! Protocol (as implemented by the shipped bit-banged driver):
//!
//! - One bit is **1 µs total**:
//!   - logic `1` → HIGH for 1 µs (then LOW, no delay),
//!   - logic `0` → (HIGH skipped) LOW for 1 µs.
//! - One byte = 8 bits, MSB first.
//! - One pixel = 24 bits in **GRB** order (Green, Red, Blue) = 24 µs.
//! - Frame reset = LOW ≥ 50 µs after the last bit.
//! - Full frame = 24 + 50 = **74 µs** of blocking pin activity.
//!
//! Colors in this crate are `[R, G, B]` (the config/UI order). Conversion to
//! the GRB wire order happens in [`encode_rgb`].

/// A single timed pin event: the pin goes to `high`, held for `us` µs.
///
/// **LEGACY / NON-COMPLIANT.** One event carries only ONE phase of a bit,
/// at µs granularity, so neither the real T0H/T0L nor the T1H/T1L of the
/// WS2812 datasheet can be expressed — see the module-level warning and the
/// replacement [`Ws2812BitPhase`]. Kept only so existing firmware still
/// compiles; do not use on real hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812Event {
    pub high: bool,
    pub us: u32,
}

/// Width of one bit in µs (the legacy driver holds the active level for 1 µs).
///
/// **LEGACY / NON-COMPLIANT** (1 µs per bit, one phase only): the compliant
/// per-bit budget is 1250 ns with BOTH phases timed — see [`T0_TOTAL_NS`] /
/// [`T1_TOTAL_NS`].
pub const BIT_US: u32 = 1;

/// Reset pulse width (µs) used by the LEGACY encoder: 50 µs.
///
/// **LEGACY / NON-COMPLIANT for WS2812B**: 50 µs meets only the bare
/// WS2812 t_RST ≥ 50 µs minimum. The compliant value this crate now
/// transmits is [`RESET_NS`] (300 µs ≥ the WS2812B 280 µs spec).
pub const RESET_US: u32 = 50;

/// Number of bits per pixel (3 channels × 8).
pub const BITS_PER_PIXEL: u32 = 24;

/// Total µs to transmit one pixel under the LEGACY 1 µs-per-bit model
/// (24 bits × 1 µs). The compliant value is [`PIXEL_NS`] (24 × 1250 ns =
/// 30 µs).
pub const PIXEL_US: u32 = BITS_PER_PIXEL * BIT_US;

/// Total µs for one full frame under the LEGACY model (pixel + 50 µs reset).
/// The compliant value is [`FRAME_NS`] (30 µs + 300 µs = 330 µs).
pub const FRAME_US: u32 = PIXEL_US + RESET_US;

/// A complete frame: one GRB pixel followed by the reset pulse.
///
/// **LEGACY / NON-COMPLIANT** — its events cannot express both timed
/// phases of a bit. Compliant replacement: [`Ws2812FrameV2`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812Frame {
    pub pixel: [Ws2812Event; 24],
    pub reset: Ws2812Event,
}

/// Encode one bit as an event.
///
/// **LEGACY / NON-COMPLIANT**: the single 1 µs phase cannot express the
/// datasheet's two timed phases (T0H/T0L or T1H/T1L). Compliant
/// replacement: [`bit_phase`].
pub const fn bit_event(bit: bool) -> Ws2812Event {
    Ws2812Event {
        high: bit,
        us: BIT_US,
    }
}

/// Encode one byte (MSB first) as 8 events.
///
/// **LEGACY / NON-COMPLIANT** — see [`Ws2812Event`]; replacement:
/// [`byte_phases`].
pub fn byte_events(byte: u8) -> [Ws2812Event; 8] {
    let mut out = [Ws2812Event { high: false, us: 0 }; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = bit_event((byte >> (7 - i)) & 1 != 0);
    }
    out
}

/// Encode a single GRB pixel (g, r, b) as 24 events, **without** the reset.
///
/// **LEGACY / NON-COMPLIANT** — see [`Ws2812Event`]; replacement:
/// [`grb_pixel_phases`].
pub fn grb_pixel_events(g: u8, r: u8, b: u8) -> [Ws2812Event; 24] {
    let mut out = [Ws2812Event { high: false, us: 0 }; 24];
    for (bi, &byte) in [g, r, b].iter().enumerate() {
        out[8 * bi..8 * bi + 8].copy_from_slice(&byte_events(byte));
    }
    out
}

/// Encode a GRB pixel as a complete frame (pixel + reset pulse).
///
/// **LEGACY / NON-COMPLIANT — PRODUCES A NON-COMPLIANT WAVEFORM on real
/// hardware** (one un-timed phase per bit; consecutive `1` bits merge).
/// Use [`encode_grb_v2`] instead.
pub fn encode_grb(g: u8, r: u8, b: u8) -> Ws2812Frame {
    Ws2812Frame {
        pixel: grb_pixel_events(g, r, b),
        reset: Ws2812Event {
            high: false,
            us: RESET_US,
        },
    }
}

/// Encode an RGB color (`[R, G, B]`) as a complete frame, applying the
/// RGB→GRB wire-order conversion.
///
/// **LEGACY / NON-COMPLIANT — PRODUCES A NON-COMPLIANT WAVEFORM on real
/// hardware.** Use [`encode_rgb_v2`] instead (it applies the identical
/// RGB→GRB conversion to the compliant two-phase encoding).
pub fn encode_rgb(rgb: &[u8; 3]) -> Ws2812Frame {
    encode_grb(rgb[1], rgb[0], rgb[2])
}

/// A pin + delay the frame can be replayed onto. The firmware implements this
/// for `(esp_hal Output, esp_hal Delay)`; tests implement it with a recorder.
///
/// **LEGACY / NON-COMPLIANT**: `delay_us` has only µs granularity, so the
/// sub-microsecond phases of the real protocol (T0H = 400 ns) cannot be
/// expressed at all. Compliant replacement: [`PinPulseNs`] with `delay_ns`.
pub trait PinPulse {
    fn set_high(&mut self);
    fn set_low(&mut self);
    fn delay_us(&mut self, us: u32);
}

/// Replay a frame onto a pin — exactly the GPIO sequence the firmware driver
/// performs:
///
/// ```text
/// for each bit event e:
///     pin.set_high()
///     if e.high { delay(e.us) }
///     pin.set_low()
///     if !e.high { delay(e.us) }
/// pin.set_low(); delay(reset.us)
/// ```
///
/// **LEGACY / NON-COMPLIANT — THIS IS THE BIT-BANG SEQUENCE THAT PRODUCED
/// THE GARBOLED-COLORS BUG ON HARDWARE**: only one phase per bit is timed,
/// so a `1` bit ends ~0 ns LOW and consecutive `1` bits merge on the wire.
/// The compliant sequence (both phases per bit) is [`replay_frame_v2`].
pub fn replay_frame<P: PinPulse>(pin: &mut P, frame: &Ws2812Frame) {
    for e in &frame.pixel {
        pin.set_high();
        if e.high {
            pin.delay_us(e.us);
        }
        pin.set_low();
        if !e.high {
            pin.delay_us(e.us);
        }
    }
    pin.set_low();
    pin.delay_us(frame.reset.us);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn total_high_us(events: &[Ws2812Event]) -> u32 {
        events.iter().filter(|e| e.high).map(|e| e.us).sum()
    }

    #[test]
    fn frame_shape() {
        let f = encode_grb(0, 0, 0);
        assert_eq!(f.pixel.len(), 24, "24 bits = 24 events");
        assert!(!f.reset.high);
        assert_eq!(f.reset.us, RESET_US);
        assert!(f.reset.us >= 50, "reset must be ≥ 50 µs");
    }

    #[test]
    fn timing_constants_match_shipped_driver() {
        // 24 bits × 1 µs + 50 µs reset = 74 µs/frame (the ~74 µs blocking
        // window the LED task yields on).
        assert_eq!(BIT_US, 1);
        assert_eq!(PIXEL_US, 24);
        assert_eq!(FRAME_US, 74);
    }

    #[test]
    fn black_pixel_has_no_high_pulse() {
        let f = encode_grb(0, 0, 0);
        assert_eq!(total_high_us(&f.pixel), 0, "black = all zeros = no HIGH");
    }

    #[test]
    fn full_pixel_has_24_high_pulses() {
        let f = encode_grb(255, 255, 255);
        // 24 logic-1 bits → 24 HIGH events of 1 µs each.
        assert_eq!(total_high_us(&f.pixel), 24);
    }

    #[test]
    fn byte_msb_first() {
        // 0xA0 = 0b1010_0000: NON-palindromic, so an MSB-first and an
        // LSB-first encoder produce different event sequences (the previous
        // test byte 0x81 = 0b1000_0001 is palindromic and cannot distinguish
        // them — the test would pass with the encoder reversed). Assert the
        // full per-bit pattern, not just the first/last bits.
        let evs = byte_events(0xA0);
        let highs = [true, false, true, false, false, false, false, false];
        for (i, ev) in evs.iter().enumerate() {
            assert_eq!(ev.high, highs[i], "bit {i} (MSB first) of 0xA0");
            assert_eq!(ev.us, BIT_US);
        }
    }

    #[test]
    fn grb_order_green_first() {
        // encode_grb(255, 0, 0) = pure green: the FIRST wire byte is 255.
        let f = encode_grb(255, 0, 0);
        assert_eq!(total_high_us(&f.pixel[0..8]), 8, "green byte first");
        assert_eq!(total_high_us(&f.pixel[8..16]), 0, "red byte second");
        assert_eq!(total_high_us(&f.pixel[16..24]), 0, "blue byte third");
    }

    #[test]
    fn rgb_to_grb_conversion() {
        // encode_rgb([255,0,0]) = pure RED in config order → on the wire the
        // GREEN byte is 0, the RED byte is 255, the BLUE byte is 0.
        let f = encode_rgb(&[255, 0, 0]);
        assert_eq!(total_high_us(&f.pixel[0..8]), 0, "wire green must be 0");
        assert_eq!(total_high_us(&f.pixel[8..16]), 8, "wire red must be 255");
        assert_eq!(total_high_us(&f.pixel[16..24]), 0, "wire blue must be 0");
    }

    #[test]
    fn rgb_to_grb_green_and_blue() {
        let g = encode_rgb(&[0, 255, 0]);
        assert_eq!(total_high_us(&g.pixel[0..8]), 8);
        assert_eq!(total_high_us(&g.pixel[8..24]), 0);

        let b = encode_rgb(&[0, 0, 255]);
        assert_eq!(total_high_us(&b.pixel[0..16]), 0);
        assert_eq!(total_high_us(&b.pixel[16..24]), 8);
    }

    #[test]
    fn encode_rgb_matches_manual_grb() {
        for (r, g, b) in [(1u8, 2, 3), (255, 0, 128), (10, 200, 0)] {
            assert_eq!(encode_rgb(&[r, g, b]), encode_grb(g, r, b));
        }
    }

    /// Records the exact (level, delay) sequence **in order** so a test can
    /// assert the *order* of the replayed GPIO behavior, not just its totals.
    ///
    /// The shipped driver holds the pin at the *active* level for the 1 µs
    /// (HIGH for a logic-1 bit, LOW for a logic-0 bit). Aggregates alone
    /// (total highs / lows / µs) cannot catch a timing inversion — delaying
    /// while the pin is at the *wrong* level keeps every total identical but
    /// garbles every colour on real hardware. Logging each `(level, µs)` in
    /// sequence order makes such an inversion fail the test.
    #[derive(Default)]
    struct Recorder {
        high: u32,
        low: u32,
        delay_total: u32,
        /// Current pin level, as driven by the most recent `set_high`/`set_low`.
        level: bool,
        /// Every `delay_us` call in order: (pin level during the delay, µs).
        delays: Vec<(bool, u32)>,
    }

    impl PinPulse for Recorder {
        fn set_high(&mut self) {
            self.high += 1;
            self.level = true;
        }
        fn set_low(&mut self) {
            self.low += 1;
            self.level = false;
        }
        fn delay_us(&mut self, us: u32) {
            self.delay_total += us;
            self.delays.push((self.level, us));
        }
    }

    #[test]
    fn replay_matches_shipped_driver_sequence() {
        // Driver per bit:
        //   set_high; if bit { delay(1) }; set_low; if !bit { delay(1) }
        // and per frame: set_low; delay(50).
        //
        // g = 0b1010_0000 is non-palindromic, so its bit order (and hence the
        // order of the delays) is unique — reversing the bits, or delaying at
        // the wrong level, changes `rec.delays` and fails the test. The
        // aggregates below are kept as a cheap sanity net; the per-delay
        // assertion is what actually pins the timing to the correct level.
        let f = encode_grb(0b1010_0000, 0b0000_0001, 0);
        let mut rec = Recorder::default();
        replay_frame(&mut rec, &f);

        // 24 bits → 24 set_high + 24 set_low, plus the final reset set_low.
        assert_eq!(rec.high, 24);
        assert_eq!(rec.low, 25);
        // 24 bits × 1 µs + 50 µs reset.
        assert_eq!(rec.delay_total, 74);

        // Each bit holds the *active* level for 1 µs: logic-1 bits delay while
        // HIGH, logic-0 bits delay while LOW; the 50 µs reset trails a LOW.
        // Reconstructed from the frame's own events (the ground truth being
        // tested), in sequence order.
        let mut expected: Vec<(bool, u32)> = Vec::new();
        for e in &f.pixel {
            if e.high {
                expected.push((true, e.us));
            } else {
                expected.push((false, e.us));
            }
        }
        expected.push((false, f.reset.us));
        assert_eq!(
            rec.delays, expected,
            "delay (level, µs) order must match the shipped bit-bang sequence"
        );
    }

    #[test]
    fn replay_black_and_white_timing() {
        let mut rec = Recorder::default();
        replay_frame(&mut rec, &encode_rgb(&[0, 0, 0]));
        assert_eq!(rec.delay_total, 74, "timing is color-independent");

        let mut rec = Recorder::default();
        replay_frame(&mut rec, &encode_rgb(&[255, 255, 255]));
        assert_eq!(rec.delay_total, 74);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CORRECTED, PROTOCOL-COMPLIANT TIMING MODEL (V2)
// ═══════════════════════════════════════════════════════════════════════════

// The items above this marker are the **legacy** model (one 1 µs phase per
// bit, µs-granularity). They remain for source compatibility but, as
// documented on each item, they produce a non-compliant waveform on real
// hardware. This section is the replacement: nanosecond-resolution, both
// phases of every bit carried explicitly.

/// Width (ns) of the HIGH phase of a logic-`0` bit — datasheet T0H.
///
/// WS2812 datasheet §"Data Format": logic 0 = a ~0.4 µs high pulse followed
/// by a ~0.85 µs low, i.e. T0H = 400 ns with a ±150 ns tolerance window
/// (250..550 ns). The LED samples the bit on the rising edge, so the HIGH
/// phase width is what must stay in-window.
pub const T0H_NS: u32 = 400;

/// Width (ns) of the LOW phase of a logic-`0` bit — datasheet T0L.
///
/// WS2812 datasheet §"Data Format": 850 ns ± 150 ns (700..1000 ns).
pub const T0L_NS: u32 = 850;

/// Width (ns) of the HIGH phase of a logic-`1` bit — datasheet T1H.
///
/// WS2812 datasheet §"Data Format": logic 1 = a ~0.8 µs high pulse followed
/// by a ~0.45 µs low, i.e. T1H = 800 ns with a ±150 ns tolerance window
/// (650..950 ns).
pub const T1H_NS: u32 = 800;

/// Width (ns) of the LOW phase of a logic-`1` bit — datasheet T1L.
///
/// WS2812 datasheet §"Data Format": 450 ns ± 150 ns (300..600 ns).
///
/// NOTE: this LOW phase is load-bearing — it is the falling edge that
/// separates one bit from the next. If it collapses to ~0 ns, consecutive
/// `1` bits merge into one continuous HIGH pulse and the receiver latches
/// fewer than 8 bits per byte (this is the failure mode of the legacy
/// model above).
pub const T1L_NS: u32 = 450;

/// Duration (ns) of one logic-`0` bit: T0H + T0L.
///
/// 400 + 850 = 1250 ns = the datasheet's ~1.25 µs per bit.
pub const T0_TOTAL_NS: u32 = T0H_NS + T0L_NS;

/// Duration (ns) of one logic-`1` bit: T1H + T1L.
///
/// 800 + 450 = 1250 ns = the datasheet's ~1.25 µs per bit.
pub const T1_TOTAL_NS: u32 = T1H_NS + T1L_NS;

/// Minimum reset pulse width (ns): the line must be LOW for at least this
/// long for the LED to latch the frame.
///
/// WS2812 datasheet "Reset" spec: t_RST ≥ 50 µs for the WS2812; the
/// WS2812B spec raises this to t_RST ≥ 280 µs. 50 µs is the minimum the
/// name refers to; see [`RESET_NS`] for the value this crate transmits.
pub const RESET_NS_MIN: u32 = 50_000;

/// Reset pulse width (ns) this crate transmits after each frame.
///
/// 300 µs: satisfies the WS2812B t_RST ≥ 280 µs requirement (and therefore
/// the weaker WS2812 ≥ 50 µs one) with 20 µs margin. It is intentionally
/// larger than the legacy [`RESET_US`] (50 µs) model, which only meets the
/// bare WS2812 minimum and the *shipped* hardware has been confirmed to
/// misbehave with it.
pub const RESET_NS: u32 = 300_000;

/// Total ns to transmit one pixel (24 bits × 1250 ns) — no reset included.
pub const PIXEL_NS: u32 = BITS_PER_PIXEL * T0_TOTAL_NS;

/// Total ns for one full frame: 24 bits + the [`RESET_NS`] reset pulse.
pub const FRAME_NS: u32 = PIXEL_NS + RESET_NS;

/// One bit of the compliant waveform: BOTH phases are carried explicitly,
/// each in nanoseconds.
///
/// A bit is the sequence: pin HIGH for `high_ns`, then pin LOW for
/// `low_ns`. Unlike [`Ws2812Event`] (one µs value, one phase), neither
/// phase may be zero: the falling edge between `low_ns` and the next bit's
/// `high_ns` is what separates bits on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812BitPhase {
    /// HIGH phase width in ns ([`T0H_NS`] for a `0`, [`T1H_NS`] for a `1`).
    pub high_ns: u32,
    /// LOW phase width in ns ([`T0L_NS`] for a `0`, [`T1L_NS`] for a `1`).
    pub low_ns: u32,
}

/// A complete, compliant frame: 24 explicit bit phases followed by the
/// reset LOW pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812FrameV2 {
    /// 24 bit phases in **GRB** wire order (Green byte first, MSB first).
    pub pixel: [Ws2812BitPhase; 24],
    /// Width of the trailing reset LOW pulse in ns ([`RESET_NS`]).
    pub reset_ns: u32,
}

/// Encode one bit as both of its phases (ns).
pub const fn bit_phase(bit: bool) -> Ws2812BitPhase {
    if bit {
        Ws2812BitPhase {
            high_ns: T1H_NS,
            low_ns: T1L_NS,
        }
    } else {
        Ws2812BitPhase {
            high_ns: T0H_NS,
            low_ns: T0L_NS,
        }
    }
}

/// Encode one byte (MSB first) as 8 two-phase bit entries.
pub fn byte_phases(byte: u8) -> [Ws2812BitPhase; 8] {
    let mut out = [Ws2812BitPhase {
        high_ns: 0,
        low_ns: 0,
    }; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = bit_phase((byte >> (7 - i)) & 1 != 0);
    }
    out
}

/// Encode a GRB pixel (g, r, b) as 24 two-phase bit entries, **without** the
/// reset pulse.
pub fn grb_pixel_phases(g: u8, r: u8, b: u8) -> [Ws2812BitPhase; 24] {
    let mut out = [Ws2812BitPhase {
        high_ns: 0,
        low_ns: 0,
    }; 24];
    for (bi, &byte) in [g, r, b].iter().enumerate() {
        out[8 * bi..8 * bi + 8].copy_from_slice(&byte_phases(byte));
    }
    out
}

/// Encode a GRB pixel as a complete compliant frame (24 bit phases + reset).
pub fn encode_grb_v2(g: u8, r: u8, b: u8) -> Ws2812FrameV2 {
    Ws2812FrameV2 {
        pixel: grb_pixel_phases(g, r, b),
        reset_ns: RESET_NS,
    }
}

/// Encode an RGB color (`[R, G, B]`) as a complete compliant frame, applying
/// the RGB→GRB wire-order conversion (same conversion as [`encode_rgb`]).
pub fn encode_rgb_v2(rgb: &[u8; 3]) -> Ws2812FrameV2 {
    encode_grb_v2(rgb[1], rgb[0], rgb[2])
}

/// A pin + delay a compliant frame can be replayed onto.
///
/// Sub-microsecond resolution is REQUIRED: the shortest phase is 400 ns
/// (T0H), which a µs-granularity delay cannot express.
///
/// Firmware contract for `delay_ns`:
/// - wait **at least** `ns` (never less);
/// - overshoot must be bounded well under the ±150 ns bit tolerance — a
///   cycle-counted wait (e.g. a SYSTIMER compare) is the clean
///   implementation;
/// - **`esp_hal::delay::Delay::delay_nanos` is NOT sufficient.** It
///   quantises to whole microseconds (`Duration::from_micros(ns.div_ceil
///   (1000))`), so every one of the four phase widths (400 / 450 / 800 /
///   850 ns) rounds up to 1 µs. A `0`-bit and a `1`-bit would then both be
///   ~1 µs HIGH + ~1 µs LOW — indistinguishable to the receiver, i.e. the
///   protocol is *still* violated (a `0` reads as a `1`). The firmware must
///   therefore provide a genuine sub-microsecond `delay_ns` (SYSTIMER
///   compare or a calibrated cycle-counted wait); µs-granular delays cannot
///   carry this protocol at all.
pub trait PinPulseNs {
    fn set_high(&mut self);
    fn set_low(&mut self);
    /// Wait at least `ns` nanoseconds while the pin stays at its current
    /// level.
    fn delay_ns(&mut self, ns: u32);
}

/// Replay a compliant frame onto a pin: for EVERY bit, emit the HIGH phase
/// and the LOW phase (both timed), then the reset LOW.
///
/// ```text
/// for each bit phase p:
///     pin.set_high(); pin.delay_ns(p.high_ns)
///     pin.set_low();  pin.delay_ns(p.low_ns)
/// pin.set_low(); pin.delay_ns(frame.reset_ns)
/// ```
///
/// Every bit therefore ends in a falling edge, so consecutive `1` bits are
/// always separated by a measurable LOW (≥ T1L) and cannot merge.
pub fn replay_frame_v2<P: PinPulseNs>(pin: &mut P, frame: &Ws2812FrameV2) {
    for p in &frame.pixel {
        pin.set_high();
        pin.delay_ns(p.high_ns);
        pin.set_low();
        pin.delay_ns(p.low_ns);
    }
    pin.set_low();
    pin.delay_ns(frame.reset_ns);
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests for the corrected (V2) model.
//
// The existing `mod tests` above covers the LEGACY path (bit order, GRB
// order, timing inversion) and is deliberately left untouched. This module
// encodes what a REAL WS2812 requires of a waveform:
//   1. both phases present and timed for every bit (no zero-width phase);
//   2. every HIGH phase inside its datasheet window (T0H 250..550 ns or
//      T1H 650..950 ns, the 400/800 ns ±150 ns figures);
//   3. a measurable LOW (≥ 300 ns, the T1L window's lower bound) between
//      consecutive `1` bits so they cannot merge into one continuous HIGH
//      pulse;
//   4. a reset LOW meeting the spec (≥ 50 µs WS2812; ≥ 280 µs WS2812B).
// `protocol_violations` is the single invariant checker; it PASSES on the
// V2 replay. It FAILS on the legacy replay — the three
// `v2_replay_complies_with_ws2812_invariants_*` tests exercise it against a
// compliant replay, and running the same suite with the legacy timings
// (T0H=0/T0L=1000/T1H=1000/T1L=0, reset=50 µs) makes exactly those three
// fail while the bit-order/GRB tests keep passing, proving the invariants
// discriminate the broken waveform from a compliant one.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests_v2 {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Records every timed wait at ns resolution as (pin level during the
    /// wait, duration ns), in order. Implements BOTH pin seams so the same
    /// probe can measure the legacy (µs) and the compliant (ns) replays.
    #[derive(Default)]
    struct NsRecorder {
        level: bool,
        high_calls: u32,
        low_calls: u32,
        /// Every delay call in order: (pin level during it, ns).
        delays: Vec<(bool, u32)>,
    }

    impl NsRecorder {
        fn highs(&self) -> Vec<u32> {
            self.delays
                .iter()
                .filter(|x| x.0)
                .map(|(_, ns)| *ns)
                .collect()
        }
        fn lows(&self) -> Vec<u32> {
            self.delays
                .iter()
                .filter(|x| !x.0)
                .map(|(_, ns)| *ns)
                .collect()
        }
        fn total_ns(&self) -> u64 {
            self.delays.iter().map(|(_, ns)| *ns as u64).sum()
        }
    }

    impl PinPulseNs for NsRecorder {
        fn set_high(&mut self) {
            self.high_calls += 1;
            self.level = true;
        }
        fn set_low(&mut self) {
            self.low_calls += 1;
            self.level = false;
        }
        fn delay_ns(&mut self, ns: u32) {
            self.delays.push((self.level, ns));
        }
    }

    impl PinPulse for NsRecorder {
        fn set_high(&mut self) {
            self.high_calls += 1;
            self.level = true;
        }
        fn set_low(&mut self) {
            self.low_calls += 1;
            self.level = false;
        }
        fn delay_us(&mut self, us: u32) {
            // µs seam, measured in ns.
            self.delays.push((self.level, us.saturating_mul(1000)));
        }
    }

    /// The protocol invariants a real WS2812 receiver needs from one
    /// 24-bit frame replay. Returns a list of human-readable violations
    /// (empty = compliant).
    fn protocol_violations(rec: &NsRecorder) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        let highs = rec.highs();
        let lows = rec.lows();

        // (1) Both phases per bit: 24 timed HIGH phases and 24 timed bit
        // LOW phases, plus the trailing reset LOW = 25 LOW phases total.
        if highs.len() != 24 {
            v.push(format!(
                "expected 24 timed HIGH bit phases, found {} — a bit is missing its HIGH phase",
                highs.len()
            ));
        }
        if lows.len() != 25 {
            v.push(format!(
                "expected 25 timed LOW phases (24 bit + 1 reset), found {} — a bit is missing its LOW phase (bits merge)",
                lows.len()
            ));
        }

        // (2) Every HIGH phase inside a datasheet window: T0H 400±150 or
        // T1H 800±150.
        for (i, ns) in highs.iter().enumerate() {
            if !((250..=550).contains(ns) || (650..=950).contains(ns)) {
                v.push(format!(
                    "bit {i}: HIGH phase {ns} ns is outside the T0H (250..550) and T1H (650..950) windows"
                ));
            }
        }

        // (3) Every bit LOW phase ≥ 300 ns (T1L window lower bound): the
        // falling edge between consecutive `1` bits must be measurable.
        for (i, ns) in lows.iter().enumerate().take(24) {
            if *ns < 300 {
                v.push(format!(
                    "bit {i}: LOW phase {ns} ns < 300 ns — consecutive 1-bits merge on the wire"
                ));
            }
        }

        // (4) Reset LOW: the final (25th) LOW phase.
        if let Some(&reset) = lows.last() {
            if reset < RESET_NS_MIN {
                v.push(format!(
                    "reset {reset} ns < 50 000 ns (WS2812 t_RST minimum)"
                ));
            }
            if reset < 280_000 {
                v.push(format!(
                    "reset {reset} ns < 280 000 ns (WS2812B t_RST spec)"
                ));
            }
        }

        v
    }

    // ── Constants ─────────────────────────────────────────────────────────

    #[test]
    fn v2_constants_match_datasheet() {
        // Nominal values (WS2812 datasheet, "Data Format" table).
        assert_eq!(T0H_NS, 400);
        assert_eq!(T0L_NS, 850);
        assert_eq!(T1H_NS, 800);
        assert_eq!(T1L_NS, 450);
        // Derived per-bit and per-frame totals (24 bits × 1250 ns pixel,
        // + 300 µs reset).
        assert_eq!(T0_TOTAL_NS, 1250);
        assert_eq!(T1_TOTAL_NS, 1250);
        assert_eq!(PIXEL_NS, 30_000);
        assert_eq!(FRAME_NS, 330_000);
        // The nominal values sit at the centre of their ±150 ns windows.
        assert!((250..=550).contains(&T0H_NS));
        assert!((700..=1000).contains(&T0L_NS));
        assert!((650..=950).contains(&T1H_NS));
        assert!((300..=600).contains(&T1L_NS));
        // Reset meets both the WS2812 (≥50 µs) and WS2812B (≥280 µs)
        // specs: pinned to their exact nominal values below (50 000 ns and
        // 300 000 ns > 280 000 ns).
        assert_eq!(RESET_NS_MIN, 50_000);
        assert_eq!(RESET_NS, 300_000);
    }

    // ── Encoding ──────────────────────────────────────────────────────────

    #[test]
    fn v2_zero_bit_phases() {
        let p = bit_phase(false);
        assert_eq!(
            p,
            Ws2812BitPhase {
                high_ns: T0H_NS,
                low_ns: T0L_NS
            }
        );
        assert!(
            p.high_ns > 0 && p.low_ns > 0,
            "both phases of a 0-bit must be timed"
        );
        assert_eq!(p.high_ns + p.low_ns, T0_TOTAL_NS);
    }

    #[test]
    fn v2_one_bit_phases() {
        let p = bit_phase(true);
        assert_eq!(
            p,
            Ws2812BitPhase {
                high_ns: T1H_NS,
                low_ns: T1L_NS
            }
        );
        assert!(
            p.high_ns > 0 && p.low_ns > 0,
            "both phases of a 1-bit must be timed"
        );
        assert_eq!(p.high_ns + p.low_ns, T1_TOTAL_NS);
        // The 1-bit LOW is the bit separator — it must be measurable.
        assert!(p.low_ns >= 300);
    }

    #[test]
    fn v2_byte_msb_first() {
        // 0xA0 = 0b1010_0000 (non-palindromic) — same ordering net as the
        // legacy byte_msb_first, applied to the two-phase encoding.
        let evs = byte_phases(0xA0);
        let one = bit_phase(true);
        let zero = bit_phase(false);
        let expected = [one, zero, one, zero, zero, zero, zero, zero];
        for (i, ev) in evs.iter().enumerate() {
            assert_eq!(*ev, expected[i], "bit {i} (MSB first) of 0xA0");
        }
    }

    #[test]
    fn v2_grb_order_green_first() {
        let f = encode_grb_v2(255, 0, 0);
        let one = bit_phase(true);
        let zero = bit_phase(false);
        assert!(
            f.pixel[0..8].iter().all(|p| *p == one),
            "green byte first, all 1s"
        );
        assert!(
            f.pixel[8..16].iter().all(|p| *p == zero),
            "red byte second, all 0s"
        );
        assert!(
            f.pixel[16..24].iter().all(|p| *p == zero),
            "blue byte third, all 0s"
        );
        assert_eq!(f.reset_ns, RESET_NS);
    }

    #[test]
    fn v2_rgb_to_grb_conversion() {
        // encode_rgb_v2([255,0,0]) = pure RED in config order → wire GRB =
        // 0, 255, 0.
        let f = encode_rgb_v2(&[255, 0, 0]);
        let one = bit_phase(true);
        let zero = bit_phase(false);
        assert!(
            f.pixel[0..8].iter().all(|p| *p == zero),
            "wire green must be 0"
        );
        assert!(
            f.pixel[8..16].iter().all(|p| *p == one),
            "wire red must be 255"
        );
        assert!(
            f.pixel[16..24].iter().all(|p| *p == zero),
            "wire blue must be 0"
        );

        let g = encode_rgb_v2(&[0, 255, 0]);
        assert!(g.pixel[0..8].iter().all(|p| *p == one));
        assert!(g.pixel[8..24].iter().all(|p| *p == zero));

        let b = encode_rgb_v2(&[0, 0, 255]);
        assert!(b.pixel[0..16].iter().all(|p| *p == zero));
        assert!(b.pixel[16..24].iter().all(|p| *p == one));
    }

    #[test]
    fn v2_encode_rgb_matches_manual_grb() {
        for (r, g, b) in [(1u8, 2, 3), (255, 0, 128), (10, 200, 0)] {
            assert_eq!(encode_rgb_v2(&[r, g, b]), encode_grb_v2(g, r, b));
        }
    }

    // ── Replay: the waveform a real LED would see ─────────────────────────

    #[test]
    fn v2_replay_emits_both_phases_per_bit_in_order() {
        // 0xA0 green (non-palindromic) pins the order; a 1 in the red byte
        // makes the delay sequence non-uniform.
        let f = encode_grb_v2(0b1010_0000, 0b0000_0001, 0);
        let mut rec = NsRecorder::default();
        replay_frame_v2(&mut rec, &f);

        // 24 bits → 24 set_high + 24 set_low, plus the final reset set_low.
        assert_eq!(rec.high_calls, 24);
        assert_eq!(rec.low_calls, 25);

        // Exact (level, ns) sequence: both phases of every bit, then reset.
        let mut expected: Vec<(bool, u32)> = Vec::new();
        for p in &f.pixel {
            expected.push((true, p.high_ns));
            expected.push((false, p.low_ns));
        }
        expected.push((false, f.reset_ns));
        assert_eq!(rec.delays, expected);

        // Per-pixel duration = 24 × 1250 ns; full frame = pixel + reset.
        assert_eq!(rec.total_ns() - f.reset_ns as u64, PIXEL_NS as u64);
        assert_eq!(rec.total_ns(), FRAME_NS as u64);
    }

    #[test]
    fn v2_replay_complies_with_ws2812_invariants_black() {
        let mut rec = NsRecorder::default();
        replay_frame_v2(&mut rec, &encode_rgb_v2(&[0, 0, 0]));
        assert!(
            protocol_violations(&rec).is_empty(),
            "black frame must satisfy every WS2812 invariant: {:?}",
            protocol_violations(&rec)
        );
    }

    #[test]
    fn v2_replay_complies_with_ws2812_invariants_white() {
        // White = 24 consecutive 1-bits: the strictest case for the
        // "bits cannot merge" invariant — every bit LOW phase separates two
        // 1-bits.
        let mut rec = NsRecorder::default();
        replay_frame_v2(&mut rec, &encode_rgb_v2(&[255, 255, 255]));
        assert!(
            protocol_violations(&rec).is_empty(),
            "white frame must satisfy every WS2812 invariant: {:?}",
            protocol_violations(&rec)
        );
        // Every one of the 24 bit LOWs is the separator between 1-bits and
        // must be at least the T1L lower tolerance bound.
        let lows = rec.lows();
        for (i, ns) in lows.iter().enumerate().take(24) {
            assert!(*ns >= 300, "1-bit {i} separator LOW {ns} ns < 300 ns");
        }
    }

    #[test]
    fn v2_replay_complies_with_ws2812_invariants_mixed() {
        let mut rec = NsRecorder::default();
        replay_frame_v2(&mut rec, &encode_rgb_v2(&[0xA0, 0x55, 0x81]));
        assert!(
            protocol_violations(&rec).is_empty(),
            "mixed frame must satisfy every WS2812 invariant: {:?}",
            protocol_violations(&rec)
        );
    }

    // ── The legacy model, measured: proof the invariants catch it ─────────
    //
    // These three tests PASS today. Each drives the REAL legacy `replay_frame`
    // (µs seam, one phase per bit) through an ns-resolution recorder and pins
    // exactly how the resulting waveform violates the protocol — a 1-bit with
    // a 0-ns LOW phase, a 0-bit with no timed HIGH phase, a 1 µs (not 1.25 µs)
    // bit period — so the defect is recorded, not just asserted away. They
    // also assert `protocol_violations` is non-empty on that legacy replay,
    // i.e. the invariant checker flags it. (Running the *V2* replay through the
    // same checker yields zero violations; running the V2 suite with the
    // legacy timings back-constant-folded makes the three
    // `v2_replay_complies_with_ws2812_invariants_*` tests fail — the direct
    // proof the invariants reject the old model.)

    #[test]
    fn legacy_replay_has_zero_width_low_phases_for_one_bits() {
        // Legacy white frame: all 24 bits are 1.
        let mut rec = NsRecorder::default();
        replay_frame(&mut rec, &encode_grb(255, 255, 255));
        let highs = rec.highs();
        let lows = rec.lows();
        // Each 1-bit holds HIGH for the legacy 1 µs (outside the T1H
        // 650..950 ns window) and emits NO timed LOW phase: the only timed
        // LOW in the whole frame is the reset. Consecutive 1-bits are
        // separated by zero ns — this is the measured merge failure.
        assert_eq!(highs.len(), 24);
        assert!(
            highs.iter().all(|ns| *ns == 1000),
            "legacy 1-bit HIGH is 1 µs"
        );
        assert_eq!(lows.len(), 1, "legacy: only the reset produces a timed LOW");
        assert_eq!(rec.total_ns(), FRAME_US as u64 * 1000);
        assert!(
            !protocol_violations(&rec).is_empty(),
            "the invariant checker must flag the legacy white replay"
        );
    }

    #[test]
    fn legacy_replay_has_no_timed_high_phase_for_zero_bits() {
        // Legacy black frame: all 24 bits are 0.
        let mut rec = NsRecorder::default();
        replay_frame(&mut rec, &encode_grb(0, 0, 0));
        // Each 0-bit skips the HIGH delay entirely: zero timed HIGH phases
        // where a real WS2812 needs 24 (T0H ≈ 400 ns each).
        assert_eq!(
            rec.highs().len(),
            0,
            "legacy 0-bits emit no timed HIGH phase"
        );
        assert!(
            !protocol_violations(&rec).is_empty(),
            "the invariant checker must flag the legacy black replay"
        );
    }

    #[test]
    fn legacy_replay_one_bit_duration_outside_protocol_window() {
        // Legacy bit period is 1 µs; the datasheet per-bit budget is 1250
        // ns (± tolerance) with both phases timed.
        let mut rec = NsRecorder::default();
        replay_frame(&mut rec, &encode_grb(1, 0, 0)); // single 1-bit, rest 0
                                                      // The one 1-bit: 1000 ns HIGH, 0 ns LOW → period 1000 ns ≠ 1250 ns,
                                                      // and the LOW phase that separates it from neighbours is absent.
        let highs = rec.highs();
        assert_eq!(highs.len(), 1);
        assert!(
            !(650..=950).contains(&highs[0]),
            "legacy 1-bit HIGH outside T1H window"
        );
        let lows: Vec<u32> = rec
            .lows()
            .iter()
            .filter(|ns| **ns < 1000)
            .copied()
            .collect();
        assert!(
            lows.is_empty(),
            "legacy 0-bits emit no timed LOW phase either"
        );
    }
}
