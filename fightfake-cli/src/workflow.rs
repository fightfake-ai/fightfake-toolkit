//! Full prove-edit workflow.
//!
//! # Pipeline
//!
//! ```text
//! Input MP4
//!   │ ffmpeg decode
//!   ▼
//! Raw YUV 4:2:0 frames
//!   │ yuv420_to_macroblocks  (Eva)
//!   ▼
//! Eva macroblocks (orig_*_enc)
//!   ├──► Griffin hash chain ──► h1
//!   │
//!   │ apply edit gadget (brightness / grayscale / invert / redact)
//!   ▼
//! Edited macroblocks
//!   ├──► Griffin hash chain ──► h2
//!   │
//!   │ [eva-backend] Nova IVC + Groth16 ──► proof.bin
//!   │
//!   │ macroblocks_to_yuv420 + ffmpeg re-encode
//!   ▼
//! Edited MP4  +  capture.signed.mp4  +  edited.signed.mp4
//! ```
//!
//! # Proof modes
//!
//! | Feature flag             | What `prove-edit` does                           |
//! |--------------------------|--------------------------------------------------|
//! | *(none)*                 | Level 0: real edit + real hashes, stub proof     |
//! | `--features eva-backend` | Level 1+: real Nova IVC + Groth16 proof          |
//!
//! The Level-0 stub proof is a 32-byte zero blob whose SHA-256 is recorded in
//! the assertion, so a later full re-prove can replace it without breaking the
//! C2PA manifest chain.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use fightfake_core::assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use sha2::{Digest, Sha256};

use crate::c2pa_signer::{sign_capture_asset, sign_edit_asset, SignMaterial};
use crate::ffmpeg::{ffmpeg_decode_to_yuv, ffmpeg_encode_from_yuv, probe_video};

// ── Public types ──────────────────────────────────────────────────────────────

/// User-supplied configuration for a single `prove-edit` run.
#[derive(Debug, Clone)]
pub struct ProveEditConfig {
    /// Input video file (MP4 or any container ffmpeg can decode).
    pub input: PathBuf,
    /// Edit gadget to apply.
    pub gadget: Gadget,
    /// Output directory — all artefacts are written here.
    pub out_dir: PathBuf,
    /// PEM certificate for C2PA signing.
    pub cert_pem: PathBuf,
    /// PEM private key for C2PA signing.
    pub key_pem: PathBuf,
    /// Opaque device identifier embedded in the capture assertion.
    pub device_id: String,
    /// Number of macroblocks processed per Nova IVC step (eva-backend only).
    pub blocks_per_step: usize,
}

/// Supported edit operations.
#[derive(Debug, Clone)]
pub enum Gadget {
    /// Scale luma by `scale / 1024` — Eva `BrightnessCfg(scale)`.
    Brightness { scale: u16 },
    /// Convert to grayscale: Y unchanged, U = V = 128.
    Grayscale,
    /// Invert all channels: pixel = 255 − pixel.
    Invert,
    /// Redact (blackout) a fixed pixel rectangle, only for a limited frame range.
    ///
    /// `[x, x+w) × [y, y+h)` is overwritten with `fill_y` (luma) and neutral
    /// chroma (128) for frames `[frame_start, frame_end)`.  All other pixels
    /// and all other frames are left byte-for-byte unchanged.  This is the
    /// right primitive for e.g. blurring/blacking out one face for a couple
    /// of seconds without editing (or having to prove) the rest of the clip.
    Redact {
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        frame_start: usize,
        frame_end: usize,
        fill_y: u8,
    },
}

impl Gadget {
    pub fn id(&self) -> &'static str {
        match self {
            Gadget::Brightness { .. } => "brightness",
            Gadget::Grayscale => "grayscale",
            Gadget::Invert => "invert",
            Gadget::Redact { .. } => "redact",
        }
    }

    /// Gadget-specific parameters to embed in the edit-proof assertion, so a
    /// verifier can see exactly what was edited without re-running anything.
    pub fn params_json(&self) -> Option<serde_json::Value> {
        match self {
            Gadget::Brightness { scale } => Some(serde_json::json!({ "scale": scale })),
            Gadget::Grayscale | Gadget::Invert => None,
            Gadget::Redact { x, y, w, h, frame_start, frame_end, fill_y } => {
                Some(serde_json::json!({
                    "x": x, "y": y, "w": w, "h": h,
                    "frame_start": frame_start, "frame_end": frame_end,
                    "fill_y": fill_y,
                }))
            }
        }
    }
}

