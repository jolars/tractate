//! Panache integration and the boundary for source semantic lowering.

use panache_parser::{Flavor, ParsedDocument, ParserOptions, parse_document};

mod lowering;
mod yaml;

pub(crate) use lowering::lower;

#[cfg(test)]
mod tests;

/// Parse source with the same Quarto syntax settings for every compiler entry point.
fn parse(source: &str) -> ParsedDocument {
    parse_document(source, Some(ParserOptions::for_flavor(Flavor::Quarto)))
}
