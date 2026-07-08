mod assertions;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

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
