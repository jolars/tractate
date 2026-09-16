//! Map Markdown destinations to portable output paths without filesystem I/O.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{Resource, error};
use crate::document::{Diagnostic, DiagnosticCode, Origin};

pub(super) fn resolve(
    destination: &str,
    origin: &Origin,
    resources: &mut BTreeMap<String, Resource>,
) -> Result<String, Diagnostic> {
    let scheme = destination.split_once(':').is_some_and(|(prefix, _)| {
        prefix.starts_with(|c: char| c.is_ascii_alphabetic())
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    });
    if destination.starts_with("//") || scheme {
        return Ok(destination.to_owned());
    }
    let suffix_start = destination.find(['?', '#']).unwrap_or(destination.len());
    let (path, suffix) = destination.split_at(suffix_start);
    if path.is_empty() {
        return Ok(destination.to_owned());
    }
    let invalid = || {
        error(
            DiagnosticCode::ResourceUnavailable,
            origin,
            format!("invalid local resource URL `{destination}`"),
        )
    };
    let path = decode_path(path).ok_or_else(invalid)?;
    let resource = resources.entry(path.clone()).or_insert_with(|| {
        // Names depend on the source path, so query strings and fragments share
        // one asset while traversal and generated-file collisions are impossible.
        let digest = format!("{:x}", Sha256::digest(path.as_bytes()));
        let extension = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        Resource {
            source: path,
            destination: format!("resources/{digest}{extension}"),
            origin: origin.clone(),
        }
    });
    Ok(format!("{}{suffix}", resource.destination))
}

fn decode_path(path: &str) -> Option<String> {
    let mut bytes = path.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        });
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.contains(['\0', '\\'])).then_some(decoded)
}
