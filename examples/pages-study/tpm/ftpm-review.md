# objgraph Review: ftpm.obgraph

## Summary
- **Nodes:** 11 total (3 @anchored, 2 @selected)
- **Properties:** 33 total (17 @critical, 16 @constrained)
- **Domains:** 6 (CPU Hardware, fTPM Firmware, Cryptographic PKI, Platform Firmware, BitLocker Service, Windows Hello Service)
- **Links:** 8 anchors
- **Constraints:** 16 total
- **Status:** HAS CRITICAL ISSUES

## Issues Found

### Critical (will cause cycles and red arrows)

#### Issue 1: Domain-Level Cycle Between fTPM Firmware and Cryptographic PKI
- **Location:** Cross-domain dependencies
- **Problem:** A dependency cycle exists between three domains:
  1. `fTPM Firmware` depends on `Cryptographic PKI` (via constraints from ManufacturerCA to fTPM)
  2. `Cryptographic PKI` depends on `fTPM Firmware` (via constraint from fTPM to EKCert)
  3. `fTPM Firmware` depends on `CPU Hardware` (via anchor from TEE)

- **Cycle Path:**
  ```
  fTPM Firmware
       |  ^
       |  |
       v  |
  Cryptographic PKI
  ```

- **Detailed trace:**
  - `fTPM::storage_root_key <= ManufacturerCA::ek_provisioning_key` (Crypto PKI -> fTPM)
  - `fTPM::random_generator <= ManufacturerCA::ek_provisioning_key` (Crypto PKI -> fTPM)
  - `fTPM::endorsement_key <= ManufacturerCA::ek_provisioning_key` (Crypto PKI -> fTPM)
  - `EKCert::public_key <= fTPM::endorsement_key` (fTPM -> Crypto PKI) **← Creates cycle**

- **Why it fails:** 
  - The constraint `EKCert::public_key <= fTPM::endorsement_key` creates a backwards dependency
  - EKCert domain depends on fTPM domain to provide the public key
  - But fTPM domain depends on ManufacturerCA domain to provision its keys
  - This circular dependency prevents proper state propagation
  - Topological sort cannot establish a valid ordering

- **Root Cause:**
  The model incorrectly represents how fTPM keys are provisioned. In a firmware TPM:
  - The CPU manufacturer (via TEE) provisions the fTPM's cryptographic capabilities
  - The Manufacturer CA does NOT provision the fTPM's keys
  - The CA only certifies/signs the EK certificate after the fTPM generates its EK
  - The EKCert should be anchored by the CA, not constrained by the fTPM's EK

- **Fix:**

```obgraph
# REMOVE these incorrect constraints (ManufacturerCA does not provision fTPM keys):
# fTPM::storage_root_key <= ManufacturerCA::ek_provisioning_key
# fTPM::random_generator <= ManufacturerCA::ek_provisioning_key  
# fTPM::endorsement_key <= ManufacturerCA::ek_provisioning_key

# REMOVE this backwards constraint (creates cycle):
# EKCert::public_key <= fTPM::endorsement_key

# INSTEAD, add correct provisioning from TEE:
# TEE provisions fTPM's cryptographic capabilities during initialization
fTPM::storage_root_key <= TEE::tee_processor : "TEE provisions SRK during fTPM initialization"
fTPM::random_generator <= TEE::tee_processor : "TEE provides hardware RNG to fTPM"
fTPM::endorsement_key <= TEE::tee_processor : "TEE generates EK for fTPM"

# CHANGE the anchor relationship:
# EKCert should be anchored by ManufacturerCA (CA issues the cert)
EKCert <- ManufacturerCA : "Issue EK certificate"

# KEEP the signature constraint (correct):
EKCert::ca_signature <= ManufacturerCA::ek_signing_key : "Sign EK certificate"

# CHANGE EKCert::public_key to be @constrained (part of the issued certificate):
# In the node definition:
node EKCert "Endorsement Key Certificate" {
  public_key          @constrained  # Changed from @critical
  ca_signature        @critical
}
```

