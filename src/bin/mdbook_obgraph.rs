use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// Object graph visualizer for mdbook.
///
/// When invoked with no subcommand, runs as an mdbook preprocessor
/// (reads [context, book] JSON from stdin, writes modified book to stdout).
#[derive(Parser)]
#[command(name = "mdbook-obgraph", version, about, long_about = None, subcommand_required = false)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Render an .obgraph file to HTML or SVG.
    Render(RenderArgs),

    /// Validate and analyze layout quality.
    Check(CheckArgs),

    /// Inspect layout internals for debugging.
    Inspect(InspectArgs),

    /// Register the preprocessor in book.toml.
    Install(InstallArgs),

    /// Check renderer support (used by mdbook at build time).
    Supports(SupportsArgs),

    /// Generate shell completions.
    Completions(CompletionsArgs),
}

// ---------------------------------------------------------------------------
// Subcommand arguments
// ---------------------------------------------------------------------------

#[derive(clap::Args)]
struct RenderArgs {
    /// Input .obgraph file (use '-' for stdin).
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Write output to a file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = RenderFormat::Html)]
    format: RenderFormat,
}

#[derive(Clone, ValueEnum)]
enum RenderFormat {
    /// Full HTML page ready to open in a browser.
    Html,
    /// Raw SVG element for embedding.
    Svg,
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Input .obgraph files (use '-' for stdin).
    #[arg(required = true, value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = CheckFormat::Text)]
    format: CheckFormat,

    /// Exit 1 on any quality issue (warnings become errors).
    #[arg(long)]
    strict: bool,

    /// Compare against a previous JSON report; exit 1 on regression.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
}

#[derive(Clone, ValueEnum)]
enum CheckFormat {
    /// Human-readable colored output.
    Text,
    /// Machine-readable JSON.
    Json,
}

#[derive(clap::Args)]
struct InspectArgs {
    /// Input .obgraph file (use '-' for stdin).
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = InspectFormat::Text)]
    format: InspectFormat,
}

#[derive(Clone, ValueEnum)]
enum InspectFormat {
    /// Human-readable text.
    Text,
    /// Machine-readable JSON.
    Json,
}

#[derive(clap::Args)]
struct InstallArgs {
    /// Path to the mdbook root directory.
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(clap::Args)]
struct SupportsArgs {
    /// Renderer name to check.
    renderer: String,
}

#[derive(clap::Args)]
struct CompletionsArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    shell: clap_complete::Shell,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let code = match cli.command {
        None => run_preprocessor(),
        Some(Command::Render(args)) => run_render(args),
        Some(Command::Check(args)) => run_check(args),
        Some(Command::Inspect(args)) => run_inspect(args),
        Some(Command::Install(args)) => run_install(args),
        Some(Command::Supports(args)) => run_supports(args),
        Some(Command::Completions(args)) => run_completions(args),
    };

    process::exit(code);
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Read input from a file path, or from stdin if the path is "-".
fn read_input(path: &Path) -> Result<String, io::Error> {
    if path == Path::new("-") {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
    }
}

