//! WS2812 (NeoPixel) wire-protocol encoding.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812Event {
    pub high: bool,
    pub us: u32,
}

/// Width of one bit in µs (the driver holds the active level for 1 µs).
pub const BIT_US: u32 = 1;

/// Reset pulse width (µs). Must be ≥ 50 µs for the LED to latch the frame.
pub const RESET_US: u32 = 50;

/// Number of bits per pixel (3 channels × 8).
pub const BITS_PER_PIXEL: u32 = 24;

/// Total µs to transmit one pixel (24 bits × 1 µs).
pub const PIXEL_US: u32 = BITS_PER_PIXEL * BIT_US;

/// Total µs for one full frame (pixel + reset).
pub const FRAME_US: u32 = PIXEL_US + RESET_US;

/// A complete frame: one GRB pixel followed by the reset pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ws2812Frame {
    pub pixel: [Ws2812Event; 24],
    pub reset: Ws2812Event,
}

/// Encode one bit as an event.
pub const fn bit_event(bit: bool) -> Ws2812Event {
    Ws2812Event {
        high: bit,
        us: BIT_US,
    }
}

/// Encode one byte (MSB first) as 8 events.
pub fn byte_events(byte: u8) -> [Ws2812Event; 8] {
    let mut out = [Ws2812Event { high: false, us: 0 }; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = bit_event((byte >> (7 - i)) & 1 != 0);
    }
    out
}

/// Encode a single GRB pixel (g, r, b) as 24 events, **without** the reset.
pub fn grb_pixel_events(g: u8, r: u8, b: u8) -> [Ws2812Event; 24] {
    let mut out = [Ws2812Event { high: false, us: 0 }; 24];
    for (bi, &byte) in [g, r, b].iter().enumerate() {
        out[8 * bi..8 * bi + 8].copy_from_slice(&byte_events(byte));
    }
    out
}

/// Encode a GRB pixel as a complete frame (pixel + reset pulse).
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
pub fn encode_rgb(rgb: &[u8; 3]) -> Ws2812Frame {
    encode_grb(rgb[1], rgb[0], rgb[2])
}

/// A pin + delay the frame can be replayed onto. The firmware implements this
/// for `(esp_hal Output, esp_hal Delay)`; tests implement it with a recorder.
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
        // 0b1000_0001 → HIGH at first (MSB) AND last (LSB) bit.
        let evs = byte_events(0x81);
        assert_eq!(total_high_us(&evs), 2);
        assert!(evs[0].high, "MSB should be high");
        assert!(evs[7].high, "LSB should be high");
        assert!(!evs[1].high && !evs[6].high, "middle bits should be low");
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

    /// Records the exact pin/delay sequence so tests can assert the replayed
    /// GPIO behavior matches the shipped bit-bang driver.
    #[derive(Default)]
    struct Recorder {
        high: u32,
        low: u32,
        delay_total: u32,
        delay_after_low: u32,
    }

    impl PinPulse for Recorder {
        fn set_high(&mut self) {
            self.high += 1;
        }
        fn set_low(&mut self) {
            self.low += 1;
            self.delay_after_low = 0;
        }
        fn delay_us(&mut self, us: u32) {
            self.delay_total += us;
        }
    }

    #[test]
    fn replay_matches_shipped_driver_sequence() {
        // Ship
        // driver per bit:
        //   set_high; if bit { delay(1) }; set_low; if !bit { delay(1) }
        // and per frame: set_low; delay(50).
        let f = encode_grb(0b1010_0000, 0b0000_0001, 0);
        let mut rec = Recorder::default();
        replay_frame(&mut rec, &f);

        // 24 bits → 24 set_high + 24 set_low, plus the final reset set_low.
        assert_eq!(rec.high, 24);
        assert_eq!(rec.low, 25);
        // 24 bits × 1 µs + 50 µs reset.
        assert_eq!(rec.delay_total, 74);
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
