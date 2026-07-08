//! Generate a self-signed ECDSA P-256 certificate for testing.
//!
//! C2PA mandates ES256 (ECDSA over P-256 with SHA-256).  The generated cert
//! is suitable for local testing but should NOT be used in production — real
//! deployments need certificates issued by a CA trusted by the C2PA verifier.

use std::path::Path;

use anyhow::{Context, Result};
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType};

/// Generate a self-signed P-256 cert and write PEM files.
///
/// Creates:
/// - `<out_dir>/signer-cert.pem`  — certificate (give to verifiers)
/// - `<out_dir>/signer-key.pem`   — private key  (keep secret)
pub fn generate_test_cert(out_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let mut params = CertificateParams::default();

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "fightfake-toolkit test signer");
    dn.push(DnType::OrganizationName, "fightfake-toolkit");
    params.distinguished_name = dn;

    // Subject Alternative Name (required by c2pa-rs).
    params.subject_alt_names = vec![rcgen::SanType::DnsName(
        "fightfake.example.com".to_owned(),
    )];

    // Use ECDSA P-256 — the only algorithm accepted by the c2pa-rs ES256 signer.
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256;

    let cert = Certificate::from_params(params)
        .context("failed to generate certificate")?;

    let cert_pem = cert.serialize_pem().context("failed to serialize certificate")?;
    let key_pem  = cert.serialize_private_key_pem();

    let cert_path = out_dir.join("signer-cert.pem");
    let key_path  = out_dir.join("signer-key.pem");

    std::fs::write(&cert_path, cert_pem.as_bytes())
        .with_context(|| format!("failed to write {}", cert_path.display()))?;
    std::fs::write(&key_path, key_pem.as_bytes())
        .with_context(|| format!("failed to write {}", key_path.display()))?;

    println!("cert : {}", cert_path.display());
    println!("key  : {}", key_path.display());
    println!();
    println!("Pass these to `prove-edit` via --cert and --key.");
    println!("NOTE: self-signed certs are for local testing only.");

    Ok(())
}
