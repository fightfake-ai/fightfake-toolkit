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

# fightfake (capture + edit)
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

### When the hash is computed (standard C2PA)

In a normal C2PA workflow, the BMFF hash is computed over the **output file** —
the video as it exists *after* the declared edit has already been applied.  The
manifest is created and embedded at the end of the editing process:

```
original.mp4
    │
    ▼  editor applies brightness adjustment
edited.mp4              ← c2pa.hash.bmff is computed over these bytes
    │
    ▼  c2pa-rs signs and injects manifest into uuid box
edited.signed.mp4
  ├── uuid: { c2pa.actions: "brightness edit",
  │           c2pa.hash.bmff: SHA-256(edited container bytes) }
  ├── mdat: H.264 encoded edited frames
  └── moov: metadata
```

The manifest has **no reference to the original file**.  It records only:
1. a human-readable declaration of what was done (`c2pa.actions`), and
2. a hash of the edited encoded container (`c2pa.hash.bmff.v2`).

So standard C2PA proves **forward integrity** from the moment of signing ("this
file has not been tampered with since I signed it"), not **backward provenance**
("this file came from that specific original, and only the declared edit was
applied").  That backward link is what fightfake adds via h1, h2, ingredients,
and the ZK proof.

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
- The file has not been modified since it was signed (hard binding over the **post-edit** bytes).
- Someone with the private key matching the certificate declared a brightness edit.
- **What it cannot tell:** what the original looked like, whether the declared edit is the
  only change that was made, or whether this file is even related to any particular source
  asset (there is no ingredient link and no pixel fingerprint of an original).

---

## fightfake manifests — annotated

`prove-edit` takes one input video and produces two output MP4 files:

- **`capture.signed.mp4`** — the **original** video, unchanged, with a C2PA manifest
  embedded that records h1 (the pixel fingerprint of the original frames) and a BMFF hard
  binding over the original encoded bytes.
- **`edited.signed.mp4`** — the **edited** video (e.g. blurred, brightness-adjusted) with
  a C2PA manifest that records h2 (the pixel fingerprint of the edited frames), a reference
  to the ZK proof, and an ingredient link back to the capture manifest.

The edit manifest embeds a copy of the capture manifest inside itself, forming a
cryptographic chain: `original → capture-signed → edit-signed`.

**Why two files?**

First, the key point: the ZK proof is **self-contained in the edit manifest**.
A public verifier needs only:
- `edited.signed.mp4` (which contains h1, h2, the gadget id, and an embedded copy of the
  capture manifest)
- `proof.bin`

**The original video never needs to be published.**  h1 is a hash — it reveals nothing
about what the original looks like.  A verifier learns: *whatever the original was,
applying the declared gadget to it produces exactly this edited video, and nothing else was
changed.*  This is the zero-knowledge property: the proof discloses only what the gadget
declaration says, not the original content.

**Concrete example — drone footage with blurred faces:**
1. A drone records footage of criminal activity.
2. The owner blurs faces to protect victims, using `prove-edit`.
3. They publish only `edited.signed.mp4` + `proof.bin` (blurred video + proof).
4. They keep `capture.signed.mp4` (original footage) **private**.
5. Any public verifier can confirm: the blurred video differs from the original only by the
   blur gadget — no other pixel was changed. They never see the original.
6. If subpoenaed, the owner can produce `capture.signed.mp4` to a court or trusted
   authority, who can verify h1 matches the original footage.

So `capture.signed.mp4` is not a public artefact — it is the owner's private, signed
record of the original.  Its manifest is embedded inside `edited.signed.mp4`, so the
C2PA signature chain can be validated by public verifiers without the original file.

**Architecture for Level 2+.**  When h1 is eventually produced inside trusted camera
hardware (a TEE or secure silicon), the capture manifest is where the hardware attestation
lives: a device certificate chain, a TEE signature over h1.  The ingredient link in
`edited.signed.mp4` then chains to that hardware-rooted h1.  In Level 0 (software only),
the same software that runs the edit also computed h1, so a malicious actor could fabricate
any h1 they like.  The capture level (0→3) determines how hard it is to forge h1 — see the
README for details.

**Does h1 appear in the edit manifest too?  Yes — in two places.**
The `org.zkedit.edit_proof` assertion in the edit manifest contains both h1 and h2 directly,
so a verifier can check the proof without the capture file.  Additionally, the full capture
manifest (including its `org.zkedit.capture` assertion) is copied verbatim into the edit
manifest as the ingredient.  A verifier cross-checks that the h1 in `edit_proof` matches
the h1 in the embedded capture manifest — a mismatch would mean the proof covers a
different original than the one declared.

**Is the capture manifest the same as what `c2pa-sign` would produce on the original?  No.**
`c2pa-sign` records only a human-readable action declaration (`c2pa.actions`) and a BMFF
hard binding over the encoded container bytes — it has no knowledge of pixels.  The capture
manifest omits `c2pa.actions` but adds `org.zkedit.capture` with **h1**, the pixel
fingerprint.  h1 is what ties the original video into the ZK proof chain: the edit manifest
references the same h1, and the proof certifies that applying the gadget to pixels(h1)
produces pixels(h2).  Without h1 in the capture manifest there is nothing for the proof to
anchor to.

### Capture manifest (`capture.signed.mp4`)

File: [`example-fightfake-capture-manifest.json`](example-fightfake-capture-manifest.json)

```json
{
  "active_manifest": "urn:uuid:4f8d135e-…",
  "manifests": {
    "urn:uuid:4f8d135e-…": {

      "title": "fightfake capture: dev-0",

      "ingredients": [],     // root — no parent

      "assertions": [

        // ── 1. c2pa.actions ──────────────────────────────────────────────────
        // Optional.  Present only when the toolkit did something worth
        // declaring about the original before signing (currently unused —
        // the toolkit requires pre-aligned input and performs no implicit edits).
        // Omitted in the current example.

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

      "title": "fightfake edit proof",

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

| Field | Standard C2PA | fightfake capture | fightfake edit |
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
