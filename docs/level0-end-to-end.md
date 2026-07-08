# Level 0 End-to-End Workflow

This document shows the complete Level 0 flow currently implemented in this repository.

## Inputs

- Original capture asset: `capture.mp4`
- Edited asset: `edited.mp4`
- Proof bytes from proving pipeline: `proof.bin`
- Signer cert/key PEM files for C2PA signing.

## Steps

1. Emit capture assertion payload:

```bash
cargo run -- emit-capture \
  --device-id "demo-cam-01" \
  --pipeline-stage post_isp \
  --h1 0x1234 \
  --out ./capture.assertion.json
```

2. Emit edit-proof assertion payload:

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

3. Sign capture asset with custom assertion embedded:

```bash
cargo run -- sign-capture-manifest \
  --source-asset ./capture.mp4 \
  --dest-asset ./capture.signed.mp4 \
  --capture-assertion ./capture.assertion.json \
  --cert-pem ./certs/signer-cert.pem \
  --key-pem ./certs/signer-key.pem
```

4. Sign edited asset with ingredient + edit proof assertion:

```bash
cargo run -- sign-edit-manifest \
  --source-asset ./edited.mp4 \
  --dest-asset ./edited.signed.mp4 \
  --parent-capture-asset ./capture.signed.mp4 \
  --edit-assertion ./edit.assertion.json \
  --cert-pem ./certs/signer-cert.pem \
  --key-pem ./certs/signer-key.pem
```

5. Verify schema/linkage/proof integrity checks:

```bash
cargo run -- verify-level0-bundle \
  --capture-assertion ./capture.assertion.json \
  --edit-assertion ./edit.assertion.json \
  --proof-path ./proof.bin
```

Expected output: `ok`

## What this verifier checks

- JSON schema compliance for both custom assertions.
- `capture.h1 == edit.h1`.
- Proof SHA256 and proof byte-size match the metadata in `org.zkedit.edit_proof.v1`.

## What this verifier does not yet check

- Groth16 cryptographic verification (public inputs and key material).
- Deep C2PA policy evaluation against extracted custom assertions.

Those are planned next additions.
