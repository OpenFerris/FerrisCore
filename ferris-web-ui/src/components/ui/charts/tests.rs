//! Chart parsing and geometry.
//!
//! Coordinates and tick labels are compared **exactly** on purpose: the point
//! of these tests is the arithmetic that places each sample, and an approximate
//! comparison would hide a wrong scale.
#![allow(clippy::float_cmp)]
//!
//! The component is a thin projection of [`ChartData`] onto SVG, so these cover
//! the two things a wrong chart can lie about: what was parsed, and where a
//! point lands.

use super::ChartCurve;
use super::model::{
    ChartData, MAX_X_LABELS, PAD_BOTTOM, PAD_LEFT, PAD_TOP, VIEW_HEIGHT, VIEW_WIDTH, nice_max,
};

fn sample() -> ChartData {
    ChartData::parse(
        "[1,4,2,6]",
        r#"["2026-11-01","2026-11-02","2026-11-03","2026-11-04"]"#,
        Some(r#"["Creados"]"#),
        "",
    )
}

#[test]
fn a_flat_array_is_one_series() {
    let data = sample();
    assert_eq!(data.series.len(), 1);
    assert_eq!(data.series[0].values, vec![1.0, 4.0, 2.0, 6.0]);
    assert_eq!(data.series[0].name.as_deref(), Some("Creados"));
    assert_eq!(data.labels.len(), 4);
    assert!(!data.is_empty());
}

#[test]
fn nested_arrays_are_several_series_and_names_cycle_independently() {
    let data = ChartData::parse(
        "[[1,2,3],[4,5,6]]",
        r#"["a","b","c"]"#,
        Some("Creados, Enviados"),
        "",
    );
    assert_eq!(data.series.len(), 2);
    assert_eq!(data.series[1].values, vec![4.0, 5.0, 6.0]);
    assert_eq!(data.series[1].name.as_deref(), Some("Enviados"));
}

#[test]
fn numbers_as_strings_are_values_not_labels() {
    // The engine's JSON blob serialises counters as strings often enough that
    // treating them as zero would silently flatten real activity.
    let data = ChartData::parse(r#"["3","4"]"#, r#"["a","b"]"#, None, "");
    assert_eq!(data.series[0].values, vec![3.0, 4.0]);
    assert_eq!(data.series[0].name, None);
}

#[test]
fn numeric_labels_are_rendered_as_text() {
    let data = ChartData::parse("[1,2]", "[2026,2027]", None, "");
    assert_eq!(data.labels, vec!["2026".to_string(), "2027".to_string()]);
}

#[test]
fn labels_are_padded_to_the_widest_series() {
    let data = ChartData::parse("[[1,2,3]]", r#"["only"]"#, None, "");
    assert_eq!(data.labels.len(), 3);
    assert_eq!(data.labels[1], "");
    // A label-less position is not drawn, so its date is not guessed.
    assert_eq!(data.x_labels().len(), 1);
}

#[test]
fn malformed_or_empty_input_is_empty_not_zero() {
    for values in ["", "[]", "{oops}", "null", "[]"] {
        let data = ChartData::parse(values, r#"["a"]"#, None, "");
        assert!(data.is_empty(), "{values}");
    }
}

#[test]
fn the_axis_rounds_up_to_a_tick_friendly_maximum() {
    assert_eq!(nice_max(0.0), 1.0);
    assert_eq!(nice_max(-3.0), 1.0);
    assert_eq!(nice_max(7.0), 10.0);
    assert_eq!(nice_max(23.0), 25.0);
    assert_eq!(nice_max(100.0), 100.0);
    assert_eq!(nice_max(0.7), 1.0);
}

#[test]
fn points_span_the_plot_box_without_touching_the_edges() {
    let data = sample();
    let first = data.point(0, 0).expect("first point");
    let last = data.point(0, 3).expect("last point");
    assert_eq!(first.0, PAD_LEFT);
    assert_eq!(last.0, VIEW_WIDTH - super::model::PAD_RIGHT);
    // The axis top sits at the top padding and the zero baseline at the bottom
    // padding; the tallest sample stays inside because the axis rounds up.
    assert_eq!(data.y(data.axis_max()), PAD_TOP);
    assert!(data.y(data.max()) > PAD_TOP);
    assert_eq!(data.y(0.0), VIEW_HEIGHT - PAD_BOTTOM);
    assert!(first.1 > PAD_TOP && first.1 < VIEW_HEIGHT - PAD_BOTTOM);
}

#[test]
fn a_single_sample_is_a_point_not_a_fake_line() {
    let data = ChartData::parse("[5]", r#"["solo"]"#, None, "");
    assert_eq!(data.point_count(), 1);
    let (x, y) = data.point(0, 0).expect("the point exists");
    assert!(x > PAD_LEFT && x < VIEW_WIDTH);
    assert!(y < VIEW_HEIGHT);
    // One point has no segment: the path is just a move.
    assert!(!data.line_path(0, ChartCurve::Straight).contains('L'));
}

#[test]
fn the_y_axis_has_a_tick_per_interval_and_keeps_integers_integral() {
    let ticks = sample().y_ticks();
    assert_eq!(ticks.len(), super::model::Y_TICKS + 1);
    assert_eq!(ticks[0].0, "0");
    assert_eq!(ticks[ticks.len() - 1].0, "10");
    assert!(ticks.iter().all(|(label, _)| !label.contains('.')));
}

#[test]
fn x_labels_are_capped_and_always_include_both_ends() {
    let labels: Vec<String> = (0..40).map(|day| format!("day-{day}")).collect();
    let values = format!("[{}]", vec!["1"; 40].join(","));
    let data = ChartData::parse(&values, &serde_json::to_string(&labels).unwrap(), None, "");
    let drawn = data.x_labels();
    assert!(drawn.len() <= MAX_X_LABELS);
    assert_eq!(drawn.first().expect("first").1, "day-0");
    assert_eq!(drawn.last().expect("last").1, "day-39");
}

#[test]
fn straight_stepline_and_smooth_paths_are_distinguishable() {
    let data = sample();
    let straight = data.line_path(0, ChartCurve::Straight);
    assert_eq!(straight.matches('L').count(), 3);
    let stepline = data.line_path(0, ChartCurve::Stepline);
    assert_eq!(stepline.matches('H').count(), 3);
    assert_eq!(stepline.matches('V').count(), 3);
    let smooth = data.line_path(0, ChartCurve::Smooth);
    assert_eq!(smooth.matches('Q').count(), 3);
    for path in [straight, stepline, smooth] {
        assert!(path.starts_with('M'), "{path}");
    }
}

#[test]
fn the_area_closes_on_the_baseline() {
    let data = sample();
    let area = data.area_path(0, ChartCurve::Straight);
    assert!(area.ends_with('Z'));
    let baseline = data.y(0.0);
    let closing = format!("{baseline:.2}");
    assert_eq!(area.matches(&closing).count(), 2, "{area}");
}

#[test]
fn stacking_accumulates_and_percent_normalises() {
    let stacked = ChartData::parse("[[1,2],[3,4]]", r#"["a","b"]"#, None, "normal");
    assert_eq!(stacked.series[0].values, vec![1.0, 2.0]);
    assert_eq!(stacked.series[1].values, vec![4.0, 6.0]);

    let percent = ChartData::parse("[[1,1],[3,1]]", r#"["a","b"]"#, None, "percent");
    assert_eq!(percent.series[0].values, vec![25.0, 50.0]);
    assert_eq!(percent.series[1].values, vec![100.0, 100.0]);
    assert_eq!(percent.axis_max(), 100.0);
}

#[test]
fn overlay_is_not_stackable() {
    for mode in ["", "none", "overlay", "false"] {
        let data = ChartData::parse("[[1,2],[3,4]]", r#"["a","b"]"#, None, mode);
        assert_eq!(data.series[1].values, vec![3.0, 4.0], "{mode}");
    }
}

#[test]
fn annotations_snap_to_a_real_label_and_drop_unknown_ones() {
    let data = sample();
    let markers = data
        .annotations(r#"[{"x":"2026-11-02","title":"Deploy"},{"x":"2026-01-01","title":"Nope"}]"#);
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].1, "Deploy");
    assert_eq!(markers[0].0, data.x(1));
    assert!(data.annotations("not json").is_empty());
}
