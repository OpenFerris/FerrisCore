//! Grid placement rules for dashboard layouts, shared by server and Desk.
//!
//! A dashboard is a `grid-12` layout of [`WidgetInstance`]s: each has a
//! footprint (`span` columns × `rows` rows) and a top-left cell (`col`, `row`).
//! Neither side may act on impossible geometry — the server sanitizes what a
//! client `PUT`s and the Desk normalizes both what it renders and what it
//! saves — and both must agree, or the layout a user saves would come back with
//! a widget shifted or stacked on `(0,0)`.
//!
//! The rules therefore live here, in the pure-serde wire crate both sides
//! compile against, instead of being mirrored per side. A clamp that only one
//! side knows is exactly how the two drift.
//!
//! The constants echo the reference dashboard editor (12 columns, a 3-row
//! default height, at most 64 instances, 72 px per row); the upper bound on
//! widget height is tighter here because 256 rows of 72 px is a page nobody can
//! read.

use std::collections::HashSet;

use super::ui::{GridSize, WidgetInstance};

/// Grid columns of the `grid-12` layout.
pub const COLUMN: u8 = 12;
/// Rows a widget occupies when neither the instance nor its definition says.
pub const DEFAULT_ROWS: u8 = 3;
/// Instances one dashboard may hold; anything past it is dropped.
pub const MAX_ITEMS: usize = 64;
/// Row height, in CSS pixels, of the layout's `grid-auto-rows`.
pub const CELL_HEIGHT_PX: u16 = 72;
/// Upper bound on a widget's height, in rows (~4.6k px of page).
pub const MAX_ROWS: u8 = 64;
/// Separation between two tiles, in CSS pixels, when neither the board's style
/// nor the global default declares one.
///
/// The tiles are absolutely positioned (no CSS `gap` applies to them), so the
/// separation is arithmetic: [`super::style::DashboardStyle::gap`] falls back to
/// this value, and the Desk's `tile_style`/`board_style` turn it into the row
/// step and the box offsets. One default, used everywhere.
pub const DEFAULT_GAP_PX: u8 = 12;

/// A resolved grid box: where one instance sits and how much it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placement {
    /// 0-based column of the top-left cell.
    pub col: u8,
    /// 0-based row of the top-left cell.
    pub row: u16,
    /// Width in columns (1..=12).
    pub span: u8,
    /// Height in rows (1..=64).
    pub rows: u8,
}

/// A widget definition's effective sizing: the footprint a new instance gets
/// and the floor the builder clamps it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sizing {
    /// Default footprint when an instance declares none.
    pub size: GridSize,
    /// Smallest footprint the builder allows.
    pub min: GridSize,
}

impl GridSize {
    /// The fallback footprint for a widget `kind`: stats are small, tables own
    /// the row, anything else (charts included) takes half the grid.
    ///
    /// This is the one place the `kind` → geometry mapping is decided, so the
    /// Desk's builder and the server's sanitizer cannot pick different
    /// defaults for the same widget.
    #[must_use]
    pub const fn for_kind(kind: &str) -> Self {
        Self {
            span: match kind.as_bytes() {
                b"stat" => 3,
                b"table" => 12,
                _ => COLUMN / 2,
            },
            rows: DEFAULT_ROWS,
        }
    }

    /// Resolve a definition's declared `size`/`min` into the effective pair:
    /// the declared `size` wins over the `kind` default, the declared `min`
    /// (else one cell) floors it, and both are clamped inside the grid.
    #[must_use]
    pub fn resolve(kind: &str, size: Option<Self>, min: Option<Self>) -> Sizing {
        let min = min.unwrap_or(Self { span: 1, rows: 1 });
        let min = Self {
            span: min.span.clamp(1, COLUMN),
            rows: min.rows.clamp(1, MAX_ROWS),
        };
        let size = size.unwrap_or_else(|| Self::for_kind(kind));
        Sizing {
            size: Self {
                span: size.span.clamp(min.span, COLUMN),
                rows: size.rows.clamp(min.rows, MAX_ROWS),
            },
            min,
        }
    }
}

