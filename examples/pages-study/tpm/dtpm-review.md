# objgraph Review: dtpm.obgraph

## Summary
- **Nodes:** 10 total (3 @anchored, 2 @selected)
- **Properties:** 42 total (21 @critical, 12 @constrained)
- **Links:** 7 anchor edges
- **Constraints:** 17 total (14 bonds, 3 bridges)
- **Status:** HAS ISSUES

## Issues Found

### Critical (will cause red arrows)

#### Issue 1: Domain-Level Cycle via Cryptographic PKI Domain
- **Location:** Cryptographic PKI domain with ManufacturerCA node
- **Problem:** The ManufacturerCA node is marked @anchored independently, creating a second root that should be derived from the Hardware Security domain instead
- **Why it fails:** 
  - ManufacturerCA is @anchored, making it an independent root
  - EKCert (child of ManufacturerCA) receives constraint from dTPM::endorsement_key (line 156)
  - This creates a domain-level cycle: Hardware Security -> Cryptographic PKI (via constraint) -> Cryptographic PKI (via anchor)
  - The independent @anchored status breaks the hierarchical trust flow from TPMManufacturer
- **Fix:**
```obgraph
# Remove @anchored from ManufacturerCA and derive it from TPMManufacturer
node ManufacturerCA "TPM Manufacturer CA" {
    root_certificate    @critical
    ek_signing_key      @critical
}

# Add anchor edge to establish trust flow
EKCert <- ManufacturerCA : "Issue EK certificate"
ManufacturerCA <- TPMManufacturer : "Manufacturer CA derives authority from TPM manufacturing root"

# Add constraints to verify ManufacturerCA's properties
ManufacturerCA::root_certificate <= TPMManufacturer::root_certificate : "CA cert derives from manufacturer root"
ManufacturerCA::ek_signing_key <= TPMManufacturer::ek_provisioning_key : "CA signing key authorized by provisioning key"
```

#### Issue 2: UEFI Independently Anchored (Creates Parallel Root)
- **Location:** Platform Firmware domain, UEFI node (line 72)
- **Problem:** UEFI is marked @anchored, creating a third independent root of trust separate from the TPM hardware chain
- **Why it fails:**
  - In a dTPM model, the UEFI firmware should either be:
    1. Measured and verified by the TPM (making UEFI depend on TPM), OR
    2. Independent parallel root (if representing separate Platform Root of Trust)
  - Currently UEFI has no incoming constraints from Hardware Security domain
  - Bootloader depends on UEFI, but UEFI has no dependency on dTPM
  - This creates a disconnected trust branch that doesn't integrate with the TPM root
- **Fix (Option A - UEFI measured by TPM):**
```obgraph
# Remove @anchored from UEFI and make it depend on dTPM verification
node UEFI "UEFI Firmware (CRTM)" {
    platform_key        @critical
    secure_boot_db      @critical
    crtm_code           @critical
}

# Add anchor from dTPM (TPM measures and verifies UEFI)
UEFI <- dTPM : "TPM measures UEFI firmware integrity"

# Add constraints showing TPM verification
UEFI::platform_key <= dTPM::endorsement_key : "Platform key verified by TPM"
UEFI::crtm_code <= dTPM::pcr_registers : "CRTM measured into PCRs"
```
**OR**

**Fix (Option B - Keep UEFI independent, add bridge to connect domains):**
```obgraph
# Keep @anchored on UEFI (representing independent Platform Root of Trust)
# But add bridge constraint to show the trust relationship

# Add bridge showing dTPM is bound to platform
dTPM::pcr_registers <= UEFI::crtm_code : "TPM PCRs record platform measurements from CRTM"
```

#### Issue 3: Missing Constraint on EKCert::public_key
- **Location:** EKCert node, public_key property (line 64)
- **Problem:** EKCert::public_key is @critical but the constraint on line 156 sources from dTPM::endorsement_key, which is never constrained
- **Why it fails:**
  - dTPM::endorsement_key is @critical but has only one incoming constraint (line 153) from TPMManufacturer::ek_provisioning_key
  - However, the state propagation requires dTPM to be verified before it can source constraints
  - dTPM has 5 @critical properties, but only 3 become constrained (endorsement_key, pcr_registers, storage_root_key, random_generator, tamper_resistance)
  - dTPM never becomes verified because not all @critical properties are constrained
  - Therefore dTPM cannot source the constraint on line 156 to EKCert::public_key
