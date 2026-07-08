//! WASM bindings for the fightfake browser verifier.
//!
//! Build with:
//! ```bash
//! wasm-pack build fightfake-wasm --target web --release
//! ```
//!
//! The output is a JS/WASM bundle you can import directly in a browser:
//! ```js
//! import init, { verifyAssertionLinkage, extractAssertions } from './fightfake_wasm.js';
//! await init();
//! ```
//!
//! # Cryptographic proof verification
//!
//! Verifying a Nova IVC + Groth16 proof in WASM requires the full arkworks
//! stack (ark-bn254, ark-groth16).  These compile to WASM but are heavy
//! (~2 MB gzipped).  Integration is tracked in the roadmap; for now, the
//! cryptographic check is gated behind the `full-verify` feature and marked
//! as `todo!()`.
//!
//! The assertion-linkage checks (h1 consistency, proof SHA-256) are
//! implemented today and are sufficient for fightfake.ai's display workflow.

use wasm_bindgen::prelude::*;

// ── Public types ──────────────────────────────────────────────────────────────

#[wasm_bindgen]
pub struct AssertionBundle {
    pub h1_matches: bool,
    pub proof_sha_matches: bool,
    capture_h1: String,
    edit_h1: String,
    gadget_id: String,
    proof_system: String,
}

#[wasm_bindgen]
impl AssertionBundle {
    pub fn capture_h1(&self) -> String { self.capture_h1.clone() }
    pub fn edit_h1(&self) -> String { self.edit_h1.clone() }
    pub fn gadget_id(&self) -> String { self.gadget_id.clone() }
    pub fn proof_system(&self) -> String { self.proof_system.clone() }
}

// ── Assertion-linkage check ───────────────────────────────────────────────────

/// Check h1 consistency and proof SHA-256 between two assertion JSON strings.
///
/// - `capture_json`: the `org.zkedit.capture.v1` assertion payload (JSON string)
/// - `edit_json`:    the `org.zkedit.edit_proof.v1` assertion payload (JSON string)
/// - `proof_bytes`:  the raw proof binary as a `Uint8Array`
///
/// Returns an [`AssertionBundle`] with individual pass/fail flags.
#[wasm_bindgen(js_name = verifyAssertionLinkage)]
pub fn verify_assertion_linkage(
    capture_json: &str,
    edit_json: &str,
    proof_bytes: &[u8],
) -> Result<AssertionBundle, JsError> {
    use fightfake_core::assertions::{CaptureAssertionV1, EditProofAssertionV1};
    use sha2::{Digest, Sha256};

    let capture: CaptureAssertionV1 = serde_json::from_str(capture_json)
        .map_err(|e| JsError::new(&format!("capture assertion JSON error: {e}")))?;
    let edit: EditProofAssertionV1 = serde_json::from_str(edit_json)
        .map_err(|e| JsError::new(&format!("edit assertion JSON error: {e}")))?;

    let h1_matches = capture.h1 == edit.h1;
    let actual_sha = hex::encode(Sha256::digest(proof_bytes));
    let proof_sha_matches = actual_sha == edit.proof_sha256;

    Ok(AssertionBundle {
        h1_matches,
        proof_sha_matches,
        capture_h1: capture.h1,
        edit_h1: edit.h1,
        gadget_id: edit.gadget_id,
        proof_system: edit.proof_system,
    })
}

/// Parse a capture assertion JSON string and return it as a JS object.
#[wasm_bindgen(js_name = parseCaptureAssertion)]
pub fn parse_capture_assertion(json: &str) -> Result<JsValue, JsError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| JsError::new(&format!("JSON parse error: {e}")))?;
    serde_wasm_bindgen::to_value(&v)
        .map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

/// Parse an edit-proof assertion JSON string and return it as a JS object.
#[wasm_bindgen(js_name = parseEditProofAssertion)]
pub fn parse_edit_proof_assertion(json: &str) -> Result<JsValue, JsError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| JsError::new(&format!("JSON parse error: {e}")))?;
    serde_wasm_bindgen::to_value(&v)
        .map_err(|e| JsError::new(&format!("serialization error: {e}")))
}

// ── Cryptographic proof verification (roadmap) ────────────────────────────────

/// Verify a Nova IVC + Groth16 proof in the browser.
///
/// **Not yet implemented.**  Returns `false` until the arkworks WASM build is
/// integrated.  Tracked in fightfake-wasm roadmap.
#[wasm_bindgen(js_name = verifyGroth16Proof)]
pub fn verify_groth16_proof(
    _proof_bytes: &[u8],
    _vk_bytes: &[u8],
    _h1_hex: &str,
    _h2_hex: &str,
    _num_steps: u32,
) -> bool {
    // TODO: deserialize Proof<Bn254> and VerifyingKey<Bn254> from bytes,
    //       call ark_groth16::Groth16::verify(), return result.
    //
    // Blocked on: WASM-compatible build of ark-bn254 + patched ark-groth16.
    false
}
