//! Apple Lossless (ALAC) decoder.
//!
//! [`Alac`] decodes one frame of an ALAC stream at a time and answers with
//! little endian samples. It reads no container, so the caller supplies the
//! frames and the values from the magic cookie that describe them.
//!
//! [`Mp4Alac`] reads an MP4 file, finds its ALAC track and decodes any stretch
//! of it.
//!
//! ```no_run
//! use alacrs::{Alac, Config};
//!
//! // the numbers below come from the magic cookie of the stream
//! let mut decoder = Alac::new(Config {
//!     max_samples_per_frame: 4096,
//!     sample_size: 16,
//!     channels: 2,
//!     ..Config::default()
//! });
//! let frame: &[u8] = &[];
//! let samples = decoder.decode(frame);
//! # let _ = samples;
//! ```

mod bits;
mod decode;
mod mp4;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub use mp4::Mp4Alac;

use decode::Buffers;

/// What the stream states about itself, the values an ALAC magic cookie holds
/// in the order it holds them. The fields with no known meaning are named by
/// their offset in the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// How many samples a frame holds, unless the frame states otherwise.
    pub max_samples_per_frame: u32,
    /// A byte of the cookie with no known meaning.
    pub v7a: u8,
    /// Bits per sample. 16 and 24 are decoded, 20 and 32 are not.
    pub sample_size: u8,
    pub rice_history_mult: u8,
    pub rice_initial_history: u8,
    pub rice_k_modifier: u8,
    /// A byte of the cookie with no known meaning.
    pub v7f: u8,
    /// A pair of bytes of the cookie with no known meaning.
    pub v80: u16,
    /// The largest a frame gets, in bytes.
    pub max_frame_bytes: u32,
    /// The average number of bits a second, as the stream was written.
    pub average_bit_rate: u32,
    /// Samples a second.
    pub sample_rate: u32,
    /// How many channels a frame holds.
    pub channels: usize,
}

impl Default for Config {
    /// The values an AirPlay stream uses. A stream from a file states its own
    /// in its magic cookie.
    fn default() -> Self {
        Config {
            max_samples_per_frame: 352,
            v7a: 0,
            sample_size: 16,
            rice_history_mult: 40,
            rice_initial_history: 10,
            rice_k_modifier: 14,
            v7f: 2,
            v80: 255,
            max_frame_bytes: 0,
            average_bit_rate: 0,
            sample_rate: 44100,
            channels: 2,
        }
    }
}

/// How long an ALAC magic cookie is, the four bytes of version and flags an
/// MP4 box starts with and the twenty four bytes of values after them.
const COOKIE_BYTES: usize = 28;

impl Config {
    /// Reads the ALAC magic cookie an MP4 file keeps in its sample
    /// description, and answers `None` for anything too short to hold one.
    ///
    /// The bytes wanted here are the contents of the `alac` box inside the
    /// sample entry, version and flags included.
    pub fn from_magic_cookie(cookie: &[u8]) -> Option<Config> {
        if cookie.len() < COOKIE_BYTES {
            return None;
        }
        let word = |at: usize| -> u32 {
            u32::from_be_bytes([cookie[at], cookie[at + 1], cookie[at + 2], cookie[at + 3]])
        };
        let channels = cookie[13] as usize;
        Some(Config {
            max_samples_per_frame: word(4),
            v7a: cookie[8],
            sample_size: cookie[9],
            rice_history_mult: cookie[10],
            rice_initial_history: cookie[11],
            rice_k_modifier: cookie[12],
            v7f: cookie[13],
            v80: u16::from_be_bytes([cookie[14], cookie[15]]),
            max_frame_bytes: word(16),
            average_bit_rate: word(20),
            sample_rate: word(24),
            channels: if channels == 0 { 2 } else { channels },
        })
    }

    /// How many bytes one sample of every channel takes once decoded.
    pub fn bytes_per_sample(&self) -> usize {
        (self.sample_size as usize / 8) * self.channels
    }
}

/// An ALAC decoder.
///
/// It holds the buffers the decoding works in, so decoding a stream frame by
/// frame allocates nothing beyond the answer for each frame.
pub struct Alac {
    config: Config,
    buffers: Buffers,
}

impl Alac {
    /// A decoder for a stream the config describes.
    pub fn new(config: Config) -> Self {
        let buffers = Buffers::new(config.max_samples_per_frame);
        Alac { config, buffers }
    }

    /// What the decoder was told about the stream.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Decodes one frame into little endian samples, the channels one after
    /// another for each point in time.
    ///
    /// A frame that names a channel count the decoder does not handle, or a
    /// sample size of 20 or 32, gives an empty answer rather than an error.
    pub fn decode(&mut self, frame: &[u8]) -> Vec<u8> {
        decode::decode_frame(&self.config, &mut self.buffers, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::decode::{count_leading_zeros, sign_extend24, sign_extended32};
    use super::*;

    #[test]
    fn sign_extend_24_fills_from_bit_twenty_three() {
        let cases: [(i32, i32); 7] = [
            (0x000000, 0),
            (0x000001, 1),
            (0x7FFFFF, 8_388_607),
            (0x800000, -8_388_608),
            (0xFFFFFF, -1),
            (0x800001, -8_388_607),
            (0xFFFFFE, -2),
        ];
        for (input, want) in cases {
            assert_eq!(sign_extend24(input), want, "sign_extend24(0x{input:06X})");
        }
    }

    #[test]
    fn sign_extend_32_fills_from_the_named_bit() {
        assert_eq!(sign_extended32(0b0111, 4), 7);
        assert_eq!(sign_extended32(0b1000, 4), -8);
        assert_eq!(sign_extended32(1, 32), 1);
        assert_eq!(sign_extended32(-1, 32), -1);
    }

    #[test]
    fn leading_zeros_counts_a_thirty_two_bit_value() {
        assert_eq!(count_leading_zeros(0), 32);
        assert_eq!(count_leading_zeros(1), 31);
        assert_eq!(count_leading_zeros(3), 30);
        assert_eq!(count_leading_zeros(0xFF), 24);
        assert_eq!(count_leading_zeros(0x0100_0000), 7);
        assert_eq!(count_leading_zeros(0x8000_0000), 0);
    }

    #[test]
    fn a_default_config_is_the_airplay_one() {
        let config = Config::default();
        assert_eq!(config.max_samples_per_frame, 352);
        assert_eq!(config.sample_size, 16);
        assert_eq!(config.channels, 2);
        assert_eq!(config.bytes_per_sample(), 4);
    }

    #[test]
    fn a_frame_with_a_channel_count_we_cannot_read_gives_nothing() {
        let mut decoder = Alac::new(Config::default());
        // the first three bits name the channel count, and 7 means eight
        assert!(decoder.decode(&[0xE0, 0, 0, 0]).is_empty());
    }

    #[test]
    fn a_frame_cut_short_does_not_bring_the_decoder_down() {
        let mut decoder = Alac::new(Config::default());
        let _ = decoder.decode(&[0x20]);
        let _ = decoder.decode(&[]);
        let _ = decoder.decode(&[0x20, 0x00, 0x00, 0x04]);
    }
}