/// Derive a display name from a file path (stem without extension).
fn display_name(path: &Path) -> String {
    if path == Path::new("-") {
        return "stdin".to_string();
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("obgraph")
        .to_string()
}

/// Write output to a file or stdout.
fn write_output(dest: Option<&Path>, content: &str) -> Result<(), io::Error> {
    match dest {
        Some(path) => std::fs::write(path, content),
        None => {
            io::stdout().write_all(content.as_bytes())?;
            io::stdout().flush()
        }
    }
}

// ---------------------------------------------------------------------------
// supports
// ---------------------------------------------------------------------------

fn run_supports(args: SupportsArgs) -> i32 {
    if args.renderer == "html" { 0 } else { 1 }
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

fn run_install(args: InstallArgs) -> i32 {
    let toml_path = args.path.join("book.toml");

    let existing = if toml_path.exists() {
        match std::fs::read_to_string(&toml_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mdbook-obgraph install error: {e}");
                return 1;
            }
        }
    } else {
        String::new()
    };

    if existing.contains("[preprocessor.obgraph]") {
        eprintln!("mdbook-obgraph: [preprocessor.obgraph] already present in book.toml");
        return 0;
    }

    let mut updated = existing;
    if !updated.ends_with('\n') && !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str("\n[preprocessor.obgraph]\n");

    if let Err(e) = std::fs::write(&toml_path, updated) {
        eprintln!("mdbook-obgraph install error: {e}");
        return 1;
    }

    eprintln!(
        "mdbook-obgraph: added [preprocessor.obgraph] to {}",
        toml_path.display()
    );
    0
}

// ---------------------------------------------------------------------------
// completions
// ---------------------------------------------------------------------------

fn run_completions(args: CompletionsArgs) -> i32 {
    let mut cmd = Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "mdbook-obgraph", &mut io::stdout());
    0
}

// ---------------------------------------------------------------------------
// preprocess (default mdbook mode)
// ---------------------------------------------------------------------------

fn run_preprocessor() -> i32 {
    match preprocess_inner() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("mdbook-obgraph error: {e}");
            1
        }
    }
}

fn preprocess_inner() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    let mut value: serde_json::Value = serde_json::from_str(&input)?;

    let book = value
        .get_mut(1)
        .ok_or("expected a [context, book] JSON array")?;

    walk_book(book)?;

    let output = serde_json::to_string(&book)?;
    io::stdout().write_all(output.as_bytes())?;
    io::stdout().flush()?;

    Ok(())
}

