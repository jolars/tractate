//! Pure preparation of a source-only HTML deck and its resource dependencies.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::queries::{DependencyGraph, QueryKey, Reads};

use crate::document::*;
use crate::render::html::{self, CellContent, RenderedSlide};

mod resources;

pub(super) type FragmentCache = BTreeMap<SlideId, CachedFragment>;

#[derive(Debug, PartialEq, Eq)]
enum SlideInput {
    Title(String, ScalarStyle),
    Content(Vec<Arc<super::reuse::SemanticNode>>),
}

#[derive(Debug)]
pub(super) struct CachedFragment {
    slide: SlideInput,
    policies: BTreeMap<CellId, DisplayOptions>,
    references: BTreeMap<String, Option<html::ReferenceTarget>>,
    reads: BTreeSet<QueryKey>,
    origins: Vec<Origin>,
    result: Result<(RenderedSlide, BTreeMap<String, Resource>), Diagnostic>,
}

fn slide_input(snapshot: &super::CompilerSnapshot, slide: &Slide) -> SlideInput {
    match &slide.kind {
        SlideKind::Title(title) => {
            let YamlKind::Scalar { text, style } = &title.kind else {
                unreachable!("title is a scalar")
            };
            SlideInput::Title(text.clone(), *style)
        }
        SlideKind::Content(blocks) => SlideInput::Content(
            blocks
                .iter()
                .map(|block| {
                    Arc::clone(
                        snapshot
                            .nodes()
                            .iter()
                            .find(|node| node.id() == block.id)
                            .expect("semantic block template"),
                    )
                })
                .collect(),
        ),
    }
}

fn slide_origins(slide: &Slide) -> Vec<Origin> {
    match &slide.kind {
        SlideKind::Title(title) => vec![title.origin.clone()],
        SlideKind::Content(blocks) => blocks.iter().map(|block| block.origin.clone()).collect(),
    }
}

