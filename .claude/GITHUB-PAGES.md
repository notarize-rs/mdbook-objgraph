# GitHub Pages Workflow for objgraph Files

This guide explains how to create objgraph files that will be properly rendered on the GitHub Pages site.

## Quick Reference

### 1. File Location
Place your .obgraph files in: `examples/pages-study/<section>/`

Sections:
- `tpm/` - Trusted Platform Module visualizations
- `snp/` - AMD SEV-SNP attestation visualizations
- `container-signing/` - Container signing trust chains
- `website/` - Web PKI and TLS visualizations
- Create new sections as needed

### 2. File Naming
✓ **Good**: `tpm-bitlocker.obgraph`, `cosign-keyless-signing.obgraph`, `amd-sev-snp-attestation.obgraph`

✗ **Bad**: `notes.obgraph`, `tpm-notes.obgraph`, `trust-model.obgraph`

### 3. Required Metadata
Add these special comments at the top of your .obgraph file:

```obgraph
#@ title: Short Title
#@ heading: Full Descriptive Heading
#@ description: Brief description shown on index page card (1-2 sentences)
```

### 4. Optional Metadata (Recommended)
```obgraph
#@ badge: Badge Text
#@ badge_type: hardware|enterprise|slsa|recommended|deprecated
#@ card_style: hardware|recommended|deprecated
#@ meta: Technology • Keywords • Separated by bullets
#@ stats: {"nodes": 19, "domains": 7}
#@ key_features: Feature 1, Feature 2, Feature 3
#@ best_for: Primary use cases and applications
```

## Complete Example

```obgraph
# TPM and fTPM Trust Model for BitLocker and Windows Hello
# Trust visualization for discrete TPMs (dTPM) and firmware TPMs (fTPM)
# showing how hardware-rooted cryptographic trust enables BitLocker disk
# encryption and Windows Hello credential protection.
#
#@ title: TPM & fTPM
#@ heading: TPM and fTPM Trust Model for BitLocker and Windows Hello
#@ badge: Hardware Root
#@ badge_type: hardware
#@ card_style: hardware
#@ meta: Trusted Platform Module • BitLocker • Windows Hello
#@ stats: {"nodes": 19, "domains": 7}
#@ description: Hardware-rooted cryptographic trust for discrete TPMs (dTPM) and firmware TPMs (fTPM). Shows dual implementation paths from manufacturer roots through measured boot to BitLocker disk encryption and Windows Hello credential protection.
#@ key_features: Hardware identity (EK certificates), measured boot (PCR sealing), key hierarchy (SRK/AIK), dual implementation paths (dTPM/fTPM), platform integrity binding.
#@ best_for: Windows disk encryption (BitLocker), biometric authentication (Windows Hello), platform attestation, confidential computing
#
# Trust Model:
#   - Manufacturing Domain: TPM/CPU manufacturers establish hardware identity
#   - Platform Boot Domain: Measured boot creates cryptographic audit trail
#   - TPM Key Hierarchy: EK -> SRK -> Application keys (BitLocker, Hello)
#
# References:
#   - TCG TPM 2.0 Library Specification
#   - Microsoft TPM Technology Overview

domain "dTPM Manufacturing" {
  node MfgRootCA "TPM Manufacturer Root CA" @anchored {
    root_ca_cert        @constrained
    signing_key         @constrained
  }

  # ... rest of your obgraph content
}
```

## Badge Types Reference

| Badge Type | Visual | Use For |
|------------|--------|---------|
| `hardware` | Red | TPM, SEV-SNP, hardware security modules, physical roots of trust |
| `enterprise` | Blue | Production systems, enterprise solutions, mature technologies |
| `slsa` | Purple | SLSA framework, supply chain security, provenance tracking |
| `recommended` | Green | Current best practices, recommended approaches, modern standards |
| `deprecated` | Red | Legacy systems, deprecated approaches, historical reference |

## Build Process

The GitHub Actions workflow (`.github/workflows/deploy-pages.yml`) automatically:

1. **Triggers** on push to main when files change in `examples/`
2. **Builds** the obgraph-render tool with Cargo
3. **Scans** `examples/pages-study/` for .obgraph files
4. **Extracts** metadata from `#@` comments using `scripts/build-pages.py`
5. **Renders** each .obgraph file to HTML in `docs/<section>/`
6. **Generates** `docs/index.html` with cards for all visualizations
7. **Deploys** the `docs/` directory to GitHub Pages

## What Happens Without Metadata?

If a .obgraph file in `examples/pages-study/` is **missing** the required metadata:
- ⚠️ The build script will print a warning and **skip** the file
- ❌ The file will **NOT appear** on the index page
- ❌ No HTML file will be generated
- ❌ Users won't be able to find or view your visualization

## Testing Locally

Before pushing, test your file locally:

```bash
# Render your obgraph file
cargo run -- render examples/pages-study/tpm/my-visualization.obgraph -o examples/pages-study/tpm/my-visualization.html

# Run the full build script
python3 scripts/build-pages.py

# Check that your file appears in docs/
ls -la docs/tpm/
```

## Stats Calculation

Count the nodes and domains in your .obgraph file:

```bash
# Count domains
grep -c '^domain ' examples/pages-study/tpm/my-file.obgraph

# Count nodes
grep -c '^\s*node ' examples/pages-study/tpm/my-file.obgraph
```

Then update your metadata:
```obgraph
#@ stats: {"nodes": 19, "domains": 7}
```

## Troubleshooting

### My file doesn't appear on the index page
- ✓ Check that the file is in `examples/pages-study/<section>/`
- ✓ Verify all required metadata (`title`, `heading`, `description`) is present
- ✓ Check the `#@` format (space after `#@`, colon after key)
- ✓ Look at build logs for warnings

### The card looks wrong
- Check `badge_type` matches one of: hardware, enterprise, slsa, recommended, deprecated
- Verify `stats` is valid JSON: `{"nodes": N, "domains": M}`
- Check that `description` is concise (1-2 sentences max)

### The visualization doesn't render
- Run `cargo run -- render path/to/file.obgraph` locally to check for syntax errors
- Use `/objgraph-debug path/to/file.obgraph` for interactive debugging
- Check for red arrows in the HTML output (indicates invalid state propagation)

## Best Practices

1. **Descriptive names**: Use the trust system name, not "notes"
2. **Complete metadata**: Include all optional fields for better presentation
3. **Accurate stats**: Count actual nodes/domains in your file
4. **Concise descriptions**: 1-2 sentences that explain the trust model
5. **Meaningful badges**: Choose badge types that match the content
6. **Test locally**: Always render and check before pushing

## Related Documentation

- `.claude/skills/objgraph-pipeline/SKILL.md` - Full pipeline documentation
- `.claude/agents/objgraph-create/agent.md` - File creation agent
- `scripts/build-pages.py` - Build script source code
- `.github/workflows/deploy-pages.yml` - GitHub Actions workflow
