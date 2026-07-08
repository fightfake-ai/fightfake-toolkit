use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use c2pa::{create_signer, Builder, SigningAlg};
use fightfake_core::assertions::{
    read_capture_assertion, read_edit_proof_assertion, CAPTURE_ASSERTION_TYPE,
    EDIT_PROOF_ASSERTION_TYPE,
};
use serde_json::json;

pub struct SignMaterial<'a> {
    pub cert_path: &'a Path,
    pub key_path: &'a Path,
}

pub fn sign_capture_asset(
    source_asset: &Path,
    dest_asset: &Path,
    capture_assertion_path: &Path,
    signer: &SignMaterial<'_>,
) -> Result<()> {
    let capture = read_capture_assertion(capture_assertion_path)?;
    let mut builder = Builder::from_json(
        &json!({ "title": format!("FightFake capture: {}", capture.device_id) }).to_string(),
    )
    .context("failed to initialize capture C2PA builder")?;

    builder
        .add_assertion(CAPTURE_ASSERTION_TYPE, &capture)
        .context("failed to add org.zkedit.capture.v1")?;

    let signer = make_signer(signer)?;
    builder
        .sign_file(&*signer, source_asset, dest_asset)
        .with_context(|| {
            format!(
                "failed to sign {} → {}",
                source_asset.display(),
                dest_asset.display()
            )
        })?;
    Ok(())
}

pub fn sign_edit_asset(
    source_asset: &Path,
    dest_asset: &Path,
    parent_capture_asset: &Path,
    edit_assertion_path: &Path,
    signer: &SignMaterial<'_>,
) -> Result<()> {
    let edit = read_edit_proof_assertion(edit_assertion_path)?;
    let format = mime_guess::from_path(parent_capture_asset)
        .first_raw()
        .unwrap_or("application/octet-stream");

    let mut builder =
        Builder::from_json(&json!({ "title": "FightFake edit proof" }).to_string())
            .context("failed to initialize edit C2PA builder")?;

    let mut parent_file = File::open(parent_capture_asset).with_context(|| {
        format!(
            "failed to open parent capture asset {}",
            parent_capture_asset.display()
        )
    })?;
    builder
        .add_ingredient_from_stream(
            json!({
                "title": parent_capture_asset
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "capture".to_owned()),
                "relationship": "parentOf",
                "label": "capture_parent"
            })
            .to_string(),
            format,
            &mut parent_file,
        )
        .context("failed to add capture ingredient")?;

    builder
        .add_assertion(EDIT_PROOF_ASSERTION_TYPE, &edit)
        .context("failed to add org.zkedit.edit_proof.v1")?;

    let signer = make_signer(signer)?;
    builder
        .sign_file(&*signer, source_asset, dest_asset)
        .with_context(|| {
            format!(
                "failed to sign {} → {}",
                source_asset.display(),
                dest_asset.display()
            )
        })?;
    Ok(())
}

fn make_signer(m: &SignMaterial<'_>) -> Result<Box<dyn c2pa::Signer>> {
    create_signer::from_files(m.cert_path, m.key_path, SigningAlg::Es256, None)
        .context("failed to build C2PA signer from cert/key files")
}
