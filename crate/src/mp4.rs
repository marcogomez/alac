//! Finding ALAC frames inside an MP4 file.
//!
//! An `.m4a` is a tree of boxes. The audio itself is one long run of bytes in
//! the `mdat` box, and the tables that record where each frame of it starts,
//! how long it is and how many samples it holds are spread across five boxes
//! under `stbl`. This reads those tables once and then hands out the frames.
//!
//! Only what ALAC playback needs is read. Nothing here decodes video, and a
//! file with no ALAC track is turned away.

use crate::{Alac, Config};

/// Where one encoded frame is in the file.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// Where the frame starts in the file.
    at: u64,
    /// How many bytes it takes.
    bytes: u32,
    /// The sample this frame's audio starts at.
    first_sample: u32,
    /// How many samples it holds.
    samples: u32,
}

/// A four character box name.
type Name = [u8; 4];

/// Reads a big endian number of the given width.
fn be(data: &[u8], at: usize, width: usize) -> u64 {
    let mut value: u64 = 0;
    for step in 0..width {
        value = (value << 8) | u64::from(*data.get(at + step).unwrap_or(&0));
    }
    value
}

/// Reads a big endian thirty two bit number.
fn be32(data: &[u8], at: usize) -> u32 {
    be(data, at, 4) as u32
}

/// Walks the boxes of a stretch of file, calling `visit` for each one with its
/// name and its contents.
fn each_box<F: FnMut(Name, usize, usize)>(data: &[u8], from: usize, to: usize, mut visit: F) {
    let mut at = from;
    while at + 8 <= to {
        let stated = be32(data, at) as usize;
        let mut header = 8;
        let mut size = stated;
        if stated == 1 {
            // a size of one means the real size is the sixty four bit number
            // that follows the name
            size = be(data, at + 8, 8) as usize;
            header = 16;
        } else if stated == 0 {
            // a size of zero means the box runs to the end
            size = to - at;
        }
        if size < header || at + size > to {
            return;
        }
        let name: Name = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        visit(name, at + header, at + size);
        at += size;
    }
}

/// Finds one box among the children of a stretch of file.
fn find_box(data: &[u8], from: usize, to: usize, wanted: Name) -> Option<(usize, usize)> {
    let mut found = None;
    each_box(data, from, to, |name, body, end| {
        if found.is_none() && name == wanted {
            found = Some((body, end));
        }
    });
    found
}

/// Follows a path of box names down the tree.
fn find_path(data: &[u8], from: usize, to: usize, path: &[Name]) -> Option<(usize, usize)> {
    let mut span = (from, to);
    for name in path {
        span = find_box(data, span.0, span.1, *name)?;
    }
    Some(span)
}

/// The tables under `stbl` that record where the frames are.
struct Tables {
    /// How many samples each frame holds, one entry per frame.
    samples_per_frame: Vec<u32>,
    /// How many bytes each frame takes, one entry per frame.
    bytes_per_frame: Vec<u32>,
    /// Where each chunk of frames starts in the file.
    chunk_offsets: Vec<u64>,
    /// Runs of chunks that hold the same number of frames each.
    chunk_runs: Vec<(u32, u32)>,
}

/// Reads the sample count of each frame out of `stts`.
fn read_stts(data: &[u8], body: usize, end: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let count = be32(data, body + 4) as usize;
    for entry in 0..count {
        let at = body + 8 + entry * 8;
        if at + 8 > end {
            break;
        }
        let repeats = be32(data, at);
        let samples = be32(data, at + 4);
        for _ in 0..repeats {
            out.push(samples);
        }
    }
    out
}

/// Reads the byte size of each frame out of `stsz`.
fn read_stsz(data: &[u8], body: usize, end: usize) -> Vec<u32> {
    let shared = be32(data, body + 4);
    let count = be32(data, body + 8) as usize;
    if shared != 0 {
        return vec![shared; count];
    }
    let mut out = Vec::with_capacity(count);
    for entry in 0..count {
        let at = body + 12 + entry * 4;
        if at + 4 > end {
            break;
        }
        out.push(be32(data, at));
    }
    out
}

