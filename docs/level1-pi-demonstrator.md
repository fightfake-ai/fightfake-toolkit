# Level 1 Raspberry Pi Demonstrator Plan

This document describes a practical Level 1 demonstrator where hash/signing runs in a trusted
execution path on a Raspberry Pi camera stack.

## Goal

Demonstrate the industry integration pattern:

1. Capture pre-encode frames on-device.
2. Convert to Eva macroblock layout.
3. Compute h1 in a trusted component.
4. Sign h1 with a device key.
5. Emit `org.zkedit.capture.v1`.
6. Later, produce `org.zkedit.edit_proof.v1` off-device.

## Hardware

- Raspberry Pi 5 (or Pi 4 with sufficient thermal headroom).
- Official Pi Camera Module 3 (or HQ camera).
- microSD 64GB+.
- Optional secure element (ATECC608A) over I2C for device key isolation.

## Software stack

- Raspberry Pi OS Lite (64-bit).
- `libcamera` for frame capture.
- Rust service:
  - frame callback adapter,
  - YUV -> macroblock conversion,
  - Griffin hashing backend,
  - capture assertion emitter.
- Optional OP-TEE path if using platform with full TrustZone userspace TA toolchain.

Current repository status:

- Level 0 C2PA embedding commands exist (`sign-capture-manifest`, `sign-edit-manifest`).
- Level 0 verifier command exists (`verify-level0-bundle`).
- First Pi capture interface contract exists in `src/pi_capture.rs` and is printable with
  `print-pi-capture-contract`.

## Data path (Level 1)

1. Camera ISP outputs YUV420 frames.
2. Capture service receives frames before final H.264 encode.
3. Service tiles frame into macroblocks (`orig_y_enc`, `orig_u_enc`, `orig_v_enc` order).
4. Hash service updates rolling h1.
5. On stop: hash finalization + signature.
6. Write:
   - recording file (`.mp4`),
   - capture assertion JSON (`org.zkedit.capture.v1`),
   - signed metadata envelope (COSE/CBOR, later C2PA embedding).

## Trusted component options

### Option A (pragmatic first build): process isolation + secure element

- Hashing runs in dedicated daemon process.
- Device signing key is in ATECC608 (private key non-exportable).
- Strong demo value, easy to deploy.

### Option B (stronger Level 1): TEE TA for hash + signing

- Camera process in Normal World forwards frame chunks to TEE.
- Griffin hash and signing execute in Secure World TA.
- Better representation of production mobile/embedded integration.

## Sprint plan

### Sprint 1 (1 week): Capture + macroblock conversion

- CLI capture tool (`capture-pi`) that records 10s YUV clips.
- Deterministic macroblock tiling unit tests.
- Throughput benchmark at 1080p30.
- Implement `PiFrameSource` using `libcamera` callback plumbing.

Deliverable: reproducible macroblock stream + baseline FPS metrics.

### Sprint 2 (1 week): Rolling h1 + device signing

- Integrate Griffin rolling hash backend.
- Add key provider trait:
  - file-based dev key provider,
  - secure element provider.
- Emit `org.zkedit.capture.v1` JSON payload.
- Connect output directly to `sign-capture-manifest`.

Deliverable: capture session outputs `h1`, signature, and payload file.

### Sprint 3 (1 week): Level 0 bridge + proof metadata

- Off-device workflow:
  - run edit/prove pipeline,
  - produce proof bytes,
  - emit `org.zkedit.edit_proof.v1`.
- Connect to this repository's CLI.
- Automate `verify-level0-bundle` in CI for sample assets.

Deliverable: capture payload + edit-proof payload pair for one sample clip.

### Sprint 4 (1 week): Manifest integration and demo UX

- Integrate `c2pa-rs` embedding.
- Build one end-to-end public demo asset set for fightfake.ai:
  - original signed capture,
  - edited output,
  - verification result page.

Deliverable: public demonstration bundle.

## What this proves vs does not prove

### Proves

- Device-origin h1 commitment at capture time.
- Binding between capture assertion and edit proof assertion.
- Reproducible Level 1 architecture for manufacturers.

### Does not prove

- Sensor/ISP authenticity against hardware injection attacks.
- Pixel-bus tamper resistance before software callback.

Those are Level 2 concerns (dedicated silicon tap/hash block).

## Manufacturer-facing artifact checklist

- Integration callback contract:
  - frame format,
  - callback timing,
  - error handling.
- Hash/sign API surface and expected latency budget.
- Assertion schema versioning policy.
- Benchmark sheet:
  - CPU %, memory, power draw at 720p/1080p.
- Threat model table distinguishing Level 0/1/2 guarantees.
- Frame callback contract document generated from `PiFrameSource` API.