/// The placement one instance asks for, with absent fields filled in, the
/// footprint clamped to the grid, and `col` kept inside the span's reach.
///
/// This does *not* resolve overlaps — that is [`normalize`]'s job, and only the
/// whole list can see them.
#[must_use]
pub fn placement_of(widget: &WidgetInstance, sizing: Sizing) -> Placement {
    let span = widget
        .span
        .unwrap_or(sizing.size.span)
        .clamp(sizing.min.span, COLUMN);
    let rows = widget
        .rows
        .unwrap_or(sizing.size.rows)
        .clamp(sizing.min.rows, MAX_ROWS);
    let col = widget.col.unwrap_or(0).min(COLUMN.saturating_sub(span));
    Placement {
        col,
        row: widget.row.unwrap_or(0),
        span,
        rows,
    }
}

/// Normalize a widget list **in place** so no rule of the grid can be broken.
///
/// Drop entries with no reference, dedupe ids, cap the count, clamp each
/// footprint, and re-place every box that would overlap an already-placed one
/// at the first free cell.
///
/// `sizing` resolves a reference (`owner.name`) to its definition's hints; use
/// [`GridSize::resolve`] there so both callers apply one rule. Idempotent:
/// running it on its own output leaves the list unchanged, which is what lets
/// the Desk normalize after every gesture and the server normalize again
/// without the layout crawling.
pub fn normalize(widgets: &mut Vec<WidgetInstance>, sizing: impl Fn(&str) -> Sizing) {
    let source = std::mem::take(widgets);
    let cap = source.len().min(MAX_ITEMS);
    let mut out: Vec<WidgetInstance> = Vec::with_capacity(cap);
    let mut placed: Vec<Placement> = Vec::with_capacity(cap);
    let mut seen: HashSet<String> = HashSet::new();

    for (index, mut widget) in source.into_iter().take(MAX_ITEMS).enumerate() {
        let reference = widget.widget.trim().to_string();
        if reference.is_empty() {
            continue;
        }
        widget.widget = reference;

        let mut id = widget.id.trim().to_string();
        if id.is_empty() {
            id = format!("item-{}", index + 1);
        }
        if !seen.insert(id.clone()) {
            // Ids are the instance key: two widgets sharing one would make the
            // palette, the properties dialog and the DOM anchors ambiguous.
            let mut n = index + 1;
            loop {
                let candidate = format!("{id}-{n}");
                if seen.insert(candidate.clone()) {
                    id = candidate;
                    break;
                }
                n += 1;
            }
        }
        widget.id = id;

        let sizing = sizing(&widget.widget);
        let mut placement = placement_of(&widget, sizing);
        if placed.iter().any(|other| overlaps(placement, *other)) {
            let (col, row) = first_free_slot(&placed, placement.span, placement.rows);
            placement.col = col;
            placement.row = row;
        }

        widget.col = Some(placement.col);
        widget.row = Some(placement.row);
        widget.span = Some(placement.span);
        widget.rows = Some(placement.rows);
        placed.push(placement);
        out.push(widget);
    }

    *widgets = out;
}

/// Whether two grid boxes share at least one cell.
fn overlaps(a: Placement, b: Placement) -> bool {
    a.col < b.col + b.span
        && b.col < a.col + a.span
        && a.row < b.row + u16::from(b.rows)
        && b.row < a.row + u16::from(a.rows)
}

