# FightFake Proof Prototype (Level 0 + Level 1 plan)

This folder is a starter repository scaffold for `fightfake-ai` focused on:

1. **Level 0 prototype** you can run today (software-only provenance flow).
2. **Level 1 Raspberry Pi demonstrator plan** with realistic implementation details.

It is intentionally **scheme-neutral** at the assertion layer so you are not tied to Eva naming.

## License note

Eva is MIT licensed, so building a separate repository that depends on Eva components is generally fine.
Keep license notices for bundled code and dependencies.

## Neutral assertion namespace

Current draft assertion names in this prototype:

- `org.zkedit.capture.v1`
- `org.zkedit.edit_proof.v1`

If you prefer a different namespace later, update `schemas/` and `src/assertions.rs`.

## Repository layout

- `schemas/`: JSON schema drafts for custom C2PA assertions.
- `src/`: small Rust CLI to emit assertion payloads and proof bundle metadata.
- `docs/level1-pi-demonstrator.md`: concrete Level 1 hardware/software plan.

## Level 0 goal

Produce a reproducible manifest pair:

1. Capture-side assertion payload (`org.zkedit.capture.v1`) with `h1`.
2. Edit-side assertion payload (`org.zkedit.edit_proof.v1`) with:
   - `h1` (input chain root),
   - `h2` (output chain root),
   - proof bytes metadata,
   - circuit variant metadata.

This scaffold does not yet write full C2PA manifests. It provides the assertion payload and metadata layer first.
Next step is wiring `c2pa-rs` to embed these payloads into claim assertions.

## Eva backend strategy

The prototype treats Eva as a backend dependency:

- Default build: schema/payload tools only.
- `eva-backend` feature: enable direct linking to Eva crates for native hash/proof generation integration.

This keeps your public interface stable even if you swap proving backends later.

## Quick start

```bash
cd /Users/miha/projects/fightfake-projects/fightfake-proof-prototype
cargo run -- emit-capture --device-id "pi-cam-01" --pipeline-stage post_isp --h1 0x1234

cargo run -- emit-edit-proof \
  --proof-system nova-groth16 \
  --circuit-variant edit_only \
  --gadget-id brightness \
  --h1 0x1234 \
  --h2 0x9876 \
  --proof-path ./proof.bin
```

## Immediate next steps

1. Add a `c2pa-rs` writer step to embed emitted JSON as custom assertions.
2. Add a verifier command that:
   - validates assertion schema,
   - runs Groth16 verify,
   - cross-checks ingredient h1 linkage.
3. Add Pi capture adapter (`libcamera` frame callback -> hasher -> secure signing).
