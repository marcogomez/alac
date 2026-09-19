//! Decodes the ALAC track of an MP4 file to raw little endian samples.
//!
//! ```text
//! cargo run --release --example decode -- song.m4a song.pcm
//! ```

use std::env::args;
use std::fs;
use std::process::exit;

use alacrs::Mp4Alac;

/// How many samples are decoded in one go.
const BATCH: u32 = 4096;

fn main() {
    let mut given = args().skip(1);
    let (Some(from), Some(to)) = (given.next(), given.next()) else {
        eprintln!("usage: decode <file.m4a> <out.pcm>");
        exit(2);
    };

    let file = fs::read(&from).unwrap_or_else(|error| {
        eprintln!("{from} could not be read, {error}");
        exit(1);
    });
    let Some(mut track) = Mp4Alac::open(file) else {
        eprintln!("{from} holds no alac track this can read");
        exit(1);
    };

    let config = *track.config();
    let total = track.total_samples();
    eprintln!(
        "rate {} channels {} bits {} samples {} frame {}",
        config.sample_rate,
        config.channels,
        config.sample_size,
        total,
        config.max_samples_per_frame
    );

    let width = track.bytes_per_sample();
    let mut out = Vec::with_capacity(total as usize * width);
    let mut room = vec![0u8; BATCH as usize * width];
    let mut at = 0u32;
    while at < total {
        let wanted = BATCH.min(total - at);
        let written = track.decode_into(at, wanted, &mut room[..wanted as usize * width]);
        if written == 0 {
            break;
        }
        out.extend_from_slice(&room[..written]);
        at += wanted;
    }

    eprintln!("decoded {} bytes", out.len());
    fs::write(&to, &out).unwrap_or_else(|error| {
        eprintln!("{to} could not be written, {error}");
        exit(1);
    });
}
