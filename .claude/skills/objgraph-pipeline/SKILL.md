---
name: objgraph-pipeline
description: Orchestrate the full obgraph workflow - research, create, and review - using agents. Takes documentation and produces a validated .obgraph file.
tools: Read, Glob, Grep, Write, Task
argument-hint: [path to documentation or description of trust system]
---

# objgraph Pipeline Orchestrator

You orchestrate the full obgraph creation workflow by spawning specialized agents in sequence.

## Workflow

```
Input (docs/description)
    ↓
[objgraph-research agent] → Trust Model Specification
    ↓
[objgraph-create agent] → .obgraph file
    ↓
[objgraph-review agent] → Review report
    ↓
[render tool] → HTML visualization
    ↓
Output (validated .obgraph + HTML + review)
```

## Your Process

### Step 1: Understand the Input

Determine what the user provided:
- **Documentation path(s)** - Files describing a trust system
- **Description** - Natural language description of what to model
- **Both** - Docs plus additional context

### Step 2: Spawn Research Agent

Use the Task tool to spawn the `objgraph-research` agent:

```
Task(
  subagent_type="objgraph-research",
  prompt="Research the trust model from [input]. Verify claims against authoritative sources. Output a Trust Model Specification to [output-path]."
)
```

The research agent will:
- Read all provided documentation
- Search for and verify against official sources
- Produce a structured Trust Model Specification

### Step 3: Spawn Create Agent

Once research completes, spawn the `objgraph-create` agent:

```
Task(
  subagent_type="objgraph-create",
  prompt="Create an obgraph from the specification at [spec-path]. Output to [obgraph-path]."
)
```

The create agent will:
- Read the Trust Model Specification
- Design domains, nodes, and relationships
- Trace state propagation to ensure validity
- Produce a complete .obgraph file

### Step 4: Spawn Review Agent

Once creation completes, spawn the `objgraph-review` agent:

```
Task(
  subagent_type="objgraph-review",
  prompt="Review [obgraph-path] for correctness. Identify any issues and provide fixes."
)
```

The review agent will:
- Parse the .obgraph file
- Trace state propagation
- Identify any issues
- Produce a review report

### Step 5: Render to HTML

Once the .obgraph file is validated, render it to HTML for visual verification using the Bash tool:

```bash
cargo run -- render path/to/output.obgraph -o path/to/output.html
```

**GitHub Pages Integration**: If creating files in `examples/pages-study/`, the obgraph files need special metadata comments for the GitHub Pages build system. See the "GitHub Pages Metadata" section below.

### Step 6: Clean Up Temporary Files

Remove only temporary review files (if they exist):

```bash
rm path/to/review.md  # Only if review file was created
```

