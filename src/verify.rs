use std::path::Path;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::assertions::{read_capture_assertion, read_edit_proof_assertion};
use crate::schema_utils::validate_json_against_schema;

pub fn verify_level0_bundle(
    capture_assertion_path: &Path,
    edit_assertion_path: &Path,
    proof_path: &Path,
    capture_schema_path: &Path,
    edit_schema_path: &Path,
) -> Result<()> {
    validate_json_against_schema(capture_assertion_path, capture_schema_path, "org.zkedit.capture.v1")?;
    validate_json_against_schema(edit_assertion_path, edit_schema_path, "org.zkedit.edit_proof.v1")?;

    let capture = read_capture_assertion(capture_assertion_path)?;
    let edit = read_edit_proof_assertion(edit_assertion_path)?;

    if capture.h1 != edit.h1 {
        bail!(
            "h1 mismatch: capture assertion has {}, edit-proof assertion has {}",
            capture.h1,
            edit.h1
        );
    }

    let proof_bytes = std::fs::read(proof_path)
        .with_context(|| format!("failed to read proof bytes from {}", proof_path.display()))?;
    let expected_sha = hex::encode(Sha256::digest(&proof_bytes));

    if expected_sha != edit.proof_sha256 {
        bail!(
            "proof SHA mismatch: expected {}, assertion has {}",
            expected_sha,
            edit.proof_sha256
        );
    }

    if proof_bytes.len() as u64 != edit.proof_size_bytes {
        bail!(
            "proof size mismatch: expected {} bytes, assertion has {}",
            proof_bytes.len(),
            edit.proof_size_bytes
        );
    }

    Ok(())
}
