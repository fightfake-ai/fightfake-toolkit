//! Cryptographic verification of fightfake's Nova IVC + Groth16 "onchain
//! decider" proofs — the format `prove-edit --features eva-backend` writes
//! to `proof.bin`.
//!
//! This is the piece the README calls out as missing: WASM (and CLI)
//! verification today only checks the C2PA signature and that `proof.bin`'s
//! hash matches the manifest ([`crate::verify`]) — it never runs the actual
//! pairing checks, so a stub proof and a real one look identical. This
//! module runs those checks.
//!
//! ## Why this doesn't just call Eva's `video::decider::Decider::verify`
//!
//! That's the function fightfake-cli's prover already self-checks against
//! after proving (see `fightfake-cli/src/workflow.rs`), and it would be the
//! obvious thing to reuse. It can't be, for one reason: it (transitively,
//! through `folding-schemes`) hard-depends on `rayon`/native threads, and
//! `folding-schemes`'s non-parallel fallback code doesn't even compile —
//! its default features are `["parallel", "cpu"]` and `cpu = ["parallel"]`,
//! i.e. there's no feature combination that builds without `rayon`. Rayon's
//! thread pool doesn't exist on `wasm32-unknown-unknown`, so nothing that
//! depends on `folding-schemes` can ever run in a browser extension.
//!
//! What *does* build for `wasm32-unknown-unknown` (verified by hand: see the
//! `fightfake-wasm` `crypto-verify` feature and its CI/build check) is the
//! patched `ark-groth16` fork itself, plus `ark-bn254`/`ark-grumpkin`. The
//! actual pairing/subspace-SNARK verification math lives entirely in that
//! crate (`Groth16::verify_proof_with_prepared_inputs`) and is called here
//! unmodified. The only things reimplemented below are two small,
//! `folding-schemes`-only pieces of pure field/curve arithmetic that
//! `Decider::verify` needs on the way there:
//!
//! 1. [`fold_committed_instance`] — Nova's NIFS folding step (`nifs.rs`'s
//!    `NIFS::fold_committed_instance`, four curve additions and a scalar
//!    fold — see that function's source for the formula this mirrors).
//! 2. [`build_public_inputs`] — assembling the decider circuit's public
//!    input vector, including limb-splitting the non-native (Bn254 base
//!    field) commitment coordinates into `Fr` elements, mirroring
//!    `folding_schemes::folding::circuits::nonnative::uint::NonNativeUintVar`'s
//!    scheme (a fixed 55-bit limb width, hardcoded as [`BITS_PER_LIMB`]
//!    below — that width is a compile-time constant in the original too,
//!    not something that needs the circuit machinery to compute).
//!
//! Both are cross-checked against the real `Decider::verify` in
//! `fightfake-cli`'s test suite (`eva-backend` feature) every time a real
//! proof is generated, so this isn't verification logic running unchecked
//! in production — see `run_nova_groth16!` in `workflow.rs`.

use ark_bn254::{Bn254, Fq, Fr, G1Projective};
use ark_ec::{AffineRepr, CurveGroup, VariableBaseMSM};
use ark_ff::{BigInteger, PrimeField, Zero};
use ark_groth16::{Groth16, Proof as Groth16Proof, VerifyingKey as Groth16VerifyingKey};
use ark_grumpkin::Projective as GrumpkinProjective;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;

/// 5-byte header every real (non-stub) `proof.bin` starts with:
/// `b"FFPB"` (FightFake Proof Bundle) + a 1-byte format version.
const MAGIC: [u8; 4] = *b"FFPB";
const VERSION: u8 = 1;

/// Bit-width of each limb Eva's CycleFold augmented circuit uses to
/// represent a non-native (Bn254 base field `Fq`) coordinate as Bn254
/// scalar-field (`Fr`) public inputs — the literal `40` returned by
/// `NonNativeUintVar::<F>::bits_per_limb()` in Eva's `folding-schemes`
/// crate (that function's doc comment reasons about a value of `55`, from
/// an earlier tuning pass, but the code was since changed to return `40`
/// without updating the comment — verified against a real generated proof:
/// `40` is what actually reproduces the circuit's public-input count and
/// `55` does not). Not a function of the constraint system or any runtime
/// state, so hardcoding it here does not risk drifting from the circuit
/// unless Eva changes curves or tunes this constant again.
const BITS_PER_LIMB: usize = 40;