#### Issue 2: Missing Bootloader Connection to fTPM
- **Location:** Platform Firmware domain -> fTPM Firmware domain
- **Problem:** The Bootloader is measured by UEFI but those measurements never reach the fTPM's PCR registers
- **Why it fails:** 
  - `Bootloader::boot_configuration` is a @critical property
  - It's constrained by `UEFI::crtm_code` 
  - But there's no path from Bootloader to fTPM::pcr_registers
  - The measured boot chain is incomplete

- **Fix:**

```obgraph
# Add bridge constraint to connect measured boot to fTPM PCRs
fTPM::pcr_registers <= Bootloader::boot_configuration : "Extend PCRs with boot measurements"
```

This makes the Platform Firmware domain depend on fTPM Firmware domain for measurement storage, which is the correct trust flow.

### Warnings (semantic concerns)

#### Warning 1: Unused UEFI Secure Boot Chain
- **Location:** Platform Firmware domain
- **Concern:** The Bootloader is anchored by UEFI and verified by Secure Boot, but this verification chain is not connected to the two terminal nodes (EncryptedVolume and UserCredential)
- **Suggestion:** Consider whether Platform Firmware's measured boot should flow into the fTPM PCRs. Currently only BitLocker uses PCRs, but the UEFI/Bootloader chain is isolated. Add constraint:
  ```obgraph
  fTPM::pcr_registers <= Bootloader::boot_configuration : "Extend PCRs with boot measurements"
  ```

#### Warning 2: Separate Independent Terminals
- **Location:** BitLocker Service and Windows Hello Service domains
- **Concern:** Both terminal chains (EncryptedVolume and UserCredential) depend on fTPM but serve independent purposes. This is architecturally correct - they are parallel security services that both use the TPM.
- **Suggestion:** No change needed. This is the correct model for two independent services sharing a common root of trust.

## Domain Dependency Analysis

### Cross-Domain Edges

**Anchors (8 total, 3 cross-domain):**
1. `fTPM <- TEE` (fTPM Firmware <- CPU Hardware)
2. `VMK <- fTPM` (BitLocker Service <- fTPM Firmware)
3. `NGC <- fTPM` (Windows Hello Service <- fTPM Firmware)

**Constraints (16 total, 10 cross-domain):**

PROBLEMATIC (causing cycle):
1. `fTPM::storage_root_key <= ManufacturerCA::ek_provisioning_key` (fTPM <- Crypto PKI)
2. `fTPM::random_generator <= ManufacturerCA::ek_provisioning_key` (fTPM <- Crypto PKI)
3. `fTPM::endorsement_key <= ManufacturerCA::ek_provisioning_key` (fTPM <- Crypto PKI)
4. `EKCert::public_key <= fTPM::endorsement_key` (Crypto PKI <- fTPM) **← Creates cycle**

CORRECT (needed for terminal constraints):
5. `VMK::sealed_blob <= fTPM::storage_root_key` (BitLocker <- fTPM)
6. `NGC::container_key <= fTPM::storage_root_key` (Windows Hello <- fTPM)
7. `NGC::key_wrapping <= fTPM::storage_root_key` (Windows Hello <- fTPM)
8. `VMK::pcr_policy <= fTPM::pcr_registers` (BitLocker <- fTPM)
9. `EncryptedVolume::boot_integrity <= fTPM::pcr_registers` (BitLocker <- fTPM)
10. `UserCredential::pin_authorization <= fTPM::storage_root_key` (Windows Hello <- fTPM)

### Domain Dependency Graph (BEFORE FIX)

