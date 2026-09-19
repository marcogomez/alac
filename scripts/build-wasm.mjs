/*
 * builds the decoder in crate/ to webassembly and writes it into src/generated.
 *
 * cargo builds the module, wasm-bindgen writes the bindings, wasm-opt shrinks
 * what comes out, and the module goes in as base64 next to those bindings, so
 * the package needs no file on disk at runtime.
 *
 * the target features in crate/.cargo/config.toml and the --enable flags
 * handed to wasm-opt below have to name the same set. an optimiser that does
 * not know about a feature the module was built with either turns the module
 * away or lowers it.
 *
 * what this writes is committed. `pnpm build` runs this first and builds the
 * javascript only when it succeeds. it needs rustup, which installs the
 * toolchain rust-toolchain.toml names, and wasm-bindgen-cli at the version
 * pinned in crate/Cargo.toml on the path. wasm-opt comes from the binaryen
 * package in node_modules.
 *
 * the header of what is written records a hash of everything the module was
 * built from. `node scripts/build-wasm.mjs --check` computes that hash again
 * and fails when it differs, which is how ci tells that the crate changed and
 * the module was not rebuilt. the bytes of the module themselves are not
 * compared, since cargo mixes the absolute path of the checkout into its
 * symbol names and no two checkouts give the same bytes.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const crate = join(root, "crate");
const target = "wasm32-unknown-unknown";
const staging = join(crate, "target", "bindgen");
const generated = join(root, "src", "generated");
const wasmOpt = join(root, "node_modules", ".bin", process.platform === "win32" ? "wasm-opt.cmd" : "wasm-opt");

/** the features crate/.cargo/config.toml turns on, named the way wasm-opt names them */
const FEATURES = [
  "--enable-simd",
  "--enable-bulk-memory",
  "--enable-nontrapping-float-to-int",
  "--enable-sign-ext",
  "--enable-reference-types",
  "--enable-multivalue"
];

/** what the module has to export once the bindings are written */
const WANTED = ["memory", "open", "alactrack_decode", "alactrack_total_samples", "__wbindgen_malloc"];

/** every file under a directory, sorted by path */
function filesUnder(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...filesUnder(path));
    } else {
      found.push(path);
    }
  }
  return found.sort();
}

/**
 * the files the module is built from. the crate's sources and manifests, its
 * lockfile with the pinned wasm-bindgen crate, the target features, the
 * toolchain, the binaryen version, and this script
 */
function inputs() {
  return [
    ...filesUnder(join(crate, "src")),
    join(crate, "Cargo.toml"),
    join(crate, "Cargo.lock"),
    join(crate, ".cargo", "config.toml"),
    join(root, "rust-toolchain.toml"),
    join(root, "package.json"),
    fileURLToPath(import.meta.url)
  ];
}

/**
 * one hash over every input, each keyed by its path relative to the root so
 * the hash is the same wherever the checkout is. line endings are folded to
 * lf so a checkout that turned them into crlf hashes the same
 */
function sourceHash() {
  const hash = createHash("sha256");
  for (const path of inputs()) {
    hash.update(relative(root, path).split("\\").join("/"));
    hash.update("\0");
    hash.update(readFileSync(path, "utf8").split("\r\n").join("\n"));
    hash.update("\0");
  }
  return hash.digest("hex");
}

/** the hash the committed module records, or nothing when there is no module */
function recordedHash() {
  const path = join(generated, "alac-wasm.js");
  if (!existsSync(path)) {
    return null;
  }
  const found = /built from ([0-9a-f]{64})/.exec(readFileSync(path, "utf8"));
  return found === null ? null : found[1];
}

if (process.argv.includes("--check")) {
  const recorded = recordedHash();
  const current = sourceHash();
  if (recorded === null) {
    console.error(`${generated} holds no module. run pnpm build-wasm`);
    process.exit(1);
  }
  if (recorded !== current) {
    console.error(`the committed module was built from other sources. run pnpm build-wasm and commit src/generated`);
    process.exit(1);
  }
  console.log(`the committed module is current, built from ${current}`);
  process.exit(0);
}

console.log(`building ${crate} for ${target}`);
execFileSync("cargo", ["build", "--release", "--target", target], { cwd: crate, stdio: "inherit" });

console.log("writing the bindings");
rmSync(staging, { recursive: true, force: true });
execFileSync(
  "wasm-bindgen",
  [
    "--target",
    "web",
    "--out-dir",
    staging,
    "--out-name",
    "alac",
    join(crate, "target", target, "release", "alacrs.wasm")
  ],
  { stdio: "inherit" }
);

const built = join(staging, "alac_bg.wasm");
const shrunk = join(staging, "alac_bg.opt.wasm");
const raw = readFileSync(built);

/** runs the module through binaryen */
function shrink() {
  if (!existsSync(wasmOpt)) {
    throw new Error(`wasm-opt is not at ${wasmOpt}. run pnpm install`);
  }
  execFileSync(wasmOpt, ["-Oz", ...FEATURES, "--strip-debug", "--strip-producers", built, "-o", shrunk], {
    stdio: "inherit",
    shell: process.platform === "win32"
  });
  return readFileSync(shrunk);
}

const wasm = shrink();
console.log(`${raw.length} bytes from cargo, ${wasm.length} bytes after wasm-opt`);

// the module has to still hold what the bindings call into once the optimiser
// has been over it
const exported = WebAssembly.Module.exports(new WebAssembly.Module(wasm)).map((entry) => entry.name);
for (const name of WANTED) {
  if (!exported.includes(name)) {
    throw new Error(`the module no longer exports ${name}`);
  }
}

// the bindings fall back to fetching a file next to themselves when nothing is
// handed to them. the loader always hands over bytes, so the fallback becomes
// an error, and a bundler no longer sees a file to look for
const glue = readFileSync(join(staging, "alac.js"), "utf8").replace(
  /module_or_path = new URL\('alac_bg\.wasm', import\.meta\.url\);/,
  "throw new Error('the alac decoder is loaded from bytes, not from a url');"
);
if (glue.includes("alac_bg.wasm")) {
  throw new Error("the bindings still look for a file on disk");
}

// the module and its declaration are written as a javascript file and a
// declaration file, like the bindings, so the declaration build has no source
// under src/generated to emit for and dist holds only what index.ts declares
const header = `/*
 * the alac decoder of crate/, built to webassembly.
 *
 * generated by scripts/build-wasm.mjs. do not edit by hand, run
 * \`pnpm build-wasm\` instead.
 *
 * ${wasm.length} bytes, built with ${FEATURES.map((flag) => flag.replace("--enable-", "")).join(", ")}.
 * built from ${sourceHash()}
 */
`;

rmSync(generated, { recursive: true, force: true });
mkdirSync(generated, { recursive: true });
writeFileSync(join(generated, "alac-glue.js"), glue);
writeFileSync(join(generated, "alac-glue.d.ts"), readFileSync(join(staging, "alac.d.ts"), "utf8"));
writeFileSync(
  join(generated, "alac-wasm.js"),
  `${header}
/** the decoder module, as base64 */
export const ALAC_WASM = "${wasm.toString("base64")}";
`
);
writeFileSync(
  join(generated, "alac-wasm.d.ts"),
  `${header}
/** the decoder module, as base64 */
export declare const ALAC_WASM: string;
`
);

console.log(`wrote ${generated}`);