/// Recursively walk the mdbook JSON structure and process obgraph code blocks
/// in every chapter's `content` field.
fn walk_book(value: &mut serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(chapter) = map.get_mut("Chapter") {
                if let Some(content) = chapter.get_mut("content")
                    && let Some(s) = content.as_str()
                {
                    let processed = process_markdown(s)?;
                    *content = serde_json::Value::String(processed);
                }
                if let Some(sub_items) = chapter.get_mut("sub_items") {
                    walk_book(sub_items)?;
                }
            } else {
                for v in map.values_mut() {
                    walk_book(v)?;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                walk_book(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Replace every ```obgraph ... ``` fenced code block in `markdown` with the
/// rendered SVG/HTML fragment produced by `mdbook_obgraph::process`.
fn process_markdown(markdown: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut result = String::with_capacity(markdown.len());
    let mut remaining = markdown;

    while let Some(start) = find_obgraph_fence(remaining) {
        result.push_str(&remaining[..start]);

        let after_fence = &remaining[start + "```obgraph".len()..];

        if let Some(end_offset) = find_closing_fence(after_fence) {
            let block_content = &after_fence[..end_offset];

            match mdbook_obgraph::process(block_content) {
                Ok(svg) => result.push_str(&svg),
                Err(e) => {
                    result.push_str(&format!(
                        "<!-- mdbook-obgraph error: {e} -->\n```obgraph{block_content}```"
                    ));
                }
            }

            remaining = &after_fence[end_offset + "```".len()..];
        } else {
            result.push_str("```obgraph");
            result.push_str(after_fence);
            remaining = "";
        }
    }

    result.push_str(remaining);
    Ok(result)
}

fn find_obgraph_fence(s: &str) -> Option<usize> {
    let needle = "```obgraph";
    let mut search = s;
    let mut base = 0usize;

    loop {
        let idx = search.find(needle)?;
        let abs = base + idx;

        let at_line_start = abs == 0 || s.as_bytes()[abs - 1] == b'\n';

        let after = &search[idx + needle.len()..];
        let info_end = after.find('\n').unwrap_or(after.len());
        let info = after[..info_end].trim();
        let valid_info = info.is_empty();

        if at_line_start && valid_info {
            return Some(abs);
        }

        base += idx + needle.len();
        search = &search[idx + needle.len()..];
    }
}

fn find_closing_fence(s: &str) -> Option<usize> {
    let after_newline_offset = s.find('\n').map(|i| i + 1)?;
    let search_area = &s[after_newline_offset..];

    let mut offset = after_newline_offset;
    for line in search_area.lines() {
        if line.trim() == "```" {
            return Some(offset);
        }
        offset += line.len() + 1;
    }
    None
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

fn run_render(args: RenderArgs) -> i32 {
    let name = display_name(&args.file);

    let input = match read_input(&args.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", args.file.display());
            return 1;
        }
    };

    let svg_fragment = match mdbook_obgraph::process(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {name}: {e}");
            return 1;
        }
    };

    let output = match args.format {
        RenderFormat::Svg => {
            // Extract just the <svg>...</svg> element from the container div.
            let svg_start = svg_fragment.find("<svg ").unwrap_or(0);
            let svg_end = svg_fragment.rfind("</svg>").map(|i| i + "</svg>".len()).unwrap_or(svg_fragment.len());
            svg_fragment[svg_start..svg_end].to_string()
        }
        RenderFormat::Html => {
            let title = &name;
            format!(
                "<!DOCTYPE html>\n\
                 <html><head><meta charset=\"utf-8\"><title>{title}</title>\n\
                 <style>body {{ margin: 20px; background: #f0f0f0; }}</style>\n\
                 </head><body>\n\
                 {svg_fragment}\n\
                 </body></html>\n"
            )
        }
    };

    if let Err(e) = write_output(args.output.as_deref(), &output) {
        eprintln!("error: {}: {e}", args.output.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "stdout".into()));
        return 1;
    }

    if let Some(path) = &args.output {
        eprintln!("Wrote {}", path.display());
    }

    0
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

fn run_check(args: CheckArgs) -> i32 {
    let mut results = Vec::new();
    let mut had_pipeline_error = false;

    for path in &args.files {
        let name = display_name(path);

        let input = match read_input(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {}: {e}", path.display());
                had_pipeline_error = true;
                continue;
            }
        };

        let ast = match mdbook_obgraph::parse::parse(&input) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("error: {name}: {e}");
                had_pipeline_error = true;
                continue;
            }
        };

        let graph = match mdbook_obgraph::model::build(ast) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("error: {name}: {e}");
                had_pipeline_error = true;
                continue;
            }
        };

        let layout = match mdbook_obgraph::layout::layout(&graph) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error: {name}: {e}");
                had_pipeline_error = true;
                continue;
            }
        };

        let report = mdbook_obgraph::layout::quality::analyze(&graph, &layout);
        results.push((name, path.clone(), graph, report));
    }

    if had_pipeline_error && results.is_empty() {
        return 1;
    }

    // Determine overall pass/fail.
    let has_errors = results.iter().any(|(_, _, _, r)| r.error_count() > 0);
    let has_warnings = results.iter().any(|(_, _, _, r)| r.warning_count() > 0);

    match args.format {
        CheckFormat::Text => {
            for (name, _path, graph, report) in &results {
                print_check_text(name, graph, report);
            }
        }
        CheckFormat::Json => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|(name, _path, graph, report)| quality_to_json(name, graph, report))
                .collect();

            let output = serde_json::to_string_pretty(&json_results)
                .expect("JSON serialization cannot fail");
            println!("{output}");
        }
    }

    // Baseline comparison.
    if let Some(baseline_path) = &args.baseline {
        let baseline_data = match std::fs::read_to_string(baseline_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "error: {}: {e}",
                    baseline_path.display()
                );
                return 1;
            }
        };

        let baseline: Vec<serde_json::Value> = match serde_json::from_str(&baseline_data) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "error: {}: invalid JSON: {e}",
                    baseline_path.display()
                );
                return 1;
            }
        };

        let regressions = compare_baseline(&results, &baseline);
        if !regressions.is_empty() {
            eprintln!();
            eprintln!("Quality regressions detected:");
            for reg in &regressions {
                eprintln!("  {reg}");
            }
            return 1;
        }
        eprintln!("No quality regressions.");
    }

    if had_pipeline_error {
        return 1;
    }
    if has_errors {
        return 1;
    }
    if args.strict && has_warnings {
        return 1;
    }

    0
}

