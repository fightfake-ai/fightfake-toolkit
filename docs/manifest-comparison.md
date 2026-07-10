# C2PA manifest comparison: standard vs. fightfake

This document explains what is inside a C2PA manifest, how the MP4 container
stores it, and shows annotated, side-by-side examples of a **standard C2PA
manifest** (produced by `c2pa-sign`) and the two **fightfake manifests**
(produced by `prove-edit`).

The example JSON files in this directory were generated from
`bank-robbery-original.mp4` (1920×1072 after auto-crop, 121 frames, 5 s clip)
and can be regenerated at any time:

```bash
# Standard C2PA
./target/release/fightfake c2pa-sign \
  --input testdata/videos/input/bank-robbery-original.mp4 \
  --output out/standard-signed.mp4 \
  --action c2pa.color_adjustments --description "Brightness adjustment"
./target/release/fightfake dump-manifest \
  --input out/standard-signed.mp4 | python3 -m json.tool \
  > docs/example-standard-c2pa-manifest.json

# Fightfake (capture + edit)
./target/release/fightfake prove-edit \
  --input testdata/videos/input/bank-robbery-original.mp4 --out-dir out/
./target/release/fightfake dump-manifest \
  --input out/capture.signed.mp4 | python3 -m json.tool \
  > docs/example-fightfake-capture-manifest.json
./target/release/fightfake dump-manifest \
  --input out/edited.signed.mp4  | python3 -m json.tool \
  > docs/example-fightfake-edit-manifest.json
```

---

## Where the manifest lives inside the MP4

An MP4 file is a sequence of **boxes** (also called atoms), each with a 4-byte
type code and a length.  c2pa-rs injects the manifest into a `uuid` box —
recognised by a 16-byte UUID that identifies it as a C2PA payload.

```
bank-robbery-original.mp4  (original, unsigned)
┌─────────────────────┐
│  ftyp  (file type)  │  4 bytes type + contents
├─────────────────────┤
│  mdat  (media data) │  ← the encoded H.264 bitstream lives here
│  4.4 MB             │
├─────────────────────┤
│  moov  (metadata)   │  ← codec params, timestamps, sample table
│  ~120 KB            │
└─────────────────────┘

standard-signed.mp4  (after c2pa-sign)
┌─────────────────────┐
│  ftyp               │
├─────────────────────┤
│  uuid  ◄────────────┼── C2PA manifest (JUMBF container inside uuid box)
│  ~5 KB              │     contains: assertions, claim, COSE_Sign1 signature
├─────────────────────┤
│  mdat               │  ← unchanged (same encoded bytes)
├─────────────────────┤
│  moov               │  ← unchanged
└─────────────────────┘
```

The `c2pa.hash.bmff.v2` assertion records the SHA-256 digest of every box
**except** the `uuid` box itself (which changes when the manifest is written) and
`ftyp`/`mfra` (excluded by convention).  This is the **hard binding**: any
modification to the video bytes — a single changed pixel in the H.264 stream,
a trimmed frame — produces a different SHA-256 and the manifest fails
verification.

The manifest does **not** hash raw decoded pixels.  It hashes the compressed
H.264 container bytes.  This is fast (~0.07 s for 4 MB) but says nothing about
what the pixels look like; it only says the encoded file has not changed since
signing.

---

## Standard C2PA manifest — annotated

File: [`example-standard-c2pa-manifest.json`](example-standard-c2pa-manifest.json)