fn ser_err(e: impl core::fmt::Display) -> String {
    format!("proof bundle serialization error: {e}")
}

fn de_err(e: impl core::fmt::Display) -> String {
    format!("proof bundle deserialization error: {e}")
}

/// The subset of a Nova "committed instance" that decider verification
/// actually needs: the folding accumulator commitment (`cm_e`), the
/// RelaxedR1CS scalar `u`, and the two per-step witness commitments
/// (`cm_q`, `cm_w`). Mirrors `folding_schemes::folding::nova::{RunningInstance,
/// CurrentInstance}` — their public-IO vector `x` is omitted because
/// `Decider::verify` never reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldedInstance {
    pub cm_e: G1Projective,
    pub u: Fr,
    pub cm_q: G1Projective,
    pub cm_w: G1Projective,
}

impl FoldedInstance {
    fn write(&self, out: &mut Vec<u8>) -> Result<(), String> {
        self.cm_e.serialize_compressed(&mut *out).map_err(ser_err)?;
        self.u.serialize_compressed(&mut *out).map_err(ser_err)?;
        self.cm_q.serialize_compressed(&mut *out).map_err(ser_err)?;
        self.cm_w.serialize_compressed(&mut *out).map_err(ser_err)?;
        Ok(())
    }

    fn read(r: &mut &[u8]) -> Result<Self, String> {
        Ok(Self {
            cm_e: CanonicalDeserialize::deserialize_compressed(&mut *r).map_err(de_err)?,
            u: CanonicalDeserialize::deserialize_compressed(&mut *r).map_err(de_err)?,
            cm_q: CanonicalDeserialize::deserialize_compressed(&mut *r).map_err(de_err)?,
            cm_w: CanonicalDeserialize::deserialize_compressed(&mut *r).map_err(de_err)?,
        })
    }
}

/// Everything needed to cryptographically verify one fightfake edit proof,
/// independent of the file(s) it came from. This is `proof.bin`'s on-disk
/// format for real (`eva-backend`) proofs — see [`ProofBundle::to_bytes`]
/// and [`ProofBundle::from_bytes`]. The Level-0 stub build's `proof.bin`
/// (32 zero bytes) is a different, deliberately-not-this format — see
/// [`ProofBundle::looks_like_bundle`].
#[derive(Clone, Debug)]
pub struct ProofBundle {
    /// Number of Nova IVC steps folded (one step per `--redact-*`-sized
    /// macroblock batch; see `blocks_per_step` in `workflow.rs`).
    pub num_steps: u64,
    /// Initial IVC state `z_0` (currently always `[0, 0]`, stored
    /// explicitly rather than assumed so a future circuit change can't
    /// silently desync the verifier from the prover).
    pub z0: Vec<Fr>,
    /// Final IVC state's second component — the edited-video hash `h2`
    /// this proof attests to (see `EditProofAssertionV1::h2`).
    pub h2: Fr,
    /// The device's Schnorr public key on the CycleFold curve (Grumpkin),
    /// binding this proof to whichever key signed `h1` — the zero point if
    /// unused.
    pub device_vk: GrumpkinProjective,
    /// The Groth16 verifying key generated for this specific proof (Eva's
    /// setup here is per-proof/random, *not* a universal trusted setup —
    /// see `run_nova_groth16!` — so the key must travel with the proof).
    pub vk: Groth16VerifyingKey<Bn254>,
    /// Nova's running (folded-so-far) instance `U_i`.
    pub u_running: FoldedInstance,
    /// Nova's current (final) incoming instance `u_i`.
    pub u_current: FoldedInstance,
    /// The Groth16 SNARK proof itself.
    pub proof: Groth16Proof<Bn254>,
    /// The last fold's cross-term commitment (`cmT` in `U_{i+1} =
    /// NIFS.V(r, U_i, u_i, cmT)`).
    pub cm_t: G1Projective,
    /// The last fold's Fiat-Shamir challenge `r`.
    pub r: Fr,
}

