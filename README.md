# FightFake Proof Prototype (Level 0 + Level 1 plan)

This repository is a starter scaffold for `fightfake-ai` focused on:

1. **Level 0 prototype** you can run today (software-only provenance flow).
2. **Level 1 Raspberry Pi demonstrator plan** with realistic implementation details.

It is intentionally **scheme-neutral** at the assertion layer so you are not tied to Eva naming.

## License note

Eva is MIT licensed, so building a separate repository that depends on Eva components is generally fine.
Keep license notices for bundled code and dependencies.

## Neutral assertion namespace

Current draft assertion names in this prototype:

- `org.zkedit.capture.v1`
- `org.zkedit.edit_proof.v1`

If you prefer a different namespace later, update `schemas/` and `src/assertions.rs`.

## Repository layout

- `schemas/`: JSON schema drafts for custom C2PA assertions.
- `src/`: Rust CLI for payload emission, C2PA embedding, verification checks, and Pi interface contract.
- `docs/level1-pi-demonstrator.md`: concrete Level 1 hardware/software plan.
- `docs/level0-end-to-end.md`: command-by-command Level 0 flow.

## Level 0 goal

Produce a reproducible manifest pair:

1. Capture-side assertion payload (`org.zkedit.capture.v1`) with `h1`.
2. Edit-side assertion payload (`org.zkedit.edit_proof.v1`) with:
   - `h1` (input chain root),
   - `h2` (output chain root),
   - proof bytes metadata,
   - circuit variant metadata.

This repository now includes:

- assertion payload emitters,
- C2PA signing commands using `c2pa-rs`,
- a verifier command for schema + proof linkage checks,
- and a first Pi/libcamera adapter contract interface.

## Eva backend strategy

The prototype treats Eva as a backend dependency:

- Default build: schema/payload tools only.
- `eva-backend` feature: enable direct linking to Eva crates for native hash/proof generation integration.

This keeps your public interface stable even if you swap proving backends later.

## Quick start

```bash
cd /Users/miha/projects/fightfake-projects/fightfake-proof-prototype
cargo run -- emit-capture --device-id "pi-cam-01" --pipeline-stage post_isp --h1 0x1234

cargo run -- emit-edit-proof \
  --proof-system nova-groth16 \
  --circuit-variant edit_only \
  --gadget-id brightness \
  --h1 0x1234 \
  --h2 0x9876 \
  --proof-path ./proof.bin
```

## Commands

### 1) Emit capture assertion JSON

```bash
cargo run -- emit-capture \
  --device-id "pi-cam-01" \
  --pipeline-stage post_isp \
  --h1 0x1234 \
  --out ./capture.assertion.json
```

### 2) Emit edit-proof assertion JSON

```bash
cargo run -- emit-edit-proof \
  --proof-system nova-groth16 \
  --circuit-variant edit_only \
  --gadget-id brightness \
  --h1 0x1234 \
  --h2 0x9876 \
  --proof-path ./proof.bin \
  --out ./edit.assertion.json
```

### 3) Sign capture asset with C2PA (`org.zkedit.capture.v1`)

```bash
cargo run -- sign-capture-manifest \
  --source-asset ./capture.mp4 \
  --dest-asset ./capture.signed.mp4 \
  --capture-assertion ./capture.assertion.json \
  --cert-pem ./certs/signer-cert.pem \
  --key-pem ./certs/signer-key.pem
```

### 4) Sign edited asset with C2PA (`org.zkedit.edit_proof.v1` + ingredient)

```bash
cargo run -- sign-edit-manifest \
  --source-asset ./edited.mp4 \
  --dest-asset ./edited.signed.mp4 \
  --parent-capture-asset ./capture.signed.mp4 \
  --edit-assertion ./edit.assertion.json \
  --cert-pem ./certs/signer-cert.pem \
  --key-pem ./certs/signer-key.pem
```

### 5) Verify Level 0 bundle consistency

This verifies:
- both assertions validate against schemas,
- `capture.h1 == edit.h1`,
- `sha256(proof.bin)` matches `proof_sha256`,
- proof size matches `proof_size_bytes`.

```bash
cargo run -- verify-level0-bundle \
  --capture-assertion ./capture.assertion.json \
  --edit-assertion ./edit.assertion.json \
  --proof-path ./proof.bin
```

### 6) Print first Pi/libcamera callback contract

```bash
cargo run -- print-pi-capture-contract
```

## C2PA integration notes

- Signing commands use `c2pa-rs` directly (`Builder`, `add_assertion`, `sign_file`).
- Capture signing embeds `org.zkedit.capture.v1`.
- Edit signing embeds `org.zkedit.edit_proof.v1` and adds the capture asset as ingredient.
- This is a pragmatic Level 0 implementation. Asserting strict cross-manifest h1 equality inside C2PA
  policy itself remains custom verifier logic (implemented in `verify-level0-bundle`).

## Pi adapter interface (first version)

The `print-pi-capture-contract` command prints the contract that a `libcamera` adapter must satisfy:

- produce contiguous YUV420 frame buffers,
- attach frame metadata (timestamp, dimensions, pixel format, frame index),
- feed every frame to a consumer callback with bounded latency.

The interface traits are defined in `src/pi_capture.rs`:

- `PiFrameSource`: source abstraction (`start`, `pump`, `stop`),
- `FrameConsumer`: per-frame callback endpoint.

## Next implementation tasks

1. Add real `libcamera` implementation behind `PiFrameSource`.
2. Add signer backends:
   - file key (dev),
   - secure element (ATECC608),
   - TEE callback signer.
3. Add C2PA verifier command reading manifests and extracting custom assertions.

