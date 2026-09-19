# @mgz-dev/alac

Apple Lossless (ALAC) decoder in pure Rust, compiled to WebAssembly.

Browsers have no ALAC decoder, so an `.m4a` holding Apple Lossless cannot be
played by a media element. This decodes those files.

- **No native binaries, no GPL.** One 32 KB WebAssembly module with no
  imports. MIT.
- **The same bytes everywhere.** WebAssembly bytecode is portable, so one build
  serves macOS, Linux and Windows, on x64 and on arm64. Nothing per-platform to
  build, ship or sign.
- **Seeks properly.** Only the frames holding the samples you ask for are
  decoded, so reading from the middle of a track costs what reading from the
  front costs.
- **Fast.** About 500 times realtime on one core.

```sh
pnpm add @mgz-dev/alac
```

## Usage

```ts
import { closeTrack, decodeTrack, openTrack } from "@mgz-dev/alac";
import { readFile } from "node:fs/promises";

const track = openTrack(await readFile("song.m4a"));
if (track === null) {
  // the file holds no alac track. an aac file in the same kind of
  // container gives this, and so does anything that is not an mp4
} else {
  console.log(track.sampleRate, track.channels, track.bits, track.samples);

  // five seconds from the one minute mark, as little endian samples with
  // the channels one after another for each point in time
  const from = 60 * track.sampleRate;
  const pcm = decodeTrack(track, from, 5 * track.sampleRate);

  closeTrack(track);
}
```

### API

| | |
| --- | --- |
| `openTrack(file: Uint8Array): Track \| null` | Opens the ALAC track of an MP4 file. `null` when it holds none. |
| `decodeTrack(track, from: number, wanted: number): Uint8Array` | Decodes up to `wanted` samples starting at `from`, little endian. |
| `closeTrack(track): void` | Gives up the track and the memory it holds. |

A `Track` holds `sampleRate`, `channels`, `bits`, `samples` (points in time
across every channel) and `bytesPerSample`.

Sixteen and twenty four bit samples are decoded, in one or two channels. Twenty
and thirty two bit, and more than two channels, answer with nothing rather than
an error.

## How it is built

`crate/` holds the Rust. The MP4 side reads `stsd`, `stts`, `stsc`,
`stco`/`co64` and `stsz` to find the frames, and nothing else.

```sh
pnpm build
```

That builds the WebAssembly module first and the JavaScript only when that
succeeds. The module step runs `cargo build --target wasm32-unknown-unknown`,
then `wasm-bindgen`, then `wasm-opt -Oz`, and writes the result into
`src/generated/`, which is committed. It needs `rustup`, which installs the
toolchain `rust-toolchain.toml` names, and `wasm-bindgen-cli` pinned to the
version in `crate/Cargo.toml` on the path. `wasm-opt` comes from the `binaryen`
package that `pnpm install` brings in.

The toolchain is pinned so the module a release publishes is built the way the
committed one was. Bumping it means rebuilding the module and committing the
result together with `rust-toolchain.toml`.

`pnpm build-wasm` runs the module step on its own.

The target features in `crate/.cargo/config.toml` and the `--enable` flags the
build script hands to `wasm-opt` name the same set. An optimiser that does not
know about a feature the module was built with either turns the module away or
lowers it, so those two lists move together.

The header of the committed module records a hash of everything it was built
from. `pnpm check-wasm` computes that hash again and fails when it differs,
and CI runs it, so the module cannot drift from the crate. The bytes of the
module are not compared, since cargo mixes the path of the checkout into its
symbol names and no two checkouts give the same bytes.

## Testing

```sh
pnpm test          # the javascript side
pnpm test-rust     # the crate
pnpm lint-rust     # cargo fmt --check and clippy
```

The crate is tested against six frames of a real ALAC stream, byte for byte.
The JavaScript side decodes a fixture and checks it comes back identical to the
audio the encoder was given.

---

## Releasing

This package uses [Changesets](https://github.com/changesets/changesets) for versioning and publishing.

### When you make a change

After making your changes, create a changeset to describe what changed:

```sh
pnpm changeset
```

You'll be prompted to:

1. Select the package
2. Choose the bump type:
   - **patch** - bug fixes, dependency updates, internal refactors
   - **minor** - new API, new formats decoded, new options
   - **major** - breaking changes to the API, removed or renamed exports
3. Write a summary of the change

Commit the changeset file with your code.

### How publishing works

When you push to `master` (or merge a PR):

1. The CI **release** workflow detects pending changeset files
2. It opens a "Version Package" PR that bumps the version in `package.json` and updates `CHANGELOG.md`
3. When you merge that PR, the workflow publishes to npm automatically via trusted publishing (OIDC)

No npm tokens to manage or rotate. GitHub Actions authenticates directly with npm.

### First-time setup

The very first publish must be done manually because trusted publishing requires the package to already exist on npm.

1. Use your existing npm token (or create a temporary 90-day granular one at Profile > Access Tokens)
2. Publish manually:
   ```sh
   npm publish --access public --//registry.npmjs.org/:_authToken=$(cat ~/.npmtoken)
   ```
3. Configure trusted publishing on npm: go to `https://www.npmjs.com/package/@mgz-dev/alac/access`, click **GitHub Actions**, and fill in:
   - **Organization or user**: `marcogomez` (case-sensitive)
   - **Repository**: `alac`
   - **Workflow filename**: `release.yml`
   - **Allowed actions**: select "npm publish"
4. Delete the temporary npm token

### Manual publishing (if needed)

```sh
pnpm changeset        # create a changeset
pnpm run version      # bump version + update CHANGELOG
pnpm run release      # publish to npm
```

## License

MIT. See [LICENSE](LICENSE). Third-party notices are in [NOTICE](NOTICE).