impl ProofBundle {
    /// Serialize to the `proof.bin` on-disk format: the 5-byte
    /// [`MAGIC`]+[`VERSION`] header, followed by every field in a fixed
    /// order, each arkworks-compressed.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.num_steps.to_le_bytes());
        self.z0.serialize_compressed(&mut out).map_err(ser_err)?;
        self.h2.serialize_compressed(&mut out).map_err(ser_err)?;
        self.device_vk.serialize_compressed(&mut out).map_err(ser_err)?;
        self.vk.serialize_compressed(&mut out).map_err(ser_err)?;
        self.u_running.write(&mut out)?;
        self.u_current.write(&mut out)?;
        self.proof.serialize_compressed(&mut out).map_err(ser_err)?;
        self.cm_t.serialize_compressed(&mut out).map_err(ser_err)?;
        self.r.serialize_compressed(&mut out).map_err(ser_err)?;
        Ok(out)
    }

    /// `true` if `bytes` starts with the real-proof-bundle header. The
    /// Level-0 stub build's `proof.bin` (32 zero bytes) does not — that's
    /// how callers (`fightfake verify-proof`, the WASM binding) tell "not a
    /// cryptographic proof" apart from "malformed real proof".
    pub fn looks_like_bundle(bytes: &[u8]) -> bool {
        bytes.len() >= 5 && bytes[..4] == MAGIC
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if !Self::looks_like_bundle(bytes) {
            return Err(
                "not a fightfake proof bundle (missing \"FFPB\" header) — this looks like a \
                 Level-0 stub proof.bin (built without --features eva-backend), which carries \
                 no cryptographic proof to verify"
                    .to_owned(),
            );
        }
        if bytes[4] != VERSION {
            return Err(format!(
                "unsupported proof bundle version {} (this build understands version {VERSION})",
                bytes[4]
            ));
        }

        let mut r: &[u8] = &bytes[5..];
        if r.len() < 8 {
            return Err("truncated proof bundle (num_steps)".to_owned());
        }
        let (head, tail) = r.split_at(8);
        let num_steps = u64::from_le_bytes(head.try_into().unwrap());
        r = tail;

        let z0 = Vec::<Fr>::deserialize_compressed(&mut r).map_err(de_err)?;
        let h2 = Fr::deserialize_compressed(&mut r).map_err(de_err)?;
        let device_vk = GrumpkinProjective::deserialize_compressed(&mut r).map_err(de_err)?;
        let vk = Groth16VerifyingKey::<Bn254>::deserialize_compressed(&mut r).map_err(de_err)?;
        let u_running = FoldedInstance::read(&mut r)?;
        let u_current = FoldedInstance::read(&mut r)?;
        let proof = Groth16Proof::<Bn254>::deserialize_compressed(&mut r).map_err(de_err)?;
        let cm_t = G1Projective::deserialize_compressed(&mut r).map_err(de_err)?;
        let r_scalar = Fr::deserialize_compressed(&mut r).map_err(de_err)?;

        Ok(Self {
            num_steps,
            z0,
            h2,
            device_vk,
            vk,
            u_running,
            u_current,
            proof,
            cm_t,
            r: r_scalar,
        })
    }
}

/// Nova's NIFS folding step: `U' = U + r·u + r²·cmT` in the commitment
/// components, `u' = U.u + r·u.u` for the RelaxedR1CS scalar. Mirrors
/// `folding_schemes::folding::nova::nifs::NIFS::fold_committed_instance`
/// exactly (see that function for the reference formula) — reimplemented
/// here rather than imported because `folding-schemes` cannot build for
/// `wasm32-unknown-unknown` (see module doc comment), even though the
/// formula itself is four curve additions and nothing else.
fn fold_committed_instance(
    r: Fr,
    u_running: &FoldedInstance,
    u_current: &FoldedInstance,
    cm_t: G1Projective,
) -> FoldedInstance {
    FoldedInstance {
        cm_e: u_running.cm_e + cm_t * r,
        u: u_running.u + r * u_current.u,
        cm_q: u_running.cm_q + u_current.cm_q * r,
        cm_w: u_running.cm_w + u_current.cm_w * r,
    }
}

