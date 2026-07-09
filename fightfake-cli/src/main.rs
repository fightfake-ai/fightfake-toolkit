mod c2pa_signer;
mod cert_gen;
mod demo;
mod ffmpeg;
mod pi_capture;
mod workflow;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use fightfake_core::assertions::{
    CaptureAssertionV1, EditProofAssertionV1, CAPTURE_ASSERTION_TYPE, EDIT_PROOF_ASSERTION_TYPE,
};
use sha2::{Digest, Sha256};

use c2pa_signer::{sign_capture_asset, sign_edit_asset, sign_plain_c2pa, SignMaterial};
use cert_gen::generate_test_cert;
use demo::{run_level0_demo, Level0DemoConfig};
use fightfake_core::verify::{verify_bundle, verify_capture_asset, verify_signed_assets};
use pi_capture::LibcameraContract;
use workflow::{run_prove_edit, Gadget, ProveEditConfig};

// ── CLI structure ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "fightfake",
    version,
    about = "fightfake-toolkit — ZK-proved video editing with C2PA provenance"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Full workflow: decode → edit → prove → C2PA sign → verify.
    ///
    /// Requires ffmpeg on PATH.  With default features the proof is a Level-0
    /// stub (32 zero bytes).  Build with `--features eva-backend` to generate
    /// a real Nova IVC + Groth16 proof.
    ProveEdit {
        /// Input video (MP4 or any container ffmpeg can decode).
        #[arg(long, short)]
        input: PathBuf,

        /// Edit gadget to apply.
        #[arg(long, default_value = "brightness")]
        gadget: GadgetArg,

        /// Gadget parameter.  For brightness: luma scale in units of 1/1024
        /// (default 416 ≈ 0.41× — matches Eva's BrightnessCfg(416)).
        #[arg(long, default_value = "416")]
        gadget_param: u16,

        /// Output directory for all artefacts (created if absent).
        #[arg(long, short, default_value = "out")]
        out_dir: PathBuf,

        /// PEM certificate for C2PA signing.
        #[arg(long, default_value = "testdata/certs/signer-cert.pem")]
        cert: PathBuf,

        /// PEM private key for C2PA signing.
        #[arg(long, default_value = "testdata/certs/signer-key.pem")]
        key: PathBuf,

        /// Opaque device identifier embedded in the capture assertion.
        #[arg(long, default_value = "dev-0")]
        device_id: String,

        /// Macroblocks per Nova IVC step (eva-backend only; higher = fewer steps,
        /// more memory per step).
        #[arg(long, default_value = "256")]
        blocks_per_step: usize,
    },

    /// Verify a signed capture asset (no edit proof needed).
    ///
    /// Checks the C2PA signature and hard binding, then confirms the asset carries
    /// a valid org.zkedit.capture.v1 assertion.  Use this to confirm a video was
    /// signed at capture time before any edit was made.
    VerifyCapture {
        /// Signed capture asset (output of prove-edit or sign-capture-manifest).
        #[arg(long)]
        capture: PathBuf,
    },

    /// Verify a capture + edited asset pair and the associated proof.
    ///
    /// Checks the C2PA signatures and hard bindings on both assets, confirms
    /// org.zkedit.* assertions are present, h1 matches across the chain, and
    /// the proof binary has the expected SHA-256.
    Verify {
        /// Signed capture asset.
        #[arg(long)]
        capture: PathBuf,
        /// Signed edited asset.
        #[arg(long)]
        edited: PathBuf,
        /// Proof binary file.
        #[arg(long)]
        proof: PathBuf,
    },

    // ── Low-level plumbing commands ─────────────────────────────────────────

    /// Emit a capture assertion JSON (org.zkedit.capture.v1).
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

    /// Emit an edit-proof assertion JSON (org.zkedit.edit_proof.v1).
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

    /// Sign a source asset and embed org.zkedit.capture.v1 in a C2PA manifest.
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

    /// Sign an edited asset with org.zkedit.edit_proof.v1 + capture ingredient.
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

    /// Verify schema + linkage checks against assertion JSON side-files.
    VerifyBundle {
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

    /// Sign a video with a plain (standard) C2PA manifest — no ZK assertions.
    ///
    /// Produces a manifest that any standard C2PA viewer understands, containing
    /// only a `c2pa.actions` assertion and the automatic BMFF hard binding.
    /// No pixel fingerprints (h1/h2) and no ZK proof are embedded.
    ///
    /// Use this to compare a standard C2PA-signed video with a fightfake-signed
    /// one (produced by `prove-edit`).  The difference is:
    ///   standard: "I assert that this edit was made"  (no proof)
    ///   fightfake: "I can prove, mathematically, that only this edit was made"
    C2paSign {
        /// Input video (MP4 or any container ffmpeg can decode).
        #[arg(long, short)]
        input: PathBuf,

        /// Output signed video.
        #[arg(long, short)]
        output: PathBuf,

        /// Human-readable title for the manifest.
        #[arg(long, default_value = "C2PA-signed video")]
        title: String,

        /// C2PA action label (e.g. c2pa.color_adjustments, c2pa.cropped).
        #[arg(long, default_value = "c2pa.edited")]
        action: String,

        /// Description of the edit.
        #[arg(long, default_value = "Video processed with fightfake-toolkit")]
        description: String,

        /// PEM certificate for C2PA signing.
        #[arg(long, default_value = "testdata/certs/signer-cert.pem")]
        cert: PathBuf,

        /// PEM private key for C2PA signing.
        #[arg(long, default_value = "testdata/certs/signer-key.pem")]
        key: PathBuf,
    },

    /// Generate a self-signed P-256 test certificate (ES256, required by C2PA).
    ///
    /// Writes signer-cert.pem and signer-key.pem to the given directory.
    /// For local testing only — not suitable for production.
    MakeTestCert {
        /// Directory to write the certificate and key into.
        #[arg(long, default_value = "testdata/certs")]
        out_dir: PathBuf,
    },

    /// Print the Level 1 Raspberry Pi libcamera adapter contract.
    PrintPiCaptureContract,

    /// Run the Level 0 demo flow (pre-computed hashes + proof, C2PA signing).
    RunLevel0Demo {
        #[arg(long, default_value = "./testdata/videos/input/capture.mp4")]
        capture_input: PathBuf,
        #[arg(long, default_value = "./testdata/videos/input/edited.mp4")]
        edited_input: PathBuf,
        #[arg(long, default_value = "./testdata/proofs/proof.bin")]
        proof_path: PathBuf,
        #[arg(long, default_value = "./testdata/certs/signer-cert.pem")]
        cert_pem: PathBuf,
        #[arg(long, default_value = "./testdata/certs/signer-key.pem")]
        key_pem: PathBuf,
        #[arg(long, default_value = "demo-device")]
        device_id: String,
        #[arg(long, default_value = "post_isp")]
        pipeline_stage: String,
        #[arg(long, default_value = "nova-groth16")]
        proof_system: String,
        #[arg(long, default_value = "edit_only")]
        circuit_variant: String,
        #[arg(long, default_value = "brightness")]
        gadget_id: String,
        #[arg(long)]
        h1: String,
        #[arg(long)]
        h2: String,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum GadgetArg {
    Brightness,
    Grayscale,
    Invert,
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        // ── Verify ───────────────────────────────────────────────────────────
        Command::VerifyCapture { capture } => {
            let a = verify_capture_asset(&capture)?;
            println!("ok — C2PA signature valid");
            println!("device     : {}", a.device_id);
            println!("pipeline   : {}", a.pipeline_stage);
            println!("hash algo  : {}", a.hash_algorithm);
            println!("h1         : {}", a.h1);
        }

        Command::Verify { capture, edited, proof } => {
            verify_signed_assets(&capture, &edited, &proof)?;
            println!("ok — h1 consistent, proof hash matches, C2PA manifests valid");
        }

        // ── Full workflow ─────────────────────────────────────────────────────
        Command::ProveEdit {
            input,
            gadget,
            gadget_param,
            out_dir,
            cert,
            key,
            device_id,
            blocks_per_step,
        } => {
            let gadget = match gadget {
                GadgetArg::Brightness => Gadget::Brightness { scale: gadget_param },
                GadgetArg::Grayscale  => Gadget::Grayscale,
                GadgetArg::Invert     => Gadget::Invert,
            };
            let out = run_prove_edit(&ProveEditConfig {
                input,
                gadget,
                out_dir,
                cert_pem: cert,
                key_pem: key,
                device_id,
                blocks_per_step,
            })?;

            println!();
            println!("=== prove-edit complete ===");
            println!("edited video         : {}", out.edited_mp4.display());
            println!("proof                : {}", out.proof_bin.display());
            println!("signed capture       : {}", out.capture_signed_mp4.display());
            println!("signed edited        : {}", out.edited_signed_mp4.display());
            println!("h1                   : {}", out.h1_hex);
            println!("h2                   : {}", out.h2_hex);
            if out.proof_is_stub {
                println!();
                println!("NOTE: proof is a Level-0 stub (32 zero bytes).");
                println!("      Build with `--features eva-backend` for a real ZK proof.");
            }
        }

        // ── Plumbing commands ─────────────────────────────────────────────────
        Command::EmitCapture {
            device_id,
            pipeline_stage,
            h1,
            out,
        } => {
            let payload = CaptureAssertionV1 {
                assertion_type: CAPTURE_ASSERTION_TYPE.to_owned(),
                version: 1,
                hash_algorithm: "griffin".to_owned(),
                pipeline_stage,
                device_id,
                h1,
            };
            write_json(out, &payload)?;
        }

        Command::EmitEditProof {
            proof_system,
            circuit_variant,
            gadget_id,
            h1,
            h2,
            proof_path,
            out,
        } => {
            let proof = std::fs::read(&proof_path)
                .with_context(|| format!("failed to read {}", proof_path.display()))?;
            let payload = EditProofAssertionV1 {
                assertion_type: EDIT_PROOF_ASSERTION_TYPE.to_owned(),
                version: 1,
                proof_system,
                circuit_variant,
                gadget_id,
                h1,
                h2,
                proof_sha256: hex::encode(Sha256::digest(&proof)),
                proof_size_bytes: proof.len() as u64,
            };
            write_json(out, &payload)?;
        }

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
                &SignMaterial { cert_path: &cert_pem, key_path: &key_pem },
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
                &SignMaterial { cert_path: &cert_pem, key_path: &key_pem },
            )?;
            println!("{}", dest_asset.display());
        }

        Command::VerifyBundle {
            capture_assertion,
            edit_assertion,
            proof_path,
            capture_schema,
            edit_schema,
        } => {
            verify_bundle(
                &capture_assertion,
                &edit_assertion,
                &proof_path,
                &capture_schema,
                &edit_schema,
            )?;
            println!("ok");
        }

        Command::C2paSign { input, output, title, action, description, cert, key } => {
            sign_plain_c2pa(
                &input,
                &output,
                &title,
                &action,
                &description,
                &SignMaterial { cert_path: &cert, key_path: &key },
            )?;
            println!("signed: {}", output.display());
            println!();
            println!("This is a standard C2PA manifest with no ZK proof.");
            println!("Compare with `prove-edit` to see the fightfake additions.");
        }

        Command::MakeTestCert { out_dir } => {
            generate_test_cert(&out_dir)?;
        }

        Command::PrintPiCaptureContract => {
            println!("{}", LibcameraContract::adapter_notes());
        }

        Command::RunLevel0Demo {
            capture_input,
            edited_input,
            proof_path,
            cert_pem,
            key_pem,
            device_id,
            pipeline_stage,
            proof_system,
            circuit_variant,
            gadget_id,
            h1,
            h2,
        } => {
            run_level0_demo(Level0DemoConfig {
                capture_input,
                edited_input,
                proof_path,
                cert_pem,
                key_pem,
                device_id,
                pipeline_stage,
                proof_system,
                circuit_variant,
                gadget_id,
                h1,
                h2,
                capture_assertion_out: PathBuf::from(
                    "./testdata/assertions/capture.assertion.json",
                ),
                edit_assertion_out: PathBuf::from("./testdata/assertions/edit.assertion.json"),
                capture_signed_out: PathBuf::from(
                    "./testdata/videos/signed/capture.signed.mp4",
                ),
                edited_signed_out: PathBuf::from(
                    "./testdata/videos/signed/edited.signed.mp4",
                ),
                capture_schema: PathBuf::from(
                    "./schemas/org.zkedit.capture.v1.schema.json",
                ),
                edit_schema: PathBuf::from(
                    "./schemas/org.zkedit.edit_proof.v1.schema.json",
                ),
            })?;
        }
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn write_json<T: serde::Serialize>(out: Option<PathBuf>, payload: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(payload)?;
    if let Some(path) = out {
        std::fs::write(&path, json.as_bytes())
            .with_context(|| format!("failed to write to {}", path.display()))?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}
