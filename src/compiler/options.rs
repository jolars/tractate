//! Option values shared by pure planning and presentation passes.

use crate::document::{
    Diagnostic, DiagnosticCode, Metadata, ScalarStyle, YamlKind, YamlValue, error,
};

pub(super) fn execute_mappings(metadata: &Metadata) -> impl Iterator<Item = &YamlValue> {
    metadata
        .value
        .mapping()
        .unwrap_or_default()
        .iter()
        .filter(
            |entry| matches!(&entry.key.kind, YamlKind::Scalar { text, .. } if text == "execute"),
        )
        .map(|entry| &entry.value)
}

pub(super) fn boolean(
    key: &str,
    value: &YamlValue,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<bool> {
    let boolean = match &value.kind {
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
    if boolean.is_none() {
        diagnostics.push(error(
            DiagnosticCode::InvalidOptionValue,
            &value.origin,
            format!("`{key}` must be a YAML Boolean"),
        ));
    }
    boolean
}
