# Examples Directory

This directory contains all the trust graph examples that are automatically built into the GitHub Pages site.

## Directory Structure

```
examples/pages-study/
├── container-signing/
│   ├── cosign_keyless.obgraph        ← Trust graph with embedded metadata
│   ├── docker_content_trust.obgraph
│   ├── intoto.obgraph
│   ├── notaryv2.obgraph
│   ├── INDEX.md                      ← Section documentation
│   └── README.md
├── snp/
│   ├── snp.obgraph                   ← Trust graph with embedded metadata
│   ├── snp.md                        ← Supporting documentation
│   └── snp-trust-model.md
└── website/
    ├── github.obgraph                ← Trust graph with embedded metadata
    ├── github-trust-model.md
    └── website.md
```

## File Format: `.obgraph` with Embedded Metadata

Trust graphs use the `.obgraph` format with metadata embedded in special `#@` comment lines.

### Example `.obgraph` File

```obgraph
# Trust Model Title
# Brief description of the trust model
#
#@ title: Display Title
#@ heading: Full Page Heading
#@ badge: Recommended
#@ badge_type: recommended
#@ card_style: recommended
#@ meta: Subtitle • Additional info
#@ stats: {"nodes": 9, "domains": 5}
#@ description: Main description paragraph shown on the index page card.
#@ key_features: Optional features summary shown as "Key Features:" paragraph
#@ best_for: Optional use case summary shown as "Best for:" paragraph

# Trust graph specification follows...
domain "Example" {
  node SomeNode "Display Name" @anchored {
    field @constrained
  }
}
```

### Available Metadata Keys

See `examples/pages-study/container-signing/cosign_keyless.obgraph` for the complete metadata schema documentation.

| Key | Description | Example |
|-----|-------------|---------|
| `title` | Card title (required) | `Cosign Keyless` |
| `heading` | Full page heading | `Cosign Keyless Signing Trust Graph` |
| `badge` | Badge text | `Recommended`, `Deprecated`, `SLSA Level 3+` |
| `badge_type` | Badge color | `recommended`, `deprecated`, `enterprise`, `slsa`, `hardware` |
| `card_style` | Card border style | `recommended`, `deprecated`, `hardware` |
| `meta` | Subtitle on card | `Sigstore • Modern keyless signing` |
| `stats` | Node/domain counts (JSON) | `{"nodes": 9, "domains": 5}` |
| `description` | Main description | Multi-line text describing the trust model |
| `key_features` | Features summary | Optional paragraph |
| `best_for` | Use case summary | Optional paragraph |

### Value Parsing

- **JSON objects/arrays**: Values starting with `{` or `[` are parsed as JSON
- **Integers**: All-digit values are parsed as integers
- **Strings**: Everything else is treated as a string

## Adding a New Example

1. **Choose or create a section directory** under `examples/pages-study/`
2. **Create `<name>.obgraph`** with:
   - Metadata in `#@` comment lines at the top
   - Trust graph specification using obgraph syntax
3. **Add supporting documentation** (optional `.md` files)
4. **Test locally**: Run `scripts/build-pages.py` to verify
5. **Push to gh-pages branch** - GitHub Actions builds automatically!

## Section Configuration

Section headers are configured in `scripts/build-pages.py` via the `DEFAULT_SECTION_INFO` dictionary:

```python
DEFAULT_SECTION_INFO = {
    "container-signing": {
        "title": "Container Image Signing",
        "description": "Cryptographic trust chains for container security and provenance"
    },
    "snp": {
        "title": "AMD SEV-SNP Attestation",
        "description": "Hardware-based confidential computing trust chains"
    }
}
```

If no configuration exists, section titles are auto-generated from the directory name.

### Special Section Features

- **container-signing**: Includes a comparison table between signing methods
- **snp**: Hardware-focused styling with TEE badges
- **website**: Foundational web PKI trust models

## Build Process

The build script (`scripts/build-pages.py`):
1. Scans `examples/pages-study/` for `.obgraph` files
2. Extracts metadata from `#@` comment lines
3. Calls `obgraph-render` to generate HTML for each graph
4. Generates `docs/index.html` with cards organized by section
5. GitHub Actions deploys `docs/` to GitHub Pages
