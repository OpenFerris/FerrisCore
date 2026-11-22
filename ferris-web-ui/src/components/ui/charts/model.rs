//! Chart input parsing and plot geometry.
//!
//! Everything here is pure: the component turns these numbers into SVG, and the
//! tests in [`super::tests`] pin the behaviour without rendering anything. That
//! split is deliberate — a chart that silently mis-scales is a drawing bug, but
//! a chart that parses `[]` as "one series of zeroes" is a lying-data bug, and
//! only the second one is worth a test.
//!
//! ## Input contract
//!
//! - `json_values`: a JSON array of numbers (`[1,2,3]`) for one series, or an
//!   array of arrays (`[[1,2],[3,4]]`) for several. Numbers may be strings
//!   (`"1"`), which is what the engine's JSON `data` blob produces.
//! - `json_labels`: a JSON array of strings (`["2026-11-01", …]`). Numbers are
//!   rendered with their own `to_string`.
//! - `series_names`: a JSON array (`["Creados","Enviados"]`) or a
//!   comma-separated list. Optional: an unnamed series is still plotted.
//! - `stack_type`: `"normal"` stacks the series, `"percent"` stacks them into a
//!   0–100 column.

use serde_json::Value;
use std::fmt::Write as _;

/// Width of the `viewBox`. The SVG stretches to its CSS box, so this is a
/// coordinate space, not a pixel size.
pub const VIEW_WIDTH: f64 = 600.0;
/// Height of the `viewBox`; see [`VIEW_WIDTH`].
pub const VIEW_HEIGHT: f64 = 200.0;

/// Room for the y-axis labels inside the viewBox.
pub const PAD_LEFT: f64 = 34.0;
/// Keeps the last point off the right edge.
pub const PAD_RIGHT: f64 = 6.0;
/// Keeps the tallest point off the top edge.
pub const PAD_TOP: f64 = 8.0;
/// Room for the x-axis labels under the plot.
pub const PAD_BOTTOM: f64 = 20.0;

/// Horizontal grid lines**, intervals** drawn on the y axis (one more line than
/// this is rendered, because the baseline counts).
pub const Y_TICKS: usize = 5;

/// At most this many x labels: the first, the last and evenly spaced ones.
pub const MAX_X_LABELS: usize = 6;

/// One plotted series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartSeries {
    /// Legend name, when the caller declared one.
    pub name: Option<String>,
    pub values: Vec<f64>,
}

/// Everything the SVG needs: the labels and the parsed series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartData {
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
}

impl ChartData {
    /// Parse the component's four JSON-ish props.
    ///
    /// Malformed input is **empty**, never a fabricated series: a chart that
    /// draws a flat zero line for `"{oops}"` claims there was no activity,
    /// which is a different statement from "the data could not be read". The
    /// component renders that as its empty state.
    #[must_use]
    pub fn parse(values: &str, labels: &str, series_names: Option<&str>, stack: &str) -> Self {
        let mut series = parse_series(values, series_names);
        if series.is_empty() {
            return Self::default();
        }
        if is_stack(stack) {
            let percent = stack.eq_ignore_ascii_case("percent");
            apply_stack(&mut series, percent);
        }
        let mut labels = parse_labels(labels);
        let width = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        labels.resize(width, String::new());
        Self { labels, series }
    }

