use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use icons::{
    ArrowDownWideNarrow, ArrowUpNarrowWide, ChevronDown, CircleX, EyeOff, PanelLeft, PanelLeftClose,
};
use leptos::ev::KeyboardEvent;
use leptos::html;
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::checkbox::Checkbox;
use super::dropdown_menu::{
    DropdownMenu, DropdownMenuAction, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem,
    DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger,
};
use crate::components::hooks::use_cell_edit::CellEditContext;
use crate::components::hooks::use_virtual_scroll::{VirtualScrollState, use_virtual_scroll};

/// Enforces display logic (class + value) to live in data structs, not inline in views.
#[derive(Debug, Clone, Default)]
pub struct StyledGridCell {
    pub class: &'static str,
    pub value: String,
}

impl StyledGridCell {
    pub fn new(class: &'static str, value: String) -> Self {
        Self { class, value }
    }
}

// * Source: https://tablecn.com/data-grid

/* ========================================================== */
/*                     ✨ TYPES ✨                            */
/* ========================================================== */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    None,
    Asc,
    Desc,
}

/* ========================================================== */
/*                     ✨ TRAITS ✨                           */
/* ========================================================== */

/// Trait for columns that can be pinned in a data grid.
/// Implement this trait on your Column enum to use the pinning utility functions.
pub trait PinnableColumn: Eq + Hash + Copy {
    /// Returns the list of pinnable columns with their widths in display order.
    fn pinnable_columns() -> &'static [(Self, i32)];
}

/// Trait for columns that support sorting.
/// Generic over the row type `R` so each data table can define its own row structure.
///
/// # Example
/// ```ignore
/// impl SortableColumn<RowData> for Column {
///     fn compare(self, a: &RowData, b: &RowData) -> Option<std::cmp::Ordering> {
///         match self {
///             Self::Name => Some(a.name.cmp(&b.name)),
///             Self::Age => Some(a.age.cmp(&b.age)),
///             _ => None,
///         }
///     }
/// }
/// ```
pub trait SortableColumn<R: Default>: Copy {
    /// Compares two row items by this column's field.
    /// Returns None for columns that don't support sorting.
    fn compare(self, a: &R, b: &R) -> Option<std::cmp::Ordering>;

    /// Sorts rows in place by this column in the given direction.
    /// Default implementation uses `compare`.
    fn sort_rows(self, rows: &mut [R], direction: SortDirection) {
        if direction == SortDirection::None {
            return;
        }
        // Check if this column supports sorting
        if self.compare(&R::default(), &R::default()).is_some() {
            match direction {
                SortDirection::Asc => {
                    rows.sort_by(|a, b| self.compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
                }
                SortDirection::Desc => rows.sort_by(|a, b| {
                    self.compare(a, b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .reverse()
                }),
                SortDirection::None => {}
            }
        }
    }
}

/// Trait for row data that can be displayed in a generic data grid.
/// Implement this trait on your data struct to use with `GenericDataGrid`.
///
/// # Example
/// ```ignore
/// impl DataGridRow for LinkedinCreator {
///     type Id = i32;
///     type Column = Column;
///
///     fn id(&self) -> Self::Id { self.id }
///     fn matches_filter(&self, filter: &str) -> bool {
///         self.name.to_lowercase().contains(filter)
///     }
///     fn get_value(&self, col: Self::Column) -> String {
///         match col {
///             Column::Name => self.name.clone(),
///             // ...
///         }
///     }
///     fn render_cell(&self, col: Self::Column) -> AnyView {
///         match col {
///             Column::Name => view! { <NameCell ... /> }.into_any(),
///             // ...
///         }
///     }
/// }
/// ```
pub trait DataGridRow: Clone + Send + Sync + 'static {
    /// The ID type for this row (e.g., i32, Uuid).
    type Id: Copy + Eq + std::hash::Hash + Send + Sync + 'static;

    /// The Column enum type for this data grid.
    type Column: DataGridColumn + std::fmt::Display + strum::IntoEnumIterator;

    /// Returns the unique ID for this row.
    fn id(&self) -> Self::Id;

    /// Returns true if this row matches the given filter string.
    fn matches_filter(&self, filter: &str) -> bool;

    /// Returns the string value for a column (used for copy-to-clipboard).
    fn get_value(&self, col: Self::Column) -> String;

    /// Renders the cell content for a column.
    fn render_cell(&self, col: Self::Column) -> AnyView;
}

/// Trait for data grid columns with common utility methods.
/// Extend your Column enum with this trait to get `colindex()`, `is_visible()`, and `is_disabled()`.
///
/// # Example
/// ```ignore
/// impl DataGridColumn for Column {
///     fn colindex(self) -> i32 {
///         self as i32 + 1
///     }
/// }
/// ```
pub trait DataGridColumn: PinnableColumn + AsRef<str> + Send + Sync + 'static {
    /// Returns the 1-based column index for aria-colindex.
    /// Typically implemented as `self as i32 + 1` for enums.
    fn colindex(self) -> i32;