/// Whether stdout supports ANSI color.
fn use_color() -> bool {
    // Respect NO_COLOR convention (https://no-color.org/).
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn color(code: &str, text: &str) -> String {
    if use_color() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn print_check_text(
    name: &str,
    graph: &mdbook_obgraph::model::types::Graph,
    report: &mdbook_obgraph::layout::quality::QualityReport,
) {
    let errors = report.error_count();
    let warnings = report.warning_count();

    let status = if errors > 0 {
        color("31", "FAIL")
    } else if warnings > 0 {
        color("33", "WARN")
    } else {
        color("32", "OK")
    };

    println!("{name}: {status} ({errors} errors, {warnings} warnings)");

    if errors > 0 {
        println!("  Errors:");
        if !report.node_overlaps.is_empty() {
            println!(
                "    node overlaps: {}",
                report
                    .node_overlaps
                    .iter()
                    .map(|(a, b)| format!(
                        "{} \u{2194} {}",
                        graph.nodes[a.index()].ident,
                        graph.nodes[b.index()].ident
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !report.domain_overlaps.is_empty() {
            println!("    domain overlaps: {}", report.domain_overlaps.len());
        }
        if !report.nodes_outside_domain.is_empty() {
            println!(
                "    nodes outside domain: {}",
                report.nodes_outside_domain.len()
            );
        }
        if !report.derivs_inside_domains.is_empty() {
            println!(
                "    derivations inside domains: {}",
                report.derivs_inside_domains.len()
            );
        }
        if !report.free_nodes_inside_domains.is_empty() {
            println!(
                "    free nodes inside domains: {}",
                report.free_nodes_inside_domains.len()
            );
        }
        if !report.domain_contiguity_violations.is_empty() {
            println!(
                "    domain contiguity violations: {}",
                report.domain_contiguity_violations.len()
            );
        }
        if !report.inter_domain_edges_in_intra_corridors.is_empty() {
            println!(
                "    inter-domain edges in intra corridors: {}",
                report.inter_domain_edges_in_intra_corridors.len()
            );
        }
        if !report.channel_collisions.is_empty() {
            println!(
                "    channel collisions: {}",
                report.channel_collisions.len()
            );
        }
        if !report.node_deriv_overlaps.is_empty() {
            println!(
                "    node-derivation overlaps: {}",
                report.node_deriv_overlaps.len()
            );
        }
        if !report.deriv_deriv_overlaps.is_empty() {
            println!(
                "    derivation-derivation overlaps: {}",
                report.deriv_deriv_overlaps.len()
            );
        }
        if !report.intra_edges_in_wrong_corridor.is_empty() {
            println!(
                "    intra edges in wrong corridor: {}",
                report.intra_edges_in_wrong_corridor.len()
            );
        }
    }

    if warnings > 0 {
        println!("  Warnings: {warnings} collision/overlap issues");
    }

    println!(
        "  Metrics: {:.0}x{:.0}px, balance={:.3}, edge_crossings={}, edge_len={:.0}px",
        report.total_width,
        report.total_height,
        report.visual_balance,
        report.edge_crossings,
        report.total_edge_length,
    );
    println!();
}

fn quality_to_json(
    name: &str,
    graph: &mdbook_obgraph::model::types::Graph,
    report: &mdbook_obgraph::layout::quality::QualityReport,
) -> serde_json::Value {
    let errors = report.error_count();
    let warnings = report.warning_count();
    let status = if errors > 0 { "fail" } else { "pass" };

    let mut requirements = Vec::new();

    macro_rules! req {
        ($field:ident, $label:expr) => {
            if !report.$field.is_empty() {
                requirements.push(serde_json::json!({
                    "name": $label,
                    "count": report.$field.len(),
                }));
            }
        };
    }

    req!(node_overlaps, "node_overlaps");
    req!(domain_overlaps, "domain_overlaps");
    req!(nodes_outside_domain, "nodes_outside_domain");
    req!(derivs_inside_domains, "derivs_inside_domains");
    req!(free_nodes_inside_domains, "free_nodes_inside_domains");
    req!(domain_contiguity_violations, "domain_contiguity_violations");
    req!(
        inter_domain_edges_in_intra_corridors,
        "inter_domain_edges_in_intra_corridors"
    );
    req!(channel_collisions, "channel_collisions");
    req!(node_deriv_overlaps, "node_deriv_overlaps");
    req!(deriv_deriv_overlaps, "deriv_deriv_overlaps");
    req!(intra_edges_in_wrong_corridor, "intra_edges_in_wrong_corridor");

    let mut collisions = Vec::new();

    macro_rules! col {
        ($field:ident, $label:expr) => {
            if !report.$field.is_empty() {
                collisions.push(serde_json::json!({
                    "name": $label,
                    "count": report.$field.len(),
                }));
            }
        };
    }

    col!(node_edge_overlaps, "node_edge_overlaps");
    col!(label_node_overlaps, "label_node_overlaps");
    col!(arrowhead_node_overlaps, "arrowhead_node_overlaps");
    col!(stub_node_overlaps, "stub_node_overlaps");
    col!(
        edge_domain_boundary_crossings,
        "edge_domain_boundary_crossings"
    );
    col!(label_domain_overlaps, "label_domain_overlaps");
    col!(arrowhead_domain_overlaps, "arrowhead_domain_overlaps");
    col!(stub_domain_overlaps, "stub_domain_overlaps");
    col!(edge_deriv_overlaps, "edge_deriv_overlaps");
    col!(label_deriv_overlaps, "label_deriv_overlaps");
    col!(arrowhead_deriv_overlaps, "arrowhead_deriv_overlaps");
    col!(stub_deriv_overlaps, "stub_deriv_overlaps");
    col!(edge_label_overlaps, "edge_label_overlaps");
    col!(edge_arrowhead_overlaps, "edge_arrowhead_overlaps");
    col!(edge_stub_overlaps, "edge_stub_overlaps");
    col!(edge_domain_title_overlaps, "edge_domain_title_overlaps");
    col!(label_label_overlaps, "label_label_overlaps");
    col!(label_arrowhead_overlaps, "label_arrowhead_overlaps");
    col!(label_stub_overlaps, "label_stub_overlaps");
    col!(label_domain_title_overlaps, "label_domain_title_overlaps");
    col!(arrowhead_arrowhead_overlaps, "arrowhead_arrowhead_overlaps");
    col!(arrowhead_stub_overlaps, "arrowhead_stub_overlaps");
    col!(
        arrowhead_domain_title_overlaps,
        "arrowhead_domain_title_overlaps"
    );
    col!(stub_stub_overlaps, "stub_stub_overlaps");
    col!(stub_domain_title_overlaps, "stub_domain_title_overlaps");
    col!(
        domain_title_title_overlaps,
        "domain_title_title_overlaps"
    );

    // Add items detail for node_overlaps (resolve IDs to idents).
    let node_overlap_items: Vec<String> = report
        .node_overlaps
        .iter()
        .map(|(a, b)| {
            format!(
                "{} \u{2194} {}",
                graph.nodes[a.index()].ident,
                graph.nodes[b.index()].ident
            )
        })
        .collect();

    // Patch node_overlaps entry with items if present.
    if let Some(entry) = requirements.iter_mut().find(|e| e["name"] == "node_overlaps") {
        entry["items"] = serde_json::json!(node_overlap_items);
    }

    serde_json::json!({
        "file": name,
        "status": status,
        "errors": errors,
        "warnings": warnings,
        "requirements": requirements,
        "collisions": collisions,
        "metrics": {
            "visual_balance": report.visual_balance,
            "column_height_imbalance": report.column_height_imbalance,
            "dimensions": {
                "width": report.total_width,
                "height": report.total_height,
            },
            "aspect_ratio": report.aspect_ratio,
            "edge_crossings": report.edge_crossings,
            "total_edge_length": report.total_edge_length,
            "min_node_gap": report.min_node_gap,
            "node_width_delta": report.node_width_delta,
            "max_parent_misalignment": report.max_parent_misalignment,
            "mean_constraint_segments": report.mean_constraint_segments,
            "port_side_balance": report.port_side_balance,
            "edge_length_cv": report.edge_length_cv,
            "routing_direction_balance": report.routing_direction_balance,
            "max_column_centering_error": report.max_column_centering_error,
            "domain_size_cv": report.domain_size_cv,
        }
    })
}

/// Compare current results against a baseline JSON array.
/// Returns a list of regression descriptions.
fn compare_baseline(
    current: &[(String, PathBuf, mdbook_obgraph::model::types::Graph, mdbook_obgraph::layout::quality::QualityReport)],
    baseline: &[serde_json::Value],
) -> Vec<String> {
    let mut regressions = Vec::new();

    for (name, _path, graph, report) in current {
        let current_json = quality_to_json(name, graph, report);

        // Find matching baseline entry by file name.
        let baseline_entry = baseline.iter().find(|b| b["file"].as_str() == Some(name));
        let Some(base) = baseline_entry else {
            continue; // New file, no baseline to compare.
        };

        // Compare integer metrics (higher is worse).
        let int_metrics = [
            ("errors", current_json["errors"].as_u64(), base["errors"].as_u64()),
            ("warnings", current_json["warnings"].as_u64(), base["warnings"].as_u64()),
            ("edge_crossings", current_json["metrics"]["edge_crossings"].as_u64(), base["metrics"]["edge_crossings"].as_u64()),
        ];
        for (metric, cur, bsl) in int_metrics {
            if let (Some(c), Some(b)) = (cur, bsl)
                && c > b
            {
                regressions.push(format!("{name}: {metric}: {b} \u{2192} {c} (+{})", c - b));
            }
        }

        // Compare float metrics where lower absolute value is better.
        let lower_is_better = [
            "visual_balance",
            "column_height_imbalance",
            "total_edge_length",
            "node_width_delta",
            "max_parent_misalignment",
            "max_column_centering_error",
            "domain_size_cv",
            "edge_length_cv",
        ];
        for metric in lower_is_better {
            let cur = current_json["metrics"][metric].as_f64();
            let bsl = base["metrics"][metric].as_f64();
            if let (Some(c), Some(b)) = (cur, bsl)
                && c > b + 0.001
            {
                regressions.push(format!("{name}: {metric}: {b:.3} \u{2192} {c:.3}"));
            }
        }

        // Compare float metrics where higher is better.
        let higher_is_better = [
            "port_side_balance",
            "routing_direction_balance",
            "min_node_gap",
        ];
        for metric in higher_is_better {
            let cur = current_json["metrics"][metric].as_f64();
            let bsl = base["metrics"][metric].as_f64();
            if let (Some(c), Some(b)) = (cur, bsl)
                && c < b - 0.001
            {
                regressions.push(format!("{name}: {metric}: {b:.3} \u{2192} {c:.3}"));
            }
        }

        // Compare per-requirement counts (any increase is a regression).
        if let (Some(cur_reqs), Some(base_reqs)) = (
            current_json["requirements"].as_array(),
            base["requirements"].as_array(),
        ) {
            for cur_req in cur_reqs {
                let req_name = cur_req["name"].as_str().unwrap_or("");
                let cur_count = cur_req["count"].as_u64().unwrap_or(0);
                let base_count = base_reqs
                    .iter()
                    .find(|r| r["name"].as_str() == Some(req_name))
                    .and_then(|r| r["count"].as_u64())
                    .unwrap_or(0);
                if cur_count > base_count {
                    regressions.push(format!(
                        "{name}: {req_name}: {base_count} \u{2192} {cur_count} (+{})",
                        cur_count - base_count
                    ));
                }
            }
        }
    }

    regressions
}

// ---------------------------------------------------------------------------
// inspect
// ---------------------------------------------------------------------------

fn run_inspect(args: InspectArgs) -> i32 {
    let name = display_name(&args.file);

    let input = match read_input(&args.file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", args.file.display());
            return 1;
        }
    };

    let ast = match mdbook_obgraph::parse::parse(&input) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {name}: {e}");
            return 1;
        }
    };

    let graph = match mdbook_obgraph::model::build(ast) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {name}: {e}");
            return 1;
        }
    };

    let layout = match mdbook_obgraph::layout::layout(&graph) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {name}: {e}");
            return 1;
        }
    };

    use mdbook_obgraph::model::types::Edge;

    match args.format {
        InspectFormat::Text => {
            println!("=== Edge routes ===");
            let all_paths: Vec<_> = layout
                .anchors
                .iter()
                .chain(layout.intra_domain_constraints.iter())
                .chain(
                    layout
                        .cross_domain_constraints
                        .iter()
                        .map(|c| &c.full_path),
                )
                .collect();

            for ep in &all_paths {
                let edge = &graph.edges[ep.edge_id.index()];
                let desc = edge_description(&graph, edge);
                let kind = match edge {
                    Edge::Anchor { .. } => "anchor",
                    Edge::Constraint { source_prop, dest_prop, .. } => {
                        let src_dom =
                            graph.nodes[graph.properties[source_prop.index()].node.index()].domain;
                        let dst_dom =
                            graph.nodes[graph.properties[dest_prop.index()].node.index()].domain;
                        if src_dom != dst_dom {
                            "cross"
                        } else {
                            "intra"
                        }
                    }
                    Edge::DerivInput { .. } => "deriv",
                };
                println!("  [{kind:>5}] {desc}");
                println!("          path: {}", ep.svg_path);
            }

            println!("\n=== Nodes ===");
            for nl in &layout.nodes {
                let node = &graph.nodes[nl.id.index()];
                let dom = node
                    .domain
                    .map(|d| graph.domains[d.index()].display_name.as_str())
                    .unwrap_or("(none)");
                println!(
                    "  {} [{}]: x={:.0}..{:.0} y={:.0}..{:.0} ({:.0}x{:.0})",
                    node.ident,
                    dom,
                    nl.x,
                    nl.x + nl.width,
                    nl.y,
                    nl.y + nl.height,
                    nl.width,
                    nl.height,
                );
            }

            println!("\n=== Domains ===");
            for dl in &layout.domains {
                println!(
                    "  {} (id={}): x={:.0}..{:.0} y={:.0}..{:.0} ({:.0}x{:.0})",
                    dl.display_name,
                    dl.id.index(),
                    dl.x,
                    dl.x + dl.width,
                    dl.y,
                    dl.y + dl.height,
                    dl.width,
                    dl.height,
                );
            }

            println!("\n=== Derivations ===");
            for dl in &layout.derivations {
                println!(
                    "  deriv_{}: x={:.0}..{:.0} y={:.0}..{:.0}",
                    dl.id.index(),
                    dl.x,
                    dl.x + dl.width,
                    dl.y,
                    dl.y + dl.height,
                );
            }
        }
        InspectFormat::Json => {
            let edges: Vec<serde_json::Value> = layout
                .anchors
                .iter()
                .chain(layout.intra_domain_constraints.iter())
                .chain(
                    layout
                        .cross_domain_constraints
                        .iter()
                        .map(|c| &c.full_path),
                )
                .map(|ep| {
                    let edge = &graph.edges[ep.edge_id.index()];
                    let desc = edge_description(&graph, edge);
                    let kind = match edge {
                        Edge::Anchor { .. } => "anchor",
                        Edge::Constraint { source_prop, dest_prop, .. } => {
                            let src_dom = graph.nodes
                                [graph.properties[source_prop.index()].node.index()]
                            .domain;
                            let dst_dom = graph.nodes
                                [graph.properties[dest_prop.index()].node.index()]
                            .domain;
                            if src_dom != dst_dom {
                                "cross"
                            } else {
                                "intra"
                            }
                        }
                        Edge::DerivInput { .. } => "deriv",
                    };
                    serde_json::json!({
                        "id": ep.edge_id.index(),
                        "kind": kind,
                        "description": desc,
                        "svg_path": ep.svg_path,
                    })
                })
                .collect();

            let nodes: Vec<serde_json::Value> = layout
                .nodes
                .iter()
                .map(|nl| {
                    let node = &graph.nodes[nl.id.index()];
                    let dom = node
                        .domain
                        .map(|d| graph.domains[d.index()].display_name.clone());
                    serde_json::json!({
                        "id": nl.id.index(),
                        "ident": node.ident,
                        "domain": dom,
                        "x": nl.x,
                        "y": nl.y,
                        "width": nl.width,
                        "height": nl.height,
                    })
                })
                .collect();

            let domains: Vec<serde_json::Value> = layout
                .domains
                .iter()
                .map(|dl| {
                    serde_json::json!({
                        "id": dl.id.index(),
                        "name": dl.display_name,
                        "x": dl.x,
                        "y": dl.y,
                        "width": dl.width,
                        "height": dl.height,
                    })
                })
                .collect();

            let derivations: Vec<serde_json::Value> = layout
                .derivations
                .iter()
                .map(|dl| {
                    serde_json::json!({
                        "id": dl.id.index(),
                        "x": dl.x,
                        "y": dl.y,
                        "width": dl.width,
                        "height": dl.height,
                    })
                })
                .collect();

            let output = serde_json::json!({
                "edges": edges,
                "nodes": nodes,
                "domains": domains,
                "derivations": derivations,
            });

            println!(
                "{}",
                serde_json::to_string_pretty(&output).expect("JSON serialization cannot fail")
            );
        }
    }

    0
}