/// Split a base-field element's little-endian bits into [`BITS_PER_LIMB`]
/// chunks, each re-encoded as a scalar-field (`Fr`) big integer — matching
/// how Eva's CycleFold augmented circuit represents a non-native (Bn254
/// `Fq`) coordinate as `Fr` public inputs.
fn limb_bigints_of<F: PrimeField>(x: F) -> Vec<<Fr as PrimeField>::BigInt> {
    x.into_bigint()
        .to_bits_le()
        .chunks(BITS_PER_LIMB)
        .map(<Fr as PrimeField>::BigInt::from_bits_le)
        .collect()
}

/// Assemble the decider circuit's public input vector — same order and
/// encoding as `video::decider::Decider::verify` builds it from `(i, z_0,
/// h2, device_vk, U_i, u_i, cmT, r)` (deliberately *not* the folded
/// instance `U' `, which the circuit computes and checks internally; only
/// its `cm_q`/`cm_w`/`cm_e` are needed afterwards, as the SNARK's separate
/// "linked commitment" inputs — see [`verify_proof_bundle`]).
#[allow(clippy::too_many_arguments)]
fn build_public_inputs(
    num_steps: Fr,
    z0: &[Fr],
    h2: Fr,
    device_vk: GrumpkinProjective,
    u_running: &FoldedInstance,
    u_current: &FoldedInstance,
    cm_t: G1Projective,
    r: Fr,
) -> Vec<<Fr as PrimeField>::BigInt> {
    let one = <Fr as PrimeField>::BigInt::from(1u8);
    let zero = <Fr as PrimeField>::BigInt::from(0u8);

    let mut inputs = vec![one, num_steps.into_bigint()];
    inputs.extend(z0.iter().map(|f| f.into_bigint()));

    if device_vk.is_zero() {
        inputs.extend([h2.into_bigint(), zero, one, zero, u_running.u.into_bigint()]);
    } else {
        let (x, y) = device_vk
            .into_affine()
            .xy()
            .expect("device_vk is not the identity (checked above)");
        inputs.extend([
            h2.into_bigint(),
            x.into_bigint(),
            y.into_bigint(),
            one,
            u_running.u.into_bigint(),
        ]);
    }

    let points = G1Projective::normalize_batch(&[
        u_running.cm_q,
        u_running.cm_w,
        u_running.cm_e,
        u_current.cm_q,
        u_current.cm_w,
        cm_t,
    ]);
    for p in points {
        let (x, y): (Fq, Fq) = p.xy().unwrap_or((Fq::zero(), Fq::zero()));
        inputs.extend(limb_bigints_of(x));
        inputs.extend(limb_bigints_of(y));
    }

    inputs.push(r.into_bigint());
    inputs
}