/// Reads where each chunk starts out of `stco` or `co64`.
fn read_chunk_offsets(data: &[u8], body: usize, end: usize, wide: bool) -> Vec<u64> {
    let width = if wide { 8 } else { 4 };
    let count = be32(data, body + 4) as usize;
    let mut out = Vec::with_capacity(count);
    for entry in 0..count {
        let at = body + 8 + entry * width;
        if at + width > end {
            break;
        }
        out.push(be(data, at, width));
    }
    out
}

/// Reads how many frames each chunk holds out of `stsc`.
fn read_stsc(data: &[u8], body: usize, end: usize) -> Vec<(u32, u32)> {
    let count = be32(data, body + 4) as usize;
    let mut out = Vec::with_capacity(count);
    for entry in 0..count {
        let at = body + 8 + entry * 12;
        if at + 12 > end {
            break;
        }
        out.push((be32(data, at), be32(data, at + 4)));
    }
    out
}

/// Turns the tables into one entry per frame, with where it is and what it
/// holds.
fn build_frames(tables: &Tables) -> Vec<Frame> {
    let total = tables.bytes_per_frame.len();
    let mut frames = Vec::with_capacity(total);

    let mut index = 0usize;
    let mut sample = 0u32;
    for (run, (first_chunk, per_chunk)) in tables.chunk_runs.iter().enumerate() {
        // a run lasts until the chunk the next run starts at, or to the end
        let last_chunk = match tables.chunk_runs.get(run + 1) {
            Some((next, _)) => (*next).saturating_sub(1) as usize,
            None => tables.chunk_offsets.len(),
        };
        let from_chunk = (*first_chunk).max(1) as usize - 1;
        for chunk in from_chunk..last_chunk.min(tables.chunk_offsets.len()) {
            let mut at = tables.chunk_offsets[chunk];
            for _ in 0..*per_chunk {
                if index >= total {
                    return frames;
                }
                let bytes = tables.bytes_per_frame[index];
                let samples = tables.samples_per_frame.get(index).copied().unwrap_or(0);
                frames.push(Frame {
                    at,
                    bytes,
                    first_sample: sample,
                    samples,
                });
                at += u64::from(bytes);
                sample = sample.saturating_add(samples);
                index += 1;
            }
        }
    }

    frames
}

/// An ALAC track of an MP4 file, opened and ready to decode from.
pub struct Mp4Alac {
    data: Vec<u8>,
    frames: Vec<Frame>,
    decoder: Alac,
    total_samples: u32,
}

impl Mp4Alac {
    /// Opens the ALAC track of an MP4 file, or answers `None` when the file
    /// holds no ALAC track this can read.
    pub fn open(data: Vec<u8>) -> Option<Mp4Alac> {
        let end = data.len();
        let (moov_body, moov_end) = find_box(&data, 0, end, *b"moov")?;

        // a file can hold several tracks, and the first one with an alac
        // sample description is the one wanted
        let mut found: Option<(Config, Tables)> = None;
        each_box(&data, moov_body, moov_end, |name, body, trak_end| {
            if found.is_some() || name != *b"trak" {
                return;
            }
            let Some((stbl, stbl_end)) =
                find_path(&data, body, trak_end, &[*b"mdia", *b"minf", *b"stbl"])
            else {
                return;
            };
            let Some(config) = read_alac_config(&data, stbl, stbl_end) else {
                return;
            };

            let Some((stts, stts_end)) = find_box(&data, stbl, stbl_end, *b"stts") else {
                return;
            };
            let Some((stsz, stsz_end)) = find_box(&data, stbl, stbl_end, *b"stsz") else {
                return;
            };
            let Some((stsc, stsc_end)) = find_box(&data, stbl, stbl_end, *b"stsc") else {
                return;
            };
            let offsets = match find_box(&data, stbl, stbl_end, *b"stco") {
                Some((body, end)) => read_chunk_offsets(&data, body, end, false),
                None => match find_box(&data, stbl, stbl_end, *b"co64") {
                    Some((body, end)) => read_chunk_offsets(&data, body, end, true),
                    None => return,
                },
            };

            found = Some((
                config,
                Tables {
                    samples_per_frame: read_stts(&data, stts, stts_end),
                    bytes_per_frame: read_stsz(&data, stsz, stsz_end),
                    chunk_offsets: offsets,
                    chunk_runs: read_stsc(&data, stsc, stsc_end),
                },
            ));
        });

        let (config, tables) = found?;
        let frames = build_frames(&tables);
        if frames.is_empty() {
            return None;
        }
        let last = frames[frames.len() - 1];
        let total_samples = last.first_sample.saturating_add(last.samples);

        Some(Mp4Alac {
            data,
            frames,
            decoder: Alac::new(config),
            total_samples,
        })
    }