/// All artefacts produced by a single `prove-edit` run.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ProveEditOutput {
    pub edited_mp4: PathBuf,
    pub proof_bin: PathBuf,
    pub capture_assertion_json: PathBuf,
    pub edit_assertion_json: PathBuf,
    pub capture_signed_mp4: PathBuf,
    pub edited_signed_mp4: PathBuf,
    pub h1_hex: String,
    pub h2_hex: String,
    pub proof_is_stub: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run_prove_edit(cfg: &ProveEditConfig) -> Result<ProveEditOutput> {
    std::fs::create_dir_all(&cfg.out_dir)
        .with_context(|| format!("failed to create output dir {}", cfg.out_dir.display()))?;

    let out = |name: &str| cfg.out_dir.join(name);
    let wall = std::time::Instant::now();

    // 1. Probe video dimensions.
    let (orig_w, orig_h, fps_num, fps_den, num_frames) = probe_video(&cfg.input)?;
    println!(
        "[workflow] input: {orig_w}×{orig_h} @ {fps_num}/{fps_den} fps, {num_frames} frames"
    );

    // Eva's IVC circuit works on 16×16 macroblocks — both dimensions must be
    // multiples of 16.  We require the caller to pre-crop rather than doing it
    // silently, because h1 must cover exactly the pixels in the supplied file
    // and any implicit crop would make that relationship opaque.
    let width  = orig_w;
    let height = orig_h;
    if orig_w % 16 != 0 || orig_h % 16 != 0 {
        let cw = (orig_w / 16) * 16;
        let ch = (orig_h / 16) * 16;
        anyhow::bail!(
            "video dimensions {orig_w}×{orig_h} are not multiples of 16.\n\
             Eva's ZK circuit requires both width and height to be exact multiples of 16.\n\
             Pre-crop with ffmpeg before running prove-edit:\n\n\
             \x20 ffmpeg -i {:?} -vf crop={cw}:{ch}:0:0 -c:v libx264 -crf 18 cropped.mp4\n\n\
             Then pass cropped.mp4 to prove-edit.",
            cfg.input
        );
    }
    let crop_info = {
        None
    };

    // 2. Decode to raw planar YUV 4:2:0.
    let raw_yuv_path = out("raw_orig.yuv");
    let t = std::time::Instant::now();
    ffmpeg_decode_to_yuv(&cfg.input, &raw_yuv_path, width, height)?;
    let decode_s = t.elapsed().as_secs_f64();
    println!("[workflow] decoded → {} ({decode_s:.2}s)", raw_yuv_path.display());
    let yuv_bytes = std::fs::read(&raw_yuv_path).context("failed to read decoded YUV")?;

    // 3. Split into Y / U / V planes or tile into Eva macroblocks.
    let t = std::time::Instant::now();
    let (orig_y, orig_u, orig_v) = split_yuv(&yuv_bytes, width, height, num_frames)?;
    let tile_s = t.elapsed().as_secs_f64();
    println!("[workflow] macroblock tiling ({tile_s:.2}s)");

    // 4. Apply edit gadget; compute h1 (original) and h2 (edited).
    let t = std::time::Instant::now();
    let (edited_y, edited_u, edited_v, h1_hex, h2_hex) =
        apply_edit_and_hash(&orig_y, &orig_u, &orig_v, width, height, num_frames, &cfg.gadget)?;
    let hash_s = t.elapsed().as_secs_f64();
    println!("[workflow] h1 = {h1_hex}");
    println!("[workflow] h2 = {h2_hex}");
    println!("[workflow] edit + hash ({hash_s:.2}s)");

    // 5. Generate ZK proof (real or stub).
    let t = std::time::Instant::now();
    let (proof_bytes, proof_is_stub) = generate_proof(
        &orig_y,
        &orig_u,
        &orig_v,
        width,
        height,
        num_frames,
        &cfg.gadget,
        cfg.blocks_per_step,
    )?;
    let prove_s = t.elapsed().as_secs_f64();

    let proof_bin = out("proof.bin");
    std::fs::write(&proof_bin, &proof_bytes)
        .with_context(|| format!("failed to write proof to {}", proof_bin.display()))?;
    if proof_is_stub {
        println!("[workflow] proof: stub — Level 0 (32 zero bytes) ({prove_s:.2}s)");
    } else {
        println!(
            "[workflow] proof: {} bytes → {} ({prove_s:.2}s)",
            proof_bytes.len(),
            proof_bin.display()
        );
    }

    // 6. Re-encode edited video.
    let edited_yuv_path = out("raw_edited.yuv");
    let edited_yuv = assemble_yuv(&edited_y, &edited_u, &edited_v, width, height, num_frames)?;
    std::fs::write(&edited_yuv_path, &edited_yuv).context("failed to write edited YUV")?;

    let t = std::time::Instant::now();
    let edited_mp4 = out("edited.mp4");
    ffmpeg_encode_from_yuv(&edited_yuv_path, &edited_mp4, width, height, fps_num, fps_den)?;
    let encode_s = t.elapsed().as_secs_f64();
    println!("[workflow] edited video → {} ({encode_s:.2}s)", edited_mp4.display());

    // 7. Emit C2PA assertion JSONs.
    let proof_sha256 = hex::encode(Sha256::digest(&proof_bytes));

    let capture_payload = CaptureAssertionV1 {
        assertion_type: CAPTURE_ASSERTION_TYPE.to_owned(),
        version: 1,
        hash_algorithm: "griffin".to_owned(),
        pipeline_stage: "post_isp".to_owned(),
        device_id: cfg.device_id.clone(),
        h1: h1_hex.clone(),
    };
    let edit_payload = EditProofAssertionV1 {
        assertion_type: EDIT_PROOF_ASSERTION_TYPE.to_owned(),
        version: 1,
        proof_system: "nova-groth16".to_owned(),
        circuit_variant: "edit_only".to_owned(),
        gadget_id: cfg.gadget.id().to_owned(),
        h1: h1_hex.clone(),
        h2: h2_hex.clone(),
        proof_sha256,
        proof_size_bytes: proof_bytes.len() as u64,
        gadget_params: cfg.gadget.params_json(),
    };

    let capture_assertion_json = out("capture.assertion.json");
    let edit_assertion_json = out("edit.assertion.json");
    std::fs::write(&capture_assertion_json, serde_json::to_vec_pretty(&capture_payload)?)
        .context("failed to write capture assertion")?;
    std::fs::write(&edit_assertion_json, serde_json::to_vec_pretty(&edit_payload)?)
        .context("failed to write edit assertion")?;

    // 8. C2PA signing.
    let signer = SignMaterial {
        cert_path: &cfg.cert_pem,
        key_path: &cfg.key_pem,
    };
    let capture_signed_mp4 = out("capture.signed.mp4");
    let edited_signed_mp4 = out("edited.signed.mp4");

    let t = std::time::Instant::now();
    sign_capture_asset(&cfg.input, &capture_signed_mp4, &capture_assertion_json, &signer, crop_info.as_ref())?;
    sign_edit_asset(
        &edited_mp4,
        &edited_signed_mp4,
        &capture_signed_mp4,
        &edit_assertion_json,
        &signer,
    )?;
    let sign_s = t.elapsed().as_secs_f64();
    println!("[workflow] signed capture → {}", capture_signed_mp4.display());
    println!("[workflow] signed edited  → {}", edited_signed_mp4.display());
    println!("[workflow] C2PA signing ({sign_s:.2}s)");

    let total_s = wall.elapsed().as_secs_f64();

    // Timing summary.
    println!();
    println!("┌─────────────────────────────────────────┬──────────┐");
    println!("│ Phase                                   │     Time │");
    println!("├─────────────────────────────────────────┼──────────┤");
    println!("│ ffmpeg decode                           │ {decode_s:>6.2}s │");
    println!("│ macroblock tiling                       │ {tile_s:>6.2}s │");
    println!("│ edit + hashing (h1, h2)                 │ {hash_s:>6.2}s │");
    println!("│ ZK proving (Nova IVC + Groth16)         │ {prove_s:>6.2}s │");
    println!("│ ffmpeg re-encode                        │ {encode_s:>6.2}s │");
    println!("│ C2PA signing                            │ {sign_s:>6.2}s │");
    println!("├─────────────────────────────────────────┼──────────┤");
    println!("│ Total                                   │ {total_s:>6.2}s │");
    println!("└─────────────────────────────────────────┴──────────┘");

    Ok(ProveEditOutput {
        edited_mp4,
        proof_bin,
        capture_assertion_json,
        edit_assertion_json,
        capture_signed_mp4,
        edited_signed_mp4,
        h1_hex,
        h2_hex,
        proof_is_stub,
    })
}