impl CachedFragment {
    fn bind(
        &self,
        slide: &Slide,
    ) -> Result<(RenderedSlide, BTreeMap<String, Resource>), Diagnostic> {
        let origins = slide_origins(slide);
        let bind = |origin: &Origin| {
            let range = origin.source_span().range();
            let (index, previous) = self
                .origins
                .iter()
                .enumerate()
                .find(|(_, previous)| {
                    let parent = previous.source_span().range();
                    parent.start <= range.start && range.end <= parent.end
                })
                .expect("fragment origin belongs to a slide input");
            let current = origins[index].source_span();
            super::reuse::rebind_origin(
                origin,
                current.file(),
                current.range().start as i128 - previous.source_span().range().start as i128,
            )
        };
        match &self.result {
            Ok((rendered, resources)) => {
                let mut rendered = rendered.clone();
                rendered.origin = slide.origin.derived("html-section");
                let resources = resources
                    .iter()
                    .map(|(path, resource)| {
                        let mut resource = resource.clone();
                        resource.origin = bind(&resource.origin);
                        (path.clone(), resource)
                    })
                    .collect();
                Ok((rendered, resources))
            }
            Err(diagnostic) => {
                let mut diagnostic = diagnostic.clone();
                diagnostic.primary = bind(&diagnostic.primary);
                for related in &mut diagnostic.related {
                    related.origin = bind(&related.origin);
                }
                Err(diagnostic)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledHtml {
    pub title: String,
    pub slides: Vec<RenderedSlide>,
    pub resources: Vec<Resource>,
}

#[derive(Debug, Clone)]
pub(crate) struct Resource {
    pub source: String,
    pub destination: String,
    pub origin: Origin,
}

/// A memoized query outcome retains dependencies even when preparation fails.
#[derive(Debug)]
pub(super) struct HtmlCompilation {
    pub result: Result<CompiledHtml, Vec<Diagnostic>>,
    pub dependencies: DependencyGraph,
}

pub(crate) fn compile(
    snapshot: &super::CompilerSnapshot,
    render_options: super::RenderOptions,
) -> Result<CompiledHtml, Vec<Diagnostic>> {
    snapshot.html(render_options).result.clone()
}

pub(super) fn prepare(
    snapshot: &super::CompilerSnapshot,
    render_options: super::RenderOptions,
) -> HtmlCompilation {
    let mut dependencies = snapshot.dependencies().clone();
    let reads = Reads::default();
    let result = compile_deck(snapshot, render_options, &mut dependencies, &reads);
    dependencies.finish(QueryKey::HtmlDeck(render_options), reads);
    let compilation = HtmlCompilation {
        result,
        dependencies,
    };
    debug_assert!(compilation.dependencies.is_closed());
    compilation
}

fn compile_deck(
    snapshot: &super::CompilerSnapshot,
    render_options: super::RenderOptions,
    graph: &mut DependencyGraph,
    deck: &Reads,
) -> Result<CompiledHtml, Vec<Diagnostic>> {
    let mut diagnostics = deck
        .read(QueryKey::Inspection, snapshot.inspection())
        .diagnostics
        .clone();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let presentation = deck.read(QueryKey::SlideLayout, snapshot.presentation());

    let defaults = graph.query(QueryKey::DisplayDefaults, |reads| {
        let mut defaults = DisplayOptions::default();
        let mut diagnostics = Vec::new();
        for metadata in reads.read(QueryKey::Metadata, &presentation.metadata) {
            defaults.apply_metadata(metadata, &mut diagnostics);
        }
        reads
            .read(QueryKey::SlideLayout, presentation)
            .visit_blocks(&mut |block| {
                if let BlockKind::Metadata(metadata) = &block.kind {
                    defaults.apply_metadata(
                        reads.read(QueryKey::SemanticBlock(block.id), metadata),
                        &mut diagnostics,
                    );
                }
            });
        (defaults, diagnostics)
    });
    let (defaults, default_diagnostics) = deck.read(QueryKey::DisplayDefaults, defaults);
    diagnostics.extend(default_diagnostics);
    let mut policies = BTreeMap::new();
    presentation.visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind {
            let options = graph.query(QueryKey::CellPolicy(cell.id), |reads| {
                let cell = reads.read(QueryKey::SemanticBlock(block.id), cell);
                let mut options = reads.read(QueryKey::DisplayDefaults, defaults);
                let mut diagnostics = Vec::new();
                if let Some(preamble) = &cell.preamble {
                    options.apply(preamble, &mut diagnostics);
                }
                (options, diagnostics)
            });
            let (options, cell_diagnostics) = deck.read(QueryKey::CellPolicy(cell.id), options);
            diagnostics.extend(cell_diagnostics);
            policies.insert(cell.id, options);
        }
    });
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    presentation.visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind
            && policies[&cell.id].eval
        {
            // Result availability is a pure input. No runner or result store
            // exists yet, so every enabled cell currently fails this check.
            let cell = deck.read(QueryKey::SemanticBlock(block.id), cell);
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

    let title = graph.query(QueryKey::HtmlTitle, |reads| {
        let title = reads
            .read(QueryKey::SlideLayout, &presentation.slides)
            .iter()
            .find_map(|slide| match &slide.kind {
                SlideKind::Title(title) => Some(html::title_text(
                    reads.read(QueryKey::Slide(slide.id), title),
                )),
                _ => None,
            })
            .transpose()?;
        Ok::<_, Diagnostic>(title.unwrap_or_else(|| {
            reads
                .read(QueryKey::Source, snapshot.source())
                .path()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Tractate presentation".into())
        }))
    });
    let title = deck.read(QueryKey::HtmlTitle, title).map_err(|d| vec![d])?;
    let references = reference_queries(presentation, graph);
    let mut resources = BTreeMap::new();
    let mut slides = Vec::new();
    for slide in &presentation.slides {
        let key = QueryKey::HtmlSlide(slide.id);
        let input = slide_input(snapshot, slide);
        let mut cache = snapshot.fragments().lock().expect("fragment cache lock");
        let cached = cache.get(&slide.id).filter(|cached| {
            cached.slide == input
                && cached
                    .policies
                    .iter()
                    .all(|(id, policy)| policies.get(id) == Some(policy))
                && cached
                    .references
                    .iter()
                    .all(|(label, target)| references.get(label) == Some(target))
        });
        let fragment = if let Some(cached) = cached {
            graph.reuse(key.clone(), cached.reads.clone());
            cached.bind(slide)
        } else {
            let fragment = graph.query(key.clone(), |reads| {
                let mut resources = BTreeMap::new();
                let rendered = html::render_slide(
                    reads.read(QueryKey::Slide(slide.id), slide),
                    |cell| {
                        let options = reads.read(QueryKey::CellPolicy(cell.id), policies[&cell.id]);
                        let mut content = CellContent::source_only();
                        content.show_source = options.echo && options.include;
                        Ok(content)
                    },
                    |label| {
                        reads.read(
                            QueryKey::Reference(label.to_owned()),
                            references.get(label).cloned().flatten(),
                        )
                    },
                    |destination, origin| resources::resolve(destination, origin, &mut resources),
                )?;
                Ok::<_, Diagnostic>((rendered, resources))
            });
            let reads = graph.inputs(&key).clone();
            cache.insert(
                slide.id,
                CachedFragment {
                    slide: input,
                    policies: reads
                        .iter()
                        .filter_map(|key| match key {
                            QueryKey::CellPolicy(id) => Some((*id, policies[id])),
                            _ => None,
                        })
                        .collect(),
                    references: reads
                        .iter()
                        .filter_map(|key| match key {
                            QueryKey::Reference(label) => {
                                Some((label.clone(), references[label].clone()))
                            }
                            _ => None,
                        })
                        .collect(),
                    reads,
                    origins: slide_origins(slide),
                    result: fragment.clone(),
                },
            );
            fragment
        };
        let (rendered, required) = deck
            .read(QueryKey::HtmlSlide(slide.id), fragment)
            .map_err(|d| vec![d])?;
        slides.push(rendered);
        for (path, resource) in required {
            resources.entry(path).or_insert(resource);
        }
    }
    Ok(CompiledHtml {
        title,
        slides,
        resources: resources.into_values().collect(),
    })
}

fn reference_queries(
    presentation: &Presentation,
    graph: &mut DependencyGraph,
) -> BTreeMap<String, Option<html::ReferenceTarget>> {
    // Index membership is a projection of the parsed document. Each lookup
    // separately reads its winning definition, including definitions in slides.
    let definitions = graph.query(QueryKey::ReferenceIndex, |reads| {
        let mut definitions = BTreeMap::new();
        reads
            .read(QueryKey::SourceDocument, presentation)
            .visit_blocks(&mut |block| {
                if let BlockKind::ReferenceDefinition(reference) = &block.kind {
                    definitions
                        .entry(reference.label.clone())
                        .or_insert_with(|| {
                            (
                                block.id,
                                (reference.destination.clone(), reference.title.clone()),
                            )
                        });
                }
            });
        definitions
    });
    let mut labels = BTreeSet::new();
    let mut find_labels = |inline: &Inline<PresentedCell>| {
        if let InlineKind::Link(link) | InlineKind::Image(link) = &inline.kind
            && let Some(label) = &link.reference
        {
            labels.insert(label.clone());
        }
    };
    for block in &presentation.preamble {
        block.visit(&mut |_| {}, &mut find_labels);
    }
    for slide in &presentation.slides {
        if let SlideKind::Content(blocks) = &slide.kind {
            for block in blocks {
                block.visit(&mut |_| {}, &mut find_labels);
            }
        }
    }
    labels
        .into_iter()
        .map(|label| {
            let target = graph.query(QueryKey::Reference(label.clone()), |reads| {
                reads
                    .read(QueryKey::ReferenceIndex, definitions.get(&label))
                    .map(|(id, reference)| {
                        let reference = reads.read(QueryKey::SemanticBlock(*id), reference);
                        reference.clone()
                    })
            });
            (label, target)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
