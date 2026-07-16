use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use c2pa::{create_signer, Builder, SigningAlg};
use fightfake_core::assertions::{
    read_capture_assertion, read_edit_proof_assertion, CAPTURE_ASSERTION_LABEL,
    EDIT_PROOF_ASSERTION_LABEL,
};
use serde_json::json;

pub struct SignMaterial<'a> {
    pub cert_path: &'a Path,
    pub key_path: &'a Path,
}

/// Crop dimensions recorded in the capture manifest when the source video
/// was auto-cropped to satisfy Eva's 16-pixel alignment requirement.
pub struct CropInfo {
    pub orig_width: usize,
    pub orig_height: usize,
    pub cropped_width: usize,
    pub cropped_height: usize,
}

pub fn sign_capture_asset(
    source_asset: &Path,
    dest_asset: &Path,
    capture_assertion_path: &Path,
    signer: &SignMaterial<'_>,
    crop: Option<&CropInfo>,
) -> Result<()> {
    let capture = read_capture_assertion(capture_assertion_path)?;
    let mut builder = Builder::from_json(
        &json!({ "title": format!("fightfake capture: {}", capture.device_id) }).to_string(),
    )
    .context("failed to initialize capture C2PA builder")?;

    // If auto-crop was applied, record it as a standard c2pa.cropped action so
    // verifiers know h1 covers (cropped_width × cropped_height), not the full frame.
    if let Some(c) = crop {
        let crop_assertion = json!({
            "actions": [{
                "action": "c2pa.cropped",
                "softwareAgent": {
                    "name": "fightfake-toolkit",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "parameters": {
                    "description": format!(
                        "Auto-cropped {}×{} → {}×{} to satisfy 16-pixel macroblock alignment \
                         required by the ZK prover. h1 fingerprint covers the cropped frame only.",
                        c.orig_width, c.orig_height, c.cropped_width, c.cropped_height
                    )
                }
            }]
        });
        builder
            .add_assertion("c2pa.actions", &crop_assertion)
            .context("failed to add c2pa.cropped action")?;
    }

    builder
        .add_assertion(CAPTURE_ASSERTION_LABEL, &capture)
        .context("failed to add org.zkedit.capture")?;

    let signer = make_signer(signer)?;
    remove_if_exists(dest_asset)?;
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
        Builder::from_json(&json!({ "title": "fightfake edit proof" }).to_string())
            .context("failed to initialize edit C2PA builder")?;

    // Ingredient: link back to the signed capture asset.
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

    // c2pa.actions — required by standard C2PA verifiers and viewers.
    // Maps fightfake gadget IDs to the nearest standard C2PA action code.
    let c2pa_action = gadget_to_c2pa_action(&edit.gadget_id);
    let description = gadget_description(&edit.gadget_id, edit.gadget_params.as_ref());
    let actions_assertion = json!({
        "actions": [{
            "action": c2pa_action,
            "softwareAgent": {
                "name": "fightfake-toolkit",
                "version": env!("CARGO_PKG_VERSION")
            },
            "parameters": {
                "description": description
            }
        }]
    });
    builder
        .add_assertion("c2pa.actions", &actions_assertion)
        .context("failed to add c2pa.actions")?;

    // org.zkedit.edit_proof.v1 — our ZK proof assertion.
    builder
        .add_assertion(EDIT_PROOF_ASSERTION_LABEL, &edit)
        .context("failed to add org.zkedit.edit_proof")?;

    let signer = make_signer(signer)?;
    remove_if_exists(dest_asset)?;
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

/// Map our gadget IDs to the closest standard `c2pa.actions` action code.
/// See https://c2pa.org/specifications/specifications/1.4/specs/C2PA_Specification.html#_actions
///
/// Note: `redact` maps to `c2pa.drawing` ("changes using drawing tools
/// including brushes or eraser"), not `c2pa.redacted` — the latter is a
/// reserved C2PA action meaning "a manifest assertion was removed" and does
/// not describe a pixel-level edit at all.
fn gadget_to_c2pa_action(gadget_id: &str) -> &'static str {
    match gadget_id {
        "brightness" => "c2pa.color_adjustments",
        "grayscale"  => "c2pa.color_adjustments",
        "invert"     => "c2pa.color_adjustments",
        "crop"       => "c2pa.cropped",
        "redact"     => "c2pa.drawing",
        _            => "c2pa.edited",
    }
}

/// Build a human-readable description of the edit, using the actual
/// `gadget_params` recorded by the workflow rather than a generic string —
/// so the description reflects what was really done, not just which gadget
/// ran.  Falls back to a generic sentence if params were not recorded.
fn gadget_description(gadget_id: &str, params: Option<&serde_json::Value>) -> String {
    match (gadget_id, params) {
        ("brightness", Some(p)) => {
            let scale = p.get("scale").and_then(|v| v.as_u64()).unwrap_or(0);
            format!(
                "Brightness adjustment (luma scale {scale}/1024 ≈ {:.2}×)",
                scale as f64 / 1024.0
            )
        }
        ("brightness", None) => "Brightness adjustment".to_owned(),
        ("grayscale", _) => "Converted to grayscale (chroma set to neutral)".to_owned(),
        ("invert", _) => "Colour invert (all channels: 255 − pixel)".to_owned(),
        ("redact", Some(p)) => {
            let g = |k: &str| p.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
            match p.get("track").and_then(|v| v.as_array()) {
                Some(track) => format!(
                    "Blacked out a moving pixel region (tracked across {} keyframe(s)), \
                     frames {}–{} only (fill value {}). All other pixels and frames are \
                     unchanged.",
                    track.len(), g("frame_start"), g("frame_end"), g("fill_y"),
                ),
                None => format!(
                    "Blacked out a {}×{} pixel region at ({}, {}), frames {}–{} only \
                     (fill value {}). All other pixels and frames are unchanged.",
                    g("w"), g("h"), g("x"), g("y"), g("frame_start"), g("frame_end"), g("fill_y"),
                ),
            }
        }
        ("redact", None) => "Blacked out a pixel region for a limited frame range".to_owned(),
        (other, _) => format!("Edit gadget: {other}"),
    }
}

/// Sign a video with a plain (standard) C2PA manifest — no ZK assertions.
///
/// Produces a manifest that any standard C2PA viewer understands:
/// - `c2pa.actions` describing the edit in human-readable form.
/// - `c2pa.hash.bmff.v3` hard binding (added automatically by c2pa-rs).
/// - No `org.zkedit.*` assertions, no pixel fingerprints, no proof.
///
/// This is what you would produce with any standard C2PA tool.
/// Contrast with `sign_edit_asset`, which additionally embeds `org.zkedit.*`
/// assertions and links to a ZK proof.
pub fn sign_plain_c2pa(
    source_asset: &Path,
    dest_asset: &Path,
    title: &str,
    action_label: &str,
    description: &str,
    signer: &SignMaterial<'_>,
) -> Result<()> {
    let mut builder =
        Builder::from_json(&json!({ "title": title }).to_string())
            .context("failed to initialize plain C2PA builder")?;

    let actions_assertion = json!({
        "actions": [{
            "action": action_label,
            "softwareAgent": {
                "name": "fightfake-toolkit",
                "version": env!("CARGO_PKG_VERSION")
            },
            "parameters": {
                "description": description
            }
        }]
    });
    builder
        .add_assertion("c2pa.actions", &actions_assertion)
        .context("failed to add c2pa.actions")?;

    let signer = make_signer(signer)?;
    remove_if_exists(dest_asset)?;
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

/// Remove `path` if it already exists so `sign_file` can write it fresh.
/// c2pa-rs refuses to overwrite an existing file.
fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to remove existing file {}", path.display()))?;
    }
    Ok(())
}

fn make_signer(m: &SignMaterial<'_>) -> Result<Box<dyn c2pa::Signer>> {
    create_signer::from_files(m.cert_path, m.key_path, SigningAlg::Es256, None)
        .context("failed to build C2PA signer from cert/key files")
}