fn edge_description(
    graph: &mdbook_obgraph::model::types::Graph,
    edge: &mdbook_obgraph::model::types::Edge,
) -> String {
    use mdbook_obgraph::model::types::Edge;
    match edge {
        Edge::Anchor {
            parent,
            child,
            operation,
        } => {
            format!(
                "{} \u{2190} {} : {}",
                graph.nodes[parent.index()].ident,
                graph.nodes[child.index()].ident,
                operation.as_deref().unwrap_or("(none)"),
            )
        }
        Edge::Constraint {
            source_prop,
            dest_prop,
            operation,
        } => {
            let src_node = &graph.nodes[graph.properties[source_prop.index()].node.index()];
            let dst_node = &graph.nodes[graph.properties[dest_prop.index()].node.index()];
            let src_pname = &graph.properties[source_prop.index()].name;
            let dst_pname = &graph.properties[dest_prop.index()].name;
            format!(
                "{}::{} \u{2192} {}::{} [{}]",
                src_node.ident,
                src_pname,
                dst_node.ident,
                dst_pname,
                operation.as_deref().unwrap_or("(none)"),
            )
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
        } => {
            let src_node = &graph.nodes[graph.properties[source_prop.index()].node.index()];
            let src_pname = &graph.properties[source_prop.index()].name;
            format!(
                "{}::{} \u{2192} deriv_{}",
                src_node.ident,
                src_pname,
                target_deriv.index(),
            )
        }
    }
}