// ── YUV split / assemble ──────────────────────────────────────────────────────

/// Split raw planar YUV 4:2:0 into (Y, U, V) vectors.
///
/// With `eva-backend`, tiles into macroblock order via Eva's API.
/// Without it, returns planar slices (consistent for Level-0 hashing).
fn split_yuv(
    yuv: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    #[cfg(feature = "eva-backend")]
    {
        use video::yuv420_to_macroblocks;
        return yuv420_to_macroblocks(yuv, width, height, num_frames)
            .map_err(|e| anyhow::anyhow!("yuv420_to_macroblocks: {e}"));
    }
    #[cfg(not(feature = "eva-backend"))]
    {
        // Raw ffmpeg rawvideo output is frame-sequential: each frame is
        // [Y plane][U plane][V plane], then the next frame.  We de-interleave
        // into three frame-sequential plane buffers (all Y planes back to
        // back, then all U planes, then all V planes) so that per-frame,
        // per-pixel addressing (needed by e.g. Gadget::Redact) is simple.
        // assemble_yuv() below reverses this exactly.
        let y_frame = width * height;
        let uv_frame = y_frame / 4;
        let frame_bytes = y_frame + 2 * uv_frame;
        let mut y_all = Vec::with_capacity(y_frame * num_frames);
        let mut u_all = Vec::with_capacity(uv_frame * num_frames);
        let mut v_all = Vec::with_capacity(uv_frame * num_frames);
        for f in 0..num_frames {
            let base = f * frame_bytes;
            y_all.extend_from_slice(&yuv[base..base + y_frame]);
            u_all.extend_from_slice(&yuv[base + y_frame..base + y_frame + uv_frame]);
            v_all.extend_from_slice(&yuv[base + y_frame + uv_frame..base + frame_bytes]);
        }
        Ok((y_all, u_all, v_all))
    }
}