    /// What the stream states about itself.
    pub fn config(&self) -> &Config {
        self.decoder.config()
    }

    /// How many samples the track holds, counting one sample as one point in
    /// time across every channel.
    pub fn total_samples(&self) -> u32 {
        self.total_samples
    }

    /// How many bytes one sample of every channel takes once decoded.
    pub fn bytes_per_sample(&self) -> usize {
        self.decoder.config().bytes_per_sample()
    }

    /// The frame holding a sample, found by halving the list rather than
    /// walking it, since a long track holds tens of thousands of frames.
    fn frame_holding(&self, sample: u32) -> usize {
        let mut low = 0usize;
        let mut high = self.frames.len();
        while low + 1 < high {
            let middle = (low + high) / 2;
            if self.frames[middle].first_sample <= sample {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }

    /// Decodes samples into `out`, starting at `from` and giving up to
    /// `wanted` samples. Answers how many bytes were written.
    ///
    /// Only the frames that hold those samples are decoded, so reading from
    /// the middle of a track costs the same as reading from the front.
    pub fn decode_into(&mut self, from: u32, wanted: u32, out: &mut [u8]) -> usize {
        if wanted == 0 || from >= self.total_samples {
            return 0;
        }
        let width = self.bytes_per_sample();
        // the answer stops at `wanted` samples even when the buffer has room
        // for more, so the buffer's size never changes what is asked for
        let limit = (wanted as usize).saturating_mul(width).min(out.len());
        let mut written = 0usize;
        let mut index = self.frame_holding(from);

        while index < self.frames.len() && written < limit {
            let frame = self.frames[index];
            let at = frame.at as usize;
            let to = at.saturating_add(frame.bytes as usize);
            if to > self.data.len() {
                break;
            }

            let samples = self.decoder.decode(&self.data[at..to]);

            // the first frame usually starts before the sample asked for, and
            // the part of it before that sample is dropped
            let skip = from.saturating_sub(frame.first_sample) as usize * width;
            if skip < samples.len() {
                let piece = &samples[skip..];
                let room = limit - written;
                let taking = piece.len().min(room);
                out[written..written + taking].copy_from_slice(&piece[..taking]);
                written += taking;
            }

            let reached = frame.first_sample.saturating_add(frame.samples);
            if reached >= from.saturating_add(wanted) {
                break;
            }
            index += 1;
        }

        written
    }
}

/// Reads the ALAC magic cookie out of a sample description.
fn read_alac_config(data: &[u8], stbl: usize, stbl_end: usize) -> Option<Config> {
    let (stsd, stsd_end) = find_box(data, stbl, stbl_end, *b"stsd")?;
    // four bytes of version and flags, then the number of entries, then the
    // entries themselves, which are boxes
    let entries = stsd + 8;

    let mut config = None;
    each_box(data, entries, stsd_end, |name, body, entry_end| {
        if config.is_some() || name != *b"alac" {
            return;
        }
        // a sound sample entry holds twenty eight bytes of its own before the
        // boxes inside it start, and the box header took eight of those
        let inside = body + 28;
        if inside >= entry_end {
            return;
        }
        // the cookie is either a box of its own inside the entry, or wrapped
        // in a `wave` box with a `frma` beside it
        let holder = match find_box(data, inside, entry_end, *b"wave") {
            Some((wave, wave_end)) => (wave, wave_end),
            None => (inside, entry_end),
        };
        if let Some((cookie, cookie_end)) = find_box(data, holder.0, holder.1, *b"alac") {
            config = Config::from_magic_cookie(&data[cookie..cookie_end]);
        }
    });
    config
}
