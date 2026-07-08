//! Assertion-level verifier.
//!
//! Two verification modes:
//!
//! 1. **Bundle verify** — reads assertion JSON side-files and a proof binary.
//!    Useful when assets are not yet signed (Level 0 dry-run).
//!
//! 2. **Signed-asset verify** — reads `org.zkedit.*` assertions directly from
//!    C2PA-signed assets.  Checks hard binding (C2PA) and h1/h2 linkage.

use std::path::Path;

use anyhow::{bail, Context, Result};
use c2pa::{ManifestAssertion, Reader};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::assertions::{
    read_capture_assertion, read_edit_proof_assertion, CaptureAssertionV1, EditProofAssertionV1,
    CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use crate::schema_utils::validate_json_against_schema;

// ── Bundle verify (side-files) ────────────────────────────────────────────────

pub fn verify_bundle(
    capture_assertion_path: &Path,
    edit_assertion_path: &Path,
    proof_path: &Path,
    capture_schema_path: &Path,
    edit_schema_path: &Path,
) -> Result<()> {
    validate_json_against_schema(
        capture_assertion_path,
        capture_schema_path,
        CAPTURE_ASSERTION_TYPE,
    )?;
    validate_json_against_schema(
        edit_assertion_path,
        edit_schema_path,
        EDIT_PROOF_ASSERTION_TYPE,
    )?;

    let capture = read_capture_assertion(capture_assertion_path)?;
    let edit = read_edit_proof_assertion(edit_assertion_path)?;

    check_h1_linkage(&capture.h1, &edit.h1)?;
    check_proof_integrity(proof_path, &edit.proof_sha256, edit.proof_size_bytes)?;
    Ok(())
}

// ── Signed-asset verify ───────────────────────────────────────────────────────

pub fn verify_signed_assets(
    capture_asset_path: &Path,
    edited_asset_path: &Path,
    proof_path: &Path,
) -> Result<()> {
    let capture_reader = Reader::from_file(capture_asset_path)
        .with_context(|| format!("failed to read C2PA from {}", capture_asset_path.display()))?;
    let edited_reader = Reader::from_file(edited_asset_path)
        .with_context(|| format!("failed to read C2PA from {}", edited_asset_path.display()))?;

    let capture_manifest = capture_reader
        .active_manifest()
        .ok_or_else(|| anyhow::anyhow!("no active manifest in capture asset"))?;
    let edited_manifest = edited_reader
        .active_manifest()
        .ok_or_else(|| anyhow::anyhow!("no active manifest in edited asset"))?;

    let capture_assertion: CaptureAssertionV1 =
        find_assertion(capture_manifest.assertions(), CAPTURE_ASSERTION_TYPE)
            .context("capture asset missing org.zkedit.capture.v1")?;
    let edit_assertion: EditProofAssertionV1 =
        find_assertion(edited_manifest.assertions(), EDIT_PROOF_ASSERTION_TYPE)
            .context("edited asset missing org.zkedit.edit_proof.v1")?;

    check_h1_linkage(&capture_assertion.h1, &edit_assertion.h1)?;
    check_proof_integrity(proof_path, &edit_assertion.proof_sha256, edit_assertion.proof_size_bytes)?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn check_h1_linkage(capture_h1: &str, edit_h1: &str) -> Result<()> {
    if capture_h1 != edit_h1 {
        bail!(
            "h1 mismatch: capture has {capture_h1}, edit-proof has {edit_h1}\n\
             The edit-proof does not correspond to this capture."
        );
    }
    Ok(())
}

fn check_proof_integrity(
    proof_path: &Path,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<()> {
    let proof_bytes = std::fs::read(proof_path)
        .with_context(|| format!("failed to read proof bytes from {}", proof_path.display()))?;
    let actual_sha = hex::encode(Sha256::digest(&proof_bytes));
    if actual_sha != expected_sha256 {
        bail!(
            "proof SHA-256 mismatch:\n  expected: {expected_sha256}\n  actual:   {actual_sha}"
        );
    }
    if proof_bytes.len() as u64 != expected_size {
        bail!(
            "proof size mismatch: expected {expected_size} bytes, got {}",
            proof_bytes.len()
        );
    }
    Ok(())
}

fn find_assertion<T: serde::de::DeserializeOwned>(
    assertions: &[ManifestAssertion],
    label: &str,
) -> Result<T> {
    let ma = assertions
        .iter()
        .find(|a| a.label() == label)
        .ok_or_else(|| anyhow::anyhow!("assertion {label} not found in manifest"))?;
    let value: Value = ma
        .value()
        .with_context(|| format!("assertion {label} is not JSON"))?
        .clone();
    serde_json::from_value(value)
        .with_context(|| format!("assertion {label} has unexpected shape"))
}
