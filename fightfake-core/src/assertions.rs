use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// C2PA manifest assertion label (what c2pa-rs stores and looks up).
/// c2pa-rs strips the default `.v1` suffix per the C2PA spec, so the label
/// is `org.zkedit.capture`, not `org.zkedit.capture.v1`.
pub const CAPTURE_ASSERTION_LABEL: &str = "org.zkedit.capture";
pub const EDIT_PROOF_ASSERTION_LABEL: &str = "org.zkedit.edit_proof";

/// Human-readable type string embedded inside the JSON payload (NOT the manifest label).
pub const CAPTURE_ASSERTION_TYPE: &str = "org.zkedit.capture.v1";
pub const EDIT_PROOF_ASSERTION_TYPE: &str = "org.zkedit.edit_proof.v1";

/// Payload stored in the `org.zkedit.capture.v1` C2PA assertion.
///
/// `h1` is the Griffin hash chain over the original macroblock pixels produced
/// at capture time. It serves as the unforgeable anchor linking a capture to an
/// edit proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAssertionV1 {
    pub assertion_type: String,
    pub version: u32,
    /// Hash function used to produce `h1` (always `"griffin"` for Eva proofs).
    pub hash_algorithm: String,
    /// Where in the camera pipeline the hash was computed, e.g. `"post_isp"`.
    pub pipeline_stage: String,
    /// Opaque device identifier (serial number, UUID, …).
    pub device_id: String,
    /// Hex-encoded Griffin hash chain over original macroblocks.
    pub h1: String,
}

/// Payload stored in the `org.zkedit.edit_proof.v1` C2PA assertion.
///
/// Carries everything a verifier needs: both hash endpoints, a SHA-256 digest
/// of the external proof file, and metadata about the proof system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditProofAssertionV1 {
    pub assertion_type: String,
    pub version: u32,
    /// E.g. `"nova-groth16"`.
    pub proof_system: String,
    /// E.g. `"edit_only"` (lossless) or `"edit_encode"` (with H.264 constraints).
    pub circuit_variant: String,
    /// E.g. `"brightness"`, `"crop"`, `"grayscale"`.
    pub gadget_id: String,
    /// Hex h1 — must match the capture assertion of the parent asset.
    pub h1: String,
    /// Hex h2 — Griffin hash chain over the edited macroblocks.
    pub h2: String,
    /// SHA-256 of the proof binary so verifiers can locate and authenticate it.
    pub proof_sha256: String,
    pub proof_size_bytes: u64,
}

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
