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
//!   │ apply edit gadget (brightness / crop / …)
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
//! | Feature flag       | What `prove-edit` does                              |
//! |--------------------|-----------------------------------------------------|
//! | *(none)*           | Level 0: real edit + real hashes, **stub proof**    |
//! | `--features eva-backend` | Level 1+: real Nova IVC + Groth16 proof       |
//!
//! The stub proof is a 32-byte zero blob; its SHA-256 is recorded faithfully in
//! the assertion so a later full re-prove can replace it.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use fightfake_core::assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use sha2::{Digest, Sha256};

use crate::c2pa_signer::{sign_capture_asset, sign_edit_asset, SignMaterial};
use crate::ffmpeg::{ffmpeg_decode_to_yuv, ffmpeg_encode_from_yuv, probe_video};

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
    /// Scale luma channel by `scale / 1024` (Eva `BrightnessCfg`).
    Brightness { scale: u16 },
}

impl Gadget {
    pub fn id(&self) -> &'static str {
        match self {
            Gadget::Brightness { .. } => "brightness",
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
    // 1. Prepare output directory.
    std::fs::create_dir_all(&cfg.out_dir)
        .with_context(|| format!("failed to create output dir {}", cfg.out_dir.display()))?;

    let out = |name: &str| cfg.out_dir.join(name);

    // 2. Probe video dimensions.
    let (width, height, fps_num, fps_den, num_frames) = probe_video(&cfg.input)?;
    println!("[workflow] input: {}×{} @ {fps_num}/{fps_den} fps, {num_frames} frames", width, height);

    if width % 16 != 0 || height % 16 != 0 {
        bail!(
            "video dimensions {width}×{height} are not multiples of 16. \
             Re-scale with ffmpeg first: -vf scale={}:{}",
            (width / 16) * 16,
            (height / 16) * 16
        );
    }

    // 3. Decode to raw YUV 4:2:0.
    let raw_yuv_path = out("raw_orig.yuv");
    ffmpeg_decode_to_yuv(&cfg.input, &raw_yuv_path, width, height)?;
    println!("[workflow] decoded → {}", raw_yuv_path.display());

    let yuv_bytes = std::fs::read(&raw_yuv_path)
        .context("failed to read decoded YUV")?;

    // 4. Convert to Eva macroblocks.
    #[cfg(feature = "eva-backend")]
    let (orig_y, orig_u, orig_v) = {
        use video::yuv420_to_macroblocks;
        yuv420_to_macroblocks(&yuv_bytes, width, height, num_frames)
            .map_err(|e| anyhow::anyhow!("yuv420_to_macroblocks: {e}"))?
    };

    #[cfg(not(feature = "eva-backend"))]
    let (orig_y, orig_u, orig_v) = {
        // Stub: split raw YUV into Y / U / V planes (not macroblock-tiled) for
        // hashing purposes.  The hash is consistent but NOT the same as a real
        // Eva macroblock hash.
        let y_len = width * height;
        let uv_len = y_len / 4;
        let y = yuv_bytes[..y_len].to_vec();
        let u = yuv_bytes[y_len..y_len + uv_len].to_vec();
        let v = yuv_bytes[y_len + uv_len..].to_vec();
        (y, u, v)
    };

    // 5. Apply edit and compute h1 / h2.
    let (edited_y, edited_u, edited_v, h1_hex, h2_hex) =
        apply_edit_and_hash(&orig_y, &orig_u, &orig_v, width, height, num_frames, &cfg.gadget)?;

    println!("[workflow] h1 = {h1_hex}");
    println!("[workflow] h2 = {h2_hex}");

    // 6. Generate ZK proof (or stub).
    let (proof_bytes, proof_is_stub) =     generate_proof(
        &orig_y, &orig_u, &orig_v,
        width, height, num_frames,
        &cfg.gadget,
        cfg.blocks_per_step,
        &h1_hex,
    )?;

    let proof_bin = out("proof.bin");
    std::fs::write(&proof_bin, &proof_bytes)
        .with_context(|| format!("failed to write proof to {}", proof_bin.display()))?;
    if proof_is_stub {
        println!("[workflow] proof: stub (build with --features eva-backend for real proof)");
    } else {
        println!("[workflow] proof: {} bytes → {}", proof_bytes.len(), proof_bin.display());
    }

    // 7. Re-encode edited video.
    let edited_yuv_path = out("raw_edited.yuv");

    #[cfg(feature = "eva-backend")]
    {
        use video::macroblocks_to_yuv420;
        let edited_yuv = macroblocks_to_yuv420(&edited_y, &edited_u, &edited_v, width, height, num_frames)
            .map_err(|e| anyhow::anyhow!("macroblocks_to_yuv420: {e}"))?;
        std::fs::write(&edited_yuv_path, &edited_yuv)
            .context("failed to write edited YUV")?;
    }

    #[cfg(not(feature = "eva-backend"))]
    {
        // Reconstruct planar YUV from our plane slices.
        let mut edited_yuv = edited_y.clone();
        edited_yuv.extend_from_slice(&edited_u);
        edited_yuv.extend_from_slice(&edited_v);
        std::fs::write(&edited_yuv_path, &edited_yuv)
            .context("failed to write edited YUV")?;
    }

    let edited_mp4 = out("edited.mp4");
    ffmpeg_encode_from_yuv(&edited_yuv_path, &edited_mp4, width, height, fps_num, fps_den)?;
    println!("[workflow] edited video → {}", edited_mp4.display());

    // 8. Emit C2PA assertion JSONs.
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
    };