/// Reassemble (Y, U, V) into raw planar YUV 4:2:0.
///
/// With `eva-backend`, converts from macroblock order back to planar.
/// Without it, concatenates the three planes directly.
fn assemble_yuv(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
) -> Result<Vec<u8>> {
    #[cfg(feature = "eva-backend")]
    {
        use video::macroblocks_to_yuv420;
        return macroblocks_to_yuv420(y, u, v, width, height, num_frames)
            .map_err(|e| anyhow::anyhow!("macroblocks_to_yuv420: {e}"));
    }
    #[cfg(not(feature = "eva-backend"))]
    {
        // Reverses the de-interleaving in split_yuv(): re-interleave the
        // three frame-sequential plane buffers back into ffmpeg's expected
        // frame-sequential [Y][U][V] rawvideo layout.
        let y_frame = width * height;
        let uv_frame = y_frame / 4;
        let mut out = Vec::with_capacity((y_frame + 2 * uv_frame) * num_frames);
        for f in 0..num_frames {
            out.extend_from_slice(&y[f * y_frame..(f + 1) * y_frame]);
            out.extend_from_slice(&u[f * uv_frame..(f + 1) * uv_frame]);
            out.extend_from_slice(&v[f * uv_frame..(f + 1) * uv_frame]);
        }
        Ok(out)
    }
}

// ── Edit + hash ───────────────────────────────────────────────────────────────

#[allow(unused_variables)]
fn apply_edit_and_hash(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    gadget: &Gadget,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>, String, String)> {
    let h1 = sha256_hex(orig_y, orig_u, orig_v);

    match gadget {
        Gadget::Brightness { scale } => {
            #[cfg(feature = "eva-backend")]
            {
                use video::native_brightness_edit_macroblocks;
                let (ey, eu, ev) =
                    native_brightness_edit_macroblocks(orig_y, orig_u, orig_v, width, height, num_frames, *scale)
                        .map_err(|e| anyhow::anyhow!("brightness edit: {e}"))?;
                let h2 = sha256_hex(&ey, &eu, &ev);
                return Ok((ey, eu, ev, h1, h2));
            }
            #[cfg(not(feature = "eva-backend"))]
            {
                let ey = brightness_native(orig_y, *scale);
                let h2 = sha256_hex(&ey, orig_u, orig_v);
                return Ok((ey, orig_u.to_vec(), orig_v.to_vec(), h1, h2));
            }
        }
        Gadget::Grayscale => {
            let (ey, eu, ev) = grayscale_native(orig_y, orig_u.len(), orig_v.len());
            let h2 = sha256_hex(&ey, &eu, &ev);
            Ok((ey, eu, ev, h1, h2))
        }
        Gadget::Invert => {
            let (ey, eu, ev) = invert_native(orig_y, orig_u, orig_v);
            let h2 = sha256_hex(&ey, &eu, &ev);
            Ok((ey, eu, ev, h1, h2))
        }
        Gadget::Redact { x, y, w, h, frame_start, frame_end, fill_y } => {
            #[cfg(feature = "eva-backend")]
            {
                let (ey, eu, ev) = native_redact_edit_macroblocks(
                    orig_y, orig_u, orig_v, width, height, num_frames,
                    *x, *y, *w, *h, *frame_start, *frame_end, *fill_y,
                )
                .map_err(|e| anyhow::anyhow!("redact edit: {e}"))?;
                let h2 = sha256_hex(&ey, &eu, &ev);
                return Ok((ey, eu, ev, h1, h2));
            }
            #[cfg(not(feature = "eva-backend"))]
            {
                let (ey, eu, ev) = redact_native(
                    orig_y, orig_u, orig_v, width, height, num_frames,
                    *x, *y, *w, *h, *frame_start, *frame_end, *fill_y,
                );
                let h2 = sha256_hex(&ey, &eu, &ev);
                Ok((ey, eu, ev, h1, h2))
            }
        }
    }
}

// ── Native pixel transforms (Level 0 + Eva-backend reference) ─────────────────

/// Brightness: luma = min(255, luma × scale / 1024); chroma unchanged.
fn brightness_native(y: &[u8], scale: u16) -> Vec<u8> {
    y.iter()
        .map(|&p| ((p as u32 * scale as u32 / 1024).min(255)) as u8)
        .collect()
}

/// Grayscale: Y unchanged, U = V = 128 (neutral chroma).
fn grayscale_native(y: &[u8], u_len: usize, v_len: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (y.to_vec(), vec![128u8; u_len], vec![128u8; v_len])
}

