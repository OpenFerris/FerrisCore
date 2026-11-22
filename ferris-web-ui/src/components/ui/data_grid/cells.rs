use super::*;

#[component]
pub fn GridCellWrapper(children: Children, #[prop(optional, into)] class: String) -> impl IntoView {
    // TODO. slot=checkbox.

    let merged_class = tw_merge!(
        "py-1.5 px-2 text-sm text-left cursor-default outline-none size-full has-data-[slot=checkbox]:pt-2.5 **:data-[name=GridCellContent]:line-clamp-1",
        class
    );

    view! {
        <div data-name="GridCellWrapper" class=merged_class tabindex="-1">
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn GridHeaderCell(
    children: Children,
    colindex: i32,
    #[prop(into)] column: String,
    #[prop(optional, into)] class: String,
    #[prop(optional)] visible: Option<Signal<bool>>,
) -> impl IntoView {
    let merged_class = tw_merge!(
        // `shrink-0` keeps the column at its declared width; without it the flex
        // row squeezes every column on a phone and the horizontal scroll port
        // never has anything to scroll (the pinned offsets are computed from
        // these widths, so squeezing also breaks the sticky overlay).
        "relative shrink-0 border-r opacity-100 bg-background data-[visible=false]:hidden",
        class
    );

    let formatted_style = format!("width: calc(var(--header-{column}-size) * 1px);");

    // TODO. aria-sort.

    view! {
        <div
            role="columnheader"
            aria-sort="none"
            aria-colindex=colindex.to_string()
            class=merged_class
            data-name="GridHeaderCell"
            data-visible=move || visible.map(|v| v.get()).unwrap_or(true).to_string()
            tabindex="-1"
            style=formatted_style
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn GridCell(
    children: Children,
    colindex: i32,
    #[prop(into)] column: String,
    #[prop(optional, into)] class: String,
    #[prop(optional)] visible: Option<Signal<bool>>,
    #[prop(optional, into)] active: Signal<bool>,
    #[prop(optional, into)] current: Signal<bool>,
    #[prop(optional, into)] in_range: Signal<bool>,
    #[prop(optional)] on_click: Option<Callback<()>>,
    #[prop(optional)] on_contextmenu: Option<Callback<()>>,
    #[prop(optional)] on_mousedown: Option<Callback<()>>,
    #[prop(optional)] on_mouseenter: Option<Callback<()>>,
) -> impl IntoView {
    let merged_class = tw_merge!(
        "relative shrink-0 border-r opacity-100 bg-background select-none data-[visible=false]:hidden aria-selected:*:ring-2 aria-selected:*:ring-ring aria-selected:*:ring-inset aria-current:*:bg-muted-foreground/20",
        class
    );

    let formatted_style = format!("width: calc(var(--col-{column}-size) * 1px);");

    view! {
        <div
            role="gridcell"
            aria-colindex=colindex.to_string()
            aria-selected=move || active.get().to_string()
            aria-current=move || (current.get() || in_range.get()).to_string()
            class=merged_class
            data-name="GridCell"
            data-visible=move || visible.map(|v| v.get()).unwrap_or(true).to_string()
            tabindex="-1"
            style=formatted_style
            on:click=move |_| {
                if let Some(cb) = on_click {
                    cb.run(());
                }
            }
            on:contextmenu=move |_| {
                if let Some(cb) = on_contextmenu {
                    cb.run(());
                }
            }
            on:mousedown=move |ev| {
                if ev.button() == 0 && let Some(cb) = on_mousedown {
                    cb.run(());
                }
            }
            on:mouseenter=move |_| {
                if let Some(cb) = on_mouseenter {
                    cb.run(());
                }
            }
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Sticky select header cell (checkbox column header).
#[component]
pub fn GridSelectHeaderCell(children: Children) -> impl IntoView {
    view! {
        <div
            role="columnheader"
            aria-colindex="1"
            data-slot="grid-header-cell"
            tabindex="-1"
            class="relative shrink-0"
            style=format!(
                "left: 0px; position: sticky; background: var(--background); width: calc(var(--header-Select-size) * 1px); z-index: {PINNED_Z_INDEX};",
            )
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Sticky select cell (checkbox column).
#[component]
pub fn GridSelectCell(children: Children) -> impl IntoView {
    view! {
        <div
            role="gridcell"
            aria-colindex="1"
            data-slot="grid-cell"
            tabindex="-1"
            class="relative shrink-0"
            style=format!(
                "left: 0px; position: sticky; background: var(--background); width: calc(var(--col-Select-size) * 1px); z-index: {PINNED_Z_INDEX};",
            )
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Sticky pinned header cell with dynamic left position.
#[component]
pub fn GridPinnedHeaderCell(children: Children, left: i32, width: i32) -> impl IntoView {
    view! {
        <div
            role="columnheader"
            data-slot="grid-header-cell"
            tabindex="-1"
            class="relative shrink-0"
            style=format!(
                "left: {left}px; position: sticky; background: var(--background); width: {width}px; z-index: {PINNED_Z_INDEX};",
            )
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Sticky pinned cell with dynamic left position.
#[component]
pub fn GridPinnedCell<C: DataGridColumn + 'static>(
    children: Children,
    col: C,
    pinned_columns_signal: RwSignal<HashSet<C>>,
) -> impl IntoView {
    view! {
        <div
            role="gridcell"
            aria-colindex=col.colindex()
            data-name="GridCell"
            tabindex="-1"
            class="relative shrink-0 border-r opacity-100 bg-background"
            style=move || {
                let left = pinned_columns_signal.with(|p| get_pinned_left_position(col, p));
                let width = get_column_width(col);
                format!(
                    "left: {left}px; position: sticky; background: var(--background); width: {width}px; z-index: {PINNED_Z_INDEX};",
                )
            }
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn TableSeparator(
    #[prop(default = 60)] valuemin: i32,
    #[prop(default = 800)] valuemax: i32,
    valuenow: i32,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let merged_class = tw_merge!(
        "absolute top-0 -right-px z-50 w-0.5 h-full opacity-0 transition-opacity select-none hover:opacity-100 focus:outline-none after:-translate-x-1/2 cursor-ew-resize touch-none bg-border after:absolute after:inset-y-0 after:left-1/2 after:h-full after:w-[18px] after:content-[''] hover:bg-primary focus:bg-primary",
        class
    );

    view! {
        <div
            role="separator"
            data-name="TableSeparator"
            class=merged_class
            aria-orientation="vertical"
            aria-label="Redimensionar columna"
            aria-valuenow=valuenow.to_string()
            aria-valuemin=valuemin.to_string()
            aria-valuemax=valuemax.to_string()
            tabindex="0"
        ></div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */
