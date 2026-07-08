use serde::{Deserialize, Serialize};
use std::path::Path;

use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAssertionV1 {
    pub assertion_type: String,
    pub version: u32,
    pub hash_algorithm: String,
    pub pipeline_stage: String,
    pub device_id: String,
    pub h1: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditProofAssertionV1 {
    pub assertion_type: String,
    pub version: u32,
    pub proof_system: String,
    pub circuit_variant: String,
    pub gadget_id: String,
    pub h1: String,
    pub h2: String,
    pub proof_sha256: String,
    pub proof_size_bytes: u64,
}

pub const CAPTURE_ASSERTION_TYPE: &str = "org.zkedit.capture.v1";
pub const EDIT_PROOF_ASSERTION_TYPE: &str = "org.zkedit.edit_proof.v1";

pub fn read_capture_assertion(path: &Path) -> Result<CaptureAssertionV1> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read capture assertion at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid capture assertion JSON at {}", path.display()))
}

pub fn read_edit_proof_assertion(path: &Path) -> Result<EditProofAssertionV1> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read edit-proof assertion at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid edit-proof assertion JSON at {}", path.display()))
}