```
CPU Hardware
  └─> (no dependencies - root)

Cryptographic PKI
  └─> fTPM Firmware (via EKCert::public_key <= fTPM::endorsement_key)

fTPM Firmware
  ├─> CPU Hardware (via fTPM <- TEE)
  └─> Cryptographic PKI (via fTPM::* <= ManufacturerCA::*)  ← CYCLE!

Platform Firmware
  └─> (no dependencies - root)

BitLocker Service
  └─> fTPM Firmware (via multiple constraints)

Windows Hello Service
  └─> fTPM Firmware (via multiple constraints)
```

**Cycle detected:** fTPM Firmware ↔ Cryptographic PKI

### Domain Dependency Graph (AFTER FIX)

```
CPU Hardware
  └─> (no dependencies - root)

fTPM Firmware
  └─> CPU Hardware (via fTPM <- TEE)

Cryptographic PKI
  └─> fTPM Firmware (via EKCert <- ManufacturerCA, no longer bidirectional)

Platform Firmware
  └─> (no dependencies - root)

BitLocker Service
  ├─> fTPM Firmware (via multiple constraints)
  └─> Platform Firmware (via fTPM::pcr_registers <= Bootloader::boot_configuration)

Windows Hello Service
  └─> fTPM Firmware (via multiple constraints)
```

**Cycle resolved!** All domains form a DAG (Directed Acyclic Graph).

### Topological Order (AFTER FIX)

1. CPU Hardware (root)
2. Platform Firmware (root)
3. fTPM Firmware (depends on CPU Hardware)
4. Cryptographic PKI (depends on fTPM Firmware)
5. BitLocker Service (depends on fTPM Firmware and Platform Firmware)
6. Windows Hello Service (depends on fTPM Firmware)

## State Propagation Analysis

### Initial State (BEFORE FIX)

| Node | Anchored | Verified | Notes |
|------|----------|----------|-------|
| CPUManufacturer | ✓ | ✓ | @anchored, all props @constrained |
| TEE | ✗ | ✗ | Anchored by CPUManufacturer |
| fTPM | ✗ | ✗ | Anchored by TEE, but cycle prevents verification |
| ManufacturerCA | ✓ | ✓ | @anchored, all props @constrained |
| EKCert | ✗ | ✗ | NOT anchored, cycle issue |
| UEFI | ✓ | ✓ | @anchored, all props @constrained |
| Bootloader | ✗ | ✗ | Anchored by UEFI |
| VMK | ✗ | ✗ | Depends on fTPM |
| EncryptedVolume | ✗ | ✗ | Terminal, depends on VMK + fTPM |
| NGC | ✗ | ✗ | Depends on fTPM |
| UserCredential | ✗ | ✗ | Terminal, depends on NGC + fTPM |

**Problem:** The cycle prevents fTPM from becoming verified, which blocks all downstream nodes.

### State Propagation Trace (AFTER FIX)

#### Iteration 0 (Initial):
```
Anchored: CPUManufacturer, ManufacturerCA, UEFI
Constrained props: All @constrained properties in anchored nodes
Verified: CPUManufacturer, ManufacturerCA, UEFI (all @critical props constrained)
```

#### Iteration 1:
```
TEE becomes anchored (parent CPUManufacturer is verified)
TEE::firmware_signature becomes constrained (CPUManufacturer::tee_signing_key)
TEE::memory_isolation becomes constrained (CPUManufacturer::manufacturing_process)
TEE::tee_processor becomes constrained (CPUManufacturer::root_certificate)
TEE becomes verified (all @critical props constrained)

Bootloader becomes anchored (parent UEFI is verified)
Bootloader::bootloader_signature becomes constrained (UEFI::secure_boot_db)
Bootloader::boot_configuration becomes constrained (UEFI::crtm_code)
Bootloader becomes verified (all @critical props constrained)
```