- **Additional Analysis:**
  - dTPM::endorsement_key receives constraint from TPMManufacturer::ek_provisioning_key (line 153) ✓
  - dTPM::storage_root_key receives constraint from TPMManufacturer::ek_provisioning_key (line 145) ✓
  - dTPM::random_generator receives constraint from TPMManufacturer::manufacturing_process (line 146) ✓
  - dTPM::tamper_resistance receives constraint from TPMManufacturer::manufacturing_process (line 147) ✓
  - dTPM::pcr_registers receives constraint from dTPM::tamper_resistance (line 150) ✓
  - **All 5 @critical properties ARE constrained, so dTPM SHOULD become verified**
  
Let me recalculate: If dTPM becomes verified and anchored (via line 120), then it CAN source constraints. The issue might not exist after all. Let me re-verify the propagation.

Actually, reviewing again:
- TPMManufacturer is @anchored with all properties @constrained (lines 39-41) → verified immediately
- dTPM is anchored by TPMManufacturer (line 120)
- dTPM has all 5 @critical properties constrained (lines 145-147, 150, 153)
- Therefore dTPM becomes verified
- Therefore EKCert::public_key WILL receive its constraint from dTPM::endorsement_key

**This is NOT an issue - removing this item.**

## Warnings (semantic concerns)

### Warning 1: Independent Trust Roots Not Properly Integrated
- **Location:** Three @anchored roots (TPMManufacturer, ManufacturerCA, UEFI)
- **Concern:** The model has three independent roots of trust, but the relationships between them are not explicit
- **Suggestion:** 
  - Decide if ManufacturerCA should be an independent root or derive from TPMManufacturer
  - Decide if UEFI should be an independent root or derive from dTPM
  - Add bridge constraints to explicitly connect the domains if they should be independent but related

### Warning 2: Missing Bootloader to VMK Connection
- **Location:** VMK node receives no constraints from Bootloader
- **Concern:** The boot integrity chain (UEFI -> Bootloader) doesn't flow into the BitLocker chain (VMK -> EncryptedVolume)
- **Suggestion:** Add constraint showing VMK policy depends on boot configuration:
```obgraph
VMK::pcr_policy <= Bootloader::boot_configuration : "VMK policy includes bootloader measurements"
```

### Warning 3: No Connection Between UEFI and dTPM Domains
- **Location:** Platform Firmware domain and Hardware Security domain
- **Concern:** UEFI measures boot components but there's no explicit constraint showing that these measurements flow into dTPM PCRs
- **Suggestion:** Add bridge constraint:
```obgraph
dTPM::pcr_registers <= Bootloader::boot_configuration : "Bootloader measured into TPM PCRs"
```

## State Propagation Trace

### Initial State
| Node | Anchored | Verified | Notes |
|------|----------|----------|-------|
| TPMManufacturer | ✓ | ✓ | @anchored root with all props @constrained |
| dTPM | ✗ | ✗ | Awaits anchor from TPMManufacturer |
| ManufacturerCA | ✓ | ✓ | @anchored root with all props @constrained |
| EKCert | ✗ | ✗ | Awaits anchor from ManufacturerCA |
| UEFI | ✓ | ✓ | @anchored root with all props @constrained |
| Bootloader | ✗ | ✗ | Awaits anchor from UEFI |
| VMK | ✗ | ✗ | Awaits anchor from dTPM |
| EncryptedVolume | ✗ | ✗ | Awaits anchor from VMK |
| NGC | ✗ | ✗ | Awaits anchor from dTPM |
| UserCredential | ✗ | ✗ | Awaits anchor from NGC |

### After Iteration 1
| Node | Anchored | Verified | Constrained Props |
|------|----------|----------|-------------------|
| TPMManufacturer | ✓ | ✓ | all (already @constrained) |
| dTPM | ✓ | ✓ | endorsement_key, storage_root_key, random_generator, tamper_resistance, pcr_registers (pin) |
| ManufacturerCA | ✓ | ✓ | all (already @constrained) |
| EKCert | ✓ | ✗ | public_key (from dTPM) |
| UEFI | ✓ | ✓ | all (already @constrained) |
| Bootloader | ✓ | ✓ | bootloader_signature, boot_configuration |
| VMK | ✓ | ✗ | sealed_blob (from dTPM), pcr_policy (from dTPM) |
| EncryptedVolume | ✗ | ✗ | boot_integrity (from dTPM), volume_encryption (from VMK) |
| NGC | ✓ | ✓ | container_key, key_wrapping (both from dTPM) |
| UserCredential | ✗ | ✗ | pin_authorization (from dTPM), private_key (from NGC) |

