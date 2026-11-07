//! glot Edge: HTML translated by segments (see each module).
pub mod hydration;
pub mod protect;
pub mod segment;
pub mod seo;
pub mod tm;

pub use segment::{Placeholder, Segment, SegmentCtx, SegmentOpts, segment, translate_html};
