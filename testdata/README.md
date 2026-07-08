# Test data folders

Put your local testing files here:

- `videos/input/capture.mp4` - original capture video.
- `videos/input/edited.mp4` - edited video to sign as child asset.
- `proofs/proof.bin` - proof bytes corresponding to the edited assertion.
- `certs/signer-cert.pem` - C2PA signer certificate.
- `certs/signer-key.pem` - signer private key.

Generated outputs:

- `assertions/capture.assertion.json`
- `assertions/edit.assertion.json`
- `videos/signed/capture.signed.mp4`
- `videos/signed/edited.signed.mp4`

Run the full default flow from repository root:

```bash
cargo run -- run-level0-demo --h1 0x1234 --h2 0x5678
```

If your files use different names/paths, override with flags:

```bash
cargo run -- run-level0-demo \
  --capture-input ./testdata/videos/input/my_capture.mp4 \
  --edited-input ./testdata/videos/input/my_edited.mp4 \
  --proof-path ./testdata/proofs/my_proof.bin \
  --cert-pem ./testdata/certs/my-cert.pem \
  --key-pem ./testdata/certs/my-key.pem \
  --h1 0x1234 \
  --h2 0x5678
```