### After Iteration 2
| Node | Anchored | Verified | Constrained Props |
|------|----------|----------|-------------------|
| EKCert | ✓ | ✓ | public_key, ca_signature (from ManufacturerCA) |
| VMK | ✓ | ✓ | sealed_blob, pcr_policy |
| EncryptedVolume | ✓ | ✓ | boot_integrity, volume_encryption |
| UserCredential | ✓ | ✓ | pin_authorization, private_key |

### Final State
| Node | Anchored | Verified | Notes |
|------|----------|----------|-------|
| TPMManufacturer | ✓ | ✓ | Root 1: Hardware manufacturing |
| dTPM | ✓ | ✓ | Hardware security chip |
| ManufacturerCA | ✓ | ✓ | Root 2: PKI authority (CYCLE ISSUE) |
| EKCert | ✓ | ✓ | PKI certificate |
| UEFI | ✓ | ✓ | Root 3: Platform firmware (disconnected) |
| Bootloader | ✓ | ✓ | Platform boot chain |
| VMK | ✓ | ✓ | BitLocker encryption key |
| EncryptedVolume | ✓ | ✓ | Terminal: encrypted storage |
| NGC | ✓ | ✓ | Windows Hello key container |
| UserCredential | ✓ | ✓ | Terminal: user authentication |

## Domain-Level Cycle Analysis

### Detected Cycle: Hardware Security ↔ Cryptographic PKI

**Cycle Path:**
1. **ManufacturerCA** (@anchored in Cryptographic PKI) → independent root
2. **ManufacturerCA** sources constraints to **EKCert** (same domain)
3. **EKCert** receives constraint from **dTPM::endorsement_key** (Hardware Security domain)
4. This creates bidirectional dependency: Cryptographic PKI ← Hardware Security

**Why This Is a Cycle:**
- ManufacturerCA being @anchored makes it an independent root
- EKCert depends on dTPM (Hardware Security) via constraint on line 156
- This creates: Cryptographic PKI (independent) ← Hardware Security (via constraint)
- But semantically, the Manufacturer CA should derive its authority FROM the TPM Manufacturer
- The correct flow should be: TPMManufacturer → ManufacturerCA → EKCert → dTPM (references back)

**Resolution:**
Remove @anchored from ManufacturerCA and make it derive from TPMManufacturer:
```obgraph
node ManufacturerCA "TPM Manufacturer CA" {
    root_certificate    @critical
    ek_signing_key      @critical
}

# Establish trust flow from manufacturer to CA
ManufacturerCA <- TPMManufacturer : "Manufacturer CA derives from manufacturing root"

# Constrain CA properties
ManufacturerCA::root_certificate <= TPMManufacturer::root_certificate : "CA cert from manufacturer root"
ManufacturerCA::ek_signing_key <= TPMManufacturer::ek_provisioning_key : "CA signing key from provisioning key"
```

### Independent Root Analysis: UEFI

**Issue:** UEFI is @anchored independently, creating a parallel trust chain disconnected from the TPM

**Current Structure:**
- UEFI (@anchored) → Bootloader
- TPMManufacturer (@anchored) → dTPM → VMK → EncryptedVolume
- TPMManufacturer → dTPM → NGC → UserCredential

**No connection between UEFI and TPM chains except:**
- VMK::pcr_policy <= dTPM::pcr_registers (line 181)
- EncryptedVolume::boot_integrity <= dTPM::pcr_registers (line 188)

**The Problem:**
The PCR registers reference implies that boot measurements flow from UEFI/Bootloader into TPM, but there's no explicit constraint showing this relationship.

**Resolution Options:**

**Option A: Make UEFI Depend on TPM (TPM measures UEFI)**
```obgraph
node UEFI "UEFI Firmware (CRTM)" {
    platform_key        @critical
    secure_boot_db      @critical
    crtm_code           @critical
}

UEFI <- dTPM : "TPM measures UEFI firmware"
UEFI::crtm_code <= dTPM::pcr_registers : "CRTM measured into PCRs"
```

