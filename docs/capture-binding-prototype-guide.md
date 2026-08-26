# Capture binding prototype: what to buy, what to build

Companion to [hardware-requirements-for-capture-binding.md](./hardware-requirements-for-capture-binding.md).

This document is a **shopping and build list** for the first hardware prototype that proves **capture binding**: a short value `B` tied to a locked pixel stream, signed by a key the OS cannot misuse. It is not a product roadmap and not a consumer camera.

For contrast: [level1-pi-demonstrator.md](./level1-pi-demonstrator.md) describes hashing in a **trusted software path** on Raspberry Pi (Level 1). This guide describes a **hardware path** closer to the architecture in the requirements doc (locked tap, forge-proof registers, narrow secure-element attest).

---

## What the prototype must prove

Success is **security**, not 4K throughput.

| Must pass | Meaning |
|-----------|---------|
| `B` comes only from the locked tap | Hash/eval input is not Linux DRAM or a file re-read from disk |
| OS cannot write `BIND_VALUE` | Register writes from the OS are ignored or fault |
| No `sign(digest_from_os)` | Capture key signs only hardware-finalized `B` (+ session metadata) |
| Session is independent of swapped files | Replacing the MP4 on disk after attest does not change an already produced `(B, σ)` |

Nice to have later (not required for prototype v1):

- Full Eva Griffin hash at live frame rate  
- C2PA manifest packaging  
- Profile B (polynomial commitment + assistant)

---

## Recommended scope for v1

**One pipe. One scheme. One clip (~10 s). Profile A-ish.**

1. Pixel stream enters a **binding engine** (start with SHA-256 or a simple extend hash; swap to proof-friendly hash when the pipe works).  
2. Optional parallel path: write YUV or a minimal H.264 bitstream for playback (not the hash input).  
3. **Secure signer** (ATECC608, TPM, or second MCU) reads `B` and returns `σ`.  
4. Host software attaches `(B, σ)` to metadata; off-device tooling recomputes `B` from stored pixels and runs one edit proof (fightfake-toolkit / eva).

Tap choice for v1:

- **Easier:** pre-encode YUV from CSI or a file-fed pattern (proves lockdown + attest; witnesses need lossless or pre-encode archive).  
- **Profile A alignment:** recon YUV from a hardware encoder block (needs encoder IP on FPGA or camera SoC eval kit).

---

## Two build paths

### Path A — FPGA + camera input (most control)

You own the tap, registers, and lock rules. Mux source, scheme id, and lock behavior are under your control.

```
CSI / HDMI-in / test pattern
        │
        ▼
   [optional mini encoder IP] ──► bitstream to SD (playback only)
        │
        ▼
   locked tap ──► binding engine ──► BIND regs
                                        │
                                        ▼
                              SE (ATECC / MCU) ──► (B, σ) to host
```

### Path B — Camera SoC evaluation board (ISP + encoder on one chip)

Buy a vendor **eval kit** with ISP + hardware encoder. Add binding as PLD/FPGA mezzanine, coprocessor, or (if the vendor allows) a custom firmware hook — only if the tap is **not** “hash this buffer the driver copied to kmalloc.”

Examples of vendors teams use for camera pipelines (availability varies): Ambarella, Rockchip, NXP i.MX with CSI, Qualcomm Dragonboard (harder). Pick one where you can get **recon or pre-encode YUV** documentation, not only encoded output.

---

## Buy vs build

| Item | Buy | Build / integrate |
|------|-----|-------------------|
| Host CPU + Linux | SBC on eval kit, or Kria SOM + carrier | — |
| Image sensor + lens | Camera module (IMX219, OV5647, HQ cam, industrial CSI module) | — |
| CSI receiver / HDMI capture | Mezzanine or FMC module | Only if you design custom PCB |
| Pixel tap + mux | — | **RTL** (or tightly scoped FPGA logic) |
| Binding engine (hash extend) | — | **RTL** or soft core on FPGA |
| `BIND_VALUE`, `STATUS`, `CTRL` regs | — | **RTL** + simple register map (see requirements doc §3) |
| Lock after boot (`CTRL.LOCK`) | — | **RTL** + boot stub that sets mux/scheme once |
| Secure element | ATECC608B breakout, TPM 2.0 module, or STM32 as signer-only MCU | Firmware: read regs → sign → mailbox |
| Device certificate / provisioning | — | Manufacturing script or one-time flash |
| H.264 encode (optional) | SoC on eval kit, or licensed encoder IP on FPGA | Integration only |
| Proof stack | — | **Software** (fightfake-toolkit, eva) — already separate |
| Demo enclosure | Off-the-shelf project box | Custom PCB only if needed |

---

## Example bill of materials (Path A, FPGA)

Rough classes — check current stock and your region. Prices are order-of-magnitude for planning.