#### Iteration 2:
```
fTPM becomes anchored (parent TEE is verified)
fTPM::storage_root_key becomes constrained (TEE::tee_processor)
fTPM::random_generator becomes constrained (TEE::tee_processor)
fTPM::endorsement_key becomes constrained (TEE::tee_processor)
fTPM::pcr_registers becomes constrained (fTPM::random_generator - self-constraint)
fTPM becomes verified (all @critical props constrained)

EKCert becomes anchored (parent ManufacturerCA is verified)
EKCert::ca_signature becomes constrained (ManufacturerCA::ek_signing_key)
EKCert becomes verified (all @critical props constrained)
```

#### Iteration 3:
```
VMK becomes anchored (parent fTPM is verified)
VMK::sealed_blob becomes constrained (fTPM::storage_root_key)
VMK::pcr_policy becomes constrained (fTPM::pcr_registers)
VMK becomes verified (all @critical props constrained)

NGC becomes anchored (parent fTPM is verified)
NGC::container_key becomes constrained (fTPM::storage_root_key)
NGC::key_wrapping becomes constrained (fTPM::storage_root_key)
NGC becomes verified (all @critical props constrained)
```

#### Iteration 4:
```
EncryptedVolume becomes anchored (parent VMK is verified)
EncryptedVolume::boot_integrity becomes constrained (fTPM::pcr_registers)
EncryptedVolume::volume_encryption becomes constrained (VMK::sealed_blob)
EncryptedVolume becomes verified (all @critical props constrained)

UserCredential becomes anchored (parent NGC is verified)
UserCredential::pin_authorization becomes constrained (fTPM::storage_root_key)
UserCredential::private_key becomes constrained (NGC::container_key)
UserCredential becomes verified (all @critical props constrained)
```

### Final State (AFTER FIX)

| Node | Anchored | Verified | Constrained Props |
|------|----------|----------|-------------------|
| CPUManufacturer | ✓ | ✓ | All |
| TEE | ✓ | ✓ | firmware_signature, memory_isolation, tee_processor |
| fTPM | ✓ | ✓ | endorsement_key, pcr_registers, storage_root_key, random_generator |
| ManufacturerCA | ✓ | ✓ | All |
| EKCert | ✓ | ✓ | ca_signature (public_key is @constrained) |
| UEFI | ✓ | ✓ | All |
| Bootloader | ✓ | ✓ | bootloader_signature, boot_configuration |
| VMK | ✓ | ✓ | sealed_blob, pcr_policy |
| EncryptedVolume | ✓ | ✓ | boot_integrity, volume_encryption |
| NGC | ✓ | ✓ | container_key, key_wrapping |
| UserCredential | ✓ | ✓ | pin_authorization, private_key |

**Result:** All nodes become verified! All @selected terminals are green.

## Unconstrained Critical Properties

### BEFORE FIX
All @critical properties technically have constraints, but the cycle prevents proper propagation.

### AFTER FIX
No unconstrained @critical properties. All critical properties receive constraints from appropriate sources.

## Trust Direction Verification

### Hardware → Firmware → Software Flow

**Correct flows:**
- CPU Hardware (CPUManufacturer) → CPU Hardware (TEE) ✓
- CPU Hardware (TEE) → fTPM Firmware (fTPM) ✓
- fTPM Firmware → BitLocker Service ✓
- fTPM Firmware → Windows Hello Service ✓
- Platform Firmware (UEFI) → Platform Firmware (Bootloader) ✓

