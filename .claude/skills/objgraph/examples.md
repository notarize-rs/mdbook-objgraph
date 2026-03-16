# objgraph Examples

## Example 1: Simple PKI Chain

A minimal X.509 certificate chain with root CA and leaf certificate.

```obgraph
domain "Verifier" {
  node Clock "System Clock" @anchored {
    current_time    @constrained
  }
}

domain "PKI" {
  node RootCA "Root CA" @anchored {
    subject         @constrained
    issuer          @critical      # Must match subject (self-signed)
    public_key      @constrained
    not_before      @critical
    not_after       @critical
  }

  node Cert "Leaf Certificate" {
    subject
    issuer          @critical
    public_key      @critical
    signature       @critical
    not_before      @critical
    not_after       @critical
  }
}

# Link: RootCA signs Cert
Cert <- RootCA : sign

# Bonds within PKI domain
RootCA::issuer <= RootCA::subject : self_signed
RootCA::not_before <= Clock::current_time : valid_after
RootCA::not_after <= Clock::current_time : valid_before

Cert::issuer <= RootCA::subject
Cert::signature <= RootCA::public_key : verified_by
Cert::public_key <= RootCA::public_key : issued_by
Cert::not_before <= Clock::current_time : valid_after
Cert::not_after <= Clock::current_time : valid_before
```

## Example 2: Multi-Domain with Bridges

Hardware attestation bridged to software inventory.

```obgraph
domain "Hardware Attestation" {
  node HWRoot "HW Root Key" @anchored {
    public_key      @constrained
  }

  node Report "Attestation Report" {
    measurement     @critical
    signature       @critical
  }
}

domain "Software Publisher" {
  node Publisher "Publisher Root" @anchored {
    signing_key     @constrained
  }

  node Release "Software Release" {
    version         @constrained
    expected_hash   @constrained
    signature       @critical
  }
}

domain "Runtime" {
  node App "Running Application" @selected {
    loaded_hash     @critical
  }
}

# Links within domains
Report <- HWRoot : sign
Release <- Publisher : sign
App <- Report : measures

# Bonds
Report::signature <= HWRoot::public_key : verified_by
Release::signature <= Publisher::signing_key : verified_by

# Bridge: Hardware attestation meets software publisher
# The measured hash must match the published expected hash
App::loaded_hash <= Release::expected_hash : matches
```

## Example 3: Federated Trust with Policy

Enterprise policy controlling which software versions are approved.

```obgraph
domain "Enterprise Policy" {
  node Policy "Deployment Policy" @anchored {
    min_version         @constrained
    approved_publishers @constrained
  }
}

domain "CVE Database" {
  node NVD "NIST NVD" @anchored {
    cve_list            @constrained
  }
}

domain "Software Publisher" {
  node Vendor "Software Vendor" @anchored {
    name                @constrained
    signing_key         @constrained
  }

  node Release "Release v2.1.0" {
    version             @critical
    publisher           @critical
    hash                @constrained
    signature           @critical
    cves                @critical
  }
}

# Link
Release <- Vendor : sign

# Bonds
Release::signature <= Vendor::signing_key : verified_by

# Bridges
Release::version <= Policy::min_version : gte
Release::publisher <= Policy::approved_publishers : in
Release::cves <= NVD::cve_list : no_critical
```

## Example 4: Terminal Node Pattern

When the image/artifact being verified is the terminal node (trust flows TO it).

```obgraph
domain "Signing Infrastructure" {
  node CA "Certificate Authority" @anchored {
    public_key      @constrained
  }

  node Signer "Signing Certificate" {
    public_key      @critical
    signature       @critical
  }
}

domain "Artifact Storage" {
  node Signature "Detached Signature" {
    artifact_hash   @constrained   # Covered by signature
    signature_bytes @critical
  }

  # Terminal node - what we're verifying
  node Artifact "Build Artifact" @selected {
    computed_hash   @critical
  }
}

# Links
Signer <- CA : sign
Signature <- Signer : sign
Artifact <- Signature : validates

# Bonds
Signer::signature <= CA::public_key : verified_by
Signature::signature_bytes <= Signer::public_key : verified_by

# Terminal constraint - artifact hash matches signed hash
Artifact::computed_hash <= Signature::artifact_hash : matches
```

## Anti-Patterns to Avoid

### 1. Circular Trust
```obgraph
# WRONG - creates cycle
A <- B : sign
B <- A : sign
```

### 2. Orphan Constraints
```obgraph
# WRONG - Node::prop sources constraint but Node has no anchor
node Orphan {
  prop    @constrained
}
Other::value <= Orphan::prop  # Will be invalid - Orphan not anchored
```

### 3. Redundant Annotations
```obgraph
# WRONG - validator rejects this
node N {
  prop    @constrained
}
N::prop <= Other::value  # Can't constrain an already @constrained property
```

### 4. Trust Flowing From Terminal
```obgraph
# WRONG - Image should receive trust, not source it
node Image @anchored {
  digest    @constrained
}
Signature::image_hash <= Image::digest  # Arrows go wrong direction
```

### 5. Correct Terminal Pattern
```obgraph
# RIGHT - Image receives trust
node Image @selected {
  digest    @critical
}
Image <- Signature : validates
Image::digest <= Signature::image_hash : matches
```

## State Propagation Trace

For the PKI example, here's how state propagates:

1. **Initial state:**
   - Clock: anchored=true (annotation), verified=true (no @critical props)
   - RootCA: anchored=true (annotation), verified=false (5 @critical props unconstrained)
   - Cert: anchored=false, verified=false

2. **First iteration:**
   - Clock's `current_time` is @constrained, can source constraints
   - RootCA::issuer <= RootCA::subject - INVALID (RootCA not verified yet)
   - RootCA::not_before <= Clock::current_time - VALID (Clock is A+V)
   - RootCA::not_after <= Clock::current_time - VALID

   But RootCA::issuer needs RootCA to be verified, which requires issuer to be constrained. This is a pin (self-referential).

3. **Pin resolution:**
   - RootCA::issuer <= RootCA::subject is a pin
   - For pins, we check: is source (@constrained) on same node?
   - RootCA::subject is @constrained → constraint is valid
   - RootCA::issuer becomes constrained

4. **RootCA verified:**
   - All @critical properties now constrained
   - RootCA: verified=true
   - Anchor Cert <- RootCA becomes valid

5. **Cert anchored:**
   - Cert: anchored=true
   - Constraints from RootCA to Cert now valid
   - All Cert @critical properties become constrained
   - Cert: verified=true

6. **Final state:** All nodes anchored+verified, all edges valid (green).