| Qty | Part class | Example | Role |
|-----|------------|---------|------|
| 1 | AMD Kria KR260 or KV260 | SOM + carrier | Linux host, PCIe/Ethernet, runs demo app |
| 1 | Raspberry Pi HQ Camera or IMX219 CSI module | Official or Arducam | Live YUV into FPGA fabric |
| 1 | FPGA PL overlay or custom bitstream region | Vivado/Vitis flow | Tap + hash + regs |
| 1 | ATECC608B breakout (I2C) | Adafruit / SparkFun / Microchip dev board | Device key, narrow sign API |
| 1 | Logic analyzer or USB-UART | Saleae / FTDI | Debug register bus |
| 1 | microSD, PSU, cables | — | Bring-up |

**Alternative:** Lattice or Intel dev kit with CSI if your team already uses that toolchain.

**Alternative signer:** TPM 2.0 I2C/SPI module (good for “quote digest” pattern; less common on small drones).

**Alternative signer:** Second **STM32** (no WiFi): only job is I2C/SPI to binding regs, sign, expose mailbox. Keeps the signing key off the Linux CPU even without a full SE.

---

## Example bill of materials (Path B, SoC eval)

| Qty | Part class | Example | Role |
|-----|------------|---------|------|
| 1 | Camera SoC eval board | Vendor-specific (Ambarella / Rockchip / i.MX EVK) | ISP, encode, Linux BSP |
| 1 | Matching sensor board | Vendor kit accessory | Video in |
| 1 | Small FPGA or MCU bridge (optional) | If SoC cannot host binding regs | Binding engine + regs |
| 1 | ATECC608 or on-board SE | If eval board lacks SE | Signing |
| — | Vendor NDA + BSP docs | — | **Critical:** confirm where YUV/recon is visible |

Path B fails if the only interface is “here is an encoded file” or “copy this frame to userspace.” Confirm tap feasibility **before** buying.

---

## What to develop (software and firmware)

### 1. Register interface (minimal)

Implement the subset from the requirements doc:

| Register | Prototype need |
|----------|----------------|
| `CTRL` | Enable, lock, scheme id |
| `STATUS` | Busy / done / error |
| `BIND_VALUE` | Final `B` (OS read optional; OS write forbidden) |
| `SESSION_ID` | Increment per capture |
| `MB_COUNT` | Sanity check |

Map via AXI-lite (FPGA) or SPI/I2C (MCU bridge).

### 2. Boot / lock stub

Small program that runs **once** after reset:

- Select tap source and hash scheme.  
- Set `CTRL.LOCK`.  
- Refuse further mux changes from Linux.

On FPGA this can be logic that locks on first `LOCK` write from a trusted boot ROM stub; on eval kit it may be a signed bootloader fragment.

### 3. Signer firmware (ATECC / MCU)

Not a generic “sign any blob” API. Pseudocode:

```
on capture_done:
  B = read BIND_VALUE (private bus)
  session = read SESSION_ID
  σ = Sign(device_sk, B || session || ...)
  write mailbox(B, σ, session)
```

Reject any host command that supplies `B`.

### 4. Host demo daemon (Linux)

Minimal userspace:

- `start` / `stop` capture session (GPIO or register poke).  
- Poll mailbox for `(B, σ)`.  
- Write sidecar JSON (`org.zkedit.capture.v1` or similar).  
- Save MP4/YUV for later prove.

Use fightfake-toolkit for verify/prove off-device.

### 5. Negative-test harness

Automated or scripted:

1. Try `ioctl(SIGN, attacker_digest)` → must fail.  
2. Try `write(BIND_VALUE, fake)` → must not change signed value.  
3. Record session, swap file on disk, confirm `(B, σ)` unchanged.

Document results in the repo (one page is enough).

### 6. RTL / binding engine (v1 simplification)

- Input: fixed-format YUV tiles (16×16 luma, 8×8 chroma if targeting Eva later).  
- Operation: `state = H(state || tile_bytes)` with a known hash (SHA-256 acceptable for bring-up).  
- Output: write to `BIND_VALUE` on `finalize`.

Replace with proof-friendly hash once timing is understood.

---

## Block diagram (prototype v1)

```
                    ┌─────────────────────────────────┐
  CSI / pattern ──► │  FPGA or SoC + custom logic      │
                    │  ┌─────────┐    ┌──────────────┐  │
                    │  │ Pixel   │───►│ Binding      │  │
                    │  │ tap     │    │ engine       │  │
                    │  └─────────┘    └──────┬───────┘  │
                    │                        │ BIND regs
                    │  optional encode ──► SD card     │
                    └────────────────────────┼──────────┘
                                             │ I2C/SPI
                                             ▼
                                    ┌────────────────┐
                                    │ ATECC608 / MCU │
                                    │ Sign(B) → σ    │
                                    └────────┬───────┘
                                             │
                    Linux host ◄─────────────┘ mailbox
                    (demo daemon, no key)
```

