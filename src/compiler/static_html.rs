//! Pure preparation of a source-only HTML deck and its resource dependencies.

use std::collections::BTreeMap;

use crate::document::*;
use crate::render::html::{self, CellContent, RenderedSlide};

mod resources;

pub(crate) struct CompiledHtml {
    pub title: String,
    pub slides: Vec<RenderedSlide>,
    pub resources: Vec<Resource>,
}

pub(crate) struct Resource {
    pub source: String,
    pub destination: String,
    pub origin: Origin,
}

pub(crate) fn compile(
    source: SourceFile,
    render_options: super::RenderOptions,
) -> Result<CompiledHtml, Vec<Diagnostic>> {
    let lowered = crate::parser::lower(source.clone());
    let mut diagnostics = lowered.diagnostics;
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    diagnostics.extend(super::validation::validate(&lowered.document));
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut defaults = DisplayOptions::default();
    for metadata in &lowered.document.metadata {
        defaults.apply_metadata(metadata, &mut diagnostics);
    }
    lowered.document.visit_blocks(&mut |block| {
        if let BlockKind::Metadata(metadata) = &block.kind {
            defaults.apply_metadata(metadata, &mut diagnostics);
        }
    });
    let mut policies = BTreeMap::new();
    lowered.document.visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind {
            let mut options = defaults;
            if let Some(preamble) = &cell.preamble {
                options.apply(preamble, &mut diagnostics);
            }
            policies.insert(cell.id, options);
        }
    });
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    lowered.document.visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind
            && policies[&cell.id].eval
        {
            // No runner or result store exists yet, so every enabled cell's
            // result is unavailable regardless of its presentation settings.
            let (code, message) = if render_options.no_execute {
                (
                    DiagnosticCode::ResultUnavailable,
                    "required result is unavailable; `--no-execute` forbids execution, and result caching is not implemented yet",
                )
            } else {
                (
                    DiagnosticCode::ExecutionUnavailable,
                    "this cell requires execution, which is not implemented yet; use `eval: false` to display source only",
                )
            };
            diagnostics.push(error(code, &cell.origin, message));
        }
    });
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let presentation =
        super::presentation::lower_presentation(super::build_presentation(lowered.document));
    let title = presentation
        .slides
        .iter()
        .find_map(|slide| match &slide.kind {
            SlideKind::Title(title) => Some(html::title_text(title)),
            _ => None,
        })
        .transpose()
        .map_err(|d| vec![d])?
        .unwrap_or_else(|| {
            source
                .path()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Tractate presentation".into())
        });
    let mut resources = BTreeMap::new();
    let slides = html::render_with_resources(
        &presentation,
        |cell| {
            let options = policies[&cell.id];
            let mut content = CellContent::source_only();
            content.show_source = options.echo && options.include;
            Ok(content)
        },
        |destination, origin| resources::resolve(destination, origin, &mut resources),
    )
    .map_err(|d| vec![d])?;
    Ok(CompiledHtml {
        title,
        slides,
        resources: resources.into_values().collect(),
    })
}

#[derive(Clone, Copy)]
struct DisplayOptions {
    eval: bool,
    echo: bool,
    include: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            eval: true,
            echo: true,
            include: true,
        }
    }
}

impl DisplayOptions {
    fn apply_metadata(&mut self, metadata: &Metadata, diagnostics: &mut Vec<Diagnostic>) {
        if let Some(entries) = metadata.value.mapping() {
            for entry in entries {
                if matches!(&entry.key.kind, YamlKind::Scalar { text, .. } if text == "execute") {
                    self.apply(&entry.value, diagnostics);
                }
            }
        }
    }

    fn apply(&mut self, mapping: &YamlValue, diagnostics: &mut Vec<Diagnostic>) {
        for entry in mapping.mapping().unwrap_or_default() {
            let YamlKind::Scalar { text: key, .. } = &entry.key.kind else {
                continue;
            };
            let target = match key.as_str() {
                "eval" => &mut self.eval,
                "echo" => &mut self.echo,
                "include" => &mut self.include,
                _ => continue,
            };
            let boolean = match &entry.value.kind {
                YamlKind::Scalar {
                    text,
                    style: ScalarStyle::Plain,
                } => match text.as_str() {
                    "true" | "True" | "TRUE" => Some(true),
                    "false" | "False" | "FALSE" => Some(false),
                    _ => None,
                },
                _ => None,
            };
            if let Some(value) = boolean {
                *target = value;
            } else {
                diagnostics.push(error(
                    DiagnosticCode::InvalidOptionValue,
                    &entry.value.origin,
                    format!("`{key}` must be a YAML Boolean"),
                ));
            }
        }
    }
}

pub(crate) fn error(
    code: DiagnosticCode,
    origin: &Origin,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code,
        message: message.into(),
        primary: origin.clone(),
        related: Vec::new(),
    }
}
