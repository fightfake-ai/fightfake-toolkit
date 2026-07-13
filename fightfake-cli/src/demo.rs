use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use fightfake_core::assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use sha2::{Digest, Sha256};

use crate::c2pa_signer::{sign_capture_asset, sign_edit_asset, SignMaterial};
use fightfake_core::verify::{verify_bundle, verify_signed_assets};

/// Configuration for the Level 0 demo.
///
/// In Level 0 mode you supply pre-computed `h1`, `h2`, and a proof binary
/// (which may be a stub).  The demo assembles everything, signs C2PA manifests,
/// and runs both verification passes to confirm the pipeline is consistent.
pub struct Level0DemoConfig {
    pub capture_input: PathBuf,
    pub edited_input: PathBuf,
    pub proof_path: PathBuf,
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
    pub device_id: String,
    pub pipeline_stage: String,
    pub proof_system: String,
    pub circuit_variant: String,
    pub gadget_id: String,
    pub h1: String,
    pub h2: String,
    pub capture_assertion_out: PathBuf,
    pub edit_assertion_out: PathBuf,
    pub capture_signed_out: PathBuf,
    pub edited_signed_out: PathBuf,
    pub capture_schema: PathBuf,
    pub edit_schema: PathBuf,
}

pub fn run_level0_demo(cfg: Level0DemoConfig) -> Result<()> {
    ensure_file(&cfg.capture_input, "capture input video")?;
    ensure_file(&cfg.edited_input, "edited input video")?;
    ensure_file(&cfg.proof_path, "proof file")?;
    ensure_file(&cfg.cert_pem, "signer certificate")?;
    ensure_file(&cfg.key_pem, "signer key")?;

    ensure_parent_dir(&cfg.capture_assertion_out)?;
    ensure_parent_dir(&cfg.edit_assertion_out)?;
    ensure_parent_dir(&cfg.capture_signed_out)?;
    ensure_parent_dir(&cfg.edited_signed_out)?;

    let capture_payload = CaptureAssertionV1 {
        assertion_type: CAPTURE_ASSERTION_TYPE.to_owned(),
        version: 1,
        hash_algorithm: "griffin".to_owned(),
        pipeline_stage: cfg.pipeline_stage,
        device_id: cfg.device_id,
        h1: cfg.h1.clone(),
    };
    std::fs::write(
        &cfg.capture_assertion_out,
        serde_json::to_vec_pretty(&capture_payload)?,
    )
    .with_context(|| {
        format!(
            "failed to write capture assertion to {}",
            cfg.capture_assertion_out.display()
        )
    })?;

    let proof = std::fs::read(&cfg.proof_path)
        .with_context(|| format!("failed to read proof from {}", cfg.proof_path.display()))?;
    let edit_payload = EditProofAssertionV1 {
        assertion_type: EDIT_PROOF_ASSERTION_TYPE.to_owned(),
        version: 1,
        proof_system: cfg.proof_system,
        circuit_variant: cfg.circuit_variant,
        gadget_id: cfg.gadget_id,
        h1: cfg.h1,
        h2: cfg.h2,
        proof_sha256: hex::encode(Sha256::digest(&proof)),
        proof_size_bytes: proof.len() as u64,
        gadget_params: None,
    };
    std::fs::write(
        &cfg.edit_assertion_out,
        serde_json::to_vec_pretty(&edit_payload)?,
    )
    .with_context(|| {
        format!(
            "failed to write edit assertion to {}",
            cfg.edit_assertion_out.display()
        )
    })?;

    let signer = SignMaterial {
        cert_path: &cfg.cert_pem,
        key_path: &cfg.key_pem,
    };
    sign_capture_asset(
        &cfg.capture_input,
        &cfg.capture_signed_out,
        &cfg.capture_assertion_out,
        &signer,
        None, // demo uses pre-aligned inputs
    )?;
    sign_edit_asset(
        &cfg.edited_input,
        &cfg.edited_signed_out,
        &cfg.capture_signed_out,
        &cfg.edit_assertion_out,
        &signer,
    )?;

    verify_bundle(
        &cfg.capture_assertion_out,
        &cfg.edit_assertion_out,
        &cfg.proof_path,
        &cfg.capture_schema,
        &cfg.edit_schema,
    )?;
    verify_signed_assets(
        &cfg.capture_signed_out,
        &cfg.edited_signed_out,
        &cfg.proof_path,
    )?;

    println!("capture assertion : {}", cfg.capture_assertion_out.display());
    println!("edit assertion    : {}", cfg.edit_assertion_out.display());
    println!("signed capture    : {}", cfg.capture_signed_out.display());
    println!("signed edited     : {}", cfg.edited_signed_out.display());
    println!("status            : ok");
    Ok(())
}

fn ensure_file(path: &Path, label: &str) -> Result<()> {
    if !path.exists() {
        bail!("{label} not found at {}", path.display());
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create directory {}", parent.display())
        })?;
    }
    Ok(())
}
