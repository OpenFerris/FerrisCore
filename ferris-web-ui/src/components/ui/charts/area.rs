//! The `AreaChart` component: one inline `<svg>` computed from the props.
//!
//! ## Geometry and the label trade-off
//!
//! The plot is laid out in a `600 × 200` viewBox and stretched to its CSS box
//! (`preserveAspectRatio="none"`, like every hand-rolled sparkline): a chart that
//! kept its aspect ratio would letterbox inside the card a dashboard gives it.
//! Strokes stay one pixel wide thanks to `vector-effect="non-scaling-stroke"`,
//! and the axis labels are laid out in viewBox units so they always sit on
//! their own tick — they stretch with the box, which is the price of filling it.
//!
//! ## Colour
//!
//! No colour is written here. The series take their colour from a semantic
//! `text-*` token on a wrapping `<g>` and the shapes paint with
//! `currentColor`; the gradient stops do the same. That keeps light/dark and
//! runtime plugin themes working without a per-chart palette.

use leptos::prelude::*;
use tw_merge::tw_merge;

use crate::components::hooks::use_unique_id::use_unique_id_for;

use super::ChartCurve;
use super::model::{
    ChartData, PAD_BOTTOM, PAD_LEFT, PAD_RIGHT, PAD_TOP, VIEW_HEIGHT, VIEW_WIDTH, series_names_of,
};

/// Series colours, cycled. Each entry is a semantic text token; the shapes
/// inherit it through `currentColor`.
const SERIES_TOKENS: [&str; 4] = ["text-primary", "text-info", "text-success", "text-warning"];

/// The matching legend dot for [`SERIES_TOKENS`].
const SERIES_DOTS: [&str; 4] = ["bg-primary", "bg-info", "bg-success", "bg-warning"];

/// Line and area chart drawn from `json_values`/`json_labels`.
///
/// Every prop is the one the old marker published (`data-chart-*`), so a plugin
/// or a test that read them keeps working; what changed is that the body is now
/// the chart instead of an empty `<div>`.
#[allow(clippy::needless_pass_by_value)] // Leptos props are owned by the component; `#[prop(into)]` is the macro's contract.
#[component]
pub fn AreaChart(
    #[prop(into)] json_values: Signal<String>,
    #[prop(into)] json_labels: Signal<String>,
    #[prop(default = ChartCurve::default())] curve: ChartCurve,
    #[prop(optional, into)] class: Signal<String>,
    #[prop(optional, into)] series_names: Option<String>,
    #[prop(optional, into)] stack_type: Option<String>,
    #[prop(optional, into)] gradient: Option<bool>,
    #[prop(optional, into)] show_yaxis: Option<bool>,
    #[prop(optional, into)] show_grid: Option<bool>,
    /// JSON array of `{x: "YYYY-MM-DD", title: "..."}` for article publish markers.
    #[prop(optional, into)]
    json_annotations: Option<String>,
) -> impl IntoView {
    let area_chart_id = use_unique_id_for("AreaChart");
    // `min-w-0`: the chart can be the child of a flex/grid cell, where the
    // default `min-width:auto` lets the SVG's intrinsic width push the cell
    // wider than the screen. The body itself is a `w-full` SVG whose viewBox
    // scales it, so nothing else is fixed.
    let merged_class = Signal::derive(move || tw_merge!("w-full min-w-0 max-w-full", class.get()));

    // The props that never change after mount, owned so the reactive closure
    // below does not borrow the component's scope.
    let names = series_names.clone().unwrap_or_default();
    let stack = stack_type.clone().unwrap_or_default();
    let annotations_json = json_annotations.clone().unwrap_or_default();
    let data = Memo::new(move |_| {
        ChartData::parse(&json_values.get(), &json_labels.get(), Some(&names), &stack)
    });

    let with_gradient = gradient.unwrap_or(false);
    let y_axis = show_yaxis.unwrap_or(true);
    let grid = show_grid.unwrap_or(true);

    let chart_id = area_chart_id.clone();
    let body = move || {
        let data = data.get();
        if data.is_empty() {
            return empty_state();
        }
        chart_svg(
            &data,
            &chart_id,
            curve,
            with_gradient,
            y_axis,
            grid,
            &annotations_json,
        )
    };

    let legend = legend_entries(series_names.as_deref());
    let legend = (!legend.is_empty()).then(|| {
        view! {
            <div class="mt-1 flex flex-wrap items-center gap-3">
                {legend.into_iter().map(|(name, dot)| view! {
                    <span class="flex items-center gap-1 text-xs text-muted-foreground">
                        <span class=format!("size-2 rounded-full {dot}")></span>
                        {name}
                    </span>
                }).collect_view()}
            </div>
        }
    });

    view! {
        <div
            id=area_chart_id
            class=move || merged_class.get()
            data-name="AreaChart"
            data-chart-curve=curve.to_string()
            data-chart-values=move || json_values.get()
            data-chart-labels=move || json_labels.get()
            data-chart-series-names=series_names
            data-chart-stack-type=stack_type
            data-chart-gradient=gradient.map(|g| g.to_string())
            data-chart-show-yaxis=show_yaxis.map(|y| y.to_string())
            data-chart-show-grid=show_grid.map(|g| g.to_string())
            data-chart-annotations=json_annotations
            data-chart-empty=move || data.get().is_empty().to_string()
        >
            {body}
            {legend}
        </div>
    }
}

/// The honest empty state: no data is said in words, not drawn as a flat zero
/// line (which would claim there was activity, only none of it).
fn empty_state() -> AnyView {
    view! {
        <p class="flex h-full min-h-24 items-center justify-center text-sm text-muted-foreground">
            {crate::i18n::t("ui.chart.empty")}
        </p>
    }
    .into_any()
}