/// Verify a [`ProofBundle`] — a full re-check of the Nova IVC + Groth16
/// "onchain decider" proof. This is the function both `fightfake
/// verify-proof` and the browser-extension WASM binding
/// (`verifyGroth16Proof`, gated behind fightfake-wasm's `crypto-verify`
/// feature) call; there is deliberately only one implementation of this
/// math for native and WASM to share, rather than the WASM side trusting a
/// second, separately-written copy.
///
/// Returns `Ok(true)`/`Ok(false)` for a well-formed bundle that
/// cryptographically checks out or doesn't; `Err` for a malformed bundle
/// (wrong lengths, points that don't decompress, etc).
pub fn verify_proof_bundle(bundle: &ProofBundle) -> Result<bool, String> {
    let folded = fold_committed_instance(
        bundle.r,
        &bundle.u_running,
        &bundle.u_current,
        bundle.cm_t,
    );

    let num_steps = Fr::from(bundle.num_steps);
    let public_inputs = build_public_inputs(
        num_steps,
        &bundle.z0,
        bundle.h2,
        bundle.device_vk,
        &bundle.u_running,
        &bundle.u_current,
        bundle.cm_t,
        bundle.r,
    );

    let pvk = Groth16::<Bn254>::process_vk(&bundle.vk).map_err(|e| format!("process_vk failed: {e}"))?;
    let prepared_inputs = G1Projective::msm_bigint(&pvk.vk.gamma_abc_g1.0, &public_inputs);

    let link_d = G1Projective::normalize_batch(&[folded.cm_q, folded.cm_w, folded.cm_e]);

    Groth16::<Bn254>::verify_proof_with_prepared_inputs(
        &pvk,
        &(bundle.proof.clone(), link_d),
        &prepared_inputs,
    )
    .map_err(|e| format!("groth16 verification error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::UniformRand;
    use rand::{rngs::StdRng, SeedableRng};

    fn thread_rng() -> StdRng {
        StdRng::from_entropy()
    }

    fn sample_instance(rng: &mut impl rand::Rng) -> FoldedInstance {
        FoldedInstance {
            cm_e: G1Projective::rand(rng),
            u: Fr::rand(rng),
            cm_q: G1Projective::rand(rng),
            cm_w: G1Projective::rand(rng),
        }
    }

    fn sample_bundle(rng: &mut impl rand::Rng) -> ProofBundle {
        ProofBundle {
            num_steps: 7,
            z0: vec![Fr::from(0u64), Fr::from(0u64)],
            h2: Fr::rand(rng),
            device_vk: GrumpkinProjective::rand(rng),
            vk: Groth16VerifyingKey {
                alpha_g1: ark_bn254::G1Affine::rand(rng),
                beta_g2: ark_bn254::G2Affine::rand(rng),
                gamma_g2: ark_bn254::G2Affine::rand(rng),
                delta_g2: ark_bn254::G2Affine::rand(rng),
                gamma_abc_g1: (
                    vec![ark_bn254::G1Affine::rand(rng); 4],
                    vec![ark_bn254::G1Affine::rand(rng); 4],
                ),
                eta_gamma_inv_g1: ark_bn254::G1Affine::rand(rng),
                link_pp: Default::default(),
                link_vk: Default::default(),
            },
            u_running: sample_instance(rng),
            u_current: sample_instance(rng),
            proof: Groth16Proof {
                a: ark_bn254::G1Affine::rand(rng),
                b: ark_bn254::G2Affine::rand(rng),
                c: ark_bn254::G1Affine::rand(rng),
                d: ark_bn254::G1Affine::rand(rng),
                link_pi: ark_bn254::G1Affine::rand(rng),
            },
            cm_t: G1Projective::rand(rng),
            r: Fr::rand(rng),
        }
    }

    #[test]
    fn bundle_round_trips_through_bytes() {
        let rng = &mut thread_rng();
        let bundle = sample_bundle(rng);
        let bytes = bundle.to_bytes().expect("serialize");

        assert!(ProofBundle::looks_like_bundle(&bytes));
        // The Level-0 stub proof (32 zero bytes) must never be mistaken for
        // a real bundle.
        assert!(!ProofBundle::looks_like_bundle(&[0u8; 32]));

        let round_tripped = ProofBundle::from_bytes(&bytes).expect("deserialize");
        assert_eq!(round_tripped.num_steps, bundle.num_steps);
        assert_eq!(round_tripped.z0, bundle.z0);
        assert_eq!(round_tripped.h2, bundle.h2);
        assert_eq!(round_tripped.device_vk, bundle.device_vk);
        assert_eq!(round_tripped.u_running, bundle.u_running);
        assert_eq!(round_tripped.u_current, bundle.u_current);
        assert_eq!(round_tripped.proof, bundle.proof);
        assert_eq!(round_tripped.cm_t, bundle.cm_t);
        assert_eq!(round_tripped.r, bundle.r);
    }

    #[test]
    fn stub_and_truncated_bytes_are_rejected() {
        assert!(ProofBundle::from_bytes(&[0u8; 32]).is_err());
        assert!(ProofBundle::from_bytes(&[]).is_err());
        assert!(ProofBundle::from_bytes(b"FFPB").is_err());
    }

    #[test]
    fn garbage_proof_bundle_fails_verification_rather_than_panicking() {
        let rng = &mut thread_rng();
        let bundle = sample_bundle(rng);
        // Random points/scalars have no reason to satisfy the pairing
        // check; the important thing is that verification returns `Ok(false)`
        // (or a clean `Err`), not a panic.
        match verify_proof_bundle(&bundle) {
            Ok(ok) => assert!(!ok, "random bundle should not verify"),
            Err(_) => {}
        }
    }
}
