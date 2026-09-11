//! Presentation backends.
//!
//! Backends consume `document::Presentation`, whose cell leaves contain source
//! display data and `ResultSlot` references. Ordered output is supplied through
//! a separately validated result store; backends do not own execution. HTML
//! emission and result lookup are subsequent compiler stages.
