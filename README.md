# fightfake-toolkit

A ZK-proved video editing toolkit with C2PA content provenance.

Given an input video and an edit operation, **fightfake-toolkit**:

1. Applies the edit (brightness, crop, …)
2. Generates a zero-knowledge proof that the edit was applied exactly as declared — without revealing the original frames
3. Embeds the proof and both hash-chain endpoints in a C2PA manifest, creating a tamper-evident provenance chain

The toolkit is scheme-neutral: the Eva proving backend is an optional dependency.  Without it, the workflow runs in Level-0 mode (real edits, real hashes, placeholder proof) that is enough for prototyping and UI integration.

---

## Repository layout

```
fightfake-toolkit/
├── fightfake-core/       shared library — assertions, schemas, verifier
│                         (no heavy deps; compiles to native + WASM)
├── fightfake-cli/        full-workflow CLI binary `fightfake`
├── fightfake-wasm/       browser verifier (wasm-bindgen; for fightfake.ai)
├── schemas/              org.zkedit.* JSON Schemas
├── testdata/             test videos, certs, proof stubs
└── docs/                 detailed documentation
```

---

## Quick start

### Prerequisites

```bash
brew install ffmpeg      # macOS
apt install ffmpeg       # Linux
cargo install wasm-pack  # for WASM build only
```

### Level 0 — real edit, stub proof

```bash
# Build (fast; no heavy crypto)
cargo build -p fightfake-cli --release

# Run the full workflow
./target/release/fightfake prove-edit \
  --input testdata/videos/input/capture.mp4 \
  --gadget brightness \
  --gadget-param 416 \
  --out-dir out/ \
  --cert testdata/certs/signer-cert.pem \
  --key  testdata/certs/signer-key.pem

# Verify
./target/release/fightfake verify \
  --capture out/capture.signed.mp4 \
  --edited  out/edited.signed.mp4 \
  --proof   out/proof.bin
```

The `out/` directory will contain:

| File | Description |
|---|---|
| `edited.mp4` | Re-encoded edited video |
| `proof.bin` | ZK proof (stub in Level 0; real in Level 1+) |
| `capture.assertion.json` | `org.zkedit.capture.v1` payload |
| `edit.assertion.json` | `org.zkedit.edit_proof.v1` payload |
| `capture.signed.mp4` | Original + C2PA capture manifest |
| `edited.signed.mp4` | Edited + C2PA edit-proof manifest + capture ingredient |

### Level 1 — real Nova IVC + Groth16 proof

```bash
# Build with Eva backend (first build: 10–20 min)
cargo build -p fightfake-cli --release --features eva-backend

./target/release/fightfake prove-edit \
  --input  capture.mp4 \
  --gadget brightness \
  --out-dir out/
```

Proving time depends on video length and `--blocks-per-step`.  A 10-second 352×288 clip takes ~5 min on an M2 Mac.

---

## Commands

### `prove-edit` — the main workflow

```
fightfake prove-edit [OPTIONS] --input <FILE>

Options:
  -i, --input <FILE>           Input video (MP4 or any ffmpeg-decodeable format)
      --gadget <GADGET>        Edit operation [brightness] [default: brightness]
      --gadget-param <N>       Gadget parameter (brightness: luma scale × 1/1024) [default: 416]
  -o, --out-dir <DIR>         Output directory [default: out]
      --cert <FILE>            PEM signer certificate [default: testdata/certs/signer-cert.pem]
      --key  <FILE>            PEM signer private key  [default: testdata/certs/signer-key.pem]
      --device-id <ID>         Opaque device ID in capture assertion [default: dev-0]
      --blocks-per-step <N>    Macroblocks per Nova step [default: 256]
```

### `verify` — check a signed asset pair

```
fightfake verify --capture <FILE> --edited <FILE> --proof <FILE>
```

Reads `org.zkedit.*` assertions directly from the C2PA manifests and checks:
- h1 is identical in capture and edit assertions
- proof SHA-256 matches the proof binary
- C2PA hard binding (container hash) is intact

### Low-level plumbing commands

| Command | Purpose |
|---|---|
| `emit-capture` | Emit a `org.zkedit.capture.v1` JSON (without signing) |
| `emit-edit-proof` | Emit a `org.zkedit.edit_proof.v1` JSON |
| `sign-capture-manifest` | Sign a source video and embed a capture assertion |
| `sign-edit-manifest` | Sign an edited video with an edit-proof assertion + parent ingredient |
| `verify-bundle` | Schema + linkage check against assertion JSON side-files |
| `run-level0-demo` | Legacy Level-0 demo (pre-computed h1/h2, any proof blob) |
| `print-pi-capture-contract` | Print the Raspberry Pi libcamera adapter contract |

---

## WASM — browser verifier

```bash
wasm-pack build fightfake-wasm --target web --release
```

The output (`fightfake-wasm/pkg/`) exports:

```js
import init, { verifyAssertionLinkage } from './fightfake_wasm.js';
await init();

const result = verifyAssertionLinkage(captureJson, editJson, proofBytes);
console.log(result.h1_matches, result.proof_sha_matches);
```

Cryptographic Groth16 verification in the browser is on the roadmap (`verifyGroth16Proof` returns `false` until integrated).

---

## Assertion namespace

All assertions use the `org.zkedit.*` namespace to remain scheme-neutral:

| Label | Description |
|---|---|
| `org.zkedit.capture.v1` | Device ID, pipeline stage, h1 (Griffin hash over original macroblocks) |
| `org.zkedit.edit_proof.v1` | Gadget, h1, h2, proof SHA-256, proof system metadata |

JSON Schemas are in `schemas/`.

---

## Proof levels

| Level | What is real | What is simulated |
|---|---|---|
| **0** | Edit, hashes (SHA-256), C2PA manifests | ZK proof (32-byte stub) |
| **1** | Edit, Griffin hashes, Nova IVC + Groth16 proof, C2PA | Ingest chain (trust ffmpeg decode) |
| **2** | + TEE-computed Griffin hash at capture | Trust TEE implementation |
| **3** | + Camera ISP → hash engine hardware bus | Full hardware trust |

Build `--features eva-backend` to reach Level 1.

---

## Roadmap

- [ ] Crop, grayscale, invert gadgets in `prove-edit`
- [ ] `verify-proof` command (cryptographic Groth16 check in CLI)
- [ ] WASM Groth16 verifier (`verifyGroth16Proof`)
- [ ] Level 1 Raspberry Pi demonstrator (see `docs/level1-pi-demonstrator.md`)
- [ ] Proof serialization format and public key distribution spec

---

## License

MIT
