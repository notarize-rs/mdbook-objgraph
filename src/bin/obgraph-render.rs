/// Standalone CLI for rendering .obgraph files to HTML.
///
/// This binary provides a formal command-line interface for converting
/// .obgraph files into standalone HTML files with embedded SVG visualizations.
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "obgraph-render",
    about = "Convert .obgraph files to standalone HTML",
    long_about = "A tool for rendering .obgraph trust graph specifications into \
                  standalone HTML files with embedded SVG visualizations and interactive elements."
)]
struct Args {
    /// Input .obgraph file to render
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Output .html file path
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,

    /// HTML page title (shown in browser tab)
    #[arg(short, long, value_name = "TEXT")]
    title: String,

    /// Main heading text (shown at top of page)
    #[arg(short = 'H', long, value_name = "TEXT")]
    heading: String,

    /// Optional description paragraph (can include HTML)
    #[arg(short, long, value_name = "TEXT")]
    description: Option<String>,
}

fn main() {
    let args = Args::parse();

    // Read input file
    let input_content = match fs::read_to_string(&args.input) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error: Failed to read input file {:?}: {}", args.input, e);
            process::exit(1);
        }
    };

    // Process obgraph to SVG
    let svg = match mdbook_obgraph::process(&input_content) {
        Ok(svg) => svg,
        Err(e) => {
            eprintln!("Error: Failed to process obgraph: {}", e);
            process::exit(1);
        }
    };

    // Build description HTML if provided
    let description_html = args
        .description
        .as_ref()
        .map(|d| format!(r#"<div class="description">{}</div>"#, d))
        .unwrap_or_default();

    // Escape the source content for embedding in HTML
    let escaped_source = html_escape(&input_content);

    // Generate HTML with source viewer
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>
body {{ margin: 20px; background: #f5f5f5; font-family: system-ui, sans-serif; }}
h1 {{ color: #333; }}
.description {{ max-width: 800px; margin-bottom: 20px; color: #555; line-height: 1.6; }}

/* Source viewer styles */
.view-controls {{
    margin: 20px 0;
    display: flex;
    gap: 10px;
    align-items: center;
}}
.toggle-source-btn {{
    background: #667eea;
    color: white;
    border: none;
    padding: 10px 20px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
    font-weight: 600;
    transition: all 0.3s ease;
}}
.toggle-source-btn:hover {{
    background: #5568d3;
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
}}
.content-wrapper {{
    display: flex;
    gap: 20px;
    align-items: flex-start;
}}
.content-wrapper.split-view .obgraph-container {{
    flex: 1;
    max-width: 50%;
}}
.source-panel {{
    display: none;
    flex: 1;
    background: #1e1e1e;
    border-radius: 8px;
    padding: 20px;
    overflow: auto;
    max-height: 90vh;
    position: sticky;
    top: 20px;
}}
.content-wrapper.split-view .source-panel {{
    display: block;
}}
.source-panel pre {{
    margin: 0;
    color: #d4d4d4;
    font-family: 'Consolas', 'Monaco', 'Courier New', monospace;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-wrap: break-word;
}}
.source-header {{
    color: #4ec9b0;
    font-weight: bold;
    margin-bottom: 15px;
    padding-bottom: 10px;
    border-bottom: 1px solid #3e3e42;
}}
</style>
</head><body>
<h1>{heading}</h1>
{description_html}
<div class="view-controls">
    <button class="toggle-source-btn" onclick="toggleSource()">Show .obgraph Source</button>
</div>
<div class="content-wrapper" id="contentWrapper">
{svg}
<div class="source-panel" id="sourcePanel">
    <div class="source-header">.obgraph source file</div>
    <pre>{source}</pre>
</div>
</div>
<script>
function toggleSource() {{
    const wrapper = document.getElementById('contentWrapper');
    const btn = document.querySelector('.toggle-source-btn');

    if (wrapper.classList.contains('split-view')) {{
        wrapper.classList.remove('split-view');
        btn.textContent = 'Show .obgraph Source';
    }} else {{
        wrapper.classList.add('split-view');
        btn.textContent = 'Hide .obgraph Source';
    }}
}}
</script>
</body></html>"#,
        title = html_escape(&args.title),
        heading = html_escape(&args.heading),
        description_html = description_html,
        svg = svg,
        source = escaped_source
    );

    // Write output
    if let Err(e) = fs::write(&args.output, &html) {
        eprintln!("Error: Failed to write output file {:?}: {}", args.output, e);
        process::exit(1);
    }

    println!("[OK] Rendered {:?} ({} bytes)", args.output, html.len());
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