    /// Whether there is anything to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.series.iter().all(|series| series.values.is_empty())
    }

    /// The widest series; every series is drawn over this many positions.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0)
    }

    /// The largest value plotted, or `0.0` when there is none.
    #[must_use]
    pub fn max(&self) -> f64 {
        self.series
            .iter()
            .flat_map(|series| series.values.iter().copied())
            .filter(|value| value.is_finite())
            .fold(0.0_f64, f64::max)
    }

    /// The rounded-up top of the y axis.
    #[must_use]
    pub fn axis_max(&self) -> f64 {
        nice_max(self.max())
    }

    /// X coordinate of `index`, in viewBox units.
    #[must_use]
    pub fn x(&self, index: usize) -> f64 {
        let count = self.point_count();
        if count <= 1 {
            return (PAD_LEFT + VIEW_WIDTH - PAD_RIGHT) / 2.0;
        }
        let span = VIEW_WIDTH - PAD_LEFT - PAD_RIGHT;
        // `count >= 2` here, so the division cannot be by zero. Axis
        // coordinates are `f64` by definition; the indices are tiny.
        #[allow(clippy::cast_precision_loss)]
        let position = index as f64 / (count - 1) as f64;
        span.mul_add(position, PAD_LEFT)
    }

    /// Y coordinate of `value`, in viewBox units (0 at the bottom).
    #[must_use]
    pub fn y(&self, value: f64) -> f64 {
        let top = PAD_TOP;
        let bottom = VIEW_HEIGHT - PAD_BOTTOM;
        let max = self.axis_max();
        let ratio = if max > 0.0 {
            (value / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (bottom - top).mul_add(-ratio, bottom)
    }

    /// The `(x, y)` of one point, or `None` for a series that short.
    #[must_use]
    pub fn point(&self, series: usize, index: usize) -> Option<(f64, f64)> {
        let value = *self.series.get(series)?.values.get(index)?;
        Some((self.x(index), self.y(value)))
    }

    /// Every point of one series, as drawn.
    #[must_use]
    pub fn points(&self, series: usize) -> Vec<(f64, f64)> {
        self.series
            .get(series)
            .map(|s| {
                (0..s.values.len())
                    .map(|index| (self.x(index), self.y(s.values[index])))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// SVG path through the points of one series.
    #[must_use]
    pub fn line_path(&self, series: usize, curve: super::ChartCurve) -> String {
        path(&self.points(series), curve)
    }

    /// SVG path of the filled area under one series: the line, closed down to
    /// the baseline.
    #[must_use]
    pub fn area_path(&self, series: usize, curve: super::ChartCurve) -> String {
        let points = self.points(series);
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return String::new();
        };
        let baseline = self.y(0.0);
        let mut path = path(&points, curve);
        // Writing into the buffer cannot fail; the error is `fmt::Error`,
        // which a `String` never returns.
        let _ = write!(
            path,
            " L{:.2} {:.2} L{:.2} {:.2} Z",
            last.0, baseline, first.0, baseline
        );
        path
    }

    /// `(label, y)` for every y-axis grid line, bottom-up.
    #[must_use]
    pub fn y_ticks(&self) -> Vec<(String, f64)> {
        let max = self.axis_max();
        (0..=Y_TICKS)
            .map(|tick| {
                // Tick counts are single digits; the conversion is exact.
                #[allow(clippy::cast_precision_loss)]
                let value = max * tick as f64 / Y_TICKS as f64;
                (format_tick(value), self.y(value))
            })
            .collect()
    }

    /// `(index, label, x)` for the x labels worth drawing: first, last and
    /// evenly spaced ones between them, never more than [`MAX_X_LABELS`].
    #[must_use]
    pub fn x_labels(&self) -> Vec<(usize, String, f64)> {
        let count = self.point_count();
        if count == 0 {
            return Vec::new();
        }
        let wanted = MAX_X_LABELS.min(count);
        let mut seen: Vec<usize> = Vec::with_capacity(wanted);
        for step in 0..wanted {
            let index = if wanted == 1 {
                0
            } else {
                // Both ends are included, so the first and last dates are
                // always readable.
                step * (count - 1) / (wanted - 1)
            };
            if !seen.contains(&index) {
                seen.push(index);
            }
        }
        seen.into_iter()
            .filter_map(|index| {
                let label = self.labels.get(index).cloned().unwrap_or_default();
                (!label.is_empty()).then(|| (index, label, self.x(index)))
            })
            .collect()
    }

    /// Vertical markers parsed from `[{x: "<label>", title: "…"}]`.
    ///
    /// An annotation whose `x` matches no label is dropped: a marker has to
    /// point at a real position, and guessing one would put a line under the
    /// wrong date.
    #[must_use]
    pub fn annotations(&self, json: &str) -> Vec<(f64, String)> {
        let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(json) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| {
                let x = entry.get("x").and_then(label_of)?;
                let index = self.labels.iter().position(|label| *label == x)?;
                let title = entry.get("title").and_then(label_of).unwrap_or_default();
                Some((self.x(index), title))
            })
            .collect()
    }
}

/// Whether `stack` asks for stacking rather than a plain overlay.
fn is_stack(stack: &str) -> bool {
    let stack = stack.trim();
    !stack.is_empty()
        && !stack.eq_ignore_ascii_case("none")
        && !stack.eq_ignore_ascii_case("overlay")
        && stack != "false"
}

/// Round `value` up to a tick-friendly axis top (1, 2, 2.5 or 5 × a power of
/// ten), so the labels are round numbers instead of `7.0000000001`.
#[must_use]
pub fn nice_max(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        return 1.0;
    }
    let exponent = value.log10().floor();
    let base = 10f64.powf(exponent);
    let normalised = value / base;
    let step = if normalised <= 1.0 {
        1.0
    } else if normalised <= 2.0 {
        2.0
    } else if normalised <= 2.5 {
        2.5
    } else if normalised <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * base
}

/// One axis label: integers stay integers, fractions keep one decimal.
fn format_tick(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// `M…`/`L`/`H`/`V`/`Q` path through `points`.
fn path(points: &[(f64, f64)], curve: super::ChartCurve) -> String {
    use super::ChartCurve;
    let Some((first, rest)) = points.split_first() else {
        return String::new();
    };
    let mut out = format!("M{:.2} {:.2}", first.0, first.1);
    if rest.is_empty() {
        return out;
    }
    match curve {
        ChartCurve::Straight => {
            for (x, y) in rest {
                let _ = write!(out, " L{x:.2} {y:.2}");
            }
        }
        ChartCurve::Stepline => {
            for (x, y) in rest {
                let _ = write!(out, " H{x:.2} V{y:.2}");
            }
        }
        ChartCurve::Smooth => {
            // One quadratic per gap, controlled by the point it starts at and
            // ending at the midpoint: no overshoot, and the curve passes
            // through every midpoint while the final `L` lands on the last
            // sample.
            let mut previous = *first;
            for point in rest {
                let mid_x = f64::midpoint(previous.0, point.0);
                let mid_y = f64::midpoint(previous.1, point.1);
                let _ = write!(
                    out,
                    " Q{:.2} {:.2} {mid_x:.2} {mid_y:.2}",
                    previous.0, previous.1
                );
                previous = *point;
            }
            let _ = write!(out, " L{:.2} {:.2}", previous.0, previous.1);
        }
    }
    out
}

/// Parse `json_labels` into display strings.
fn parse_labels(raw: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(entries)) => entries.iter().filter_map(label_of).collect(),
        _ => Vec::new(),
    }
}