/// Invert: every channel pixel = 255 − pixel.
fn invert_native(y: &[u8], u: &[u8], v: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let inv = |s: &[u8]| s.iter().map(|&p| 255 - p).collect::<Vec<_>>();
    (inv(y), inv(u), inv(v))
}

/// Redact: overwrite the pixel rectangle `[x, x+w) × [y, y+h)` with `fill_y`
/// luma and neutral (128) chroma, only for frames `[frame_start, frame_end)`.
/// Everything else — every other pixel, every other frame — is copied through
/// byte-for-byte unchanged.  Requires frame-sequential planar `y`/`u`/`v`
/// buffers, i.e. the output of `split_yuv` above.
#[allow(clippy::too_many_arguments)]
fn redact_native(
    y: &[u8],
    u: &[u8],
    v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    x: usize,
    y_off: usize,
    w: usize,
    h: usize,
    frame_start: usize,
    frame_end: usize,
    fill_y: u8,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut ey = y.to_vec();
    let mut eu = u.to_vec();
    let mut ev = v.to_vec();

    let chroma_w = width / 2;
    let chroma_h = height / 2;

    // Clamp the rectangle to the frame bounds; clamp the frame range to the
    // clip length.  Chroma (4:2:0) is subsampled 2×2, so the luma rectangle
    // maps to a chroma rectangle at half the coordinates and half the size.
    let x1 = x.min(width);
    let y1 = y_off.min(height);
    let x2 = (x + w).min(width);
    let y2 = (y_off + h).min(height);
    let cx1 = x1 / 2;
    let cy1 = y1 / 2;
    let cx2 = x2.div_ceil(2);
    let cy2 = y2.div_ceil(2);
    let f_end = frame_end.min(num_frames);

    for f in frame_start.min(f_end)..f_end {
        let y_base = f * width * height;
        for row in y1..y2 {
            let row_start = y_base + row * width;
            for px in &mut ey[row_start + x1..row_start + x2] {
                *px = fill_y;
            }
        }
        let uv_base = f * chroma_w * chroma_h;
        for row in cy1..cy2 {
            let row_start = uv_base + row * chroma_w;
            for px in &mut eu[row_start + cx1..row_start + cx2] {
                *px = 128;
            }
            for px in &mut ev[row_start + cx1..row_start + cx2] {
                *px = 128;
            }
        }
    }

    (ey, eu, ev)
}

// ── Proof generation ──────────────────────────────────────────────────────────

fn generate_proof(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    gadget: &Gadget,
    blocks_per_step: usize,
) -> Result<(Vec<u8>, bool)> {
    #[cfg(feature = "eva-backend")]
    {
        let bytes = prove_with_eva(
            orig_y,
            orig_u,
            orig_v,
            width,
            height,
            num_frames,
            gadget,
            blocks_per_step,
        )?;
        return Ok((bytes, false));
    }
    #[cfg(not(feature = "eva-backend"))]
    {
        let _ = (
            orig_y,
            orig_u,
            orig_v,
            width,
            height,
            num_frames,
            gadget,
            blocks_per_step,
        );
        // 32 zero bytes — a deterministic, recordable placeholder.
        // Replace by re-running with `--features eva-backend`.
        Ok((vec![0u8; 32], true))
    }
}

// ── Eva proving backend ───────────────────────────────────────────────────────

#[cfg(feature = "eva-backend")]
type EvaMacroblock = (
    video::encode::Matrix<u8, 16, 16>,
    video::encode::Matrix<u8, 8, 8>,
    video::encode::Matrix<u8, 8, 8>,
);

/// Pack Eva macroblock byte streams into the matrix triples the IVC circuit expects.
#[cfg(feature = "eva-backend")]
fn macroblock_streams_to_blocks(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
) -> Vec<EvaMacroblock> {
    use video::encode::Matrix;
    use video::macroblock_yuv::{MB_UV_BYTES, MB_Y_BYTES};

    let n_mbs = orig_y.len() / MB_Y_BYTES;
    (0..n_mbs)
        .map(|i| {
            (
                Matrix::from_vec(orig_y[i * MB_Y_BYTES..(i + 1) * MB_Y_BYTES].to_vec()),
                Matrix::from_vec(orig_u[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES].to_vec()),
                Matrix::from_vec(orig_v[i * MB_UV_BYTES..(i + 1) * MB_UV_BYTES].to_vec()),
            )
        })
        .collect()
}

#[cfg(feature = "eva-backend")]
fn prove_with_eva(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    gadget: &Gadget,
    blocks_per_step: usize,
) -> Result<Vec<u8>> {
    let blocks = macroblock_streams_to_blocks(orig_y, orig_u, orig_v);

    match gadget {
        Gadget::Brightness { scale } => {
            prove_nova_groth16_brightness(blocks, blocks_per_step, *scale)
        }
        Gadget::Grayscale => prove_nova_groth16_grayscale(blocks, blocks_per_step),
        Gadget::Invert => prove_nova_groth16_invert(blocks, blocks_per_step),
        Gadget::Redact {
            x,
            y,
            w,
            h,
            frame_start,
            frame_end,
            fill_y,
        } => prove_nova_groth16_redact(
            blocks,
            blocks_per_step,
            width,
            height,
            num_frames,
            *x,
            *y,
            *w,
            *h,
            *frame_start,
            *frame_end,
            *fill_y,
        ),
    }
}