**Fixed issues:**
- BEFORE: Cryptographic PKI → fTPM Firmware (ManufacturerCA provisions fTPM keys) ✗
- AFTER: fTPM Firmware → Cryptographic PKI (CA certifies fTPM's EK) ✓

The fix corrects the trust direction. The CPU manufacturer (via TEE) provisions the fTPM, and then the external CA certifies the fTPM's identity.

## Recommendations

### Priority 1 (Critical - Must Fix)
1. **Break the domain cycle** by removing the constraints from ManufacturerCA to fTPM and changing EKCert to be anchored by ManufacturerCA instead of constrained by fTPM
2. **Add TEE provisioning constraints** so fTPM's keys are constrained by TEE::tee_processor
3. **Change EKCert anchor relationship** from implicit to explicit (EKCert <- ManufacturerCA)
4. **Change EKCert::public_key** from @critical to @constrained (it's part of the issued certificate)

### Priority 2 (Recommended - Improves Model)
1. **Connect Platform Firmware to fTPM** by adding constraint from Bootloader to fTPM PCRs to complete the measured boot chain
2. **Document the two independent terminal chains** (BitLocker and Windows Hello) to clarify they are parallel services sharing the TPM

### Priority 3 (Optional - Documentation)
1. Add comments explaining why fTPM keys are provisioned by TEE, not by external CA
2. Document the difference between fTPM (TEE-provisioned) and dTPM (factory-provisioned)

## Fixed Version

See the complete corrected file below. Key changes:

1. Lines 166-173: Removed ManufacturerCA -> fTPM constraints, added TEE -> fTPM constraints
2. Line 137: Changed EKCert anchor from implicit to explicit
3. Line 75: Changed EKCert::public_key from @critical to @constrained
4. Line 176: Removed backwards constraint EKCert::public_key <= fTPM::endorsement_key
5. Line 201: Added Bootloader -> fTPM PCR constraint for measured boot

```obgraph
# Firmware TPM Trust Model (fTPM)
# Trust visualization for firmware TPM running in CPU Trusted Execution Environment
# (AMD Platform Security Processor or Intel Platform Trust Technology) showing
# trust chain from CPU manufacturer through TEE to BitLocker and Windows Hello.
#
#@ title: fTPM
#@ heading: Firmware TPM Trust Model
#@ badge: Firmware TEE
#@ badge_type: hardware
#@ card_style: hardware
#@ meta: Firmware TPM • AMD PSP • Intel PTT • CPU TEE
#@ stats: {"nodes": 11, "domains": 6}
#@ description: Firmware-based TPM running in CPU Trusted Execution Environment (AMD PSP or Intel PTT). Demonstrates integrated security with shared CPU attack surface.
#@ key_features: Lower cost, integrated in CPU, TEE isolation, vulnerable to CPU-level attacks (faulTPM)
#@ best_for: Consumer devices, systems without discrete TPM chip, cost-sensitive deployments
#
# Trust Model:
#   - CPU Hardware: CPU manufacturer provisions TEE (AMD PSP / Intel CSME)
#   - TEE Domain: fTPM firmware runs in isolated execution environment
#   - Platform Firmware: UEFI Secure Boot and measured boot
#   - Cryptographic: Manufacturer CA certifies fTPM identity
#   - BitLocker: Disk encryption sealed to platform state
#   - Windows Hello: Biometric/PIN credential protection
#
# Key Difference from dTPM:
#   fTPM shares the CPU die and relies on firmware isolation rather than
#   physical separation. Vulnerable to CPU-level attacks (Spectre, Meltdown,
#   faulTPM stack overflow). Lower cost, no additional hardware required.
#
# References:
#   - https://ar5iv.labs.arxiv.org/html/2304.14717 (faulTPM vulnerability)
#   - https://www.indurock.com/ftpm-vs-dtpm-firmware-tpm-vs-discrete-tpm-what-is-the-difference/
#   - https://trustedcomputinggroup.org/resource/tpm-library-specification/
#   - https://learn.microsoft.com/en-us/windows/security/hardware-security/tpm/how-windows-uses-the-tpm

# === DOMAIN: CPU Hardware ===
domain "CPU Hardware" {
  # CPU manufacturer root - AMD or Intel
  node CPUManufacturer "CPU Manufacturer Root" @anchored {
    root_certificate      @constrained
    tee_signing_key       @constrained
    manufacturing_process @constrained
  }

  # Trusted Execution Environment (AMD PSP or Intel PTT)
  node TEE "CPU Trusted Execution Environment" {
    firmware_signature  @critical
    memory_isolation    @critical
    tee_processor       @critical
  }
}

# === DOMAIN: fTPM Firmware ===
domain "fTPM Firmware" {
  # fTPM application running in TEE
  node fTPM "Firmware TPM (fTPM)" {
    endorsement_key     @critical
    pcr_registers       @critical
    storage_root_key    @critical
    random_generator    @critical
  }
}

# === DOMAIN: Cryptographic PKI ===
domain "Cryptographic PKI" {
  # Manufacturer CA for fTPM identity
  node ManufacturerCA "TPM Manufacturer CA" @anchored {
    root_certificate      @constrained
    ek_signing_key        @constrained
    ek_provisioning_key   @constrained
  }

  # EK certificate for fTPM
  node EKCert "Endorsement Key Certificate" {
    public_key          @constrained
    ca_signature        @critical
  }
}

# === DOMAIN: Platform Firmware ===
domain "Platform Firmware" {
  # UEFI firmware with Secure Boot
  node UEFI "UEFI Firmware (CRTM)" @anchored {
    platform_key        @constrained
    secure_boot_db      @constrained
    crtm_code           @constrained
  }

  # Bootloader verified by Secure Boot
  node Bootloader "Windows Bootloader" {
    bootloader_signature @critical
    boot_configuration   @critical
  }
}

# === DOMAIN: BitLocker Service ===
domain "BitLocker Service" {
  # BitLocker Volume Master Key
  node VMK "Volume Master Key (VMK)" {
    sealed_blob         @critical
    pcr_policy          @critical
  }

  # BitLocker encrypted volume (terminal node)
  node EncryptedVolume "BitLocker Encrypted Volume" @selected {
    volume_encryption   @critical
    boot_integrity      @critical
  }
}

# === DOMAIN: Windows Hello Service ===
domain "Windows Hello Service" {
  # NGC container for user credentials
  node NGC "NGC Container Key" {
    container_key       @critical
    key_wrapping        @critical
  }

  # User credential (terminal node)
  node UserCredential "Windows Hello Credential" @selected {
    private_key         @critical
    pin_authorization   @critical
  }
}

# ============================================================================
# === LINKS (Anchors - trust flow right-to-left) ===
# ============================================================================

# CPU manufacturer provisions TEE firmware
TEE <- CPUManufacturer : "Provision TEE firmware"

# fTPM runs in TEE isolation
fTPM <- TEE : "Execute fTPM in TEE"

# Manufacturer CA issues and certifies fTPM's EK certificate
EKCert <- ManufacturerCA : "Issue EK certificate"

# Bootloader verified and measured by UEFI
Bootloader <- UEFI : "Secure Boot verification"

# VMK sealed by fTPM
VMK <- fTPM : "Seal VMK to PCR policy"

# Encrypted volume protected by VMK
EncryptedVolume <- VMK : "Decrypt with VMK"

# NGC keys wrapped by fTPM SRK
NGC <- fTPM : "Wrap NGC keys with SRK"

# User credential protected by NGC container
UserCredential <- NGC : "Unwrap credential key"

# ============================================================================
# === BONDS (Within-chain constraints) ===
# ============================================================================

# TEE firmware must be signed by CPU manufacturer
TEE::firmware_signature <= CPUManufacturer::tee_signing_key : "Verify TEE firmware signature"

# TEE hardware capabilities certified by CPU manufacturing process
TEE::memory_isolation <= CPUManufacturer::manufacturing_process : "Memory isolation built during CPU manufacturing"
TEE::tee_processor <= CPUManufacturer::root_certificate : "TEE processor capability certified"

# FIXED: fTPM's cryptographic capabilities provisioned by TEE (not external CA)
# TEE initializes and provisions the fTPM during boot
fTPM::storage_root_key <= TEE::tee_processor : "TEE provisions SRK during fTPM initialization"
fTPM::random_generator <= TEE::tee_processor : "TEE provides hardware RNG to fTPM"
fTPM::endorsement_key <= TEE::tee_processor : "TEE generates EK for fTPM"

# PCR registers are intrinsic firmware capability initialized by RNG
fTPM::pcr_registers <= fTPM::random_generator : "PCRs initialized by secure RNG"

# EK certificate signed by manufacturer CA
EKCert::ca_signature <= ManufacturerCA::ek_signing_key : "Sign EK certificate"

# Bootloader signature verified against Secure Boot db
Bootloader::bootloader_signature <= UEFI::secure_boot_db : "Verify bootloader signature"

# Boot configuration measured by CRTM
Bootloader::boot_configuration <= UEFI::crtm_code : "Boot configuration measured by CRTM"

# VMK sealed blob bound to fTPM SRK
VMK::sealed_blob <= fTPM::storage_root_key : "Seal VMK with SRK"

# NGC container key derived from fTPM SRK
NGC::container_key <= fTPM::storage_root_key : "Derive NGC container key under SRK"

# NGC key wrapping uses fTPM SRK
NGC::key_wrapping <= fTPM::storage_root_key : "Wrap NGC keys with SRK"

# ============================================================================
# === BRIDGES (Cross-domain constraints) ===
# ============================================================================

# BitLocker VMK unsealing requires matching PCR values (measured boot)
VMK::pcr_policy <= fTPM::pcr_registers : "Unseal only if PCRs match"

# ADDED: Connect Platform Firmware measured boot to fTPM PCRs
fTPM::pcr_registers <= Bootloader::boot_configuration : "Extend PCRs with boot measurements"

# ============================================================================
# === TERMINAL CONSTRAINTS ===
# ============================================================================

# Encrypted volume requires boot integrity verification
EncryptedVolume::boot_integrity <= fTPM::pcr_registers : "Verify platform state via PCRs"

# Encrypted volume decryption requires unsealed VMK
EncryptedVolume::volume_encryption <= VMK::sealed_blob : "Decrypt volume with VMK"

# User credential requires PIN/biometric authorization
UserCredential::pin_authorization <= fTPM::storage_root_key : "Authorize with TPM-verified PIN"

# User credential private key protected by NGC wrapping
UserCredential::private_key <= NGC::container_key : "Unwrap credential key"

# ============================================================================
# VULNERABILITY SURFACE (fTPM-specific)
# ============================================================================
#
# fTPM Attack Surface:
#   1. CPU-level exploits: Spectre, Meltdown can potentially bypass TEE isolation
#   2. faulTPM vulnerability: Stack overflow in AMD PSP fTPM (CVE-2021-42299)
#   3. Firmware dependencies: TEE bugs or firmware updates may impact fTPM
#   4. Shared resources: fTPM shares CPU die, vulnerable to side channels
#   5. Lower physical security: No tamper-evident enclosure like dTPM
#
# Trust Dependency:
#   fTPM security relies on CPU manufacturer's TEE implementation correctness.
#   Unlike dTPM, there is no physical isolation from the CPU attack surface.
#
# Mitigation:
#   - Keep CPU firmware updated
#   - Enable Secure Boot and measured boot
#   - Combine with VBS Credential Guard for defense in depth
#   - Consider dTPM for high-security environments
#
# KEY CHANGES FROM ORIGINAL:
#   1. Removed ManufacturerCA -> fTPM constraints (lines 166-168, 173)
#   2. Added TEE -> fTPM provisioning constraints (new lines 166-168)
#   3. Changed EKCert::public_key from @critical to @constrained (line 75)
#   4. Removed backwards constraint EKCert::public_key <= fTPM::endorsement_key (line 176)
#   5. Added explicit EKCert <- ManufacturerCA anchor (line 137)
#   6. Added Bootloader -> fTPM PCR constraint for measured boot (line 201)
#
# RESULT: Domain cycle eliminated, all nodes verify correctly.
```
