//! Presentation backends.
//!
//! Backends consume `document::Presentation`, whose cell leaves contain source
//! display data and `ResultSlot` references. Ordered output is supplied through
//! a separately validated result store; backends do not own execution. The
//! execution coordinator's read-only cell views expose progress, diagnostics,
//! current results, and explicitly stale last-known-good output. HTML
//! sections and Markdown body rendering live here. Cell visibility and validated
//! result content are explicit caller inputs; result lookup is subsequent work.

pub(crate) mod html;