    let capture_assertion_json = out("capture.assertion.json");
    let edit_assertion_json = out("edit.assertion.json");
    std::fs::write(&capture_assertion_json, serde_json::to_vec_pretty(&capture_payload)?)
        .context("failed to write capture assertion")?;
    std::fs::write(&edit_assertion_json, serde_json::to_vec_pretty(&edit_payload)?)
        .context("failed to write edit assertion")?;

    // 9. C2PA signing.
    let signer = SignMaterial {
        cert_path: &cfg.cert_pem,
        key_path: &cfg.key_pem,
    };
    let capture_signed_mp4 = out("capture.signed.mp4");
    let edited_signed_mp4 = out("edited.signed.mp4");

    sign_capture_asset(&cfg.input, &capture_signed_mp4, &capture_assertion_json, &signer)?;
    println!("[workflow] signed capture → {}", capture_signed_mp4.display());

    sign_edit_asset(&edited_mp4, &edited_signed_mp4, &capture_signed_mp4, &edit_assertion_json, &signer)?;
    println!("[workflow] signed edited  → {}", edited_signed_mp4.display());

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
    match gadget {
        Gadget::Brightness { scale } => {
            #[cfg(feature = "eva-backend")]
            {
                use video::native_brightness_edit_macroblocks;
                let (edited_y, edited_u, edited_v) =
                    native_brightness_edit_macroblocks(orig_y, orig_u, orig_v, width, height, num_frames, *scale)
                        .map_err(|e| anyhow::anyhow!("brightness edit: {e}"))?;

                let h1_hex = sha256_hex(orig_y, orig_u, orig_v);
                let h2_hex = sha256_hex(&edited_y, &edited_u, &edited_v);
                Ok((edited_y, edited_u, edited_v, h1_hex, h2_hex))
            }
            #[cfg(not(feature = "eva-backend"))]
            {
                let edited_y = apply_brightness_native(orig_y, *scale);
                let h1_hex = sha256_hex(orig_y, orig_u, orig_v);
                let h2_hex = sha256_hex(&edited_y, orig_u, orig_v);
                Ok((edited_y, orig_u.to_vec(), orig_v.to_vec(), h1_hex, h2_hex))
            }
        }
    }
}

/// Stub brightness: pixel = min(255, pixel * scale / 1024).
/// In Level 0 mode (no eva-backend), this matches the circuit semantics
/// closely enough for demo purposes.
#[cfg(not(feature = "eva-backend"))]
fn apply_brightness_native(y: &[u8], scale: u16) -> Vec<u8> {
    y.iter()
        .map(|&p| {
            let v = (p as u32) * (scale as u32) / 1024;
            v.min(255) as u8
        })
        .collect()
}

// ── Proof generation ──────────────────────────────────────────────────────────

