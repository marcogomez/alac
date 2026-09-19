/* tslint:disable */
/* eslint-disable */

/**
 * An ALAC track of an MP4 file, opened and ready to decode from.
 */
export class AlacTrack {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Decodes up to `wanted` samples starting at `from`, and answers the
     * little endian bytes of them.
     *
     * Only the frames holding those samples are decoded, so reading from the
     * middle of a track costs the same as reading from the front.
     */
    decode(from: number, wanted: number): Uint8Array;
    /**
     * Bits per sample, per channel.
     */
    readonly bits: number;
    /**
     * How many bytes one sample of every channel takes once decoded.
     */
    readonly bytes_per_sample: number;
    /**
     * How many channels the track holds.
     */
    readonly channels: number;
    /**
     * Samples a second.
     */
    readonly sample_rate: number;
    /**
     * How many samples the track holds, counting one sample as one point in
     * time across every channel.
     */
    readonly total_samples: number;
}

/**
 * Opens the ALAC track of an MP4 file, or answers nothing when the file holds
 * no ALAC track this can read.
 */
export function open(file: Uint8Array): AlacTrack | undefined;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_alactrack_free: (a: number, b: number) => void;
    readonly alactrack_bits: (a: number) => number;
    readonly alactrack_bytes_per_sample: (a: number) => number;
    readonly alactrack_channels: (a: number) => number;
    readonly alactrack_decode: (a: number, b: number, c: number) => [number, number];
    readonly alactrack_sample_rate: (a: number) => number;
    readonly alactrack_total_samples: (a: number) => number;
    readonly open: (a: number, b: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
