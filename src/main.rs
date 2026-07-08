mod assertions;
mod c2pa_manifest;
mod pi_capture;
mod schema_utils;
mod verify;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use c2pa_manifest::{sign_capture_asset, sign_edit_asset, SignMaterial};
use clap::{Parser, Subcommand};
use pi_capture::LibcameraContract;
use sha2::{Digest, Sha256};
use verify::verify_level0_bundle;

#[derive(Parser, Debug)]
#[command(version, about = "FightFake Level 0 assertion emitter")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Emit neutral capture assertion payload JSON
    EmitCapture {
        #[arg(long)]
        device_id: String,
        #[arg(long, default_value = "post_isp")]
        pipeline_stage: String,
        #[arg(long)]
        h1: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Emit neutral edit-proof assertion payload JSON
    EmitEditProof {
        #[arg(long, default_value = "nova-groth16")]
        proof_system: String,
        #[arg(long, default_value = "edit_only")]
        circuit_variant: String,
        #[arg(long)]
        gadget_id: String,
        #[arg(long)]
        h1: String,
        #[arg(long)]
        h2: String,
        #[arg(long)]
        proof_path: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Sign a source asset and embed org.zkedit.capture.v1 in a C2PA manifest
    SignCaptureManifest {
        #[arg(long)]
        source_asset: PathBuf,
        #[arg(long)]
        dest_asset: PathBuf,
        #[arg(long)]
        capture_assertion: PathBuf,
        #[arg(long)]
        cert_pem: PathBuf,
        #[arg(long)]
        key_pem: PathBuf,
    },
    /// Sign an edited asset with org.zkedit.edit_proof.v1 + capture ingredient
    SignEditManifest {
        #[arg(long)]
        source_asset: PathBuf,
        #[arg(long)]
        dest_asset: PathBuf,
        #[arg(long)]
        parent_capture_asset: PathBuf,
        #[arg(long)]
        edit_assertion: PathBuf,
        #[arg(long)]
        cert_pem: PathBuf,
        #[arg(long)]
        key_pem: PathBuf,
    },
    /// Verify schema and linkage checks for Level 0 assertion bundle
    VerifyLevel0Bundle {
        #[arg(long)]
        capture_assertion: PathBuf,
        #[arg(long)]
        edit_assertion: PathBuf,
        #[arg(long)]
        proof_path: PathBuf,
        #[arg(long, default_value = "./schemas/org.zkedit.capture.v1.schema.json")]
        capture_schema: PathBuf,
        #[arg(long, default_value = "./schemas/org.zkedit.edit_proof.v1.schema.json")]
        edit_schema: PathBuf,
    },
    /// Print the Level 1 Raspberry Pi libcamera adapter contract
    PrintPiCaptureContract,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::EmitCapture {
            device_id,
            pipeline_stage,
            h1,
            out,
        } => emit_capture(device_id, pipeline_stage, h1, out)?,
        Command::EmitEditProof {
            proof_system,
            circuit_variant,
            gadget_id,
            h1,
            h2,
            proof_path,
            out,
        } => emit_edit_proof(proof_system, circuit_variant, gadget_id, h1, h2, proof_path, out)?,
        Command::SignCaptureManifest {
            source_asset,
            dest_asset,
            capture_assertion,
            cert_pem,
            key_pem,
        } => {
            sign_capture_asset(
                &source_asset,
                &dest_asset,
                &capture_assertion,
                &SignMaterial {
                    cert_path: &cert_pem,
                    key_path: &key_pem,
                },
            )?;
            println!("{}", dest_asset.display());
        }
        Command::SignEditManifest {
            source_asset,
            dest_asset,
            parent_capture_asset,
            edit_assertion,
            cert_pem,
            key_pem,
        } => {
            sign_edit_asset(
                &source_asset,
                &dest_asset,
                &parent_capture_asset,
                &edit_assertion,
                &SignMaterial {
                    cert_path: &cert_pem,
                    key_path: &key_pem,
                },
            )?;
            println!("{}", dest_asset.display());
        }
        Command::VerifyLevel0Bundle {
            capture_assertion,
            edit_assertion,
            proof_path,
            capture_schema,
            edit_schema,
        } => {
            verify_level0_bundle(
                &capture_assertion,
                &edit_assertion,
                &proof_path,
                &capture_schema,
                &edit_schema,
            )?;
            println!("ok");
        }
        Command::PrintPiCaptureContract => {
            println!("{}", LibcameraContract::adapter_notes());
        }
    }
    Ok(())
}

fn emit_capture(device_id: String, pipeline_stage: String, h1: String, out: Option<PathBuf>) -> Result<()> {
    let payload = CaptureAssertionV1 {
        assertion_type: CAPTURE_ASSERTION_TYPE.to_owned(),
        version: 1,
        hash_algorithm: "griffin".to_owned(),
        pipeline_stage,
        device_id,
        h1,
    };
    write_json(out, &payload)
}

fn emit_edit_proof(
    proof_system: String,
    circuit_variant: String,
    gadget_id: String,
    h1: String,
    h2: String,
    proof_path: PathBuf,
    out: Option<PathBuf>,
) -> Result<()> {
    let proof = fs::read(&proof_path)
        .with_context(|| format!("failed to read proof bytes from {}", proof_path.display()))?;
    let proof_sha256 = hex::encode(Sha256::digest(&proof));

    let payload = EditProofAssertionV1 {
        assertion_type: EDIT_PROOF_ASSERTION_TYPE.to_owned(),
        version: 1,
        proof_system,
        circuit_variant,
        gadget_id,
        h1,
        h2,
        proof_sha256,
        proof_size_bytes: proof.len() as u64,
    };
    write_json(out, &payload)
}

fn write_json<T: serde::Serialize>(out: Option<PathBuf>, payload: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(payload)?;
    if let Some(path) = out {
        fs::write(&path, json.as_bytes())
            .with_context(|| format!("failed to write output to {}", path.display()))?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}