```json
{
  "active_manifest": "urn:uuid:30fced02-…",    // which manifest is the current one
  "manifests": {
    "urn:uuid:30fced02-…": {

      "title": "C2PA-signed video",             // human-readable label

      "ingredients": [],                        // no parent asset (this is a root)

      "assertions": [

        // ── 1. c2pa.actions ───────────────────────────────────────────────────
        // Human-readable description of what was done.  This is a DECLARATION:
        // the signer says "I did this", but there is no proof.
        {
          "label": "c2pa.actions.v2",
          "data": {
            "actions": [{
              "action": "c2pa.color_adjustments",
              "softwareAgent": { "name": "fightfake-toolkit", "version": "0.1.0" },
              "parameters": { "description": "Brightness adjustment" }
            }]
          }
        },

        // ── 2. c2pa.hash.bmff.v2 ─────────────────────────────────────────────
        // The hard binding.  SHA-256 over the encoded MP4 container bytes
        // (excluding the uuid box that holds this very manifest).
        // If one byte of the video changes after signing, this hash breaks.
        {
          "label": "c2pa.hash.bmff.v2",
          "data": {
            "exclusions": [
              { "xpath": "/uuid" },   // the manifest box itself — excluded to
                                      // avoid chicken-and-egg
              { "xpath": "/ftyp" },   // file-type box — excluded by convention
              { "xpath": "/mfra" }    // movie-fragment random access — excluded
            ],
            "alg": "sha256",
            "hash": "cJxTnQN0/…=="   // base64(SHA-256 of all other box bytes)
          }
        }

      ],

      // ── Signature ────────────────────────────────────────────────────────────
      // COSE_Sign1 envelope: ES256 signature over the claim hash.
      // The claim hash covers all assertions above (including the BMFF hash),
      // so the signature transitively covers the encoded video bytes.
      "signature_info": {
        "alg": "Es256",
        "issuer": "fightfake-toolkit",
        "cert_serial_number": "454365…"
      }
    }
  },

  "validation_state": "Valid"   // c2pa-rs self-check result
}
```

**What this tells a verifier:**
- The file has not been modified since it was signed (hard binding).
- Someone with the private key matching the certificate declared a brightness edit.
- **What it cannot tell:** whether the pixels actually differ from some original in
  any specific way, or whether the edit is the only change that was made.

---

## Fightfake manifests — annotated

`prove-edit` produces **two signed MP4 files**, each with its own embedded manifest.
The edit manifest additionally carries the capture manifest as an ingredient, forming
a cryptographic chain: `original → capture-signed → edit-signed`.

### Capture manifest (`capture.signed.mp4`)

File: [`example-fightfake-capture-manifest.json`](example-fightfake-capture-manifest.json)

```json
{
  "active_manifest": "urn:uuid:4f8d135e-…",
  "manifests": {
    "urn:uuid:4f8d135e-…": {

      "title": "FightFake capture: dev-0",

      "ingredients": [],     // root — no parent

      "assertions": [

        // ── 1. c2pa.actions — auto-crop disclosure ────────────────────────────
        // If the video needed cropping to reach 16-pixel alignment, it is
        // recorded here so verifiers know h1 covers the cropped frame.
        {
          "label": "c2pa.actions.v2",
          "data": {
            "actions": [{
              "action": "c2pa.cropped",
              "parameters": {
                "description": "Auto-cropped 1920×1080 → 1920×1072 …"
              }
            }]
          }
        },

        // ── 2. org.zkedit.capture ─────────────────────────────────────────────
        // fightfake-specific.  Records the pixel fingerprint of the original
        // (cropped) frames.  h1 is the SHA-256 of raw YUV planes in the stub
        // build, or a Griffin hash chain in the Eva backend.
        {
          "label": "org.zkedit.capture",
          "data": {
            "assertion_type": "org.zkedit.capture.v1",
            "version": 1,
            "device_id": "dev-0",           // opaque device identifier
            "pipeline_stage": "post_isp",   // where in the pipeline h1 was computed
            "hash_algorithm": "griffin",    // declares which hash was used for h1
            "h1": "66508738fcc1fa…"         // hex pixel fingerprint of the original
          }
        },

        // ── 3. c2pa.hash.bmff.v2 ─────────────────────────────────────────────
        // Same hard binding as standard C2PA (covers the original MP4 bytes).
        { "label": "c2pa.hash.bmff.v2", "data": { "alg": "sha256", "hash": "…" } }
      ],

      "signature_info": { "alg": "Es256", "issuer": "fightfake-toolkit" }
    }
  }
}
```

### Edit manifest (`edited.signed.mp4`)

File: [`example-fightfake-edit-manifest.json`](example-fightfake-edit-manifest.json)

