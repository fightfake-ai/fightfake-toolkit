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
//! Build with the real cryptographic Groth16 verifier (heavier — ~700 KB
//! release + `wasm-opt`, vs. tens of KB for the default build — from the
//! arkworks stack; this crate does *not* pull in Eva's `folding-schemes`/
//! `video` crates, which cannot target `wasm32-unknown-unknown` at all,
//! see [`verify_groth16_proof`]):
//! ```bash
//! wasm-pack build fightfake-wasm --target web --release --features crypto-verify
//! ```
//!
//! # Cryptographic proof verification
//!
//! [`verify_groth16_proof`] runs the real Nova IVC + Groth16 "onchain
//! decider" pairing checks — the same math `fightfake verify-proof` runs
//! natively, and the same math the prover self-checks against right after
//! generating a proof (see `fightfake_core::proof_bundle` for why this is
//! one shared implementation rather than a native one and a separate,
//! browser-trusted one). Gated behind the `crypto-verify` feature (off by
//! default — see the module list above) since it pulls in the full
//! arkworks stack.
//!
//! Without that feature, [`verify_groth16_proof`] always returns `false` —
//! this is the pre-`crypto-verify` behaviour, kept for callers that build
//! without it.
//!
//! The assertion-linkage checks (h1 consistency, proof SHA-256) are
//! implemented unconditionally and are sufficient for fightfake.ai's
//! display workflow, but — see the README's verification-trust ladder —
//! they cannot catch a stub (non-cryptographic) proof pretending to be a
//! real one. Only `verify_groth16_proof` (with `crypto-verify`) can.

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

// ── Cryptographic proof verification ──────────────────────────────────────────

/// Cryptographically verify a fightfake edit proof — the actual Nova IVC +
/// Groth16 pairing checks, not just the assertion/hash bookkeeping
/// [`verify_assertion_linkage`] does.
///
/// `proof_bytes` is the raw contents of `proof.bin` as produced by
/// `fightfake prove-edit --features eva-backend` (the whole file — vk,
/// folded instances, and SNARK proof are all bundled together, see
/// `fightfake_core::proof_bundle::ProofBundle`).
///
/// Returns:
/// - `Ok(true)` — the proof is cryptographically valid.
/// - `Ok(false)` — well-formed bundle, but the pairing check failed (the
///   proof does not actually attest to what the manifest claims).
/// - `Err` — `proof_bytes` isn't a real proof bundle at all: either it's a
///   Level-0 stub `proof.bin` (32 zero bytes, built without
///   `--features eva-backend`, i.e. there is no cryptographic proof here to
///   check), or it's truncated/malformed.
///
/// Requires the `crypto-verify` feature (off by default — see the module
/// doc comment); without it, this always returns `Ok(false)`.
#[wasm_bindgen(js_name = verifyGroth16Proof)]
pub fn verify_groth16_proof(proof_bytes: &[u8]) -> Result<bool, JsError> {
    #[cfg(feature = "crypto-verify")]
    {
        use fightfake_core::proof_bundle::{verify_proof_bundle, ProofBundle};
        let bundle = ProofBundle::from_bytes(proof_bytes).map_err(|e| JsError::new(&e))?;
        verify_proof_bundle(&bundle).map_err(|e| JsError::new(&e))
    }
    #[cfg(not(feature = "crypto-verify"))]
    {
        let _ = proof_bytes;
        Ok(false)
    }
}
