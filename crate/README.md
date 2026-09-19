# alacrs

Apple Lossless (ALAC) decoder in pure Rust. No dependencies, no build script,
no C.

```rust
use alacrs::Mp4Alac;

let mut track = Mp4Alac::open(std::fs::read("song.m4a")?).expect("an alac track");
let width = track.bytes_per_sample();
let mut out = vec![0u8; 4096 * width];

// only the frames holding these samples are decoded
let written = track.decode_into(44_100 * 60, 4096, &mut out);
```

`Mp4Alac` reads the MP4 container, finds the ALAC track and decodes any stretch
of it. `Alac` decodes bare frames for callers that already have them, such as
an AirPlay stream.

Sixteen and twenty four bit samples are decoded, in one or two channels. Twenty
and thirty two bit, and more than two channels, answer with nothing rather than
an error.

A frame that has been cut short reads as zeros past its end. A frame that
states a sample count larger than the buffers hold is clamped to what they
hold, and a run of zeros that would run past the end of a buffer stops at the
end.

## Testing

```
cargo test
```

`tests/vectors.rs` holds six frames of a real ALAC stream and the samples each
has to decode to, byte for byte.

This crate is what [@mgz-dev/alac](https://www.npmjs.com/package/@mgz-dev/alac)
is built from, through WebAssembly.
