# objgraph Skills and Agents

Skills and agents for creating and debugging trust graph visualizations. They are automatically available when working in this repository.

## Skills (Interactive)

| Skill | Purpose |
|-------|---------|
| `/objgraph` | Interactive expert - syntax, terminology, concepts |
| `/objgraph-pipeline` | Orchestrate full research → create → review workflow |
| `/objgraph-debug` | Step-by-step debugging sessions |

## Agents (Autonomous)

| Agent | Purpose |
|-------|---------|
| `objgraph-research` | Research trust systems, verify against sources |
| `objgraph-create` | Create .obgraph files from specs |
| `objgraph-review` | Review and debug .obgraph files |

## Usage

```
/objgraph What's the difference between a bond and a bridge?
/objgraph-pipeline Create an obgraph for Sigstore keyless signing from docs/sigstore-notes.md
/objgraph-debug visualizations/broken.obgraph
"Use the objgraph-research agent to analyze the SEV-SNP attestation docs"
```

## Important Guidelines

### File Naming
- Use **clear, descriptive names** for .obgraph files: `tpm-bitlocker.obgraph`, `cosign-signing.obgraph`
- **NEVER use generic names**: `notes.obgraph`, `foo-notes.obgraph`, `trust-model.obgraph`
- Both .obgraph and .html files should have the same descriptive base name

### GitHub Pages Integration
Files in `examples/pages-study/` require special `#@` metadata comments for proper rendering on the GitHub Pages site.

**Required metadata:**
- `#@ title:` - Short title for the card
- `#@ heading:` - Full heading for the detail page
- `#@ description:` - Summary for the index card

**See [GITHUB-PAGES.md](.claude/GITHUB-PAGES.md) for:**
- Complete metadata reference
- Badge types and styling options
- Example files
- Build process documentation
- Local testing instructions
- Troubleshooting guide

## Workflow

```
Documentation
     ↓
/objgraph-pipeline (or objgraph-research agent)
     ↓
Trust Model Specification
     ↓
objgraph-create agent
     ↓
.obgraph file
     ↓
objgraph-review agent (or /objgraph-debug for interactive)
     ↓
Validated visualization
```

## Recommended Settings

To enable seamless operation without permission prompts, add tool permissions to `.claude/settings.local.json` (project-level) or `~/.claude/settings.json` (global). The format uses `"permissions": { "allow": [...] }`:

```json
{
  "permissions": {
    "allow": [
      "WebSearch",
      "WebFetch",
      "Bash(cargo build:*)",
      "Bash(cargo test:*)",
      "Bash(*:/dev/null)"
    ]
  }
}
```

This allows agents and the main session to automatically search the web, fetch documentation, and run cargo commands without requiring user approval for each request.

**Note:** Do NOT use a separate `permissions.json` file — Claude Code does not read that format. All permission configuration goes in `settings.local.json` or `settings.json`.