**Keep these important artifacts**:
- The Trust Model Specification (.md) - important research documentation
- The .obgraph file (source)
- The .html file (rendered visualization)
- The original documentation (user's input)

### Step 7: Report Results

Summarize for the user:
- Location of Trust Model Specification
- Location of .obgraph file
- Location of rendered HTML file
- Review status (valid or issues found)
- Any fixes that were applied

## File Naming Convention

**IMPORTANT**: Use clear, descriptive names (NOT generic names like "notes" or "foo"):

If user provides `docs/sigstore-notes.md` about Sigstore keyless signing:
- Spec: `docs/sigstore-keyless-signing-trust-model.md`
- obgraph: `docs/sigstore-keyless-signing.obgraph`
- HTML: `docs/sigstore-keyless-signing.html`
- Review: `docs/sigstore-keyless-signing-review.md`

If working in `examples/pages-study/container-signing/`:
- Use descriptive names like: `cosign-trust-model.obgraph`, `docker-content-trust.obgraph`
- NOT generic names like: `notes.obgraph`, `container-signing-notes.obgraph`

**File Location Rules**:
- Write all files to the working directory or subdirectories within the project
- NEVER use `/tmp/` or `tmp/` directories
- Keep related files together in the same directory

Or use user-specified output locations.

## Example Invocations

### From documentation:
```
/objgraph-pipeline planning/container-signing.md
```

### From description:
```
/objgraph-pipeline Create an obgraph for AMD SEV-SNP attestation
```

### With output location:
```
/objgraph-pipeline Research UEFI Secure Boot and output to visualizations/uefi-secureboot.obgraph
```

## Parallel vs Sequential

By default, agents run sequentially because each depends on the previous:
1. Research must complete before Create can start
2. Create must complete before Review can start

However, if the user already has a Trust Model Specification, you can skip research:
```
/objgraph-pipeline Create from existing spec at docs/my-spec.md
```

Or if they have an .obgraph file, just review:
```
/objgraph-pipeline Review visualizations/existing.obgraph
```

## Error Handling

If an agent reports issues:
- **Research fails**: Ask user for clarification or additional sources
- **Create fails**: Report what's missing from the spec
- **Review finds issues**: Report issues and offer to re-run create with fixes

## GitHub Pages Metadata

When creating obgraph files in `examples/pages-study/`, the files MUST include special `#@` metadata comments for the GitHub Pages build system to render them correctly.

### Required Metadata

Every .obgraph file in `examples/pages-study/` needs these comment lines at the top:

```obgraph
#@ title: Short Title
#@ heading: Full Descriptive Heading
#@ description: Brief description shown on index page
```

### Optional Metadata for Styling

```obgraph
#@ badge: Badge Text
#@ badge_type: hardware|enterprise|slsa|recommended|deprecated
#@ card_style: hardware|recommended|deprecated
#@ meta: Technology • Keywords • Separated by bullets
#@ stats: {"nodes": 19, "domains": 7}
#@ key_features: Feature 1, Feature 2, Feature 3
#@ best_for: Use case description
```

### Example from tpm/tpm-notes.obgraph

```obgraph
#@ title: TPM & fTPM
#@ heading: TPM and fTPM Trust Model for BitLocker and Windows Hello
#@ badge: Hardware Root
#@ badge_type: hardware
#@ card_style: hardware
#@ meta: Trusted Platform Module • BitLocker • Windows Hello
#@ stats: {"nodes": 19, "domains": 7}
#@ description: Hardware-rooted cryptographic trust for discrete TPMs (dTPM) and firmware TPMs (fTPM)...
#@ key_features: Hardware identity (EK certificates), measured boot (PCR sealing), key hierarchy...
#@ best_for: Windows disk encryption (BitLocker), biometric authentication (Windows Hello)...
```

### GitHub Pages Workflow

1. Files are in `examples/pages-study/<section>/`
2. Each section directory becomes a category on the index page
3. The build script (`scripts/build-pages.py`) scans for .obgraph files
4. Metadata from `#@` comments is extracted
5. Each .obgraph file is rendered to HTML in `docs/<section>/`
6. An index.html is generated with cards for each visualization
7. GitHub Actions workflow deploys to GitHub Pages

### Badge Types

- `hardware`: Red badge and card styling (TPM, SEV-SNP, hardware roots)
- `enterprise`: Blue badge (production systems, enterprise solutions)
- `slsa`: Purple badge (SLSA framework, supply chain security)
- `recommended`: Green badge and card styling (current best practices)
- `deprecated`: Red badge and card styling (legacy or deprecated systems)

### Important Notes

- **Without metadata**: Files won't appear on the GitHub Pages index
- **File naming**: Use descriptive names like `tpm-bitlocker.obgraph`, not `notes.obgraph`
- **Section organization**: Place files in appropriate subdirectories (tpm/, snp/, container-signing/, etc.)
- **Stats accuracy**: Count actual nodes and domains in your obgraph file

## Related Skills

| Skill | Use When |
|-------|----------|
| `/objgraph` | Quick questions about syntax or concepts |
| `/objgraph-debug` | Interactive debugging (user wants to understand, not just fix) |
