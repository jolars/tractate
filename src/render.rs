//! Presentation backends.
//!
//! Backends consume `document::Presentation`, whose cell leaves contain source
//! display data and `ResultSlot` references. Ordered output is supplied through
//! a separately validated result store; backends do not own execution. HTML
//! section assembly lives here; Markdown body rendering and result lookup are
//! subsequent compiler stages.

pub(crate) mod html;
