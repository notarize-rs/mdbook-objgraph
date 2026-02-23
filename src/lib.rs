#![forbid(unsafe_code)]

pub mod layout;
pub mod model;
pub mod parse;
pub mod render;

use thiserror::Error;

/// Errors produced by the obgraph pipeline.
#[derive(Debug, Error)]
pub enum ObgraphError {
    #[error("parse error at line {line}, col {col}: {message}")]
    Parse {
        line: usize,
        col: usize,
        message: String,
    },

    #[error("validation error: {0}")]
    Validation(String),

    #[error("layout error: {0}")]
    Layout(String),
}

/// Process an obgraph definition string into a self-contained HTML/SVG fragment.
///
/// This is the top-level library API composing all pipeline phases:
/// parse -> validate -> state propagation -> layout -> render.
pub fn process(input: &str) -> Result<String, ObgraphError> {
    let ast = parse::parse(input)?;
    let graph = model::build(ast)?;
    let trust = model::state::propagate(&graph);
    let layout_result = layout::layout(&graph)?;
    Ok(render::render(&graph, &layout_result, &trust))
}
