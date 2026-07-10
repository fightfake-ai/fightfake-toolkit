# fightfake-toolkit

A toolkit for proving that a video edit is genuine — and for verifying that claim without
trusting the editor.

---

## The problem

When someone shares an edited video — a colour-corrected aerial shot, a brightness-adjusted
security clip, a cropped news footage — there is currently no way to verify that the only
change made was the declared edit.

**Wait — doesn't C2PA already solve this?**  Partly.  The
[C2PA standard](https://c2pa.org) (used by Adobe Photoshop, Lightroom, and many cameras
today) lets a signer *declare* what edits were made and attach a certificate to that
declaration.  You can open such a file in a C2PA viewer and read: "brightness was adjusted,
signed by Adobe".  What you cannot do is *verify* that claim independently — you have to
trust that Adobe's signing pipeline was not compromised, that the signer's private key was
not misused, and that the description is accurate.  A deep-fake with a stolen or misleading
certificate is indistinguishable from a legitimate edit.

**fightfake-toolkit** closes that gap.  Instead of a declaration backed by institutional
trust, it produces a **mathematical proof** — a compact blob of bytes that any verifier can
check independently, without trusting the signer, without access to the original footage, and
without any knowledge of who produced the video.  The proof shows that a specific pixel-level
transformation (brightness, grayscale, invert, …) is the *only* difference between the
captured original and the published version.  If even a single pixel was changed in any other
way, the proof does not verify.

In short:
- **Standard C2PA:** *"Trust me — I declare this is what was edited."*
- **fightfake:** *"Don't trust me — verify it yourself. The math guarantees it."*

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

This is precisely what standard C2PA cannot provide: not a declaration of what happened, but
an unforgeable mathematical certificate that *only* the declared edit occurred.  A verifier
checks the proof in milliseconds.  The math underlying it (Nova IVC + Groth16) ensures that
generating a convincing false proof is computationally infeasible — even for the person who
signed the video.

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

The binary is `./target/release/fightfake`.  All commands below assume you run them from the
`fightfake-toolkit/` directory.  Paths to input files must be relative to that directory (or
absolute).

### Step 1 — generate a test certificate

C2PA manifests must be signed.  For testing, generate a self-signed certificate:

```bash
./target/release/fightfake make-test-cert
# writes testdata/certs/signer-cert.pem  (certificate — public)
#         testdata/certs/signer-key.pem   (private key  — keep secret)
```

#### What is a certificate and why does C2PA need one?

A digital certificate is a signed statement:

> *"I, a trusted authority, confirm that this public key belongs to this entity."*

More concretely: a certificate is a small file that bundles together a **public key** and an
**identity** (name, organisation, …), with a signature from a Certificate Authority (CA)
confirming they belong together.  The corresponding **private key** is kept secret and is
used to produce signatures.

