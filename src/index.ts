/*
 * apple lossless decoding.
 *
 * the decoder is the rust crate in crate/, built to webassembly by
 * scripts/build-wasm.mjs and kept in src/generated as base64. it takes an mp4
 * file, finds the alac track in it, and decodes any stretch of that track.
 *
 * every value crossing into or out of the module is copied by the bindings,
 * so nothing here holds a pointer into the module's memory.
 */

import { initSync, open } from "./generated/alac-glue.js";
import { ALAC_WASM } from "./generated/alac-wasm.js";

import type { AlacTrack } from "./generated/alac-glue.js";

/** true once the module has been started, since starting it twice is an error */
let started = false;

/** turns the base64 the module is kept as into bytes, in node or in a browser */
function moduleBytes(): Uint8Array {
  if (typeof Buffer !== "undefined") {
    return Buffer.from(ALAC_WASM, "base64");
  }
  const text = atob(ALAC_WASM);
  const bytes = new Uint8Array(text.length);
  for (let at = 0; at < text.length; at += 1) {
    bytes[at] = text.charCodeAt(at);
  }
  return bytes;
}

/** starts the module, once */
function start(): void {
  if (started) {
    return;
  }
  initSync({ module: moduleBytes() });
  started = true;
}

/** an open alac track */
export interface Track {
  sampleRate: number;
  channels: number;
  bits: number;
  /** how many points in time the track holds */
  samples: number;
  /** how many bytes one point in time takes across every channel */
  bytesPerSample: number;
}

/** the decoder behind each open track. a closed track has no entry */
const handles = new WeakMap<Track, AlacTrack>();

/**
 * opens the alac track of an mp4 file, or answers null when the file holds
 * none, which is what an aac file in the same kind of container does.
 */
export function openTrack(file: Uint8Array): Track | null {
  start();
  const handle = open(file);
  if (handle === undefined) {
    return null;
  }
  const track: Track = {
    sampleRate: handle.sample_rate,
    channels: handle.channels,
    bits: handle.bits,
    samples: handle.total_samples,
    bytesPerSample: handle.bytes_per_sample
  };
  handles.set(track, handle);
  return track;
}

/**
 * decodes a stretch of a track, starting at a point in time and giving up to
 * `wanted` points in time. only the frames holding them are decoded, so
 * reading from the middle of a track costs what reading from the front costs.
 * throws when the track has been closed.
 */
export function decodeTrack(track: Track, from: number, wanted: number): Uint8Array {
  const handle = handles.get(track);
  if (handle === undefined) {
    throw new Error("the track is closed");
  }
  return handle.decode(from, wanted);
}

/** gives up a track and the memory it holds. closing a closed track does nothing */
export function closeTrack(track: Track): void {
  const handle = handles.get(track);
  if (handle === undefined) {
    return;
  }
  handles.delete(track);
  handle.free();
}