**Option B: Keep Independent but Add Bridges**
```obgraph
# Keep UEFI @anchored (independent Platform Root of Trust)
# Add bridges to show the interaction

dTPM::pcr_registers <= Bootloader::boot_configuration : "Bootloader measured into TPM PCRs"
VMK::pcr_policy <= Bootloader::boot_configuration : "VMK sealed to bootloader state"
```

## Unconstrained Critical Properties

After full propagation, all @critical properties become constrained. However, the propagation is only valid if the cycle issues are resolved.

| Node | Property | Status | Source Constraint |
|------|----------|--------|-------------------|
| dTPM | endorsement_key | ✓ | TPMManufacturer::ek_provisioning_key |
| dTPM | pcr_registers | ✓ | dTPM::tamper_resistance (pin) |
| dTPM | storage_root_key | ✓ | TPMManufacturer::ek_provisioning_key |
| dTPM | random_generator | ✓ | TPMManufacturer::manufacturing_process |
| dTPM | tamper_resistance | ✓ | TPMManufacturer::manufacturing_process |
| EKCert | public_key | ✓ | dTPM::endorsement_key |
| EKCert | ca_signature | ✓ | ManufacturerCA::ek_signing_key |
| Bootloader | bootloader_signature | ✓ | UEFI::secure_boot_db |
| Bootloader | boot_configuration | ✓ | UEFI::crtm_code |
| VMK | sealed_blob | ✓ | dTPM::storage_root_key |
| VMK | pcr_policy | ✓ | dTPM::pcr_registers |
| EncryptedVolume | boot_integrity | ✓ | dTPM::pcr_registers |
| EncryptedVolume | volume_encryption | ✓ | VMK::sealed_blob |
| NGC | container_key | ✓ | dTPM::storage_root_key |
| NGC | key_wrapping | ✓ | dTPM::storage_root_key |
| UserCredential | private_key | ✓ | NGC::container_key |
| UserCredential | pin_authorization | ✓ | dTPM::storage_root_key |

## Recommendations

### Priority 1: Fix Domain-Level Cycle (CRITICAL)
Remove @anchored from ManufacturerCA and make it derive from TPMManufacturer. Change its @constrained properties to @critical and add explicit constraints.

### Priority 2: Integrate UEFI and TPM Domains
Choose one approach:
- **Preferred:** Add bridge constraints to show boot measurements flow into TPM PCRs
- **Alternative:** Make UEFI derive from dTPM if TPM measures UEFI integrity

### Priority 3: Add Missing Cross-Domain Constraints
Add constraints showing:
- Bootloader measurements flow into dTPM PCRs
- VMK policy depends on bootloader configuration

### Priority 4: Semantic Clarity
Add comments explaining why there are multiple @anchored roots (if intentional) or consolidate into a single trust hierarchy.

## Fixed Version

