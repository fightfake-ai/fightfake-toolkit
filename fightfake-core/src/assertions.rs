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
    /// Gadget-specific parameters, e.g. `{"scale": 416}` for brightness or
    /// `{"x":.., "y":.., "w":.., "h":.., "frame_start":.., "frame_end":..}`
    /// for redact.  Omitted (not `null`) when a gadget takes no parameters.
    /// This is what lets a verifier — or a human reading the C2PA manifest —
    /// see exactly which pixels, and for which frames, the proof covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gadget_params: Option<serde_json::Value>,
    /// Present when the ZK proof only covers a scoped "touched" frame range
    /// (the `--touched-window` prove-edit flag): pre/post segments outside
    /// this range are attested by a plain hash instead of the Nova/Groth16
    /// circuit, since they are declared to be byte-identical to the original.
    /// `h1`/`h2` above are then the outer combination of the three segment
    /// hashes, not a single hash over the whole clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched_window: Option<TouchedWindowInfo>,
}

/// SHA-256 (hex) of one of the three segments a "touched window" proof splits
/// the macroblock sequence into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentHashes {
    /// Hash over the untouched macroblocks before `frame_start`.
    pub pre: String,
    /// Hash over the macroblocks in `[frame_start, frame_end)` — the segment
    /// actually covered by the Nova IVC + Groth16 circuit.
    pub mid: String,
    /// Hash over the untouched macroblocks from `frame_end` onward.
    pub post: String,
}

/// Metadata recorded when `prove-edit --touched-window` scopes the real ZK
/// proof to a frame range instead of the whole clip.
///
/// `h1`/`h2` in the enclosing [`EditProofAssertionV1`] are computed as
/// `SHA256("pre" ‖ pre ‖ "mid" ‖ mid ‖ "post" ‖ post)` over the corresponding
/// [`SegmentHashes`] (as raw 32-byte digests, not hex strings). A verifier who
/// has the published edited video can recompute `h2_segments` directly from
/// its pixels and confirm the pre/post segments were not tampered with,
/// without needing the original video or a Groth16 verifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchedWindowInfo {
    /// First frame (inclusive, 0-based) covered by the real circuit.
    pub frame_start: usize,
    /// Last frame (exclusive) covered by the real circuit.
    pub frame_end: usize,
    /// Total frame count of the clip, so pre/post segment sizes are
    /// unambiguous even without decoding the video.
    pub num_frames: usize,
    /// Segment hashes over the original (h1) pixel bytes.
    pub h1_segments: SegmentHashes,
    /// Segment hashes over the edited (h2) pixel bytes.
    pub h2_segments: SegmentHashes,
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