---

## Phased milestones

### Milestone 0 — Fake tap (1–2 weeks)

File-fed YUV into binding engine on FPGA. Linux cannot feed the engine. Signer returns `(B, σ)`. **Negative tests 1–2 pass.**

Proves: register rules + narrow attest API.

### Milestone 1 — Live CSI (2–4 weeks)

Same as M0 with real camera module. Record 10 s clip. Store YUV or MP4. Recompute `B` off-device and compare.

Proves: real pixels flow through tap.

### Milestone 2 — Encode side path (optional, 2–4 weeks)

Add minimal H.264 encode in parallel. Hash still not from file. If recon tap exists, switch hash input to recon.

Proves: Eva-shaped storage story (MP4 for witnesses).

### Milestone 3 — One edit proof (software)

Use stored clip + `(B, σ)` from fightfake-toolkit to verify capture and generate one edit proof (crop or redact).

Proves: end-to-end capture → attest → off-device prove.

---

## Phones: what must change

Short answer: stock app-level integration is not enough for strong capture binding. A phone can support this, but only with OEM-side camera and TEE integration.

### What works on a stock phone (weaker)

- App captures frames through Camera APIs, hashes in app or normal OS process, then asks TEE/Secure Enclave to sign.
- This can be useful for UX and metadata plumbing, but a compromised OS can still feed fake frames or fake digests.
- Treat this as a software demonstrator, not as hardware capture binding.

### What an OEM must modify for strong binding

1. **Trusted tap in camera pipeline**  
   Add a tap at ISP or encoder-recon stage that is not replaceable by userspace buffers.
2. **Binding engine connected to that tap**  
   Compute `B` from tapped samples inside trusted camera/SoC path, not from app memory.
3. **Register and lock policy**  
   Expose `STATUS` and session metadata to OS; keep `BIND_VALUE` write-protected; lock source/scheme after secure boot.
4. **Signer policy in TEE/Secure Enclave**  
   Signer must read hardware-produced `B` (or trusted mailbox) and reject host-provided digest inputs.
5. **Attestation chain and rollback protection**  
   Include firmware measurements, version counters, and anti-rollback so older vulnerable camera firmware cannot be loaded.

### Practical migration path for phones

- **Stage P0 (today):** app-level hash + key attestation; good for integration experiments.
- **Stage P1:** vendor camera HAL/driver hook where hash input is pre-encode or recon samples.
- **Stage P2:** move hash/finalize and signer interface fully behind TEE/Secure Enclave policy.
- **Stage P3:** production hardening (anti-rollback, cert provisioning, audit logs, abuse testing).

### Phone-specific negative tests

1. Compromised app requests `sign(fake_digest)` -> must fail.
2. Rooted OS attempts to swap tap source after lock -> must fail.
3. Replaying stale signed outputs from another session/device -> must fail with session counters and nonce policy.

If an OEM cannot provide a trusted tap outside app/OS memory, phone support should be treated as a weaker provenance mode.

---

## What to defer

| Defer | Why |
|-------|-----|
| Profile B / PCS / assistant | Second prototype; binding engine differs |
| Full Griffin at 4K30 | Throughput after security tests pass |
| Custom ASIC | After FPGA/eval proves tap + attest |
| C2PA packaging polish | Metadata wrapper is enough for demo |
| Broad phone rollout | Needs OEM camera + TEE integration; see section above |

---

## Team skills (minimum)

| Role | Skills |
|------|--------|
| FPGA / RTL | Verilog/VHDL, AXI-lite, Vivado or equivalent |
| Embedded | I2C/SPI, ATECC608 or STM32, simple secure firmware |
| Linux bring-up | Device tree or module for mailbox, demo daemon in Rust/C |
| Crypto / proofs | fightfake-toolkit integration, test vectors (can be same person as Pi Level 1 work) |

One experienced FPGA engineer plus one embedded/crypto person is a realistic minimum for Path A.

---

## Demo deliverables

When the prototype works, you should have:

1. Register map and sequence diagram (requirements doc).  
2. A short recording: capture → `(B, σ)` → negative tests pass.  
3. One off-device edit proof on the same clip.

---

## Related docs

- [hardware-requirements-for-capture-binding.md](./hardware-requirements-for-capture-binding.md) — architecture and security rules  
- [level1-pi-demonstrator.md](./level1-pi-demonstrator.md) — software/TEE path on Raspberry Pi  
- [level0-end-to-end.md](./level0-end-to-end.md) — proof bundle format without hardware binding  
- Public summary: [fightfake.ai/hardware](https://fightfake.ai/hardware)