/// Build a per-macroblock [`MaskCfg`] for the `redact` gadget.
///
/// `global_mb` is the macroblock's index in Eva's linear order (frame-major,
/// row-major within each frame). Pixels inside `[x, x+w) × [y, y+h)` during
/// frames `[frame_start, frame_end)` are marked for replacement.
#[cfg(feature = "eva-backend")]
fn build_redact_mask_cfg(
    global_mb: usize,
    width: usize,
    height: usize,
    mbs_per_frame: usize,
    x: usize,
    y_off: usize,
    w: usize,
    h: usize,
    frame_start: usize,
    frame_end: usize,
    fill_y: u8,
) -> video::edit::constraints::MaskCfg {
    use ndarray::Array2;
    use video::edit::constraints::MaskCfg;
    use video::macroblock_yuv::macroblock_xy;

    let frame = global_mb / mbs_per_frame;
    let mb_idx = global_mb % mbs_per_frame;
    let (mb_x, mb_y) = macroblock_xy(width, mb_idx);
    let origin_x = mb_x * 16;
    let origin_y = mb_y * 16;

    let in_frame_range = frame >= frame_start && frame < frame_end;

    let x1 = x.min(width);
    let y1 = y_off.min(height);
    let x2 = (x + w).min(width);
    let y2 = (y_off + h).min(height);

    let y_mask = Array2::from_shape_fn((16, 16), |(m, n)| {
        let px = origin_x + m;
        let py = origin_y + n;
        let in_box = in_frame_range && px >= x1 && px < x2 && py >= y1 && py < y2;
        (fill_y, in_box)
    });

    let u_mask = Array2::from_shape_fn((8, 8), |(m, n)| {
        let px = origin_x + m * 2;
        let py = origin_y + n * 2;
        let in_box = in_frame_range && px >= x1 && px < x2 && py >= y1 && py < y2;
        (128u8, in_box)
    });
    let v_mask = u_mask.clone();

    MaskCfg(y_mask, u_mask, v_mask)
}

/// Apply Eva's [`Masking`] gadget per macroblock — reference path that matches
/// the in-circuit edit for `redact`.
#[cfg(feature = "eva-backend")]
#[allow(clippy::too_many_arguments)]
fn native_redact_edit_macroblocks(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    width: usize,
    height: usize,
    num_frames: usize,
    x: usize,
    y_off: usize,
    w: usize,
    h: usize,
    frame_start: usize,
    frame_end: usize,
    fill_y: u8,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    use video::edit::constraints::{EditGadget, Masking};
    use video::encode::Matrix;
    use video::macroblock_yuv::{macroblocks_per_frame, MB_UV_BYTES, MB_Y_BYTES};

    let mbs_per_frame = macroblocks_per_frame(width, height)?;
    let total_mbs = orig_y.len() / MB_Y_BYTES;
    if total_mbs != mbs_per_frame * num_frames {
        return Err(format!(
            "macroblock count mismatch: got {total_mbs}, expected {} for {num_frames} frame(s)",
            mbs_per_frame * num_frames
        ));
    }
    let f_end = frame_end.min(num_frames);
    let need_y = total_mbs * MB_Y_BYTES;
    let need_uv = total_mbs * MB_UV_BYTES;

    let mut out_y = vec![0u8; need_y];
    let mut out_u = vec![0u8; need_uv];
    let mut out_v = vec![0u8; need_uv];

    for global in 0..total_mbs {
        let cfg = build_redact_mask_cfg(
            global,
            width,
            height,
            mbs_per_frame,
            x,
            y_off,
            w,
            h,
            frame_start,
            f_end,
            fill_y,
        );

        let y = Matrix::<u8, 16, 16>::from_vec(
            orig_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].to_vec(),
        );
        let u = Matrix::<u8, 8, 8>::from_vec(
            orig_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );
        let v = Matrix::<u8, 8, 8>::from_vec(
            orig_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].to_vec(),
        );

        let (y, u, v) = Masking::edit_native(&y, &u, &v, &cfg);

        let y_arr: [u8; MB_Y_BYTES] = y.iter().copied().collect::<Vec<_>>().try_into().unwrap();
        let u_arr: [u8; MB_UV_BYTES] = u.iter().copied().collect::<Vec<_>>().try_into().unwrap();
        let v_arr: [u8; MB_UV_BYTES] = v.iter().copied().collect::<Vec<_>>().try_into().unwrap();

        out_y[global * MB_Y_BYTES..(global + 1) * MB_Y_BYTES].copy_from_slice(&y_arr);
        out_u[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&u_arr);
        out_v[global * MB_UV_BYTES..(global + 1) * MB_UV_BYTES].copy_from_slice(&v_arr);
    }

    Ok((out_y, out_u, out_v))
}

