# fightfake-toolkit

A toolkit for proving that a video edit is genuine — and for verifying that claim without
trusting the editor.  This is the open-source toolkit behind [fightfake.ai](https://fightfake.ai).

---

## The problem

When someone shares an edited video — a colour-corrected aerial shot, a brightness-adjusted
security clip, a cropped news footage — there is currently no way to verify that the only
change made was the declared edit.

**C2PA covers part of this problem, but not all of it.**  The
[C2PA standard](https://c2pa.org) (used by Adobe Photoshop, Lightroom, and many cameras
today) lets a signer *declare* what edits were made and attach a certificate to that
declaration.  You can open such a file in a C2PA viewer and read: "brightness was adjusted,
signed by Adobe".  What you cannot do is *verify* that claim independently — you have to
trust that Adobe's signing pipeline was not compromised, that the signer's private key was
not misused, and that the description is accurate.  A deep-fake with a stolen or misleading
certificate is indistinguishable from a legitimate edit.

**fightfake-toolkit** closes that gap.  Instead of a declaration backed by institutional
trust, it produces a **mathematical proof** (a [zero-knowledge proof](https://en.wikipedia.org/wiki/Zero-knowledge_proof)) — a compact blob of bytes that any verifier can
check independently, without trusting the signer, without access to the original footage, and
without any knowledge of who produced the video.  The proof shows that a specific pixel-level
transformation — whole-frame (brightness, grayscale, invert) or scoped to one region and a
handful of frames (`redact`, e.g. blacking out a single bystander's face for a couple of
seconds) — is the *only* difference between the captured original and the published version.
If even a single pixel was changed in any other way, the proof does not verify.

In short:
- **Standard C2PA** records a signed declaration of what was edited; a verifier trusts the
  signer and their pipeline.
- **fightfake-toolkit** adds a zero-knowledge proof of that claim; a verifier checks the math
  independently of who signed.

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

This toolkit produces C2PA manifests automatically.  Existing tools such as the
[Digimarc C2PA Content Credentials extension](https://chromewebstore.google.com/detail/c2pa-content-credentials/mjkaocdlpjmphfkjndocehcdhbigaafp)
and the [Content Credentials verify page](https://verify.contentauthenticity.org) already know
how to read them.

### Layer 2 — pixel fingerprints (h1 and h2)

fightfake-toolkit computes two hash values over the video pixels:

- **h1** — a hash over the *original* pixels at capture time.
- **h2** — a hash over the *edited* pixels.

Both hashes are embedded in the C2PA manifests and linked by the ingredient chain.  This
makes the capture-to-edit relationship explicit and machine-checkable, rather than a
human-readable text note.

At the software level (Level 0/1), these are SHA-256 hashes of the raw YUV planes.  With
full hardware integration (Levels 2/3), they are [Griffin](https://eprint.iacr.org/2022/403)
hash chains computed directly from the camera's pixel bus — a construction designed to be
efficiently provable inside a zero-knowledge circuit.

### Layer 3 — the zero-knowledge proof

The ZK proof (produced by [Eva](https://github.com/fightfake-ai/eva)) is a single compact
blob (~200 bytes for BN254/Groth16) that proves, without revealing the original frames:

> "The edited pixels are the result of applying the declared edit gadget to the original
> pixels identified by h1, producing h2. No other change was made."

Standard C2PA can declare that claim; it cannot prove it.  The ZK layer supplies an
unforgeable mathematical certificate that *only* the declared edit occurred.  A verifier
checks the proof in milliseconds.  Under Nova IVC + Groth16, generating a convincing false
proof is computationally infeasible — including for the person who signed the video.

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

#### Certificates and C2PA signatures

A digital certificate is a small file that bundles a **public key** with an **identity**
(name, organisation, …), signed by a Certificate Authority (CA) confirming they belong
together.  The corresponding **private key** is kept secret and is used to produce
signatures.

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
out/capture.signed.mp4    ← the ORIGINAL video + C2PA manifest (h1 + device ID + BMFF hard binding)
out/edited.signed.mp4     ← the EDITED  video + C2PA manifest (h2 + proof reference + link to capture)
out/proof.bin             ← ZK proof (or 32-byte stub in stub build)
```

**Two signed MP4 files.** The ZK proof is **self-contained in the edit manifest**.  A public
verifier needs only `edited.signed.mp4` (which contains h1, h2, the gadget id, and an
embedded copy of the capture manifest) plus `proof.bin`.  **The original video never needs
to be published.**

h1 is a hash — it reveals nothing about the original content.  A verifier learns that
whatever the original was, applying the declared gadget produces exactly this edited video,
and nothing else was changed.  That is the zero-knowledge property.

**Example — drone footage.** A drone records sensitive activity; the owner blurs faces with
`prove-edit` and publishes only `edited.signed.mp4` + `proof.bin`, keeping
`capture.signed.mp4` private.  Any verifier can confirm the blur is the only change.  If the
original is later disclosed (e.g. under subpoena), a court can check that h1 matches it.

`capture.signed.mp4` is therefore a **private evidence artefact**, not something that needs
to be published alongside the proof.  Its manifest is embedded inside `edited.signed.mp4`
so the C2PA signature chain validates without the original file.

For Level 2+ hardware cameras, the capture manifest is where the hardware attestation
(device certificate chain, TEE signature over h1) will live, making h1 itself
hardware-rooted rather than software-computed.  In Level 0, h1 is software-computed
scaffolding — see [_Capture levels_](#capture-levels--trustworthiness-of-h1).

`c2pa-sign` produces a completely different, simpler thing: a single signed file with only a
`c2pa.actions` declaration and a BMFF hash.  It is not one of the two fightfake files.

**16-pixel alignment requirement:** Eva's IVC circuit works on 16×16 macroblocks, so both
width and height must be exact multiples of 16.  The toolkit enforces this and exits with
a clear error and an ffmpeg command if your video does not meet the requirement.  Many
common resolutions already satisfy it (e.g. 1920×1072, 1280×720, 1280×960); many others
do not (e.g. 1920×1080 — 1080 ÷ 16 = 67.5).  See
[_16-pixel alignment_](#16-pixel-alignment--requirement-and-future-options) below for the
reasoning and future options.

**Macroblocks.** Eva's IVC circuit processes video one 16×16 pixel block at a time.  Each
IVC step proves that the edit gadget was applied correctly to one (or more) macroblocks and
that the hash chain (h1 or h2) advanced correctly.  The proof is therefore incremental: a
1-second clip and a 10-minute clip use the same per-step circuit, just with more steps.

**Stub build vs. full build:** by default (no `--features eva-backend`) the proof is a
32-byte zero placeholder — call this the *stub build*.  The edit, hashes, and C2PA manifests
are real and fully usable for integration testing.  Build with `--features eva-backend` for
a real Nova IVC + Groth16 proof — the *full build*.

This is different from the **capture levels** (Level 0–3) described in the
[trust model section](#capture-levels--trustworthiness-of-h1).  The levels are about how
much you trust h1 (the original pixel fingerprint): Level 0 means a software app computed
h1 from an existing recording; Level 3 means silicon-level hardware produced it.  The ZK
proof is cryptographically sound regardless of the level — the level only tells you how hard
it is for an attacker to substitute a different recording before h1 is computed.

**Timing:** at the end of the run a table is printed showing time per phase.  Below are
real measurements on `bank-robbery-original.mp4` (1920×1072 after auto-crop, 121 frames,
5 s clip), averaged over 3 runs on an Apple M1 MacBook Pro (stub build, no ZK proof):

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
With the Eva backend, Griffin replaces SHA-256 for h1/h2; see below for why.

‡ C2PA signing covers the encoded H.264 file (≈4 MB), not raw pixels.  See the
[C2PA signing section](#how-c2pa-rs-signs-the-video) for details.

Measured on an Apple M1 MacBook Pro.  To reproduce on your machine:

```bash
./bench.sh testdata/videos/input/bank-robbery-original.mp4 3
```

With `--features eva-backend`, the "ZK proving" row dominates. For a 5-second 1920×1072
clip that is typically several minutes on an M1 Mac; for a 4K redact with `--touched-window`
see the [measured `demos1` results](#measured-results-demos1-touched-window)
(~88–92 minutes for 12 touched frames on an M1 MacBook Pro, depending on `--blocks-per-step`).

### Step 2b — editing only a small region for a couple of seconds (`redact`)

`brightness`/`grayscale`/`invert` apply to every pixel of every frame.  A common real case is
narrower: black out (or eventually blur) *one* rectangle — e.g. a bystander's face — for only
a second or two, and leave the rest of the clip untouched.  The `redact` gadget does exactly
this: it overwrites a fixed pixel rectangle with a solid fill colour, only for a chosen frame
range, and copies every other pixel and every other frame through byte-for-byte unchanged.

```bash
./target/release/fightfake prove-edit \
  --input testdata/videos/input/demos1.mp4 \
  --gadget redact \
  --redact-x 2464 --redact-y 1312 --redact-width 512 --redact-height 704 \
  --redact-frame-start 52 --redact-frame-end 64 \
  --redact-fill 0 \
  --out-dir out/demos1
```

This redacts the one clearly-recognisable, unmasked face in `demos1.mp4` (cap + full beard,
looking straight at the camera around t≈2.4s). Note the frame range here is short (12 frames,
≈0.5s) — see [_picking a box on shaky, handheld footage_](#picking-a-box-on-shaky-handheld-footage)
below for why, and what a longer redaction window would require.

| Flag | Meaning |
|---|---|
| `--redact-x`, `--redact-y` | Top-left pixel of the rectangle |
| `--redact-width`, `--redact-height` | Rectangle size in pixels |
| `--redact-frame-start` (inclusive), `--redact-frame-end` (exclusive) | Which frames get redacted; every other frame is untouched |
| `--redact-fill` | Luma fill value inside the rectangle (`0` = black); chroma is always set to neutral (128) |

**How to find the numbers for your own video.** There is no face detector wired in — you pick
the box by looking at the frames:

```bash
# 1. Find frame rate / frame count.
ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,r_frame_rate,nb_frames \
  testdata/videos/input/demos1.mp4
# → 3840×2160, 24000/1001 fps (≈23.976), 168 frames, 7.0 s

# 2. Extract a still around the moment the face is visible and crop-preview a candidate box.
ffmpeg -ss 2.4 -i testdata/videos/input/demos1.mp4 -frames:v 1 \
  -vf "crop=512:704:2464:1312" preview.png
# open preview.png, nudge x/y/w/h until the box frames the face, then convert
# the timestamp(s) you want covered to frame numbers: frame ≈ seconds × fps.
```

Picking box coordinates that are multiples of 16 (as in the example above) costs nothing today
and keeps the option open for a future `--features eva-backend` prover, since Eva's macroblock
grid is 16×16.

#### Picking a box on shaky, handheld footage

`demos1.mp4` is handheld crowd footage: the camera pans and shakes, people move, and other
people's arms/signs pass in front of the lens. A `redact` box is fixed relative to the
**frame**, not to the person — it does not track anything. Scrubbing through this clip
frame-by-frame around the subject shows the actual situation:

| time | what's at box `(2464, 1312, 512×704)` |
|---|---|
| t ≈ 2.0–2.2s | subject hasn't entered this part of the frame yet |
| t ≈ 2.3–2.6s | subject's face is fully visible and unoccluded — the usable window |
| t ≈ 2.7–2.8s | a raised arm starts crossing in front of his face |
| t ≈ 2.9–4.3s | a cardboard sign held close to the camera blocks this whole region |

So the *reliably redactable* window with one fixed box is closer to half a second (12 frames)
than the full ~2 seconds a viewer perceives the person as "in the video" — the rest of the
time he's either out of this exact box or genuinely occluded by something else in the crowd.
Two honest ways to actually cover a longer span:

1. **Enlarge the box** enough to contain his whole range of motion across the window you want
   (verify frame-by-frame, the way this example was built — the box does not need to be
   tight, it only needs to stay wide enough for the whole window).
2. **Track a moving box per frame** instead of one fixed rectangle — see
   [_moving redact rectangle_](#moving-redact-rectangle---redact-track) below.
   Occlusion by other things in the scene is still a hard limit, but camera motion and small
   subject motion stop being one.

Neither of these is specific to this toolkit — it's the same reason real redaction tools (e.g.
newsroom face-blurring software) use frame-by-frame tracking rather than one static box.

#### Moving redact rectangle (`--redact-track`)

Instead of one fixed box for the whole `[--redact-frame-start, --redact-frame-end)` range, you
can supply a sparse list of keyframes — `{frame, x, y, width, height}` — and the box is
linearly interpolated between them (each of x/y/width/height independently). Before the first
keyframe and after the last, that keyframe's box is held constant, so you don't have to supply
one keyframe per frame — a handful of points along the subject's path is enough.

```json
[
  { "frame": 52, "x": 2200, "y": 1200, "width": 512, "height": 704 },
  { "frame": 58, "x": 2464, "y": 1312, "width": 512, "height": 704 },
  { "frame": 63, "x": 2700, "y": 1400, "width": 512, "height": 704 }
]
```

```bash
./target/release/fightfake prove-edit \
  --input testdata/videos/input/demos1.mp4 \
  --gadget redact \
  --redact-track path/to/track.json \
  --redact-frame-start 52 --redact-frame-end 64 \
  --redact-fill 0 \
  --out-dir out/demos1-track
```

`--redact-track` overrides `--redact-x/-y/-width/-height` when set (you no longer need to pass
those). Everything else about `redact` — the frame gate, the fill colour, `--touched-window` —
works exactly the same; only *where* the box sits at each frame changes.

**Proof cost is unaffected.** `RedactRectCfg` is already built per-macroblock, per-frame (see
[_how redact maps onto Eva's RedactRect gadget_](#how-redact-maps-onto-evas-redactrect-gadget));
a moving box only changes the *values* fed into that per-macroblock config (which pixels count
as "inside the box" for a given frame), not the R1CS shape, the number of Nova steps, the
witness/commitment layout, or the resulting proof size. A tracked redaction over the same frame
range as a fixed-box redaction costs the same to prove.

The recorded `gadget_params` reflect this — instead of a single `x`/`y`/`w`/`h`, they carry the
`track` array verbatim, and the rendered `c2pa.actions` description reads e.g.:

> Blacked out a moving pixel region (tracked across 3 keyframe(s)), frames 52–64 only (fill
> value 0). All other pixels and frames are unchanged.

**Scoped edit vs whole-clip gadgets.** h1/h2 still cover the *entire* video (the overall
pipeline is unchanged), but the declared edit — and, once wired to the real prover, the ZK
proof — is scoped to exactly the pixels and frames that changed.  A verifier (or a human
reading the manifest) sees precisely what changed and can confirm that the remaining pixels
are unchanged relative to the original, instead of relying on a blanket whole-clip claim
such as "brightness was applied".

The exact rectangle and frame range are recorded in the `org.zkedit.edit_proof` assertion's
`gadget_params` field and rendered into the standard `c2pa.actions` description, e.g.:

> Blacked out a 512×704 pixel region at (2464, 1312), frames 52–64 only (fill value 0).
> All other pixels and frames are unchanged.

**Proof modes.** Without `--features eva-backend`, `redact` still produces a real edit, real
h1/h2, and signed C2PA manifests, but the proof is a 32-byte stub. Build with
`--features eva-backend` for a real Nova IVC + Groth16 proof over Eva's `RedactRect` gadget,
with a compact per-macroblock config (`RedactRectCfg`) derived from the declared rectangle and
frame window. Macroblocks fully inside the box use a single plane-wide replace flag (like
`Removing`); edge macroblocks carry only the per-pixel replace bits they need — not a full
384-entry `(fill, replace?)` mask. This keeps the h2 config hash and witness count much
smaller than the generic `Masking` gadget.

#### How `redact` maps onto Eva's `RedactRect` gadget

Eva's `RedactRect` edit gadget (`video/src/edit/constraints.rs`) takes a compact
`RedactRectCfg` per macroblock: rectangle bounds (clamped to the frame), macroblock origin,
whether the current frame is inside `[frame_start, frame_end)`, fill luma, and either
`full_y`/`full_u`/`full_v` (whole 16×16 / 8×8 plane replaced) or a sparse partial bitmask
on edge macroblocks only. Native and circuit paths apply the same logic: chroma is always
filled with neutral 128 inside the box.

`RedactRectCfg` carries no frame number, because it doesn't need one. Eva's Nova IVC walks
macroblocks in one long, strictly-ordered sequence — global index
`= frame_index × macroblocks_per_frame + macroblock_index` (see `yuv420_to_macroblocks` /
`macroblocks_to_yuv420` in `video/src/macroblock_yuv.rs`) — and `fs.prove_step(...)` can
already be given a different config per macroblock within each step. Temporal and spatial
targeting of a redaction box therefore falls out entirely from *which config is supplied at
which position in that sequence*; since width/height/frame count are public, the position ⇔
`(frame, row, col)` mapping is unambiguous to a verifier too.

With `--features eva-backend`, `prove-edit` builds that config per macroblock at prove time:
for each Nova step it calls `RedactRectCfg::from_rectangle(...)` for each macroblock index,
using the macroblock's pixel origin and the declared `(x, y, w, h)` / frame range. The native
edit path uses the same `RedactRect` gadget, so h2 from the reference edit matches what the
circuit proves.

#### Proving only the touched time window (`--touched-window`)

Even with per-macroblock `RedactRectCfg` variation wired up, proving *every* macroblock of a long
clip just to redact a couple of seconds is wasteful. Take `demos1.mp4`: 3840×2160 → 240×135 =
32,400 macroblocks per frame, × 168 frames = **5,443,200 macroblocks** for the whole 7-second
clip. Our redaction only touches frames 52–64 (12 frames). Running the full Nova IVC + Groth16
decider over 5.4M macroblocks to prove a 12-frame edit is not a proportionate cost.

The key observation: outside the redacted window, `RedactRectCfg` is the identity (no pixels
replaced) — i.e. "prove that edited pixel = original pixel" for those macroblocks. But that
specific claim ("these bytes are exactly these other bytes") is exactly what a **plain hash**
already proves — you don't need a SNARK to prove `A = A`; you only need the SNARK to constrain
the macroblocks that actually changed, so a verifier can be sure the *only* change is the
declared one.

`prove-edit --gadget redact --touched-window` implements exactly this: it splits the macroblock
sequence into three segments around `[--redact-frame-start, --redact-frame-end)` and treats them
differently.

```bash
cargo build -p fightfake-cli --release --features eva-backend

./target/release/fightfake prove-edit \
  --input testdata/videos/input/demos1.mp4 \
  --gadget redact \
  --redact-x 2464 --redact-y 1312 --redact-width 512 --redact-height 704 \
  --redact-frame-start 52 --redact-frame-end 64 \
  --redact-fill 0 \
  --touched-window \
  --blocks-per-step 64 \
  --out-dir out/demos1-tw
```

| segment | frames | macroblocks | how it's attested |
|---|---|---|---|
| PRE  | 0–51    | 1,652,400 | plain SHA-256 over the raw bytes (no circuit) |
| MID (touched) | 52–63 | 388,800 | real Nova IVC + Groth16, `RedactRectCfg` varying per macroblock |
| POST | 64–167  | 3,402,000 | plain SHA-256 over the raw bytes (no circuit) |

The published `h1`/`h2` become a small combination of the three segment hashes:

```
h1 = SHA256("pre" ‖ h1_pre ‖ "mid" ‖ h1_mid ‖ "post" ‖ h1_post)
h2 = SHA256("pre" ‖ h2_pre ‖ "mid" ‖ h2_mid ‖ "post" ‖ h2_post)
```

where `h*_pre`/`h*_post` are ordinary SHA-256 hashes computed directly over the untouched
original/edited pixel bytes, and `h*_mid` are SHA-256 hashes over just the touched window's
pixel bytes — the segment the real Nova IVC/Groth16 circuit actually walks. Both segment
breakdowns (`h1_segments`, `h2_segments`) plus the frame range are recorded in the
`org.zkedit.edit_proof.v1` assertion's new `touched_window` field, so a verifier — or a human —
can see exactly what's covered without re-deriving anything:

```json
"touched_window": {
  "frame_start": 52, "frame_end": 64, "num_frames": 168,
  "h1_segments": { "pre": "…", "mid": "…", "post": "…" },
  "h2_segments": { "pre": "…", "mid": "…", "post": "…" }
}
```

Because the edited video is public, anyone can independently decode it and recompute
`h2_segments.pre`/`h2_segments.post` directly from its pixels to confirm the untouched regions
really are what's claimed — no ZK verifier or original video required for that part. (`h1`'s
pre/post segments can't be independently re-derived without the original video, same as the
whole-clip `h1` today — that's expected, it's an opaque capture-time commitment either way.)

End to end this gets the same guarantee as before — `h1` still commits to every byte of the
original video, `h2` still commits to every byte of the edited video — but the expensive part
(ZK proving) now only has to cover the 388,800 macroblocks that could actually differ, not all
5.44M. That's roughly a **14×** speedup just from windowing to the touched frames. Scoping down
further to just the macroblocks the redaction box overlaps within those frames (rather than
whole frames) would buy up to ~320× and remains a possible future refinement — see the
[Roadmap](#roadmap).

`--blocks-per-step` must evenly divide the touched window's macroblock count (`--touched-window`
fails fast with a clear error otherwise, rather than silently proving a truncated slice);
`--blocks-per-step <macroblocks-per-frame>` (one full frame per Nova step) always works, since
the touched window is always a whole number of frames.

**Memory vs speed.** Peak RAM during Nova synthesis grows with `--blocks-per-step` (each step
builds a larger in-memory circuit). On an Apple M1 MacBook Pro with `--features eva-backend`
and the `demos1` touched window (388,800 macroblocks), observed behaviour:

| `--blocks-per-step` | Nova steps | Result on M1 MacBook Pro |
|---|---|---|
| 64 | 6,075 | completes (~91.5 min total) |
| 720 | 540 | completes (~87.9 min total) |
| 1,620 | 240 | `zsh: killed` (OOM during Nova synthesis) |
| 8,100 | 48 | `zsh: killed` (OOM during Nova synthesis) |
| 32,400 (1 frame) | 12 | `zsh: killed` (OOM during Nova synthesis) |

If the process exits with `zsh: killed` and no Rust backtrace, macOS almost certainly ran out
of RAM and terminated the prover. Pick a smaller `--blocks-per-step` (must still divide
388,800 evenly — e.g. 720, 540, 480, 320, 160, 96, 80, 64) or close other memory-heavy apps.
The default `256` does **not** divide 388,800 and will be rejected. On this hardware, raising
`--blocks-per-step` from 64 → 720 cuts Nova steps 11× but only saves ~4% proving time, because
per-step cost grows with step size. The practical RAM ceiling sits between **720** (safe) and
**1,620** (OOM) — use `720` as the working upper bound for 4K touched-window runs on a laptop.

#### Measured results (demos1, touched window)

Measured on an Apple M1 MacBook Pro with `--features eva-backend` and the command above
(`--touched-window`, frames `[52, 64)` → 388,800 macroblocks in the MID segment):

| Phase | 64 blocks/step (6,075 steps) | 720 blocks/step (540 steps) |
|---|---|---|
| ffmpeg decode | 0.99s | 1.52s |
| macroblock tiling | 0.36s | 1.36s |
| edit + hashing (h1, h2) | 34.52s | 37.55s |
| ZK proving (Nova IVC + Groth16) | 5437.52s (~90.6 min) | 5218.91s (~87.0 min) |
| ffmpeg re-encode | 10.08s | 10.44s |
| C2PA signing | 0.20s | 0.21s |
| **Total** | **5487.88s (~91.5 min)** | **5276.04s (~87.9 min)** |

Both runs produce identical published hashes (confirms deterministic edit + segment combine):

```
h1 = 43bce49c8f1a482b325b6ed186df3bc83a59610c4d23cb4315430a924d11ca7c
h2 = f05434d7b1d6f70bb97adf719dd07b9c0514557e5faaa871e259458ea1184317
```

Segment breakdown (frames `[52, 64)` of 168) — same for both runs:

| segment | h1 | h2 |
|---|---|---|
| pre  | `b3809ea9c1606374fe0f5bcee16269d135a14ac2338255044070dde411c5ef93` | `b3809ea9c1606374fe0f5bcee16269d135a14ac2338255044070dde411c5ef93` |
| mid  | `415f698753214d1cce677e492f3793b0e7a47aba53b4e89693fabcc3a45544dc` | `212fc9d3f0b4cc19a92b2c314a0dee6dace60585b90d4144da71a2edb1eeef57` |
| post | `ef555c2dc95d81c3bb8315fac453fd133ad14d92e22b0af0e7ce77f407eafd28` | `ef555c2dc95d81c3bb8315fac453fd133ad14d92e22b0af0e7ce77f407eafd28` |

The matching pre/post rows confirm that only the touched window differs between original and
edited video; the mid segments differ because that is where the redaction ran. For the 64
blocks/step run, Nova preprocess reported 6,865 variable-creation constraints, 135,988
step-circuit constraints, and 58,469 fold-circuit constraints (13,361 primary + 45,108
CycleFold). Both runs emit a 256-byte Groth16 proof (`out/demos1-tw/proof.bin`,
`out/demos1-tw-720/proof.bin`).

This only pays off because "untouched" has a cheap, well-defined meaning outside the circuit
(byte-identical, provable by a plain hash). It would **not** help a gadget like `brightness`
that touches every pixel of every frame — there, every macroblock is already "touched" and has
to go through the real circuit regardless, so `--touched-window` is rejected for anything other
than `redact`.

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

### Step 4b — cryptographically verify the proof itself

`verify` (above) checks hashes and C2PA signatures, but not the ZK proof's math. To run the
actual Nova IVC + Groth16 pairing check:

```bash
./target/release/fightfake verify-proof --proof out/proof.bin
```

Needs `--features crypto-verify` (implied by `eva-backend`, since if you can prove you should
be able to verify what you proved). Without it, this prints a clear error instead of silently
skipping the check. A Level-0 stub `proof.bin` (32 zero bytes) is correctly reported as "not a
cryptographic proof" rather than passing.

Note `proof.bin` is now bigger than just the ~200–450-byte Groth16 SNARK proof mentioned
elsewhere in this doc (megabytes, for a real clip) — Eva's Groth16 setup here is per-proof
(fresh randomness on every `prove-edit` run), *not* a universal trusted setup, so the
verifying key can't be assumed and shared out-of-band the way it would be for e.g. a
production SNARK with a circuit-specific ceremony. `proof.bin` bundles that key together with
the folded Nova instances and the compact SNARK proof itself, so anyone with just the one
file can verify it — see `fightfake_core::proof_bundle::ProofBundle`.

### Step 5 — verify in the browser (WASM)

Build the browser bundle:

```bash
wasm-pack build fightfake-wasm --target web --release
# output: fightfake-wasm/pkg/  (assertion/hash checks only, ~50 KB)

# with real Groth16 verification too (~700 KB release, wasm-opt'd):
wasm-pack build fightfake-wasm --target web --release --features crypto-verify
```

`getrandom` (pulled in by the arkworks stack) needs its wasm32 browser-`crypto` backend
enabled explicitly — see `.cargo/config.toml` at the repo root; `wasm-pack`/`cargo` pick it up
automatically as long as you build from within this repository.

Use it in a web page:

```js
import init, { verifyAssertionLinkage, verifyGroth16Proof } from './fightfake_wasm.js';
await init();

// captureJson / editJson: the org.zkedit.* assertion JSON strings
// proofBytes: Uint8Array of proof.bin
const result = verifyAssertionLinkage(captureJson, editJson, proofBytes);

if (result.h1_matches && result.proof_sha_matches) {
  console.log(`Edit proven: ${result.gadget_id()} by ${result.proof_system()}`);
} else {
  console.log('Assertion linkage failed.');
}

// The actual cryptographic check (requires the `crypto-verify` build above).
// Throws if proofBytes isn't a real proof bundle (e.g. a Level-0 stub);
// otherwise returns true/false for whether the pairing check passed.
try {
  const cryptoOk = verifyGroth16Proof(proofBytes);
  console.log(cryptoOk ? 'Proof cryptographically valid' : 'Proof INVALID');
} catch (e) {
  console.log('Not a cryptographic proof:', e.message);
}
```

What the browser checks with the default build: h1 consistency between the two assertions and
SHA-256 of the proof binary. With `--features crypto-verify`, `verifyGroth16Proof` additionally
runs the real Groth16/Nova-decider pairing check — see "WASM verification checks" below
and `fightfake_core::proof_bundle`'s doc comment for why this is a from-scratch reimplementation
of a small slice of Eva's decider math rather than a direct dependency on Eva's own prover crate
(that crate hard-requires native threads and cannot target `wasm32-unknown-unknown` at all).

---

## Verification trust model

For third-party media pages, the browser extension model is the secure default.
Any verification path controlled by the same page operator can be replaced with
"always valid" logic. A secure verifier must be distributed independently of the
publisher's page code.

### Browser extension (primary security model)

A browser extension is installed once from the Chrome or Firefox extension store.  After
that it runs automatically on every page the user visits — no extra steps.

Example: on proofdrop.ai a journalist publishes an article with a signed video.  A user with
the fightfake extension installed visits the page normally.  The extension automatically
detects the C2PA manifest embedded in the video, fetches `proof.bin`, runs Groth16
verification using its own bundled code (installed by the user, not served by proofdrop.ai),
and shows a badge in the video or toolbar.  proofdrop.ai cannot interfere with this — the
extension code comes from the extension store, not from the page.

This matches how existing C2PA browser extensions work for standard C2PA — for example the
[Digimarc C2PA Content Credentials extension](https://chromewebstore.google.com/detail/c2pa-content-credentials/mjkaocdlpjmphfkjndocehcdhbigaafp)
([source](https://github.com/digimarc-corp/c2pa-content-credentials-extension)).
A fightfake extension extends that model with ZK proof verification.

Trust model: users trust the extension package they installed; publishers do not control that
code path.

#### How the browser extension shows a badge on the video

The extension does **not** modify the video file or the site's video player.  It injects a
small HTML overlay into the page — the same technique the existing C2PA browser extension
uses for its "Content Credentials" indicator.

A browser extension has two parts:

1. **Background / service worker** — runs in the extension's own context.  Holds the WASM
   verifier, fetches `proof.bin`, runs Groth16 checks.  Not controlled by the page.
2. **Content script** — JavaScript injected into pages the user visits (e.g. proofdrop.ai).
   Finds `<video>` elements, asks the background worker to verify, then paints a badge.

Typical flow on proofdrop.ai:

```
User visits article page (normal browsing — no extra steps)
    │
    ▼
Content script runs on the page
    │  finds <video> elements
    │  reads C2PA manifest from the MP4 (same-origin) or from a linked proof URL
    ▼
Background worker verifies
    │  extracts org.zkedit.* assertions
    │  fetches proof.bin
    │  runs Groth16 verification (WASM bundled in the extension)
    ▼
Content script injects a badge
    │  creates a <div> positioned over the video corner
    │  e.g. "✅ Verified edit — blur only"
    │  updates position on scroll/resize
    ▼
User sees badge on the video — proofdrop.ai did not serve this UI
```

The badge is ordinary HTML/CSS added to the page DOM, positioned with `position: absolute`
relative to the video's bounding box.  It is not inside the player's native controls — it
floats on top, like a subtitle overlay.

**Capabilities and limits of the extension model:**

| Scenario | Extension can verify and badge? |
|---|---|
| `<video src="article-video.mp4">` on the same site | ✅ Yes — file is readable, manifest extractable |
| Video linked via a same-origin `proof.bin` URL | ✅ Yes |
| Any page after one-time install | ✅ Automatic — user just browses normally |

| Scenario | Limitation |
|---|---|
| YouTube / Vimeo embed | Video runs in a third-party iframe; file bytes usually not accessible |
| Cross-origin iframe without permission | Extension may not be able to read the video element |
| User has not installed the extension | No badge — any on-page badge is only as trustworthy as the site |

### CLI verification (developer and testing path)

Download the `fightfake` binary (or compile it from this repository) and run:

```bash
./fightfake verify \
  --capture capture.signed.mp4 \
  --edited  edited.signed.mp4 \
  --proof   proof.bin
```

CLI verification is fully independent of any website and is useful for integration tests,
benchmarking, and reproducible local verification.

### Summary

- **Primary secure user path:** browser extension, because verifier code is not controlled by
  the media publisher.
- **Technical/testing path:** local CLI verification, for scripted and reproducible checks in
  development and CI environments.

**WASM verification checks:**
1. C2PA signature on both manifests is structurally valid (`verifyAssertionLinkage`).
2. h1 in the capture assertion matches h1 in the edit-proof assertion (`verifyAssertionLinkage`).
3. SHA-256 of `proof.bin` matches `proof_sha256` in the edit-proof assertion (`verifyAssertionLinkage`).
4. **With `--features crypto-verify`:** the actual Groth16 pairing equations over BN254 —
   the cryptographic heart of the ZK proof (`verifyGroth16Proof`). This is what makes browser
   verification trustless rather than merely consistent-looking: a stub proof (32 bytes of
   zeros) is rejected outright, and a tampered-but-well-formed proof fails the pairing check
   rather than silently passing.

Without `crypto-verify`, `verifyGroth16Proof` always returns `false` (the pre-`crypto-verify`
default build's behaviour) — checks 1–3 alone cannot distinguish a stub proof from a real one,
so treat that build as convenience-tier, not security-tier, verification.

The CLI's equivalent is `fightfake verify-proof --proof proof.bin` (needs
`--features crypto-verify`, implied by `eva-backend`) — see "Step 4b" above.

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

# fightfake C2PA — additionally proves, mathematically, that ONLY this brightness edit
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

**When the hash is computed:** in standard C2PA, edits are applied first; the BMFF hash is
then computed over the **output file** (the post-edit encoded MP4 bytes), and only then is
the manifest signed and embedded.  The manifest has no reference to the original — it
records the declared action plus a fingerprint of the edited file.  Standard C2PA therefore
proves forward integrity from signing onward, not that the declared edit is the only change
from a specific original.  See [`docs/manifest-comparison.md`](docs/manifest-comparison.md)
for the full sequence diagram.

### Manifest contents

| Assertion | Standard C2PA (`c2pa-sign`) | fightfake C2PA (`prove-edit`) |
|---|---|---|
| `c2pa.hash.bmff.v3` (hard binding) | ✅ auto-added | ✅ auto-added |
| `c2pa.actions` (edit description) | ✅ human-readable | ✅ human-readable |
| `c2pa.ingredient` (parent link) | — | ✅ links to signed capture |
| `org.zkedit.capture.v1` (h1 fingerprint) | — | ✅ original pixel hash |
| `org.zkedit.edit_proof.v1` (h2 + proof ref) | — | ✅ edited pixel hash + proof |
| `proof.bin` (ZK proof blob) | — | ✅ (stub in Level 0; real in Level 1) |

### What each approach can prove

| Claim | Standard C2PA | fightfake C2PA |
|---|---|---|
| "This file hasn't been modified since signing" | ✅ hard binding | ✅ hard binding |
| "This video came from a specific camera/device" | ✅ (if camera has C2PA support) | ✅ |
| "A brightness edit was declared" | ✅ | ✅ |
| "Exactly this brightness edit — and *nothing else* — was applied" | ❌ trust the signer | ✅ ZK proof |
| "The edit was applied to the specific original identified by h1" | ❌ | ✅ |
| "Verifiable without access to the original footage" | ❌ | ✅ |
| "Verifiable without trusting the signer" | ❌ | ✅ |

**The core distinction.** Standard C2PA shifts the trust question to certificates: you verify
the signer's certificate chains to a trusted CA, then accept the signer's declaration.  If the
signer's key is compromised, or if the pipeline that produces the declaration is manipulated,
there is no independent check that the declared edit is the only change.

fightfake-toolkit removes that dependency on the signer's honesty.  The ZK proof is a
mathematical object: if it verifies, the declared edit is the only pixel-level change —
regardless of who signed the file, whether their certificate is trusted, or whether their
infrastructure was compromised.

Both manifests are readable by the same C2PA tools (browser extension, online validator).
Standard C2PA viewers will display the fightfake manifest correctly, showing the `c2pa.actions`
assertion and noting the `org.zkedit.*` assertions as custom extensions.

---

## How c2pa-rs signs the video

This section is useful when comparing performance claims with academic approaches such as
[VerITAS (eprint.iacr.org/2024/1066)](https://eprint.iacr.org/2024/1066.pdf).

### Where the manifest lives — BMFF box structure

An MP4 file is a sequence of **boxes** (also called atoms), each with a 4-byte type code
and a length.  c2pa-rs injects the manifest into a `uuid` box identified by a
C2PA-registered UUID:

```
bank-robbery-original.mp4  (unsigned)        standard-signed.mp4  (after c2pa-sign)
┌──────────────────────┐                     ┌──────────────────────┐
│  ftyp  (file type)   │                     │  ftyp                │
├──────────────────────┤                     ├──────────────────────┤
│  mdat  (H.264 data)  │  ──► c2pa-sign ──►  │  uuid  ◄─────────────┼── C2PA manifest
│  ~4.4 MB             │                     │  ~5 KB               │   (JUMBF container:
├──────────────────────┤                     ├──────────────────────┤    assertions +
│  moov  (metadata)    │                     │  mdat  (unchanged)   │    claim +
│  ~120 KB             │                     ├──────────────────────┤    COSE_Sign1)
└──────────────────────┘                     │  moov  (unchanged)   │
                                             └──────────────────────┘
```

### What c2pa-rs actually hashes

The `c2pa.hash.bmff.v2` assertion records a SHA-256 digest of every box **except**:
- the `uuid` box holding the manifest itself (chicken-and-egg: the manifest cannot
  hash itself before it exists),
- `ftyp` and `mfra` (excluded by convention in the BMFF hash spec).

This hashes the **encoded H.264 bytes** — not raw decoded pixels.  For the
bank-robbery clip (~4 MB encoded), this takes ~0.07 s on an M1 Mac.  The ECDSA-P256
signature over the resulting hash is computationally negligible.

The C2PA signature says: *"the encoded container bytes have not changed since signing."*
It says nothing about what the pixels look like, what the original was, or what
transformations were applied before signing.  In a typical editor workflow, those bytes are
already the **post-edit** file: the edit happens first, then c2pa-rs hashes the result and
attaches the manifest.

### Why C2PA signing is fast (and what VerITAS addresses)

The [VerITAS paper](https://eprint.iacr.org/2024/1066.pdf) observes that hashing raw pixel
data *inside a zero-knowledge circuit* is extremely expensive.  The root cause is that SHA-256
uses bitwise operations (XOR, AND, rotate) which are cheap on real hardware but very costly
to express in arithmetic circuits (which work over a prime field, not over bits):

- SHA-256 over one 64-byte block ≈ **30 000 R1CS constraints** (must decompose 32-bit
  words into bits to simulate XOR/AND)
- One 1920×1072 frame of raw YUV ≈ 2 MB → ~32 000 SHA-256 blocks → **~1 billion constraints per frame**
- 121 frames → ~100 billion constraints — completely impractical for a ZK prover

VerITAS addresses this with a lattice-based hash that stays cheap inside their proof system.

C2PA does **not** operate inside a ZK circuit at all, so it never faces this problem.

### Griffin: Eva's solution for circuit-friendly pixel hashing

Eva uses the [Griffin permutation](https://eprint.iacr.org/2022/403) for h1/h2 instead of
SHA-256.  Griffin is an algebraic hash designed to be efficient *inside* prime-field
arithmetic circuits:

- Operates natively over field elements — no bit decomposition needed
- Each Griffin permutation (16 elements, 5 rounds) costs roughly **200–300 R1CS constraints**
- SHA-256 in a circuit costs ~30 000 constraints per 64-byte block
- Poseidon (another ZK-friendly hash) costs ~220 constraints per permutation — similar to Griffin

**Griffin vs SHA-256 in-circuit.** Proving Griffin is dramatically cheaper than proving
SHA-256.  For a 374 MB raw YUV input (121 frames at 1920×1072), the constraint count with
Griffin is roughly **100–500× lower** than with SHA-256.  Griffin is slower than SHA-256 as
a plain hash on real hardware (no SIMD acceleration), but the ZK proving cost — which
dominates total run time — is orders of magnitude lower, making the proof feasible in minutes
rather than days.

| Hash | Where computed | Approx. constraints / block | Purpose |
|---|---|---|---|
| `c2pa.hash.bmff` | outside ZK, native SHA-256 | n/a | container integrity |
| h1, h2 (stub build) | outside ZK, native SHA-256 | n/a | pixel fingerprints, not ZK-provable |
| h1, h2 (Eva backend) | **inside ZK circuit**, Griffin | ~200–300 | pixel fingerprints, ZK-provable |
| SHA-256 in ZK (hypothetical) | inside ZK circuit | ~30 000 | would make proving infeasible |

The stub build uses plain SHA-256 because it never runs a ZK prover.  Only the Eva backend
needs the circuit-friendly variant.

### Comparing manifests: `dump-manifest`

The manifest is embedded inside the MP4 `uuid` box — there is no separate file unless you
extract it.  Annotated real examples are in
[`docs/manifest-comparison.md`](docs/manifest-comparison.md) with the JSON files alongside:

```bash
./target/release/fightfake dump-manifest --input out/capture.signed.mp4 | python3 -m json.tool
./target/release/fightfake dump-manifest --input out/edited.signed.mp4  | python3 -m json.tool

# Diff standard vs. fightfake
diff \
  <(./target/release/fightfake dump-manifest --input out/standard-signed.mp4 | python3 -m json.tool) \
  <(./target/release/fightfake dump-manifest --input out/edited.signed.mp4   | python3 -m json.tool)
```

Key differences visible in the diff:
- fightfake edit manifest has a non-empty `ingredients` list linking to the capture manifest.
- `org.zkedit.capture` assertion (h1, device_id, pipeline_stage) — absent in standard C2PA.
- `org.zkedit.edit_proof` assertion (h2, proof_sha256, gadget_id) — absent in standard C2PA.
- Standard C2PA has only `c2pa.actions` + `c2pa.hash.bmff.v2`.

---

## Commands reference

### `prove-edit` — full fightfake workflow

```
fightfake prove-edit --input <VIDEO> [OPTIONS]

  --input, -i <FILE>       Input video (path relative to cwd, or absolute)
  --gadget <NAME>          Edit to apply: brightness | grayscale | invert | redact  [default: brightness]
  --gadget-param <N>       brightness: luma scale in units of 1/1024 (default 416 ≈ 0.41×)
  --redact-x <N>           redact: top-left X pixel of the rectangle  [default: 0]
  --redact-y <N>           redact: top-left Y pixel of the rectangle  [default: 0]
  --redact-width <N>       redact: rectangle width in pixels  [default: 0 — must be set]
  --redact-height <N>      redact: rectangle height in pixels  [default: 0 — must be set]
  --redact-track <FILE>    redact: JSON keyframe list for a moving box — overrides
                           --redact-x/-y/-width/-height, see "Moving redact rectangle"
  --redact-frame-start <N> redact: first frame, inclusive, 0-based  [default: 0]
  --redact-frame-end <N>   redact: last frame, exclusive  [default: 0 — must be set]
  --redact-fill <N>        redact: luma fill value, 0-255 (0 = black)  [default: 0]
  --out-dir, -o <DIR>      Output directory for all artefacts  [default: out]
  --cert <FILE>            PEM signer certificate  [default: testdata/certs/signer-cert.pem]
  --key  <FILE>            PEM signer private key   [default: testdata/certs/signer-key.pem]
  --device-id <ID>         Identifier embedded in the capture assertion  [default: dev-0]
  --blocks-per-step <N>    Macroblocks per Nova IVC step (Level 1 only)  [default: 256]
  --touched-window         redact only: scope the real proof to [redact-frame-start, redact-frame-end);
                           pre/post are hash-anchored instead — see "Proving only the touched time window"
```

Without `--features eva-backend` (Level 0), the proof is a 32-byte placeholder.  The edit,
hashes, and C2PA manifests are real and can be used for integration testing.

With `--features eva-backend` (Level 1), a full Nova IVC + Groth16 proof is generated.
Expect ~5 min for a 10-second 352×288 clip on an M1 Mac; longer for higher resolutions.

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
recorded SHA-256. Does **not** run the ZK proof's own math — use `verify-proof` for that.

### `verify-proof` — cryptographically verify a proof.bin

```
fightfake verify-proof --proof <FILE>
```

Runs the real Nova IVC + Groth16 pairing check on `proof.bin` — the same check
`prove-edit --features eva-backend` self-verifies against right after generating a proof, and
the same one `verifyGroth16Proof` runs in the browser (see `fightfake-wasm`'s `crypto-verify`
feature). Needs `--features crypto-verify` (implied by `eva-backend`) to build; without it,
prints an explanatory error rather than a false pass. Correctly rejects a Level-0 stub
`proof.bin` (32 zero bytes) as "not a cryptographic proof" instead of silently accepting it.

```bash
cargo build -p fightfake-cli --release --features crypto-verify   # verify-only, no prover
./target/release/fightfake verify-proof --proof out/proof.bin
```

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

## Capture levels — trustworthiness of h1

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

The ZK proof is produced by [Eva](https://github.com/fightfake-ai/eva), which uses:

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
| `org.zkedit.edit_proof.v1` | `gadget_id`, `h1`, `h2`, `proof_system`, `circuit_variant`, `proof_sha256`, `gadget_params` (optional) | Edit declaration and proof reference |

`gadget_params` is a free-form object recording the exact parameters of the edit — e.g.
`{"scale": 416}` for brightness, or `{"x":880,"y":1184,"w":480,"h":480,"frame_start":101,"frame_end":125,"fill_y":0}`
for `redact` — so a verifier can see precisely what was edited without re-running the pipeline.
It is omitted for parameterless gadgets (grayscale, invert).

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

## 16-pixel alignment — requirement and future options

Eva's IVC circuit operates on 16×16 pixel macroblocks.  Both the width and height of the
input video must be exact multiples of 16, or the tiling step cannot partition the frame
evenly and the ZK circuit cannot be constructed.

**Current policy:** `prove-edit` rejects non-aligned videos with an error and prints the
exact `ffmpeg` command to pre-crop.  You own the crop decision — the toolkit does not do it
silently.  h1 therefore always covers exactly the pixels in the file you supply, with no
hidden pre-processing.

**Common aligned resolutions:** 1920×1072, 1920×1088, 1280×720, 1280×960, 640×480, 854×480.
**Common non-aligned:** 1920×1080 (1080 ÷ 16 = 67.5), 3840×2160 (2160 ÷ 16 = 135 — aligned!),
1280×1024 (aligned), 1920×1200 (not aligned).

**Pre-crop with ffmpeg:**

```bash
# Crop bottom 8 rows to go from 1920×1080 → 1920×1072
ffmpeg -i input.mp4 -vf crop=1920:1072:0:0 -c:v libx264 -crf 18 input-cropped.mp4
./target/release/fightfake prove-edit --input input-cropped.mp4 --out-dir out/
```

**Production options for non-aligned captures.** The current toolkit leaves alignment to the
caller.  Longer-term options include:

1. **Require aligned capture.**  Configure the camera to output a natively aligned
   resolution (e.g. 1920×1072 instead of 1920×1080).  Many cameras allow this.  h1 covers
   the full frame, no pixels are lost.  This is the cleanest option for new hardware.

2. **Provable padding.**  Extend the frame to the next aligned size by adding rows/columns
   of a known value (e.g. zero/black).  Eva's circuit would need a padding gadget to
   verify this, but the extension is lossless and provable.  The C2PA manifest declares
   the padding dimensions; verifiers strip the padding before displaying.

3. **Provable crop inside the circuit.**  Add a crop gadget that proves exactly which
   pixels were removed (and that only edge pixels were removed).  This keeps h1 covering
   the original full frame while making the alignment adjustment verifiable.

4. **Accept the limitation for now.**  For the current use case — post-capture editing on
   existing recordings — pre-cropping with ffmpeg is a reasonable manual step.  For
   real-time camera capture (Levels 2–3), the camera firmware should simply output aligned
   dimensions natively.

The toolkit currently implements option 4 with explicit user control.  Options 1 and 2 are
the most likely paths forward for production deployments.

---

## Roadmap

- [x] Cryptographic Groth16 verification in the CLI (`verify-proof` command) and in the browser
      (`verifyGroth16Proof`, `fightfake-wasm`'s `crypto-verify` feature) — see
      `fightfake_core::proof_bundle` and [Step 4b/5](#step-4b--cryptographically-verify-the-proof-itself)
      above. Both call the same reimplementation of Eva's decider-verify math (not
      `folding-schemes`/`video` directly — those hard-require native threads and cannot target
      `wasm32-unknown-unknown`), cross-checked against Eva's own `Decider::verify` on every real
      proof `prove-edit --features eva-backend` generates.
- [ ] Level 1 Raspberry Pi demonstrator (`docs/level1-pi-demonstrator.md`)
- [ ] Crop/padding gadget to handle non-16-aligned captures provably (see above)
- [x] Wire the `redact` gadget to `--features eva-backend`: per-macroblock/per-frame varying
      `RedactRectCfg` in the Nova IVC loop (see [_how redact maps onto Eva's RedactRect
      gadget_](#how-redact-maps-onto-evas-redactrect-gadget))
- [x] "Prove only the touched time window, hash-anchor the rest": `prove-edit --gadget redact
      --touched-window` scopes the ZK prover to just the declared frame range instead of an
      entire multi-thousand-macroblock clip — see
      [_proving only the touched time window_](#proving-only-the-touched-time-window---touched-window)
      above for the construction and measured speedup. Scoping down further to just the
      macroblocks the box overlaps (rather than whole frames) is still open.
- [x] Track a moving region across frames (per-frame box list) instead of one fixed rectangle,
      for subjects that move during the redacted window — `--redact-track <file>.json`, see
      [_moving redact rectangle_](#moving-redact-rectangle---redact-track) below
- [ ] A true blur/pixelate fill (currently `redact` only supports a solid fill colour)
- [ ] Proof serialisation format and public key distribution specification
- [ ] fightfake.ai integration guide for web developers

---

## License

MIT