C2PA requires every manifest to carry a **COSE_Sign1** signature — a compact binary
envelope (defined in [RFC 9052](https://www.rfc-editor.org/rfc/rfc9052), which superseded
the now-obsolete RFC 8152) that bundles:

1. The **signed payload** — a hash of all the assertions in the manifest.
2. The **algorithm** — ES256 (ECDSA P-256 with SHA-256).
3. The **certificate** — so verifiers can identify the signer and check its trust chain.
4. The **signature bytes** — produced by the private key over the payload hash.

Because the signature covers the assertions hash, and the assertions include
`c2pa.hash.bmff.v3` (a byte-range hash of the entire MP4 container), the COSE_Sign1
transitively covers every byte of the video.  Changing a single pixel after signing breaks
the manifest.

#### Self-signed vs. CA-issued certificates

| | Self-signed (test) | CA-issued (production) |
|---|---|---|
| How generated | `make-test-cert` (this toolkit) | Apply to a CA on the C2PA trust list |
| Who confirms the identity | Nobody — you assert it yourself | A trusted CA (Adobe, Truepic, Leica, …) |
| Online validator result | ⚠️ "not from a trusted source" | ✅ trusted signer |
| Signature validity | ✅ mathematically valid | ✅ mathematically valid |
| Good for | Local testing, integration tests | Production, public distribution |

Swapping in a real certificate requires no code changes — just pass different `--cert` and
`--key` files to any command.

### Step 2 — prove an edit

```bash
./target/release/fightfake prove-edit \
  --input  testdata/videos/input/my-video.mp4 \
  --gadget brightness \
  --gadget-param 416 \
  --out-dir out/
```

> **Note on paths:** the path after `--input` is relative to the directory where you run the
> command.  If the file is in `testdata/videos/input/`, use that prefix explicitly.

This runs the full pipeline in one shot:

```
testdata/videos/input/my-video.mp4
  │ ffmpeg: decode to raw YUV 4:2:0
  ▼
raw pixels — one frame = width × height luma bytes + half-res chroma
  │ tile into 16×16 macroblocks  ← Eva's unit of operation
  ▼
Eva macroblocks (orig_y / orig_u / orig_v, in macroblock order)
  ├──► SHA-256 over all macroblocks → h1  (original fingerprint)
  │
  │ apply brightness scale 416/1024 to each luma byte
  ▼
edited macroblocks
  ├──► SHA-256 over edited macroblocks → h2  (edited fingerprint)
  │
  │ [stub build]  record SHA-256 of a 32-byte zero placeholder as proof reference
  │ [full build]  Nova IVC: prove each macroblock was transformed correctly
  │               → Groth16 decider: compress IVC argument → proof.bin
  │               (build with --features eva-backend)
  │
  │ untile macroblocks → planar YUV → ffmpeg: re-encode to H.264
  ▼
out/edited.mp4
out/capture.signed.mp4    ← C2PA manifest: h1 + device ID + hard binding
out/edited.signed.mp4     ← C2PA manifest: h2 + proof reference + ingredient chain
out/proof.bin             ← ZK proof (or 32-byte stub in stub build)
```

**Auto-crop and h1 coverage:** Eva's IVC circuit requires video dimensions that are exact
multiples of 16 pixels.  Most cameras (including the 1920×1080 example above) produce
non-aligned dimensions — 1080 is not divisible by 16.  The toolkit automatically crops to
the largest aligned size: 1920×1072 in this case (8 rows removed from the bottom edge, no
pixel resampling).  This crop is recorded as a `c2pa.actions / c2pa.cropped` assertion in
the capture manifest, so any C2PA viewer knows h1 covers the cropped frame, not the
original full-resolution frame.  Verifiers must apply the same crop before computing h1.

**Why macroblocks?**  Eva's IVC circuit processes video one 16×16 pixel block at a time.
Each IVC step proves that the edit gadget was applied correctly to one (or more) macroblocks
and that the hash chain (h1 or h2) advanced correctly.  This makes the proof incremental:
a 1-second clip and a 10-minute clip use the same per-step circuit, just with more steps.

**Stub build vs. full build:** by default (no `--features eva-backend`) the proof is a
32-byte zero placeholder — call this the *stub build*.  The edit, hashes, and C2PA manifests
are real and fully usable for integration testing.  Build with `--features eva-backend` for
a real Nova IVC + Groth16 proof — the *full build*.

This is different from the **capture levels** (Level 0–3) described in the
[trust model section](#capture-levels--how-trustworthy-is-h1).  The levels are about how
much you trust h1 (the original pixel fingerprint): Level 0 means a software app computed
h1 from an existing recording; Level 3 means silicon-level hardware produced it.  The ZK
proof is cryptographically sound regardless of the level — the level only tells you how hard
it is for an attacker to substitute a different recording before h1 is computed.

**Timing:** at the end of the run a table is printed showing time per phase.  Below are
real measurements on `bank-robbery-original.mp4` (1920×1072 after auto-crop, 121 frames,
5 s clip), averaged over 3 runs on an Apple M2 MacBook Pro (stub build, no ZK proof):

```
┌─────────────────────────────────────────┬──────────┐
│ Phase                                   │      Avg │
├─────────────────────────────────────────┼──────────┤
│ ffmpeg decode                           │   0.34s  │
│ macroblock tiling                       │   0.03s  │
│ edit + hashing (h1, h2)  †             │   2.26s  │
│ ZK proving (Nova IVC + Groth16)         │   0.00s  │  ← stub; real ≈ minutes
│ ffmpeg re-encode                        │   2.68s  │
│ C2PA signing             ‡             │   0.07s  │
├─────────────────────────────────────────┼──────────┤
│ Total                                   │   5.69s  │
└─────────────────────────────────────────┴──────────┘
```

† Raw YUV data for 121 frames at 1920×1072: ~374 MB.  The 2.26 s includes a brightness
multiply per luma byte plus SHA-256 over the resulting data — effective throughput ≈ 165 MB/s.
With the Eva backend, Griffin replaces SHA-256; Griffin is circuit-friendly but roughly 3–5×
slower as a plain hash.

‡ C2PA signing covers the encoded H.264 file (≈4 MB), not raw pixels.  See the
[C2PA signing section](#how-c2pa-rs-signs-the-video) for details.

To reproduce these numbers on your machine:

```bash
./bench.sh testdata/videos/input/bank-robbery-original.mp4 3
```

With `--features eva-backend`, the "ZK proving" row dominates and typically takes
several minutes for a 5-second 1920×1072 clip on an M2 Mac.

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

### C2PA online validator

The manifests produced by this toolkit are spec-conformant and can be uploaded to
[verify.contentauthenticity.org](https://verify.contentauthenticity.org):

| Check | Self-signed cert | Production cert |
|---|---|---|
| Manifest structure | ✅ pass | ✅ pass |
| COSE_Sign1 signature | ✅ pass | ✅ pass |
| Hard binding (`c2pa.hash.bmff.v3`) | ✅ pass | ✅ pass |
| Ingredient chain (capture → edit) | ✅ pass | ✅ pass |
| `c2pa.actions` edit description | ✅ present | ✅ present |
| Certificate trust | ⚠️ warning (not on trust list) | ✅ pass |
| `org.zkedit.*` assertions | ℹ️ shown as custom | ℹ️ shown as custom |

The "custom assertion" label is expected — C2PA viewers display any assertion whose label
they don't recognise as a custom extension, without treating it as an error.

---

## Standard C2PA vs. fightfake C2PA

The `c2pa-sign` command produces a *plain* C2PA manifest — exactly what you'd get from any
standard C2PA-aware video editor.  The `prove-edit` command produces a *fightfake* manifest
that extends the standard with pixel fingerprints and a ZK proof.

```bash
# Standard C2PA — declares that a brightness edit was made (just like Adobe Photoshop,
# DaVinci Resolve, and any other C2PA-compliant editor would produce)
./target/release/fightfake c2pa-sign \
  --input  testdata/videos/input/my-video.mp4 \
  --output out/standard-signed.mp4 \
  --action c2pa.color_adjustments \
  --description "Brightness adjustment"

# Fightfake C2PA — additionally proves, mathematically, that ONLY this brightness edit
# was made and that no other pixel was changed
./target/release/fightfake prove-edit \
  --input  testdata/videos/input/my-video.mp4 \
  --gadget brightness \
  --gadget-param 416 \
  --out-dir out/
```

Standard C2PA already supports describing brightness/colour edits via the
`c2pa.color_adjustments` action code, and this is used in practice today by tools like
Photoshop and Lightroom.  The difference is not *what* edit is declared, but *how* it is
supported: standard C2PA is a **declaration** backed by the signer's identity and
certificate; fightfake adds a **cryptographic proof** that verifiers can check independently
of who signed it.

### What's inside each manifest

| Assertion | Standard C2PA (`c2pa-sign`) | Fightfake C2PA (`prove-edit`) |
|---|---|---|
| `c2pa.hash.bmff.v3` (hard binding) | ✅ auto-added | ✅ auto-added |
| `c2pa.actions` (edit description) | ✅ human-readable | ✅ human-readable |
| `c2pa.ingredient` (parent link) | — | ✅ links to signed capture |
| `org.zkedit.capture.v1` (h1 fingerprint) | — | ✅ original pixel hash |
| `org.zkedit.edit_proof.v1` (h2 + proof ref) | — | ✅ edited pixel hash + proof |
| `proof.bin` (ZK proof blob) | — | ✅ (stub in Level 0; real in Level 1) |

### What each approach can (and cannot) prove

| Claim | Standard C2PA | Fightfake C2PA |
|---|---|---|
| "This file hasn't been modified since signing" | ✅ hard binding | ✅ hard binding |
| "This video came from a specific camera/device" | ✅ (if camera has C2PA support) | ✅ |
| "A brightness edit was declared" | ✅ | ✅ |
| "Exactly this brightness edit — and *nothing else* — was applied" | ❌ trust the signer | ✅ ZK proof |
| "The edit was applied to the specific original identified by h1" | ❌ | ✅ |
| "Verifiable without access to the original footage" | ❌ | ✅ |
| "Verifiable without trusting the signer" | ❌ | ✅ |

**The core distinction:**
Standard C2PA shifts the trust question to certificates: you verify the signer's certificate
chains to a trusted CA, then accept the signer's declaration.  If the signer's key is
compromised, or if the pipeline that produces the declaration is manipulated, you have no way
to detect it.

Fightfake eliminates that trust dependency.  The ZK proof is a mathematical object: if it
verifies, the declared edit is the only pixel-level change, period — regardless of who
signed the file, whether their certificate is trusted, or whether their infrastructure was
compromised.

Both manifests are readable by the same C2PA tools (browser extension, online validator).
Standard C2PA viewers will display the fightfake manifest correctly, showing the `c2pa.actions`
assertion and noting the `org.zkedit.*` assertions as custom extensions.

---

## How c2pa-rs signs the video

Understanding this matters for evaluating performance claims and for comparing with the
approach in academic papers such as
[VerITAS (eprint.iacr.org/2024/1066)](https://eprint.iacr.org/2024/1066.pdf).

### What c2pa-rs actually hashes

C2PA's hard binding for MP4 files is called **`c2pa.hash.bmff.v2`** (or `.v3` in newer
versions).  It works at the *container level*, not the pixel level:

```
MP4 file (BMFF boxes)
  ├── ftyp box   ──► included in hash input
  ├── moov box   ──► included (video metadata, codec parameters, …)
  ├── mdat box   ──► included (the H.264 encoded bitstream)
  └── uuid box   ──► EXCLUDED (this is where the C2PA manifest lives)
```

The library identifies each BMFF box by type, skips the uuid box that holds the manifest
itself (to avoid a chicken-and-egg problem), and runs **SHA-256 over the remaining byte
ranges** in one pass.  The result is a list of `(byte_offset, length, sha256_digest)` tuples
stored in the manifest.

Crucially, this is a hash of the **encoded H.264 bytes**, not raw decoded pixels.  For the
bank-robbery clip (~4 MB encoded), this takes ~0.07 s.  The ECDSA-P256 signature over the
resulting hash digest is computationally negligible.

### Why this is fast — and what VerITAS is actually about

The [VerITAS paper](https://eprint.iacr.org/2024/1066.pdf) observes that hashing raw pixel
data *inside a zero-knowledge circuit* is extremely expensive.  A ZK circuit that verifies
SHA-256 requires thousands of arithmetic constraints per block (64 bytes), making it
impractical for multi-megapixel images.  They propose a lattice-based hash that is circuit-
friendly to work around this.

C2PA does **not** operate inside a ZK circuit.  It hashes bytes on real hardware with a
native SHA-256 implementation (single-instruction on x86/ARM), so there is no circuit
overhead.  The C2PA signature simply says "the encoded container bytes have not changed since
signing" — it says nothing about what the pixels look like or what transformations were
applied.

fightfake's h1/h2 fingerprints are where the VerITAS concern is relevant:

| Hash | Where computed | Purpose | Cost |
|---|---|---|---|
| `c2pa.hash.bmff` | outside ZK, native SHA-256 | container integrity | fast (~0.07 s / 4 MB) |
| h1, h2 (stub build) | outside ZK, native SHA-256 | pixel fingerprints | ~2.3 s / 374 MB raw |
| h1, h2 (Eva backend) | **inside ZK circuit**, Griffin | pixel fingerprints + provable | slower hash, but ZK-friendly |

Eva avoids the VerITAS problem by using the
[Griffin permutation](https://eprint.iacr.org/2022/403) instead of SHA-256 for h1/h2.
Griffin is specifically designed so that verifying it inside an arithmetic circuit is
cheap — far cheaper than SHA-256.  The stub build uses plain SHA-256 for h1/h2 because it
never runs a ZK prover; only the Eva backend needs the circuit-friendly variant.

### Comparing manifests: `dump-manifest`

The C2PA manifest is embedded inside the MP4 in a `uuid` BMFF box — there is no separate
manifest file by default.  Use `dump-manifest` to extract and inspect it:

```bash
# What does a standard C2PA manifest look like?
./target/release/fightfake c2pa-sign \
  --input  testdata/videos/input/bank-robbery-original.mp4 \
  --output out/standard.mp4
./target/release/fightfake dump-manifest --input out/standard.mp4 > out/standard.manifest.json

# What does a fightfake manifest look like?
./target/release/fightfake prove-edit \
  --input  testdata/videos/input/bank-robbery-original.mp4 \
  --out-dir out/
./target/release/fightfake dump-manifest --input out/capture.signed.mp4  > out/capture.manifest.json
./target/release/fightfake dump-manifest --input out/edited.signed.mp4   > out/edited.manifest.json

# Diff them side-by-side
diff <(python3 -m json.tool out/standard.manifest.json) \
     <(python3 -m json.tool out/edited.manifest.json)
```

The key differences you will see in the fightfake edit manifest vs. standard C2PA:

- An `ingredients` entry linking back to the signed capture asset (with its own manifest hash).
- An `org.zkedit.capture` assertion containing `h1` (the pixel fingerprint of the original).
- An `org.zkedit.edit_proof` assertion containing `h2`, `proof_sha256`, and the proof-system metadata.
- The standard C2PA manifest has only `c2pa.actions` and the BMFF hash.

---

## Commands reference

### `prove-edit` — full fightfake workflow

```
fightfake prove-edit --input <VIDEO> [OPTIONS]

  --input, -i <FILE>       Input video (path relative to cwd, or absolute)
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
./target/release/fightfake prove-edit --input testdata/videos/input/clip.mp4

# Level 1 — real ZK proof (first build takes 10–20 min)
cargo build -p fightfake-cli --release --features eva-backend
./target/release/fightfake prove-edit --input testdata/videos/input/clip.mp4
```

### `c2pa-sign` — standard C2PA manifest (no ZK)

```
fightfake c2pa-sign --input <VIDEO> --output <VIDEO> [OPTIONS]

  --input, -i <FILE>       Input video
  --output, -o <FILE>      Output signed video
  --title <STR>            Manifest title  [default: "C2PA-signed video"]
  --action <LABEL>         C2PA action code  [default: c2pa.edited]
                           Common values: c2pa.color_adjustments, c2pa.cropped
  --description <STR>      Human-readable description of the edit
  --cert <FILE>            PEM certificate  [default: testdata/certs/signer-cert.pem]
  --key  <FILE>            PEM private key   [default: testdata/certs/signer-key.pem]
```

Produces a standard C2PA manifest with `c2pa.actions` + BMFF hard binding.  No pixel
fingerprints, no ZK proof.  Use this to compare with `prove-edit` output side-by-side in
the online validator.

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

### `dump-manifest` — inspect the embedded C2PA manifest

```
fightfake dump-manifest --input <FILE> [--out <FILE>]
```

Extracts the C2PA manifest from a signed MP4 and prints it as formatted JSON (to stdout or a
file).  The manifest lives inside the MP4 container in a `uuid` BMFF box — there is no
separate file unless you explicitly extract it with this command.  Use it to compare standard
C2PA and fightfake manifests side-by-side (see the
[comparing manifests](#comparing-manifests-dump-manifest) section above).

### Low-level plumbing commands

| Command | What it does |
|---|---|
| `dump-manifest` | Extract the embedded C2PA manifest from a signed MP4 as JSON |
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