/// Parse `json_values` into one or more series, naming them from
/// `series_names`.
fn parse_series(raw: &str, series_names: Option<&str>) -> Vec<ChartSeries> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    if entries.is_empty() {
        return Vec::new();
    }
    let names = series_names_of(series_names);
    let series: Vec<Vec<f64>> = if entries.iter().any(Value::is_array) {
        entries
            .iter()
            .map(|entry| match entry {
                Value::Array(values) => values.iter().map(number_of).collect(),
                _ => Vec::new(),
            })
            .collect()
    } else {
        vec![entries.iter().map(number_of).collect()]
    };
    series
        .into_iter()
        .enumerate()
        .map(|(index, values)| ChartSeries {
            name: names.get(index).cloned(),
            values,
        })
        .collect()
}

/// `["a","b"]` or `a,b` → names.
///
/// Public because the legend renders the same list the series are named from;
/// parsing it twice is how the two drift.
#[must_use]
pub fn series_names_of(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Vec::new();
    };
    if let Ok(Value::Array(names)) = serde_json::from_str::<Value>(raw) {
        return names.iter().filter_map(label_of).collect();
    }
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Accumulate the series so each one sits on top of the previous ones.
fn apply_stack(series: &mut [ChartSeries], percent: bool) {
    let width = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    let mut running = vec![0.0_f64; width];
    for entry in series.iter_mut() {
        for (index, value) in entry.values.iter_mut().enumerate() {
            running[index] += *value;
            *value = running[index];
        }
    }
    if !percent {
        return;
    }
    for entry in series.iter_mut() {
        for (index, value) in entry.values.iter_mut().enumerate() {
            let total = running[index];
            *value = if total > 0.0 {
                *value / total * 100.0
            } else {
                0.0
            };
        }
    }
}

/// A plotted value from a JSON cell; anything non-numeric counts as zero so a
/// missing sample does not shift the others.
fn number_of(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0),
        Value::String(text) => text.trim().parse::<f64>().unwrap_or(0.0),
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        _ => 0.0,
    }
}

/// A printable label from a JSON cell.
fn label_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}