```obgraph
# Discrete TPM Trust Model (dTPM)
# Trust visualization for discrete TPM hardware chip showing complete physical
# isolation from CPU and main system. Trust chain from TPM manufacturer through
# dedicated security chip to BitLocker and Windows Hello.
#
#@ title: dTPM
#@ heading: Discrete TPM Trust Model
#@ badge: Hardware Root
#@ badge_type: hardware
#@ card_style: hardware
#@ meta: Discrete TPM • Hardware Security • FIPS 140
#@ stats: {"nodes": 10, "domains": 5}
#@ description: Physical TPM chip with complete hardware isolation from CPU. Demonstrates strongest hardware security with dedicated security processor and tamper-resistant enclosure.
#@ key_features: Hardware isolation, FIPS 140 Level 3 capable, immune to CPU attacks, dedicated security processor
#@ best_for: Enterprise systems, high-security requirements, compliance (FIPS/Common Criteria), long-lifecycle systems
#
# Trust Model:
#   - Hardware Security: Dedicated TPM chip physically isolated from CPU
#   - Platform Firmware: UEFI Secure Boot and measured boot
#   - Cryptographic: Manufacturer CA certifies TPM identity
#   - BitLocker: Disk encryption sealed to platform state
#   - Windows Hello: Biometric/PIN credential protection
#
# Key Difference from fTPM:
#   dTPM is a physically separate chip with complete electrical isolation from
#   the CPU, memory, and OS. Immune to CPU-level attacks. FIPS 140 Level 3
#   certification capable. Higher cost ($1-5+ per unit) but maximum assurance.
#
# References:
#   - https://www.bvm.co.uk/faq/ftpm-vs-dtpm-understanding-trusted-platform-modules-in-industrial-computing/
#   - https://premioinc.com/blogs/blog/differences-between-ftpm-vs-dtpm
#   - https://trustedcomputinggroup.org/resource/tpm-library-specification/
#   - https://learn.microsoft.com/en-us/windows/security/hardware-security/tpm/how-windows-uses-the-tpm

# === DOMAIN: Hardware Security ===
domain "Hardware Security" {
  # TPM manufacturer root - Single root of trust for entire model
  node TPMManufacturer "TPM Manufacturer Root" @anchored {
    root_certificate        @constrained
    ek_provisioning_key     @constrained
    manufacturing_process   @constrained
  }

  # Discrete TPM hardware chip
  node dTPM "Discrete TPM Chip (dTPM)" {
    endorsement_key         @critical
    pcr_registers           @critical
    storage_root_key        @critical
    random_generator        @critical
    tamper_resistance       @critical
  }
}

# === DOMAIN: Cryptographic PKI ===
domain "Cryptographic PKI" {
  # Manufacturer CA for dTPM identity (derives from TPMManufacturer)
  node ManufacturerCA "TPM Manufacturer CA" {
    root_certificate    @critical
    ek_signing_key      @critical
  }

  # EK certificate for dTPM
  node EKCert "Endorsement Key Certificate" {
    public_key          @critical
    ca_signature        @critical
  }
}

# === DOMAIN: Platform Firmware ===
domain "Platform Firmware" {
  # UEFI firmware with Secure Boot (independent platform root)
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

# TPM manufacturer provisions discrete TPM chip during manufacturing
dTPM <- TPMManufacturer : "Manufacture and provision TPM chip"

# Manufacturer CA derives authority from TPM manufacturing root
ManufacturerCA <- TPMManufacturer : "CA derives from manufacturing root"

# Manufacturer CA certifies dTPM's EK
EKCert <- ManufacturerCA : "Issue EK certificate"

# Bootloader verified and measured by UEFI
Bootloader <- UEFI : "Secure Boot verification"

# VMK sealed by dTPM
VMK <- dTPM : "Seal VMK to PCR policy"

# Encrypted volume protected by VMK
EncryptedVolume <- VMK : "Decrypt with VMK"

# NGC keys wrapped by dTPM SRK
NGC <- dTPM : "Wrap NGC keys with SRK"

# User credential protected by NGC container
UserCredential <- NGC : "Unwrap credential key"

# ============================================================================
# === BONDS (Within-chain constraints) ===
# ============================================================================

# dTPM's intrinsic hardware capabilities derived from manufacturing
dTPM::storage_root_key <= TPMManufacturer::ek_provisioning_key : "SRK provisioned during manufacturing"
dTPM::random_generator <= TPMManufacturer::manufacturing_process : "Hardware RNG certified"
dTPM::tamper_resistance <= TPMManufacturer::manufacturing_process : "Tamper resistance built during manufacturing"

# PCR registers are intrinsic hardware capability protected by tamper resistance
dTPM::pcr_registers <= dTPM::tamper_resistance : "PCRs protected by tamper-resistant hardware"

# dTPM's EK provisioned during manufacturing
dTPM::endorsement_key <= TPMManufacturer::ek_provisioning_key : "Burn EK during manufacturing"

# Manufacturer CA derives from TPM manufacturer root
ManufacturerCA::root_certificate <= TPMManufacturer::root_certificate : "CA cert derives from manufacturer root"
ManufacturerCA::ek_signing_key <= TPMManufacturer::ek_provisioning_key : "CA signing key authorized by provisioning key"

# Certificate contains dTPM's EK public component
EKCert::public_key <= dTPM::endorsement_key : "Certificate contains dTPM's EK public component"

# EK certificate signed by manufacturer CA
EKCert::ca_signature <= ManufacturerCA::ek_signing_key : "Sign EK certificate"

# Bootloader signature verified against Secure Boot db
Bootloader::bootloader_signature <= UEFI::secure_boot_db : "Verify bootloader signature"

# Boot configuration measured by CRTM
Bootloader::boot_configuration <= UEFI::crtm_code : "Boot configuration measured by CRTM"

# VMK sealed blob bound to dTPM SRK
VMK::sealed_blob <= dTPM::storage_root_key : "Seal VMK with SRK"

# NGC container key derived from dTPM SRK
NGC::container_key <= dTPM::storage_root_key : "Derive NGC container key under SRK"

# NGC key wrapping uses dTPM SRK
NGC::key_wrapping <= dTPM::storage_root_key : "Wrap NGC keys with SRK"

# ============================================================================
# === BRIDGES (Cross-domain constraints) ===
# ============================================================================

# Platform measurements flow into TPM PCR registers
dTPM::pcr_registers <= Bootloader::boot_configuration : "Bootloader measured into TPM PCRs"

# BitLocker VMK unsealing requires matching PCR values (measured boot)
VMK::pcr_policy <= dTPM::pcr_registers : "Unseal only if PCRs match"

# VMK policy also depends on bootloader configuration
VMK::pcr_policy <= Bootloader::boot_configuration : "VMK sealed to bootloader state"

# ============================================================================
# === TERMINAL CONSTRAINTS ===
# ============================================================================

# Encrypted volume requires boot integrity verification
EncryptedVolume::boot_integrity <= dTPM::pcr_registers : "Verify platform state via PCRs"

# Encrypted volume decryption requires unsealed VMK
EncryptedVolume::volume_encryption <= VMK::sealed_blob : "Decrypt volume with VMK"

# User credential requires PIN/biometric authorization
UserCredential::pin_authorization <= dTPM::storage_root_key : "Authorize with TPM-verified PIN"

# User credential private key protected by NGC wrapping
UserCredential::private_key <= NGC::container_key : "Unwrap credential key"

# ============================================================================
# HARDWARE ISOLATION BENEFITS (dTPM-specific)
# ============================================================================
#
# dTPM Security Advantages:
#   1. Physical isolation: Dedicated chip separate from CPU and main memory
#   2. Immune to CPU attacks: Not vulnerable to Spectre, Meltdown, or faulTPM
#   3. Tamper-resistant: Physical enclosure with tamper-evident/response features
#   4. FIPS 140 Level 3: Can achieve high certification levels for compliance
#   5. Independent operation: Unaffected by CPU firmware bugs or load
#   6. Dedicated resources: Own processor, NV storage, RNG
#
# Trust Basis:
#   dTPM trust is rooted in physical hardware manufacturing and tamper-resistant
#   packaging. Keys are burned during manufacturing in secure facility and never
#   leave the chip. Physical attacks require chip de-capping or sophisticated
#   laboratory equipment.
#
# Use Cases:
#   - Enterprise servers with compliance requirements (FIPS, Common Criteria)
#   - Industrial systems in physically accessible environments
#   - High-value targets requiring maximum assurance
#   - Long-lifecycle systems where firmware stability is critical
#
# Trade-offs:
#   - Higher cost: $1-5+ BOM cost per unit
#   - PCB space: Requires dedicated chip and supporting components
#   - Potential bus attacks: LPC/SPI bus can be probed (mitigated by platform security)
#   - Cannot be added post-manufacturing (except via add-on modules)
#
# Trust Model Notes:
#   - Single hardware root: TPMManufacturer
#   - Independent platform root: UEFI (for Secure Boot chain)
#   - Manufacturer CA derives from TPMManufacturer (no independent PKI root)
#   - Bridge constraints explicitly connect platform measurements to TPM
```

## Summary of Changes

### Critical Fixes:
1. **Removed @anchored from ManufacturerCA** - Changed to derive from TPMManufacturer
2. **Changed ManufacturerCA properties** - Changed from @constrained to @critical
3. **Added anchor edge** - ManufacturerCA <- TPMManufacturer
4. **Added constraints** - ManufacturerCA::root_certificate and ek_signing_key now constrained from TPMManufacturer

### Semantic Improvements:
5. **Added bridge constraint** - dTPM::pcr_registers <= Bootloader::boot_configuration (shows boot measurements flow into TPM)
6. **Added bridge constraint** - VMK::pcr_policy <= Bootloader::boot_configuration (shows VMK sealed to bootloader state)
7. **Updated comments** - Clarified trust model structure

### Trust Model After Fixes:
- **Primary Root:** TPMManufacturer (hardware manufacturing)
- **Secondary Root:** UEFI (platform firmware - independent but bridged to TPM via measurements)
- **No More Cycles:** ManufacturerCA now properly derives from TPMManufacturer
- **Proper Integration:** Platform measurements explicitly flow into TPM PCRs via bridge constraints

This eliminates the domain-level cycle while maintaining the semantic correctness of having an independent platform root (UEFI) that interacts with the TPM hardware root.