/// Shared Nova IVC + Groth16 boilerplate.
///
/// The macro is necessary because `EditOnlyCircuit<Fr, Op>` and the associated
/// `Nova` type alias differ for each gadget, and Rust does not allow abstracting
/// over them with a single generic function without leaking heavy arkworks types
/// into the public API.
#[cfg(feature = "eva-backend")]
macro_rules! run_nova_groth16 {
    ($blocks:expr, $blocks_per_step:expr, $op:ty, $cfg_val:expr, $edit_configs:expr) => {{
        use std::marker::PhantomData;
        use std::sync::Arc;

        use ark_bn254::{constraints::GVar, Bn254, Fq, Fr, G1Projective as Projective};
        use ark_crypto_primitives::crh::{poseidon::CRH, CRHScheme};
        use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
        use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
        use ark_groth16::Groth16;
        use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
        use ark_serialize::CanonicalSerialize;
        use ark_snark::SNARK;
        use folding_schemes::{
            commitment::pedersen::Pedersen,
            folding::nova::Nova,
            transcript::poseidon::poseidon_test_config,
            FoldingScheme,
        };
        use rand::thread_rng;
        use video::{
            decider::{Decider, DeciderEthCircuit},
            griffin::params::GriffinParams,
            EditOnlyCircuit, EditOnlyExternalInputs,
        };

        type Op = $op;
        type NOVA = Nova<
            Projective, GVar, Projective2, GVar2,
            EditOnlyCircuit<Fr, Op>,
            Pedersen<Projective>, Pedersen<Projective2>,
        >;

        let blocks: Vec<EvaMacroblock> = $blocks;
        let blocks_per_step: usize = $blocks_per_step;
        let num_steps = blocks.len() / blocks_per_step;

        let rng = &mut thread_rng();
        let sk = Fq::rand(rng);
        let poseidon_config = poseidon_test_config();

        let f_circuit = EditOnlyCircuit::<Fr, Op> {
            _e: PhantomData,
            griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
        };

        let edit_configs_0: Vec<_> = $edit_configs(0, blocks_per_step);

        println!("[prover] Nova preprocess ({} MBs, {blocks_per_step} per step)", blocks.len());
        let (pp, vp) = NOVA::preprocess(
            &poseidon_config,
            &f_circuit,
            rng,
            &EditOnlyExternalInputs {
                blocks: blocks[0..blocks_per_step].to_vec(),
                edit_configs: edit_configs_0,
            },
        )
        .context("Nova preprocess failed")?;

        // Groth16 circuit setup (dummy witnesses).
        let pk = Groth16::<Bn254>::generate_random_parameters_with_reduction(
            DeciderEthCircuit::<Projective, GVar, Projective2, GVar2> {
                _gc1: PhantomData,
                _gc2: PhantomData,
                r1cs: vp.r1cs.clone(),
                cf_r1cs: vp.cf_r1cs.clone(),
                cf_pedersen_params: pp.cf_cs_params.clone(),
                poseidon_config: poseidon_config.clone(),
                i: None,
                z_0: Some(vec![Fr::rand(rng), Fr::rand(rng)]),
                u_i: None, U_i: None, W_i1: None, cmT: None, r: None,
                cf_U_i: None, cf_W_i: None, E: None, cf_E: None,
                sigma: (Fr::rand(rng), Fq::rand(rng)),
                vk: Projective2::rand(rng),
                h1: Fr::rand(rng),
                h2: Fr::rand(rng),
            },
            vec![
                (&pp.cs_params.generators[..vp.r1cs.q], pp.cs_params.h.into_affine()),
                (&pp.cs_params.generators[..vp.r1cs.A.n_cols - 1 - vp.r1cs.l - vp.r1cs.q], pp.cs_params.h.into_affine()),
                (&pp.cs_params.generators[..vp.r1cs.A.n_rows], pp.cs_params.h.into_affine()),
            ],
            rng,
        )
        .context("Groth16 setup failed")?;

        let decider_vp = Groth16::<Bn254>::process_vk(&pk.vk).context("process_vk failed")?;
        let decider_pp = pk;
        let params = (pp, vp);
        let initial_state = vec![Fr::zero(), Fr::zero()];

        // Nova IVC.
        println!("[prover] Nova IVC ({num_steps} steps)");
        let mut fs = NOVA::init(&params, f_circuit, initial_state.clone())
            .context("Nova init failed")?;

        for i in 0..num_steps {
            let cfgs: Vec<_> = $edit_configs(i, blocks_per_step);
            fs.prove_step(
                &params,
                &EditOnlyExternalInputs {
                    blocks: blocks[i * blocks_per_step..(i + 1) * blocks_per_step].to_vec(),
                    edit_configs: cfgs,
                },
            )
            .with_context(|| format!("Nova prove_step {i} failed"))?;
        }

        let last_state = fs.state();

        // Sign h1 with device key for the Groth16 decider.
        let device_vk = Projective2::generator() * sk;
        let (px, py) = device_vk.into_affine().xy().unwrap_or((Fr::zero(), Fr::zero()));
        let sigma = {
            let r = Fq::rand(rng);
            let rx = (Projective2::generator() * r).into_affine().x().unwrap_or_default();
            let e = CRH::evaluate(&poseidon_config, [rx, px, py, fs.z_i[0]])
                .map_err(|e| anyhow::anyhow!("Schnorr hash failed: {e}"))?;
            (rx, r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()))
        };

        // Capture the final Nova instances BEFORE from_nova consumes `fs`.
        let U_i = fs.U_i.clone();

        let circuit = DeciderEthCircuit::<Projective, GVar, Projective2, GVar2>::from_nova(
            fs, params, device_vk, sigma,
        )
        .context("DeciderEthCircuit::from_nova failed")?;

        let u_i = circuit
            .u_i
            .clone()
            .context("DeciderEthCircuit missing u_i witness")?;

        println!("[prover] Groth16 decider prove");
        let (groth_proof, cm_t, r) =
            Decider::prove(decider_pp, rng, circuit).context("Groth16 prove failed")?;

        // Serialize the full decider bundle (Groth16 proof + Nova commitment data).
        let mut proof_bytes = Vec::new();
        groth_proof
            .serialize_compressed(&mut proof_bytes)
            .context("proof serialization failed")?;
        cm_t.serialize_compressed(&mut proof_bytes)
            .context("cmT serialization failed")?;
        r.serialize_compressed(&mut proof_bytes)
            .context("r serialization failed")?;

        // Self-check.
        let ok = Decider::verify(
            decider_vp,
            device_vk,
            Fr::from(num_steps as u32),
            initial_state,
            last_state[1],
            &U_i,
            &u_i,
            (groth_proof, cm_t, r),
        )
        .context("Groth16 self-verify failed")?;
        if !ok {
            bail!("self-verification of generated proof failed");
        }

        anyhow::Ok(proof_bytes)
    }};
}

