//! Validate declarations before identity assignment or option resolution.

use std::collections::HashMap;

use crate::document::*;

#[cfg(test)]
pub(super) fn validate(document: &SourceDocument) -> Vec<Diagnostic> {
    declarations(document).diagnostics
}

pub(super) struct Declarations {
    pub diagnostics: Vec<Diagnostic>,
    pub labels: Vec<LabelDeclaration>,
}

pub(super) struct LabelDeclaration {
    pub label: String,
    pub origin: Origin,
    pub node: NodeId,
    pub cell: Option<CellId>,
}

pub(super) fn declarations(document: &SourceDocument) -> Declarations {
    let mut validator = Validator::default();
    let mut attributes = Vec::new();
    let mut defaults = HashMap::new();
    for metadata in &document.metadata {
        validator.metadata(metadata, &mut defaults);
    }
    document.visit_nodes(
        &mut |block| {
            validator.attributes(&block.attributes, block.id);
            match &block.kind {
                BlockKind::Cell(cell) => validator.cell(cell, block.id),
                BlockKind::Metadata(metadata) => validator.metadata(metadata, &mut defaults),
                _ => {}
            }
        },
        &mut |inline| attributes.push((inline.attributes.clone(), inline.id)),
    );
    for (attributes, node) in attributes {
        validator.attributes(&attributes, node);
    }
    // Inline attributes and cell options are gathered by separate visitors.
    // Sort declarations so every duplicate points to the first QMD occurrence.
    validator
        .labels
        .sort_by_key(|label| label.origin.source_span().range().start);
    let mut labels = HashMap::new();
    let declarations = std::mem::take(&mut validator.labels);
    for label in &declarations {
        validator.unique(
            &mut labels,
            &label.label,
            &label.origin,
            DiagnosticCode::DuplicateLabel,
            "label",
        );
    }
    validator
        .diagnostics
        .sort_by_key(|d| d.primary.source_span().range().start);
    Declarations {
        diagnostics: validator.diagnostics,
        labels: declarations,
    }
}

#[derive(Default)]
struct Validator {
    diagnostics: Vec<Diagnostic>,
    labels: Vec<LabelDeclaration>,
}

impl Validator {
    fn error(&mut self, code: DiagnosticCode, origin: &Origin, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code,
            message: message.into(),
            primary: origin.clone(),
            related: Vec::new(),
        });
    }

    fn unique(
        &mut self,
        seen: &mut HashMap<String, Origin>,
        name: &str,
        origin: &Origin,
        code: DiagnosticCode,
        kind: &str,
    ) -> bool {
        if let Some(first) = seen.get(name) {
            self.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code,
                message: format!("duplicate {kind} `{name}`"),
                primary: origin.clone(),
                related: vec![RelatedOrigin {
                    message: format!("first declaration of {kind} `{name}`"),
                    origin: first.clone(),
                }],
            });
            false
        } else {
            seen.insert(name.to_owned(), origin.clone());
            true
        }
    }

    fn attributes(&mut self, attributes: &[Attribute], node: NodeId) {
        for attribute in attributes {
            if let AttributeKind::Identifier(label) = &attribute.kind {
                self.labels.push(LabelDeclaration {
                    label: label.clone(),
                    origin: attribute.origin.clone(),
                    node,
                    cell: None,
                });
            }
        }
    }

    fn metadata(&mut self, metadata: &Metadata, defaults: &mut HashMap<String, Origin>) {
        if let Some(entries) = metadata.value.mapping() {
            for entry in entries {
                if scalar_text(&entry.key) == Some("execute") {
                    self.unique(
                        defaults,
                        "execute",
                        &entry.key.origin,
                        DiagnosticCode::DuplicateOption,
                        "option",
                    );
                    self.yaml_syntax(&entry.key);
                    self.options(&entry.value, None);
                }
            }
        }
    }

    fn cell(&mut self, cell: &Cell, node: NodeId) {
        for option in &cell.options {
            if option.source == OptionSource::Inline {
                self.error(
                    DiagnosticCode::UnsupportedOptionSyntax,
                    &option.origin,
                    "inline cell options and labels are unsupported; use leading `#|` YAML",
                );
            }
        }
        if let Some(preamble) = &cell.preamble {
            self.options(preamble, Some((node, cell.id)));
        }
    }

    fn options(&mut self, value: &YamlValue, target: Option<(NodeId, CellId)>) {
        let defaults = target.is_none();
        self.yaml_syntax(value);
        let Some(entries) = value.mapping() else {
            // Unsupported syntax has already received a precise diagnostic.
            if !matches!(value.kind, YamlKind::Unsupported(_)) && value.properties.is_empty() {
                self.error(
                    DiagnosticCode::InvalidOptionMapping,
                    &value.origin,
                    if defaults {
                        "`execute` must be a YAML mapping"
                    } else {
                        "cell options must be a YAML mapping"
                    },
                );
            }
            return;
        };
        let mut seen = HashMap::new();
        for entry in entries {
            let Some(key) = scalar_text(&entry.key) else {
                self.error(
                    DiagnosticCode::UnknownOption,
                    &entry.key.origin,
                    "option keys must be strings from the supported option vocabulary",
                );
                continue;
            };
            let unique = self.unique(
                &mut seen,
                key,
                &entry.key.origin,
                DiagnosticCode::DuplicateOption,
                "option",
            );
            match key {
                "<<" => continue,
                "label" | "fig-cap" if defaults => {
                    self.error(
                        DiagnosticCode::WrongOptionScope,
                        &entry.key.origin,
                        format!("`{key}` is only supported in cell options"),
                    );
                }
                "label"
                    if unique
                        && entry.key.properties.is_empty()
                        && entry.value.properties.is_empty() =>
                {
                    if let Some(label) = name(&entry.value) {
                        let (node, cell) = target.expect("cell label scope");
                        self.labels.push(LabelDeclaration {
                            label,
                            origin: entry.value.origin.clone(),
                            node,
                            cell: Some(cell),
                        });
                    } else if !matches!(entry.value.kind, YamlKind::Unsupported(_)) {
                        self.error(DiagnosticCode::InvalidLabel, &entry.value.origin, "`label` must be a nonempty string without whitespace or control characters");
                    }
                }
                "label" | "eval" | "echo" | "include" | "results" | "session" | "cache"
                | "inputs" | "fig-width" | "fig-height" | "fig-cap" => {}
                _ => self.error(
                    DiagnosticCode::UnknownOption,
                    &entry.key.origin,
                    format!("unsupported option `{key}`"),
                ),
            }
        }
    }

    fn yaml_syntax(&mut self, value: &YamlValue) {
        for property in &value.properties {
            self.error(
                DiagnosticCode::UnsupportedOptionSyntax,
                &property.origin,
                "YAML tags, anchors, and aliases are unsupported in options",
            );
        }
        match &value.kind {
            YamlKind::Mapping { entries, .. } => {
                for entry in entries {
                    if scalar_text(&entry.key) == Some("<<") {
                        self.error(
                            DiagnosticCode::UnsupportedOptionSyntax,
                            &entry.key.origin,
                            "YAML merge keys are unsupported in options",
                        );
                    }
                    self.yaml_syntax(&entry.key);
                    self.yaml_syntax(&entry.value);
                }
            }
            YamlKind::Sequence { items, .. } => {
                for item in items {
                    self.yaml_syntax(item);
                }
            }
            YamlKind::Unsupported(_) if value.properties.is_empty() => {
                self.error(
                    DiagnosticCode::UnsupportedOptionSyntax,
                    &value.origin,
                    "unsupported YAML syntax in options",
                );
            }
            _ => {}
        }
    }
}