/// The whole chart: gradients, grid, axes, series and markers.
fn chart_svg(
    data: &ChartData,
    chart_id: &str,
    curve: ChartCurve,
    gradient: bool,
    y_axis: bool,
    grid: bool,
    annotations_json: &str,
) -> AnyView {
    view! {
        <svg
            class="h-full w-full"
            viewBox=format!("0 0 {VIEW_WIDTH} {VIEW_HEIGHT}")
            preserveAspectRatio="none"
            role="img"
            aria-label=crate::i18n::t("ui.chart.area_label")
        >
            {gradient.then(|| view! { <defs>{gradient_defs(data, chart_id)}</defs> })}
            <g class="text-border" stroke="currentColor">{grid_lines(data, grid)}</g>
            <g class="text-muted-foreground text-xs" fill="currentColor">
                {y_labels(data, y_axis)}
                {x_labels(data)}
            </g>
            {series_paths(data, chart_id, curve, gradient)}
            {markers(data, annotations_json)}
        </svg>
    }
    .into_any()
}

/// One gradient per series, so the fill fades towards the baseline.
fn gradient_defs(data: &ChartData, chart_id: &str) -> AnyView {
    data.series
        .iter()
        .enumerate()
        .map(|(index, _)| {
            view! {
                <linearGradient id=gradient_id(chart_id, index) x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stop-color="currentColor" stop-opacity="0.35" />
                    <stop offset="100%" stop-color="currentColor" stop-opacity="0.02" />
                </linearGradient>
            }
        })
        .collect_view()
        .into_any()
}

/// The horizontal grid, one line per y tick (the baseline included).
fn grid_lines(data: &ChartData, enabled: bool) -> AnyView {
    if !enabled {
        return AnyView::from(Fragment::new(Vec::new()));
    }
    data.y_ticks()
        .into_iter()
        .map(|(_, y)| {
            view! {
                <line
                    x1=PAD_LEFT
                    x2=VIEW_WIDTH - PAD_RIGHT
                    y1=y
                    y2=y
                    stroke="currentColor"
                    stroke-width="1"
                    stroke-dasharray="2 4"
                />
            }
        })
        .collect_view()
        .into_any()
}

/// The y-axis tick labels, right-aligned against the plot.
fn y_labels(data: &ChartData, enabled: bool) -> AnyView {
    if !enabled {
        return AnyView::from(Fragment::new(Vec::new()));
    }
    data.y_ticks()
        .into_iter()
        .map(|(label, y)| {
            view! {
                <text x=PAD_LEFT - 6.0 y=y + 4.0 text-anchor="end" fill="currentColor">
                    {label}
                </text>
            }
        })
        .collect_view()
        .into_any()
}

/// The x-axis labels: at most one per tick position chosen by the model.
fn x_labels(data: &ChartData) -> AnyView {
    data.x_labels()
        .into_iter()
        .map(|(_, label, x)| {
            view! {
                <text x=x y=VIEW_HEIGHT - 6.0 text-anchor="middle" fill="currentColor">
                    {label}
                </text>
            }
        })
        .collect_view()
        .into_any()
}

/// One `<g>` per series: its area, its line and — for a single sample — the
/// point itself (a one-point "line" would draw nothing at all).
fn series_paths(data: &ChartData, chart_id: &str, curve: ChartCurve, gradient: bool) -> AnyView {
    let single = data.point_count() == 1;
    data.series
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let token = SERIES_TOKENS[index % SERIES_TOKENS.len()];
            let line = data.line_path(index, curve);
            let area = data.area_path(index, curve);
            let fill = if gradient {
                format!("url(#{})", gradient_id(chart_id, index))
            } else {
                "currentColor".to_string()
            };
            let dot = entry
                .values
                .first()
                .filter(|_| single)
                .and_then(|_| data.point(index, 0));
            view! {
                <g class=token>
                    <path d=area fill=fill fill-opacity="0.14" stroke="none" />
                    <path
                        d=line
                        fill="none"
                        stroke="currentColor"
                        stroke-width="2"
                        stroke-linejoin="round"
                        stroke-linecap="round"
                        vector-effect="non-scaling-stroke"
                    />
                    {dot.map(|(x, y)| view! {
                        <circle cx=x cy=y r="2.5" fill="currentColor" />
                    })}
                </g>
            }
        })
        .collect_view()
        .into_any()
}

/// Publish markers: a dashed vertical line under the label it belongs to.
fn markers(data: &ChartData, annotations_json: &str) -> AnyView {
    data.annotations(annotations_json)
        .into_iter()
        .map(|(x, title)| {
            view! {
                <g class="text-warning">
                    <line
                        x1=x
                        x2=x
                        y1=PAD_TOP
                        y2=VIEW_HEIGHT - PAD_BOTTOM
                        stroke="currentColor"
                        stroke-width="1"
                        stroke-dasharray="3 3"
                    >
                        <title>{title}</title>
                    </line>
                </g>
            }
        })
        .collect_view()
        .into_any()
}

/// `(name, dot class)` for the legend, from the `series_names` prop.
fn legend_entries(raw: Option<&str>) -> Vec<(String, String)> {
    series_names_of(raw)
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, SERIES_DOTS[index % SERIES_DOTS.len()].to_string()))
        .collect()
}

/// DOM id of the gradient of one series.
fn gradient_id(chart: &str, series: usize) -> String {
    format!("{chart}-series-{series}")
}
