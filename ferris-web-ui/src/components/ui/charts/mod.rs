//! Charts rendered as inline SVG.
//!
//! The workspace ships no chart library on purpose: a dashboard needs a line
//! and an area, and pulling a JS chart bundle into an SSR/hydrate app costs a
//! script tag, a client-only render and a second styling system. Everything
//! here is one `<svg>` computed from the same `data-chart-*` contract the
//! components already published, so a plugin (or a test) that reads those
//! attributes keeps working.
//!
//! Split into fragments: [`model`] parses the JSON props and owns the plot
//! geometry (pure, unit-tested), [`area`] is the component, and this `mod.rs`
//! only declares and re-exports.

mod area;
mod model;

#[cfg(test)]
mod tests;

pub use area::AreaChart;
pub use model::{ChartData, ChartSeries, series_names_of};

use strum::{AsRefStr, Display};

/// How the points of a series are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, AsRefStr, Default)]
pub enum ChartCurve {
    /// Quadratic smoothing through the midpoints of consecutive points.
    #[default]
    Smooth,
    /// A straight segment between every pair of points.
    Straight,
    /// A horizontal run then a vertical jump, as a step chart.
    Stepline,
}