/// The topmost, then leftmost, cell where a `span × rows` box fits without
/// overlapping any placed box. Scans only down to the current bottom, so an
/// empty grid answers `(0, 0)` and a full top row pushes the box down one.
fn first_free_slot(placed: &[Placement], span: u8, rows: u8) -> (u8, u16) {
    let max_col = COLUMN.saturating_sub(span);
    let bottom = placed
        .iter()
        .map(|p| p.row + u16::from(p.rows))
        .max()
        .unwrap_or(0);
    for row in 0..=bottom {
        for col in 0..=max_col {
            let candidate = Placement {
                col,
                row,
                span,
                rows,
            };
            if !placed.iter().any(|other| overlaps(candidate, *other)) {
                return (col, row);
            }
        }
    }
    (0, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: &str, widget: &str) -> WidgetInstance {
        WidgetInstance {
            id: id.to_string(),
            widget: widget.to_string(),
            label: None,
            show_title: None,
            subtitle: None,
            show_subtitle: None,
            params: Vec::new(),
            span: None,
            col: None,
            row: None,
            rows: None,
            style: None,
        }
    }

    /// Every widget resolves through this: no declaration means the generic
    /// 6×3 half-grid default.
    fn generic(_reference: &str) -> Sizing {
        GridSize::resolve("chart", None, None)
    }

    #[test]
    fn kind_defaults_match_the_documented_footprints() {
        assert_eq!(GridSize::for_kind("stat"), GridSize { span: 3, rows: 3 });
        assert_eq!(GridSize::for_kind("table"), GridSize { span: 12, rows: 3 });
        assert_eq!(GridSize::for_kind("chart"), GridSize { span: 6, rows: 3 });
        assert_eq!(
            GridSize::for_kind("whatever"),
            GridSize { span: 6, rows: 3 }
        );
    }

    #[test]
    fn declared_size_wins_over_kind_and_min_floors_it() {
        let sizing = GridSize::resolve(
            "stat",
            Some(GridSize { span: 8, rows: 5 }),
            Some(GridSize { span: 4, rows: 2 }),
        );
        assert_eq!(sizing.size, GridSize { span: 8, rows: 5 });
        assert_eq!(sizing.min, GridSize { span: 4, rows: 2 });

        // A declared size below the declared minimum is raised to it, not kept.
        let sizing = GridSize::resolve(
            "chart",
            Some(GridSize { span: 2, rows: 1 }),
            Some(GridSize { span: 4, rows: 2 }),
        );
        assert_eq!(sizing.size, GridSize { span: 4, rows: 2 });
    }

    #[test]
    fn placement_clamps_impossible_geometry() {
        let mut widget = instance("a", "acme.chart");
        widget.span = Some(99);
        widget.col = Some(50);
        widget.rows = Some(200);
        let placement = placement_of(&widget, generic(""));
        assert_eq!(
            placement,
            Placement {
                col: 0,
                row: 0,
                span: COLUMN,
                rows: MAX_ROWS,
            }
        );
    }

    #[test]
    fn absent_cells_flow_left_to_right_then_down() {
        let mut widgets = vec![
            instance("a", "acme.one"),
            instance("b", "acme.two"),
            instance("c", "acme.three"),
        ];
        normalize(&mut widgets, generic);
        assert_eq!(widgets[0].col, Some(0));
        assert_eq!(widgets[1].col, Some(6));
        assert_eq!(widgets[2].col, Some(0));
        assert_eq!(widgets[2].row, Some(3));
    }

    #[test]
    fn overlapping_boxes_are_pushed_to_the_first_free_slot() {
        let mut widgets = vec![instance("a", "acme.one"), instance("b", "acme.two")];
        widgets[0].col = Some(0);
        widgets[0].row = Some(0);
        widgets[1].col = Some(0);
        widgets[1].row = Some(0);
        normalize(&mut widgets, generic);
        assert_eq!((widgets[0].col, widgets[0].row), (Some(0), Some(0)));
        // Same cell requested, but occupied: the second lands beside it.
        assert_eq!((widgets[1].col, widgets[1].row), (Some(6), Some(0)));
    }

    #[test]
    fn normalize_is_idempotent() {
        let mut once = vec![
            instance("a", "acme.one"),
            instance("a", "acme.two"),
            instance("", "acme.three"),
        ];
        normalize(&mut once, generic);
        let mut twice = once.clone();
        normalize(&mut twice, generic);
        assert_eq!(once, twice);
        assert_eq!(once[0].col, Some(0));
        assert_eq!(once[1].id, "a-2");
        assert_eq!(once[2].id, "item-3");
    }

    #[test]
    fn entries_without_a_reference_are_dropped() {
        let mut widgets = vec![instance("a", "   "), instance("b", "acme.one")];
        normalize(&mut widgets, generic);
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].id, "b");
    }

    #[test]
    fn the_instance_count_is_capped() {
        let mut widgets: Vec<WidgetInstance> = (0..MAX_ITEMS + 5)
            .map(|i| instance(&format!("w{i}"), "acme.one"))
            .collect();
        normalize(&mut widgets, generic);
        assert_eq!(widgets.len(), MAX_ITEMS);
    }
}