    /// Returns whether this column is disabled (cannot be toggled in the view selector).
    /// Override this method to specify which columns should always be visible.
    fn is_disabled(self) -> bool {
        false
    }

    /// Returns a CSS-safe column name (removes spaces).
    /// Used for CSS variable names like `--col-IsActive-size`.
    fn css_safe_name(self) -> String {
        self.as_ref().chars().filter(|c| *c != ' ').collect()
    }

    /// Returns a signal indicating if column should be shown (not pinned AND visible).
    fn is_visible(
        self,
        pinned_columns_signal: RwSignal<HashSet<Self>>,
        visible_columns_signal: RwSignal<HashSet<String>>,
    ) -> Signal<bool> {
        Signal::derive(move || {
            !pinned_columns_signal.with(|p| p.contains(&self))
                && visible_columns_signal.with(|v| v.contains(self.as_ref()))
        })
    }
}

/// Calculate the left position for a pinned column based on which columns are pinned before it.
/// Starts at 60px to account for the checkbox column.
pub fn get_pinned_left_position<C: PinnableColumn + 'static>(col: C, pinned: &HashSet<C>) -> i32 {
    let mut left = 60; // Start after checkbox (60px)
    for (c, width) in C::pinnable_columns() {
        if *c == col {
            break;
        }
        if pinned.contains(c) {
            left += width;
        }
    }
    left
}

/// Get the width for a pinnable column, or 150 as default if not found.
pub fn get_column_width<C: PinnableColumn + 'static>(col: C) -> i32 {
    C::pinnable_columns()
        .iter()
        .find(|(c, _)| *c == col)
        .map(|(_, w)| *w)
        .unwrap_or(150)
}

/// Generates CSS custom properties for column sizes from pinnable columns.
/// Includes max-height for proper viewport sizing.
/// Use with `LazyLock` to compute once: `static GRID_STYLE: LazyLock<String> = LazyLock::new(generate_grid_style::<Column>);`
pub fn generate_grid_style<C: PinnableColumn + AsRef<str> + 'static>() -> String {
    let mut style = String::from("--header-Select-size: 60; --col-Select-size: 60; ");
    for (col, width) in C::pinnable_columns() {
        // Remove spaces for CSS-safe variable names (e.g., "Is Active" -> "IsActive")
        let name: String = col.as_ref().chars().filter(|c| *c != ' ').collect();
        style.push_str(&format!(
            "--header-{name}-size: {width}; --col-{name}-size: {width}; "
        ));
    }
    style.push_str("max-height: calc(100vh - 16rem);");
    style
}

/// Returns columns that are both pinned AND visible for rendering.
pub fn get_pinned_visible_columns<C>(
    pinned_columns_signal: RwSignal<HashSet<C>>,
    visible_columns_signal: RwSignal<HashSet<String>>,
) -> Vec<(C, i32)>
where
    C: PinnableColumn + AsRef<str> + Copy + Eq + std::hash::Hash + Send + Sync + 'static,
{
    C::pinnable_columns()
        .iter()
        .filter(|(col, _)| {
            pinned_columns_signal.with(|p| p.contains(col))
                && visible_columns_signal.with(|v| v.contains(col.as_ref()))
        })
        .copied()
        .collect()
}

/* ========================================================== */
/*                     ✨ CLX COMPONENTS ✨                   */
/* ========================================================== */

mod components {
    use super::*;
    // The wrapper is the horizontal scroll port on narrow screens: `Grid`
    // itself only ever scrolls vertically because it is the element titled to
    // the viewport (`max-height` comes from `generate_grid_style`). Pinned
    // columns and the sticky header live inside `Grid`, so they move with its
    // own scroll offset while the wrapper keeps the page from growing wider
    // than the phone.
    clx! {GridWrapper, div, "flex relative flex-col w-full max-w-full overflow-x-auto overscroll-x-contain"}
    clx! {GridCellContent, span, ""}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Z-index for pinned columns. Must be higher than TableSeparator's z-50.
const PINNED_Z_INDEX: i32 = 51;

mod cells;
mod grid;
mod header;

pub use cells::*;
pub use grid::*;
pub use header::*;