#[cfg(feature = "eva-backend")]
fn prove_nova_groth16_brightness(
    blocks: Vec<EvaMacroblock>,
    blocks_per_step: usize,
    scale: u16,
) -> Result<Vec<u8>> {
    use video::edit::constraints::{Brightness, BrightnessCfg};
    run_nova_groth16!(
        blocks,
        blocks_per_step,
        Brightness,
        BrightnessCfg(scale),
        |_step: usize, n: usize| vec![BrightnessCfg(scale); n]
    )
}

#[cfg(feature = "eva-backend")]
fn prove_nova_groth16_grayscale(blocks: Vec<EvaMacroblock>, blocks_per_step: usize) -> Result<Vec<u8>> {
    use video::edit::constraints::Grayscale;
    run_nova_groth16!(
        blocks,
        blocks_per_step,
        Grayscale,
        (),
        |_step: usize, n: usize| vec![(); n]
    )
}

#[cfg(feature = "eva-backend")]
fn prove_nova_groth16_invert(blocks: Vec<EvaMacroblock>, blocks_per_step: usize) -> Result<Vec<u8>> {
    use video::edit::constraints::InvertColor;
    run_nova_groth16!(
        blocks,
        blocks_per_step,
        InvertColor,
        (),
        |_step: usize, n: usize| vec![(); n]
    )
}

#[cfg(feature = "eva-backend")]
#[allow(clippy::too_many_arguments)]
fn prove_nova_groth16_redact(
    blocks: Vec<EvaMacroblock>,
    blocks_per_step: usize,
    width: usize,
    height: usize,
    num_frames: usize,
    x: usize,
    y_off: usize,
    w: usize,
    h: usize,
    frame_start: usize,
    frame_end: usize,
    fill_y: u8,
) -> Result<Vec<u8>> {
    use video::edit::constraints::Masking;
    use video::macroblock_yuv::macroblocks_per_frame;

    let mbs_per_frame = macroblocks_per_frame(width, height)
        .map_err(|e| anyhow::anyhow!("macroblocks_per_frame: {e}"))?;
    let f_end = frame_end.min(num_frames);

    run_nova_groth16!(
        blocks,
        blocks_per_step,
        Masking,
        video::edit::constraints::MaskCfg::default(),
        |step: usize, n: usize| {
            let base_mb = step * n;
            (0..n)
                .map(|j| {
                    build_redact_mask_cfg(
                        base_mb + j,
                        width,
                        height,
                        mbs_per_frame,
                        x,
                        y_off,
                        w,
                        h,
                        frame_start,
                        f_end,
                        fill_y,
                    )
                })
                .collect::<Vec<_>>()
        }
    )
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn sha256_hex(y: &[u8], u: &[u8], v: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(y);
    h.update(u);
    h.update(v);
    hex::encode(h.finalize())
}
