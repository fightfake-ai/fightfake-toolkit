# fightfake-toolkit

A toolkit for proving that a video edit is genuine — and for verifying that claim without
trusting the editor.

---

## The problem

When someone shares an edited video — a colour-corrected aerial shot, a brightness-adjusted
security clip, a cropped news footage — there is currently no way to verify that the only
change made was the declared edit.  The video file might carry a digital signature proving it
came from a specific camera, but the signature says nothing about what happened between
capture and publication.  A deep-fake or a subtle manipulation is indistinguishable from a
legitimate edit.

**fightfake-toolkit** lets an editor prove, mathematically, that a specific transformation
(brightness, grayscale, invert, …) is the *only* difference between an original captured video
and the version being published.  The proof is compact, embeds directly in the video's
provenance record, and can be checked by anyone — including in a web browser — without
access to the original footage.

---

## How it works — three layers

```
Camera / drone
  ↓ shoots video; records a hash fingerprint of the original pixels → capture manifest
  
Editor's machine
  ↓ applies declared edit; runs zero-knowledge prover → edit-proof manifest
  
Verifier (anyone)
  ↓ checks: (1) C2PA signatures, (2) pixel fingerprints match, (3) proof is valid
```

### Layer 1 — C2PA provenance (the container)

[C2PA](https://c2pa.org) is an open industry standard (Adobe, Microsoft, Google, camera
manufacturers) for attaching unforgeable provenance records to media files.  Each record
contains who signed the file, when, and a cryptographic hash of the file's bytes.  If the
file is altered after signing the hash breaks and any C2PA verifier will flag it.

