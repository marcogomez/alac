//! The bindings of the WebAssembly build.
//!
//! Every value crossing the boundary is copied by wasm-bindgen, so the caller
//! hands over a byte array and gets one back and never holds a pointer into
//! the module's memory, which growing that memory would leave dangling.

use wasm_bindgen::prelude::wasm_bindgen;

use crate::Mp4Alac;

/// An ALAC track of an MP4 file, opened and ready to decode from.
#[wasm_bindgen]
pub struct AlacTrack {
    inner: Mp4Alac,
}

#[wasm_bindgen]
impl AlacTrack {
    /// Samples a second.
    #[wasm_bindgen(getter)]
    pub fn sample_rate(&self) -> u32 {
        self.inner.config().sample_rate
    }

    /// How many channels the track holds.
    #[wasm_bindgen(getter)]
    pub fn channels(&self) -> u32 {
        self.inner.config().channels as u32
    }

    /// Bits per sample, per channel.
    #[wasm_bindgen(getter)]
    pub fn bits(&self) -> u32 {
        u32::from(self.inner.config().sample_size)
    }

    /// How many samples the track holds, counting one sample as one point in
    /// time across every channel.
    #[wasm_bindgen(getter)]
    pub fn total_samples(&self) -> u32 {
        self.inner.total_samples()
    }

    /// How many bytes one sample of every channel takes once decoded.
    #[wasm_bindgen(getter)]
    pub fn bytes_per_sample(&self) -> u32 {
        self.inner.bytes_per_sample() as u32
    }

    /// Decodes up to `wanted` samples starting at `from`, and answers the
    /// little endian bytes of them.
    ///
    /// Only the frames holding those samples are decoded, so reading from the
    /// middle of a track costs the same as reading from the front.
    pub fn decode(&mut self, from: u32, wanted: u32) -> Vec<u8> {
        let width = self.inner.bytes_per_sample();
        // no more is asked for than the track holds past `from`, so a caller
        // cannot make the module allocate for samples that do not exist
        let wanted = wanted.min(self.inner.total_samples().saturating_sub(from));
        let mut out = vec![0u8; wanted as usize * width];
        let written = self.inner.decode_into(from, wanted, &mut out);
        out.truncate(written);
        out
    }
}

/// Opens the ALAC track of an MP4 file, or answers nothing when the file holds
/// no ALAC track this can read.
#[wasm_bindgen]
pub fn open(file: &[u8]) -> Option<AlacTrack> {
    Mp4Alac::open(file.to_vec()).map(|inner| AlacTrack { inner })
}