```json
{
  "active_manifest": "urn:uuid:e6bbee29-…",
  "manifests": {

    // ── Ingredient manifest (embedded copy of the capture manifest) ───────────
    // The capture manifest is reproduced here verbatim so verifiers can check
    // it without needing the capture file.
    "urn:uuid:4f8d135e-…": { /* … capture manifest as above … */ },

    // ── Active (edit) manifest ────────────────────────────────────────────────
    "urn:uuid:e6bbee29-…": {

      "title": "FightFake edit proof",

      // ── Ingredient link ───────────────────────────────────────────────────
      // Points back to the capture manifest by UUID.  c2pa-rs validates that
      // the referenced manifest is intact (claimSignature.validated check).
      "ingredients": [{
        "title": "capture.signed.mp4",
        "format": "video/mp4",
        "relationship": "parentOf",
        "active_manifest": "urn:uuid:4f8d135e-…"   // ← points to capture above
      }],

      "assertions": [

        // ── 1. c2pa.actions — standard edit declaration ───────────────────────
        {
          "label": "c2pa.actions.v2",
          "data": {
            "actions": [{
              "action": "c2pa.color_adjustments",
              "parameters": { "description": "Brightness adjustment (luma scale 416/1024 ≈ 0.41×)" }
            }]
          }
        },

        // ── 2. org.zkedit.edit_proof ──────────────────────────────────────────
        // The fightfake proof assertion.  Contains everything a verifier needs.
        {
          "label": "org.zkedit.edit_proof",
          "data": {
            "assertion_type": "org.zkedit.edit_proof.v1",
            "version": 1,
            "gadget_id": "brightness",           // which edit was applied
            "proof_system": "nova-groth16",      // Nova IVC + Groth16 decider
            "circuit_variant": "edit_only",      // lossless path (pixel-domain only)

            // h1 must match the h1 in the capture manifest above.
            // If they differ, the proof does not cover this capture.
            "h1": "66508738fcc1fa…",

            // h2 is the pixel fingerprint of the edited frames.
            // The ZK proof certifies: apply(gadget, pixels(h1)) == pixels(h2)
            "h2": "d3448fdef00eb1…",

            // The ZK proof blob is stored externally (proof.bin).
            // proof_sha256 lets verifiers check they have the right file.
            "proof_sha256": "66687aadf862bd…",
            "proof_size_bytes": 32   // 32 = stub; real Groth16 ≈ 192 bytes
          }
        },

        // ── 3. c2pa.hash.bmff.v2 ─────────────────────────────────────────────
        // Hard binding over the *edited* MP4 bytes.
        { "label": "c2pa.hash.bmff.v2", "data": { "alg": "sha256", "hash": "…" } }
      ],

      "signature_info": { "alg": "Es256", "issuer": "fightfake-toolkit" }
    }
  },

  // validation_results shows c2pa-rs checked both signatures and the ingredient
  // chain, and all passed.
  "validation_state": "Valid"
}
```

---

## Side-by-side summary

| Field | Standard C2PA | Fightfake capture | Fightfake edit |
|---|---|---|---|
| `ingredients` | empty | empty | capture manifest (by UUID) |
| `c2pa.actions` | declared edit | auto-crop disclosure | declared edit |
| `c2pa.hash.bmff.v2` | ✅ hard binding | ✅ hard binding | ✅ hard binding |
| `org.zkedit.capture` | — | ✅ h1, device_id, pipeline_stage | — (in ingredient) |
| `org.zkedit.edit_proof` | — | — | ✅ h1, h2, gadget_id, proof_sha256 |
| `validation_state` | Valid | Valid | Valid |

**Verification chain:**

```
prove-edit run
  │
  ├─ original.mp4 ──────────────────────────────────────────────────────────────
  │    ↓  auto-crop to 1920×1072                                               │
  │    ↓  SHA-256 over raw YUV → h1                                            │
  │    ↓  c2pa-rs: SHA-256 over encoded MP4 bytes → BMFF hash                 │
  │    ↓  ECDSA sign (COSE_Sign1)                                              │
  └─ capture.signed.mp4 ← manifest: {h1, BMFF hash, device_id}               │
       │                                                                        │
       │  ingredient link (UUID reference + embedded copy of capture manifest) │
       │                                                                        │
  ├─ edited.mp4 ────────────────────────────────────────────────────────────────
  │    ↓  SHA-256 over edited YUV → h2
  │    ↓  ZK proof: apply(brightness(416), pixels(h1)) == pixels(h2)
  │    ↓  c2pa-rs: SHA-256 over edited MP4 bytes → BMFF hash
  │    ↓  ECDSA sign
  └─ edited.signed.mp4 ← manifest: {h2, proof_sha256, c2pa.actions, ingredient→capture}
       │
  proof.bin (external) ← referenced by proof_sha256 in the edit manifest
```