This toolkit produces C2PA manifests automatically.  Tools like the
[C2PA browser extension](https://contentauthenticity.org) or the
[c2pa.org verify page](https://verify.contentauthenticity.org) already know how to read them.

### Layer 2 — pixel fingerprints (h1 and h2)

The toolkit computes two hash values over the video pixels:

- **h1** — a hash over the *original* pixels at capture time.
- **h2** — a hash over the *edited* pixels.

Both hashes are embedded in the C2PA manifests and linked by the ingredient chain.  This
makes the capture-to-edit relationship explicit and machine-checkable, rather than a
human-readable text note.

At the software level (Level 0/1), these are SHA-256 hashes of the raw YUV planes.  With
full hardware integration (Levels 2/3), they are [Griffin](https://eprint.iacr.org/2022/403)
hash chains computed directly from the camera's pixel bus — a construction designed to be
efficiently provable inside a zero-knowledge circuit.

### Layer 3 — the zero-knowledge proof (the guarantor)

The ZK proof (produced by [Eva](https://github.com/miha-stopar/eva)) is a single compact
blob (~200 bytes for BN254/Groth16) that proves, without revealing the original frames:

> "The edited pixels are the result of applying the declared edit gadget to the original
> pixels identified by h1, producing h2. No other change was made."

A verifier checks this proof in milliseconds.  The math underlying it (Nova IVC + Groth16)
ensures that generating a false proof is computationally infeasible.

The proof embeds in the C2PA manifest as a custom assertion (`org.zkedit.edit_proof.v1`),
so existing C2PA tools can carry it without modification, and fightfake-aware tools can
verify it.

---

## Getting started — 5-minute walkthrough

### Step 0 — prerequisites

```bash
brew install ffmpeg        # macOS; or: apt install ffmpeg
cargo build -p fightfake-cli --release
```

The binary is `./target/release/fightfake`.  All commands below assume this path.

### Step 1 — generate a test certificate

C2PA manifests must be signed with a certificate.  For testing:

```bash
./target/release/fightfake make-test-cert
# writes testdata/certs/signer-cert.pem and testdata/certs/signer-key.pem
```

> For production, use a certificate from a CA on the
> [C2PA trust list](https://creator-assertions.github.io/taf/).

### Step 2 — prove an edit

```bash
./target/release/fightfake prove-edit \
  --input  my-video.mp4 \
  --gadget brightness \
  --gadget-param 416 \
  --out-dir out/
```

This runs the full pipeline:

```
my-video.mp4
  │ ffmpeg: decode to raw YUV frames
  ▼
raw pixels (YUV 4:2:0)
  ├──► hash → h1  (original fingerprint)
  │
  │ apply brightness scale 416/1024
  ▼
edited pixels
  ├──► hash → h2  (edited fingerprint)
  ├──► [Level 1] Nova IVC + Groth16 → proof.bin
  │
  │ ffmpeg: re-encode to H.264
  ▼
out/edited.mp4
out/capture.signed.mp4    ← C2PA-signed original; contains h1
out/edited.signed.mp4     ← C2PA-signed edited video; contains h2 + proof reference
out/proof.bin             ← ZK proof (or placeholder in Level 0)
```

Available gadgets: `brightness` (default), `grayscale`, `invert`.

### Step 3 — verify an edit proof

```bash
./target/release/fightfake verify \
  --capture out/capture.signed.mp4 \
  --edited  out/edited.signed.mp4 \
  --proof   out/proof.bin
```

Checks:
1. C2PA signatures and container-byte hashes on both files are valid.
2. h1 in the capture manifest equals h1 in the edit-proof manifest (the proof covers this specific capture).
3. proof.bin matches the SHA-256 recorded in the edit-proof assertion.

### Step 4 — verify a capture signature (no edit)

If you only want to confirm that a video was signed at capture time, without checking any edit:

```bash
./target/release/fightfake verify-capture \
  --capture out/capture.signed.mp4
```

Prints the device ID, pipeline stage, and h1 fingerprint embedded at capture.

### Step 5 — verify in the browser (WASM)

Build the browser bundle:

```bash
wasm-pack build fightfake-wasm --target web --release
# output: fightfake-wasm/pkg/
```

Use it in a web page:

```js
import init, { verifyAssertionLinkage } from './fightfake_wasm.js';
await init();

// captureJson / editJson: the org.zkedit.* assertion JSON strings
// proofBytes: Uint8Array of proof.bin
const result = verifyAssertionLinkage(captureJson, editJson, proofBytes);

if (result.h1_matches && result.proof_sha_matches) {
  console.log(`Edit proven: ${result.gadget_id()} by ${result.proof_system()}`);
} else {
  console.log('Assertion linkage failed.');
}
```

What the browser checks today: h1 consistency between the two assertions and SHA-256 of the
proof binary.  Cryptographic Groth16 verification (three pairing checks over BN254) is on the
roadmap for a future WASM release.

---

## Commands reference

### `prove-edit` — full workflow

```
fightfake prove-edit --input <VIDEO> [OPTIONS]

  --input, -i <FILE>       Input video (MP4 or any container ffmpeg can decode)
  --gadget <NAME>          Edit to apply: brightness | grayscale | invert  [default: brightness]
  --gadget-param <N>       Gadget-specific parameter:
                             brightness: luma scale in units of 1/1024 (default 416 ≈ 0.41×)
  --out-dir, -o <DIR>      Output directory for all artefacts  [default: out]
  --cert <FILE>            PEM signer certificate  [default: testdata/certs/signer-cert.pem]
  --key  <FILE>            PEM signer private key   [default: testdata/certs/signer-key.pem]
  --device-id <ID>         Identifier embedded in the capture assertion  [default: dev-0]
  --blocks-per-step <N>    Macroblocks per Nova IVC step (Level 1 only)  [default: 256]
```

Without `--features eva-backend` (Level 0), the proof is a 32-byte placeholder.  The edit,
hashes, and C2PA manifests are real and can be used for integration testing.

With `--features eva-backend` (Level 1), a full Nova IVC + Groth16 proof is generated.
Expect ~5 min for a 10-second 352×288 clip on an M2 Mac; longer for higher resolutions.

```bash
# Level 0 — fast, for integration testing
cargo build -p fightfake-cli --release
./target/release/fightfake prove-edit --input clip.mp4

# Level 1 — real ZK proof (first build takes 10–20 min)
cargo build -p fightfake-cli --release --features eva-backend
./target/release/fightfake prove-edit --input clip.mp4
```

### `verify-capture` — check a signed capture

```
fightfake verify-capture --capture <FILE>
```

Validates the C2PA signature and hard binding on a capture asset and prints the
`org.zkedit.capture.v1` assertion contents (device ID, h1, pipeline stage).

### `verify` — check an edit proof

```
fightfake verify --capture <FILE> --edited <FILE> --proof <FILE>
```

Validates both C2PA manifests, checks h1 linkage, and confirms the proof binary matches the
recorded SHA-256.

### `make-test-cert` — generate a test certificate

```
fightfake make-test-cert [--out-dir <DIR>]
```

Generates a self-signed P-256 / ES256 certificate suitable for local testing.  Writes
`signer-cert.pem` and `signer-key.pem`.  Do not use in production.

### Low-level plumbing commands

| Command | What it does |
|---|---|
| `emit-capture` | Write an `org.zkedit.capture.v1` JSON without signing |
| `emit-edit-proof` | Write an `org.zkedit.edit_proof.v1` JSON without signing |
| `sign-capture-manifest` | C2PA-sign a video and embed a capture assertion |
| `sign-edit-manifest` | C2PA-sign an edited video with an edit-proof assertion and parent ingredient |
| `verify-bundle` | Schema + h1 linkage check against assertion JSON side-files (pre-signing dry run) |
| `run-level0-demo` | Run the legacy Level-0 demo with pre-computed h1/h2 and any proof blob |
| `print-pi-capture-contract` | Print the Raspberry Pi libcamera adapter interface contract |

---

## Capture levels — how trustworthy is h1?

The proof itself (Nova IVC + Groth16) is cryptographically sound at any level.  What varies
is how much you trust the *input* to the proof: the h1 fingerprint of the original video.

The four levels come from the [integration design document](../eva-miha/docs/eva-c2pa-integration.md).

### Level 0 — software only (this toolkit today)

A trusted application reads an existing recording, decodes it with ffmpeg, tiles the frames
into macroblocks, and computes h1.  No firmware or hardware changes are needed.

**Limitation:** the gap between shutter press and the app running is unprotected.  Anyone with
access to the device could substitute a different recording before the app processes it.

**Use for:** validating assertion schemas, testing the verifier pipeline, prototyping integrations.

### Level 1 — camera SDK callback (near-term target)

The camera manufacturer's SDK exposes a callback with decoded frames *before* they reach the
container encoder.  The Griffin hash library runs inside the device's hardware security module
(ARM TrustZone Secure World), so the hash function itself cannot be tampered with even by a
compromised OS.  A Raspberry Pi demonstrator for this level is in `docs/level1-pi-demonstrator.md`.

**Limitation:** the pixel data arrives from the camera SDK (Normal World software).  A
compromised SDK could send fabricated pixels to the hasher.

**Requires from the manufacturer:** SDK callback exposing pre-encode frames, deployment of the
Griffin Trusted Application in their TEE.

### Level 2 — hardware pixel bus (longer-term)

A fixed-function hash block sits directly on the ISP-to-encoder data bus in silicon.  Pixels
are hashed before any software — including the OS and the camera SDK — can see them.

**Limitation:** none for the hash input.  Requires new silicon or firmware.

**Requires from the manufacturer:** a silicon tap on the ISP bus, a dedicated Griffin hash
block, and firmware routing to a secure element.

### Level 3 — published open standard (long-term goal)

The same hardware requirements as Level 2, but the assertion schema, Griffin parameters, and
hash engine interface are published as an open specification (analogous to how
`c2pa.hash.bmff.v3` is independently implemented by multiple manufacturers today).  Any
manufacturer implementing the spec produces manifests that any fightfake-aware verifier can
check without per-vendor customisation.

---

## How the proof system works (technical summary)

The ZK proof is produced by [Eva](https://github.com/miha-stopar/eva), which uses:

- **Nova IVC** (Incremental Verifiable Computation): processes the video macroblock by
  macroblock, maintaining a running accumulation of the proof state.  Each step proves the
  edit gadget was applied correctly and the hash chains advanced correctly.
- **Groth16 decider**: wraps the entire IVC argument in a compact Groth16 proof (~200 bytes).
  Also verifies an in-circuit Schnorr-style signature of h1 under the device key, linking the
  proof to a specific capture device without revealing h1 to the verifier.

Eva's `EditOnlyCircuit` is used for this toolkit (the "lossless" path): it proves the
pixel-domain transformation without constraining the H.264 re-encoding step.  A future
`EditEncodeCircuit` path would additionally prove that the specific encoded bitstream
corresponds exactly to the declared edit.

See [`eva-c2pa-integration.md`](../eva-miha/docs/eva-c2pa-integration.md) for a detailed
breakdown of the Groth16 decider circuit and the four capture levels.

---

## Assertion schemas

All custom C2PA assertions use the `org.zkedit.*` namespace, designed to be scheme-neutral
(the proof system can change without breaking the schema):

| Label | Fields | Purpose |
|---|---|---|
| `org.zkedit.capture.v1` | `device_id`, `pipeline_stage`, `hash_algorithm`, `h1` | Fingerprint of original pixels, embedded at capture |
| `org.zkedit.edit_proof.v1` | `gadget_id`, `h1`, `h2`, `proof_system`, `circuit_variant`, `proof_sha256` | Edit declaration and proof reference |

JSON Schemas are in `schemas/`.  Both assertions are validated against these schemas before
any C2PA signing step.

---

## Repository layout

```
fightfake-toolkit/
├── fightfake-core/       shared library — assertion types, schemas, verifier logic
│                         no heavy dependencies; compiles to native and WASM
├── fightfake-cli/        CLI binary `fightfake`; full workflow + plumbing commands
├── fightfake-wasm/       browser verifier — wasm-bindgen exports for fightfake.ai
├── schemas/              org.zkedit.* JSON Schema definitions
├── testdata/             test videos, generated certificates, proof stubs
└── docs/                 detailed design and Pi demonstrator docs
```

---

## Roadmap

- [ ] Cryptographic Groth16 verification in the CLI (`verify-proof` command)
- [ ] WASM Groth16 verifier — `verifyGroth16Proof` for in-browser proof checking
- [ ] Level 1 Raspberry Pi demonstrator (`docs/level1-pi-demonstrator.md`)
- [ ] Crop gadget support
- [ ] Proof serialisation format and public key distribution specification
- [ ] fightfake.ai integration guide for web developers

---

## License

MIT
