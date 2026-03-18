#!/usr/bin/env python3
"""
Build GitHub Pages from .obgraph files with embedded metadata.

This script:
1. Scans the examples/ directory for .obgraph files
2. Extracts metadata from special #@ comment lines
3. Uses obgraph-render to generate HTML files
4. Generates an index.html with all examples organized by section
"""

import json
import subprocess
import os
import shutil
import re
from pathlib import Path
from typing import Dict, List, Any

# Configuration
EXAMPLES_DIR = Path("examples/pages-study")
DOCS_DIR = Path("docs")
OBGRAPH_RENDER = Path("target/release/obgraph-render")

# Default section configuration (used if no section.json exists)
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

def load_section_config(section_dir: Path) -> Dict[str, str]:
    """Load section configuration from defaults or auto-generate."""
    section_name = section_dir.name

    # Use default config if available
    if section_name in DEFAULT_SECTION_INFO:
        return DEFAULT_SECTION_INFO[section_name]

    # Auto-generate from directory name
    return {
        "title": section_name.replace("-", " ").replace("_", " ").title(),
        "description": f"Trust graphs for {section_name}"
    }

def parse_metadata_from_obgraph(obgraph_path: Path) -> Dict[str, Any]:
    """Extract metadata from #@ comment lines in .obgraph file."""
    metadata = {}

    with open(obgraph_path, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            # Look for special metadata comments: #@ key: value
            if line.startswith('#@'):
                match = re.match(r'#@\s*(\w+):\s*(.+)', line)
                if match:
                    key, value = match.groups()
                    # Try to parse JSON values for complex types
                    try:
                        # Handle JSON objects and arrays
                        if value.strip().startswith(('{', '[')):
                            metadata[key] = json.loads(value)
                        # Handle numeric values
                        elif value.isdigit():
                            metadata[key] = int(value)
                        else:
                            metadata[key] = value.strip()
                    except json.JSONDecodeError:
                        metadata[key] = value.strip()

    return metadata

def find_obgraph_files() -> List[Dict[str, Any]]:
    """Find all .obgraph files and extract their embedded metadata."""
    files = []

    for obgraph_path in EXAMPLES_DIR.rglob("*.obgraph"):
        # Extract metadata from obgraph file comments
        metadata = parse_metadata_from_obgraph(obgraph_path)

        if not metadata:
            print(f"Warning: No metadata found in {obgraph_path}, skipping")
            continue

        # Determine section from path
        relative_path = obgraph_path.relative_to(EXAMPLES_DIR)
        section = relative_path.parts[0] if len(relative_path.parts) > 1 else "other"

        files.append({
            "obgraph_path": obgraph_path,
            "metadata": metadata,
            "section": section,
            "stem": obgraph_path.stem
        })

    return files

def render_html(obgraph_path: Path, output_path: Path, metadata: Dict[str, Any]):
    """Use obgraph-render to generate HTML."""
    cmd = [
        str(OBGRAPH_RENDER),
        "-i", str(obgraph_path),
        "-o", str(output_path),
        "-t", metadata.get("title", "Trust Graph"),
        "-H", metadata.get("heading", metadata.get("title", "Trust Graph"))
    ]

    if "description" in metadata:
        cmd.extend(["-d", metadata["description"]])

    result = subprocess.run(cmd, capture_output=True, text=True)

    if result.returncode != 0:
        print(f"Error rendering {obgraph_path}:")
        print(result.stderr)
        raise RuntimeError(f"Failed to render {obgraph_path}")

    print(f"[OK] Rendered {output_path}")

def generate_index_html(files_by_section: Dict[str, List[Dict[str, Any]]]):
    """Generate the index.html file."""

    # Group sections
    sections_html = []

    for section_key, section_files in sorted(files_by_section.items()):
        # Load section config from section.json or defaults
        section_dir = EXAMPLES_DIR / section_key
        section_config = load_section_config(section_dir)

        # Generate cards for this section
        cards_html = []
        for file_info in section_files:
            meta = file_info["metadata"]
            output_filename = f"{section_key}/{file_info['stem']}.html"

            badge_html = ""
            if "badge" in meta:
                badge_type = meta.get("badge_type", "enterprise")
                badge_html = f'<span class="badge {badge_type}">{meta["badge"]}</span>'

            stats_html = ""
            if "stats" in meta:
                stats = meta["stats"]
                stats_html = f'''<div class="stats">
                        <span>{stats.get("nodes", "?")} nodes</span>
                        <span>{stats.get("domains", "?")} domains</span>
                    </div>'''

            card_class = "model-card"
            if "card_style" in meta:
                card_class += f" {meta['card_style']}"

            description_paras = []
            if "description" in meta:
                description_paras.append(f"<p>{meta['description']}</p>")
            if "key_features" in meta:
                description_paras.append(f"<p><strong>Key Features:</strong> {meta['key_features']}</p>")
            if "best_for" in meta:
                description_paras.append(f"<p><strong>Best for:</strong> {meta['best_for']}</p>")

            card_html = f'''
                <div class="{card_class}">
                    {badge_html}
                    <h3>{meta.get("title", file_info["stem"])}</h3>
                    <div class="meta">{meta.get("meta", "")}</div>
                    {stats_html}
                    {"".join(description_paras)}
                    <a href="{output_filename}" class="btn">View Trust Graph →</a>
                </div>'''

            cards_html.append(card_html)

        section_html = f'''
            <div class="section">
                <div class="section-header">
                    <h2>{section_config["title"]}</h2>
                    <p>{section_config["description"]}</p>
                </div>

                <div class="models">
                    {"".join(cards_html)}
                </div>
            </div>'''

        sections_html.append(section_html)

    # Read the template and insert sections
    index_template = """<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Trust Graph Studies</title>
    <style>
        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            line-height: 1.6;
            color: #333;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            padding: 2rem 1rem;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: white;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
            overflow: hidden;
        }

        header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 3rem 2rem;
            text-align: center;
        }

        header h1 {
            font-size: 2.5rem;
            margin-bottom: 0.5rem;
            font-weight: 700;
        }

        header p {
            font-size: 1.1rem;
            opacity: 0.95;
        }

        .content {
            padding: 2rem;
        }

        .intro {
            background: #f8f9fa;
            border-left: 4px solid #667eea;
            padding: 1.5rem;
            margin-bottom: 2rem;
            border-radius: 4px;
        }

        .intro h2 {
            color: #667eea;
            margin-bottom: 0.5rem;
        }

        .section {
            margin-bottom: 3rem;
        }

        .section-header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 1rem 1.5rem;
            border-radius: 8px;
            margin-bottom: 1.5rem;
        }

        .section-header h2 {
            font-size: 1.8rem;
            margin-bottom: 0.25rem;
        }

        .section-header p {
            opacity: 0.9;
            font-size: 0.95rem;
        }

        .models {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }

        .model-card {
            border: 2px solid #e9ecef;
            border-radius: 8px;
            padding: 1.5rem;
            transition: all 0.3s ease;
            background: white;
        }

        .model-card:hover {
            border-color: #667eea;
            box-shadow: 0 8px 16px rgba(102, 126, 234, 0.2);
            transform: translateY(-4px);
        }

        .model-card.recommended {
            border-color: #28a745;
            background: #f1f9f3;
        }

        .model-card.deprecated {
            border-color: #dc3545;
            background: #fef3f3;
        }

        .model-card.hardware {
            border-color: #ff6b6b;
            background: #fff5f5;
        }

        .badge {
            display: inline-block;
            padding: 0.25rem 0.75rem;
            border-radius: 12px;
            font-size: 0.75rem;
            font-weight: 600;
            text-transform: uppercase;
            margin-bottom: 0.75rem;
        }

        .badge.recommended {
            background: #28a745;
            color: white;
        }

        .badge.deprecated {
            background: #dc3545;
            color: white;
        }

        .badge.enterprise {
            background: #007bff;
            color: white;
        }

        .badge.slsa {
            background: #6f42c1;
            color: white;
        }

        .badge.hardware {
            background: #ff6b6b;
            color: white;
        }

        .model-card h3 {
            font-size: 1.4rem;
            margin-bottom: 0.5rem;
            color: #2c3e50;
        }

        .model-card .meta {
            color: #6c757d;
            font-size: 0.9rem;
            margin-bottom: 1rem;
            font-style: italic;
        }

        .model-card p {
            margin-bottom: 1rem;
            color: #555;
        }

        .model-card .stats {
            display: flex;
            gap: 1rem;
            margin-bottom: 1rem;
            font-size: 0.85rem;
            color: #6c757d;
        }

        .model-card .stats span {
            display: flex;
            align-items: center;
            gap: 0.25rem;
        }

        .model-card .stats span::before {
            content: "●";
            color: #667eea;
        }

        .btn {
            display: inline-block;
            padding: 0.75rem 1.5rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            text-decoration: none;
            border-radius: 6px;
            font-weight: 600;
            transition: all 0.3s ease;
            border: none;
            cursor: pointer;
        }

        .btn:hover {
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
        }

        .comparison {
            background: #f8f9fa;
            border-radius: 8px;
            padding: 1.5rem;
            margin-top: 2rem;
        }

        .comparison h2 {
            color: #2c3e50;
            margin-bottom: 1rem;
        }

        .comparison table {
            width: 100%;
            border-collapse: collapse;
            background: white;
            border-radius: 6px;
            overflow: hidden;
        }

        .comparison th,
        .comparison td {
            padding: 0.75rem;
            text-align: left;
            border-bottom: 1px solid #e9ecef;
        }

        .comparison th {
            background: #667eea;
            color: white;
            font-weight: 600;
        }

        .comparison tr:last-child td {
            border-bottom: none;
        }

        .check {
            color: #28a745;
            font-weight: bold;
        }

        .cross {
            color: #dc3545;
            font-weight: bold;
        }

        footer {
            background: #2c3e50;
            color: white;
            padding: 1.5rem 2rem;
            text-align: center;
            font-size: 0.9rem;
        }

        footer a {
            color: #667eea;
            text-decoration: none;
        }

        footer a:hover {
            text-decoration: underline;
        }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Trust Graph Studies</h1>
            <p>Visualizing cryptographic trust chains across security domains</p>
        </header>

        <div class="content">
            <div class="intro">
                <h2>Overview</h2>
                <p>
                    This collection visualizes cryptographic trust models across different security domains,
                    showing how various systems establish and verify trust through certificates, signatures,
                    and attestations.
                </p>
            </div>

            {SECTIONS}

        </div>

        <footer>
            <p>
                Generated using <strong>mdbook-obgraph</strong> trust graph visualization engine
            </p>
            <p style="margin-top: 0.5rem; opacity: 0.8;">
                For more information, see the
                <a href="https://theupdateframework.github.io/specification/latest/">TUF Specification</a>,
                <a href="https://docs.sigstore.dev/">Sigstore Documentation</a>,
                <a href="https://github.com/in-toto/attestation">in-toto Attestation Framework</a>,
                <a href="https://notaryproject.dev/">Notary Project</a>, and
                <a href="https://www.amd.com/en/developer/sev.html">AMD SEV Documentation</a>
            </p>
        </footer>
    </div>
</body>
</html>"""

    html = index_template.replace("{SECTIONS}", "\n".join(sections_html))

    output_path = DOCS_DIR / "index.html"
    with open(output_path, 'w', encoding='utf-8') as f:
        f.write(html)

    print(f"[OK] Generated {output_path}")

def main():
    """Main build process."""
    print("Building GitHub Pages from .obgraph files...\n")

    # Clean docs directory
    if DOCS_DIR.exists():
        for item in DOCS_DIR.iterdir():
            if item.name != "design":  # Keep design docs
                if item.is_dir():
                    shutil.rmtree(item)
                else:
                    item.unlink()
    else:
        DOCS_DIR.mkdir(parents=True)

    # Find all obgraph files
    files = find_obgraph_files()
    print(f"Found {len(files)} .obgraph files\n")

    # Group by section
    files_by_section = {}
    for file_info in files:
        section = file_info["section"]
        if section not in files_by_section:
            files_by_section[section] = []
        files_by_section[section].append(file_info)

    # Render HTML for each file
    for section, section_files in files_by_section.items():
        section_dir = DOCS_DIR / section
        section_dir.mkdir(exist_ok=True)

        for file_info in section_files:
            output_path = section_dir / f"{file_info['stem']}.html"
            render_html(file_info["obgraph_path"], output_path, file_info["metadata"])

    print()

    # Generate index
    generate_index_html(files_by_section)

    print("\n[OK] Build complete!")

if __name__ == "__main__":
    main()