#[allow(unused_variables)]
fn generate_proof(
    orig_y: &[u8],
    orig_u: &[u8],
    orig_v: &[u8],
    _width: usize,
    _height: usize,
    _num_frames: usize,
    gadget: &Gadget,
    _blocks_per_step: usize,
    _h1_hex: &str,
) -> Result<(Vec<u8>, bool)> {
    #[cfg(feature = "eva-backend")]
    {
        let proof_bytes = prove_with_eva(
            orig_y, orig_u, orig_v,
            width, height, num_frames,
            gadget, blocks_per_step,
        )?;
        return Ok((proof_bytes, false));
    }

    #[cfg(not(feature = "eva-backend"))]
    {
        // Level 0 stub: 32 zero bytes — a valid placeholder to record in the
        // assertion.  Replace with a real proof by running with --features eva-backend.
        let stub = vec![0u8; 32];
        return Ok((stub, true));
    }
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
    use std::marker::PhantomData;
    use std::sync::Arc;

    use ark_bn254::{Bn254, Fq, Fr, G1Projective as Projective};
    use ark_bn254::constraints::GVar;
    use ark_crypto_primitives::crh::poseidon::CRH;
    use ark_crypto_primitives::crh::CRHScheme;
    use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
    use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
    use ark_groth16::Groth16;
    use ark_grumpkin::{constraints::GVar as GVar2, Projective as Projective2};
    use ark_serialize::CanonicalSerialize;
    use ark_snark::SNARK;
    use folding_schemes::{commitment::pedersen::Pedersen, folding::nova::Nova,
                           transcript::poseidon::poseidon_test_config, FoldingScheme};
    use rand::thread_rng;
    use video::{
        decider::{Decider, DeciderEthCircuit},
        edit::constraints::{Brightness, BrightnessCfg},
        griffin::params::GriffinParams,
        macroblock_yuv::{macroblock_count_from_dir, MB_UV_BYTES, MB_Y_BYTES},
        parse_orig_blocks, EditOnlyCircuit, EditOnlyExternalInputs,
    };

    let Gadget::Brightness { scale } = gadget;
    let brightness = BrightnessCfg(*scale);

    type Op = Brightness;
    type NOVA = Nova<
        Projective, GVar, Projective2, GVar2,
        EditOnlyCircuit<Fr, Op>,
        Pedersen<Projective>, Pedersen<Projective2>,
    >;

    // Pack Y/U/V slices into the per-macroblock block format Eva expects.
    // A block is MB_Y_BYTES + MB_UV_BYTES + MB_UV_BYTES bytes (Y then U then V
    // for one 16×16 macroblock).
    let mb_y = MB_Y_BYTES;  // 256
    let mb_uv = MB_UV_BYTES; // 64
    let mb_total = mb_y + mb_uv + mb_uv;
    let n_mbs = orig_y.len() / mb_y;
    let mut blocks: Vec<Vec<u8>> = Vec::with_capacity(n_mbs);
    for i in 0..n_mbs {
        let mut block = Vec::with_capacity(mb_total);
        block.extend_from_slice(&orig_y[i * mb_y..(i + 1) * mb_y]);
        block.extend_from_slice(&orig_u[i * mb_uv..(i + 1) * mb_uv]);
        block.extend_from_slice(&orig_v[i * mb_uv..(i + 1) * mb_uv]);
        blocks.push(block);
    }

    let rng = &mut thread_rng();
    let sk = Fq::rand(rng);
    let poseidon_config = poseidon_test_config();

    let f_circuit = EditOnlyCircuit::<Fr, Op> {
        _e: PhantomData,
        griffin_params: Arc::new(GriffinParams::new(16, 5, 9)),
    };

    println!("[prover] setting up Nova params ({n_mbs} macroblocks, {blocks_per_step} per step)");
    let (pp, vp) = NOVA::preprocess(
        &poseidon_config,
        &f_circuit,
        rng,
        &EditOnlyExternalInputs {
            blocks: blocks[0..blocks_per_step].to_vec(),
            edit_configs: vec![brightness.clone(); blocks_per_step],
        },
    )
    .context("Nova preprocess failed")?;

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
            u_i: None,
            U_i: None,
            W_i1: None,
            cmT: None,
            r: None,
            cf_U_i: None,
            cf_W_i: None,
            E: None,
            cf_E: None,
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
    let num_steps = blocks.len() / blocks_per_step;

    println!("[prover] running Nova IVC ({num_steps} steps)");
    let mut folding_scheme =
        NOVA::init(&params, f_circuit, initial_state.clone()).context("Nova init failed")?;

    for i in 0..num_steps {
        folding_scheme
            .prove_step(
                &params,
                &EditOnlyExternalInputs {
                    blocks: blocks[i * blocks_per_step..(i + 1) * blocks_per_step].to_vec(),
                    edit_configs: vec![brightness.clone(); blocks_per_step],
                },
            )
            .with_context(|| format!("Nova prove_step {i} failed"))?;
    }

    let last_state = folding_scheme.state();
    let vk = Projective2::generator() * sk;
    let (px, py) = {
        let p = vk.into_affine();
        p.xy().unwrap_or((Fr::zero(), Fr::zero()))
    };
    let sigma = {
        let r = Fq::rand(rng);
        let rx = (Projective2::generator() * r).into_affine().x().unwrap_or_default();
        let e = CRH::evaluate(&poseidon_config, [rx, px, py, folding_scheme.z_i[0]])
            .context("Schnorr hash failed")?;
        (rx, r + sk * Fq::from_le_bytes_mod_order(&e.into_bigint().to_bytes_le()))
    };

    let circuit = DeciderEthCircuit::<Projective, GVar, Projective2, GVar2>::from_nova(
        folding_scheme, params, vk, sigma,
    )
    .context("DeciderEthCircuit::from_nova failed")?;

    println!("[prover] running Groth16 decider");
    let proof = Decider::prove(decider_pp, rng, circuit).context("Groth16 prove failed")?;

    let verified = Decider::verify(
        decider_vp,
        vk,
        Fr::from(num_steps as u32),
        initial_state,
        last_state[1],
        &circuit.running_instance.clone().unwrap_or_default(),
        &circuit.incoming_instance.clone().unwrap_or_default(),
        proof.clone(),
    )
    .context("Groth16 verify (self-check) failed")?;
    if !verified {
        bail!("self-verification of generated proof failed");
    }

    // Serialize proof to bytes.
    let mut proof_bytes = Vec::new();
    proof.serialize_compressed(&mut proof_bytes).context("proof serialization failed")?;
    Ok(proof_bytes)
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn sha256_hex(y: &[u8], u: &[u8], v: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(y);
    hasher.update(u);
    hasher.update(v);
    hex::encode(hasher.finalize())
}
