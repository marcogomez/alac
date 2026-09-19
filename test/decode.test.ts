import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { closeTrack, decodeTrack, openTrack } from "../src/index";

const fixtures = join(dirname(fileURLToPath(import.meta.url)), "fixtures");

/**
 * the audio the fixtures were encoded from, a second of a two channel sweep.
 * decoding the alac fixture has to give these bytes back without a difference.
 */
function tone(): Buffer {
  const rate = 44100;
  const frames = rate;
  const pcm = Buffer.alloc(frames * 4);
  for (let at = 0; at < frames; at += 1) {
    const seconds = at / rate;
    const hz = 220 + 660 * seconds;
    pcm.writeInt16LE(Math.round(Math.sin(2 * Math.PI * hz * seconds) * 12000), at * 4);
    pcm.writeInt16LE(Math.round(Math.sin(2 * Math.PI * hz * seconds * 1.5) * 9000), at * 4 + 2);
  }
  return pcm;
}

/** the bytes of one of the fixtures */
function fixture(name: string): Buffer {
  return readFileSync(join(fixtures, name));
}

describe("openTrack", () => {
  it("reads what the stream says about itself", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    expect(track?.sampleRate).toBe(44100);
    expect(track?.channels).toBe(2);
    expect(track?.bits).toBe(16);
    expect(track?.samples).toBe(44100);
    expect(track?.bytesPerSample).toBe(4);
    closeTrack(track!);
  });

  it("answers null for a file with no alac track", () => {
    expect(openTrack(fixture("tone-aac.m4a"))).toBeNull();
  });

  it("answers null for bytes that are not an mp4 at all", () => {
    expect(openTrack(Buffer.from("this is not an mp4 file"))).toBeNull();
  });

  it("answers null for an empty file", () => {
    expect(openTrack(Buffer.alloc(0))).toBeNull();
  });
});

describe("decodeTrack", () => {
  it("gives the source audio back byte for byte, since alac is lossless", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    const decoded = Buffer.from(decodeTrack(track!, 0, track!.samples));
    expect(decoded.length).toBe(tone().length);
    expect(decoded.equals(tone())).toBe(true);
    closeTrack(track!);
  });

  it("decodes a stretch out of the middle to the same bytes as the full decode", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    const all = Buffer.from(decodeTrack(track!, 0, track!.samples));

    const from = 20000;
    const wanted = 4096;
    const piece = Buffer.from(decodeTrack(track!, from, wanted));
    expect(piece.length).toBe(wanted * track!.bytesPerSample);
    expect(piece.equals(all.subarray(from * track!.bytesPerSample, (from + wanted) * track!.bytesPerSample))).toBe(true);
    closeTrack(track!);
  });

  it("decodes a stretch that runs past the end without going over", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    const from = track!.samples - 100;
    const piece = Buffer.from(decodeTrack(track!, from, 4096));
    expect(piece.length).toBeLessThanOrEqual(4096 * track!.bytesPerSample);
    expect(piece.length).toBeGreaterThan(0);
    closeTrack(track!);
  });

  it("gives nothing for a point in time past the end", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    expect(decodeTrack(track!, track!.samples + 1000, 1024).length).toBe(0);
    closeTrack(track!);
  });

  it("gives nothing when no samples are asked for", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    expect(decodeTrack(track!, 0, 0).length).toBe(0);
    closeTrack(track!);
  });
});

describe("closeTrack", () => {
  it("refuses to decode a closed track", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    closeTrack(track!);
    expect(() => decodeTrack(track!, 0, 1024)).toThrow("the track is closed");
  });

  it("does nothing when closing a track twice", () => {
    const track = openTrack(fixture("tone-alac.m4a"));
    expect(track).not.toBeNull();
    closeTrack(track!);
    expect(() => closeTrack(track!)).not.toThrow();
  });
});

describe("the module", () => {
  it("serves two tracks at once without them treading on each other", () => {
    const first = openTrack(fixture("tone-alac.m4a"));
    const second = openTrack(fixture("tone-alac.m4a"));
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();

    const fromFirst = Buffer.from(decodeTrack(first!, 1000, 2048));
    const fromSecond = Buffer.from(decodeTrack(second!, 1000, 2048));
    expect(fromFirst.equals(fromSecond)).toBe(true);

    closeTrack(first!);
    // the second one still decodes the same after the first was let go
    expect(Buffer.from(decodeTrack(second!, 1000, 2048)).equals(fromFirst)).toBe(true);
    closeTrack(second!);
  });
});
