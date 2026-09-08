//! Panache integration and the boundary for source semantic lowering.

use panache_parser::{Flavor, ParsedDocument, ParserOptions, parse_document};

/// Parse source with the same Quarto syntax settings for every compiler entry point.
pub(crate) fn parse(source: &str) -> ParsedDocument {
    parse_document(source, Some(ParserOptions::for_flavor(Flavor::Quarto)))
}