fn scalar_text(value: &YamlValue) -> Option<&str> {
    match &value.kind {
        YamlKind::Scalar { text, .. } => Some(text),
        _ => None,
    }
}

pub(super) fn name(value: &YamlValue) -> Option<String> {
    let YamlKind::Scalar { text, style } = &value.kind else {
        return None;
    };
    if *style == ScalarStyle::Plain && plain_non_string(text) {
        return None;
    }
    let text = match style {
        ScalarStyle::Literal | ScalarStyle::Folded => block_name(text, &value.origin)?,
        _ => text.clone(),
    };
    (!text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control())).then_some(text)
}

fn plain_non_string(text: &str) -> bool {
    matches!(
        text,
        "" | "~"
            | "null"
            | "Null"
            | "NULL"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | ".inf"
            | ".Inf"
            | ".INF"
            | "+.inf"
            | "+.Inf"
            | "+.INF"
            | "-.inf"
            | "-.Inf"
            | "-.INF"
            | ".nan"
            | ".NaN"
            | ".NAN"
    ) || text
        .strip_prefix("0x")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        || text.strip_prefix("0o").is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|b| matches!(b, b'0'..=b'7'))
        })
        || (text.bytes().any(|b| b.is_ascii_digit()) && text.parse::<f64>().is_ok())
}

/// A valid block name has one content line and strips its final line break.
/// Folding multiple nonempty lines would introduce forbidden whitespace.
fn block_name(text: &str, origin: &Origin) -> Option<String> {
    let mut lines = text.lines();
    let header = lines.next()?.split('#').next()?.trim();
    if !header.contains('-') {
        return None;
    }
    let first = lines.next()?;
    let indentation = header.bytes().find(|b| matches!(b, b'1'..=b'9'));
    let content_indent = if let Some(indentation) = indentation {
        // Hashpipe declarations may have a Markdown container prefix. Measure
        // explicit indentation relative to the YAML rather than its host.
        let span = origin.source_span();
        let prefix = span.file().text()[..span.range().start]
            .rsplit('\n')
            .next()?;
        let yaml = if let Some((_, yaml)) = prefix.split_once("#|") {
            yaml.strip_prefix(' ').unwrap_or(yaml)
        } else {
            prefix
        };
        let parent_indent = yaml.bytes().take_while(|&b| b == b' ').count();
        parent_indent + usize::from(indentation - b'0')
    } else {
        first.bytes().take_while(|&b| b == b' ').count()
    };
    if lines.any(|line| line.len() > content_indent || line.bytes().any(|b| b != b' ')) {
        return None;
    }
    let first = first.get(content_indent..)?;
    Some(first.to_owned())
}
