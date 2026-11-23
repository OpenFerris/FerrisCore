use super::*;

#[component]
pub fn Grid(
    children: Children,
    #[prop(into)] rowcount: Signal<i32>,
    colcount: i32,
    style: &'static str,
    #[prop(optional, into)] class: String,
    #[prop(optional)] node_ref: Option<NodeRef<html::Div>>,
) -> impl IntoView {
    // NOTE: Avoid `select-none` here to allow text selection via double-click
    //
    // `min-w-0` keeps this element from forcing its `GridWrapper` wider than the
    // viewport: the columns are fixed widths and live inside this scroll port,
    // so a narrow screen scrolls here (header and pinned columns are sticky
    // against this element) rather than growing the page.
    let merged_class = tw_merge!(
        "grid overflow-auto relative min-w-0 rounded-md border focus:outline-none",
        class
    );

    view! {
        <div
            node_ref=node_ref.unwrap_or_default()
            role="grid"
            data-name="DataGrid"
            class=merged_class
            aria-label=crate::i18n::t("ui.data_grid.label")
            aria-rowcount=move || rowcount.get().to_string()
            aria-colcount=colcount.to_string()
            tabindex="0"
            style=style
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn GridBody(
    children: Children,
    #[prop(into)] style: Signal<String>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let merged_class = tw_merge!("grid relative", class);

    view! {
        <div
            role="rowgroup"
            data-name="GridBody"
            class=merged_class
            aria-label=crate::i18n::t("ui.data_grid.body")
            tabindex="0"
            style=move || style.get()
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ VIRTUALIZED GRID ✨                 */
/* ========================================================== */

/// A Grid wrapper that provides virtual scrolling context.
/// Use with `VirtualizedGridBody` and `VirtualFor` for automatic virtualization.
///
/// # Example
/// ```ignore
/// let total_rows = Signal::derive(move || data.get().len());
///
/// <VirtualizedGrid total_rows rowcount=1000 colcount=10 style=GRID_STYLE.as_str()>
///     <GridHeader ... />
///     <VirtualizedGridBody>
///         <VirtualFor
///             data=Signal::derive(move || data.get())
///             key=|row| row.id
///             children=move |idx, row| view! { <GridRow ... /> }
///         />
///     </VirtualizedGridBody>
/// </VirtualizedGrid>
/// ```
#[component]
pub fn VirtualizedGrid(
    children: Children,
    #[prop(into)] total_rows: Signal<usize>,
    #[prop(into)] rowcount: Signal<i32>,
    colcount: i32,
    style: &'static str,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let grid_ref = NodeRef::<html::Div>::new();
    let virtual_scroll = use_virtual_scroll(grid_ref, total_rows);

    // Provide context for child components
    provide_context(virtual_scroll);

    view! {
        <Grid rowcount=rowcount colcount=colcount style=style class=class node_ref=grid_ref>
            {children()}
        </Grid>
    }
}

/// Grid body that automatically uses virtual scroll context for height.
/// Must be used within a `VirtualizedGrid`.
#[component]
pub fn VirtualizedGridBody(
    children: Children,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let virtual_scroll = expect_context::<VirtualScrollState>();

    let merged_class = tw_merge!("grid relative", class);

    view! {
        <div
            role="rowgroup"
            data-name="GridBody"
            class=merged_class
            aria-label=crate::i18n::t("ui.data_grid.body")
            tabindex="0"
            style=move || format!("height: {}px;", virtual_scroll.total_height.get())
        >
            {children()}
        </div>
    }
}

/// A virtualized For loop that only renders visible rows.
/// Must be used within a `VirtualizedGrid`.
///
/// NOTE: The inner `<For>` is keyed by the item's key (not by index) to ensure
/// rows re-render when data changes. This fixes issues where deleting rows
/// would not update the UI because Leptos reuses views with matching keys.
#[component]
pub fn VirtualFor<T, K, KF, CF, CV>(
    #[prop(into)] data: Signal<Vec<T>>,
    key: KF,
    children: CF,
) -> impl IntoView
where
    T: Clone + Send + Sync + 'static,
    K: Eq + std::hash::Hash + Clone + Send + 'static,
    KF: Fn(&T) -> K + Clone + Send + Sync + 'static,
    CF: Fn(usize, T) -> CV + Clone + Send + Sync + 'static,
    CV: IntoView + 'static,
{
    let virtual_scroll = expect_context::<VirtualScrollState>();

    let key_fn = StoredValue::new(key);

    view! {
        <For
            each=move || {
                let start = virtual_scroll.start_index.get();
                let end = virtual_scroll.end_index.get();
                data.with(|rows| {
                    (start..end).filter_map(|idx| rows.get(idx).map(|row| (idx, row.clone()))).collect::<Vec<_>>()
                })
            }
            // Key by (index, item_key) so views update when items move position
            key=move |(idx, row): &(usize, T)| {
                let k = key_fn.with_value(|kf| kf(row));
                (*idx, k)
            }
            children=move |(idx, row)| { children.clone()(idx, row).into_any() }
        />
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

/// Grid row with absolute positioning for virtual scrolling.
/// NOTE: `index` must be a Signal to ensure translateY updates when rows are reordered
/// (e.g., during sorting). With keyed `<For>` loops, Leptos reuses DOM elements,
/// so static index values would cause rows to stay in their original visual positions.
#[component]
pub fn GridRow(
    children: Children,
    rowindex: usize,
    #[prop(into)] index: Signal<usize>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    // `w-max min-w-full` (not `w-full`) so the row border spans every column
    // when the grid is wider than its viewport; with `w-full` the row would end
    // at the scroll port edge and the trailing columns would scroll in without a
    // bottom border.
    let merged_class = tw_merge!("flex absolute w-max min-w-full border-b", class);

    view! {
        <div
            role="row"
            data-name="GridRow"
            aria-rowindex=rowindex.to_string()
            aria-selected="false"
            data-index=move || index.get().to_string()
            class=merged_class
            tabindex="-1"
            // Performance: content-visibility:auto skips rendering for off-screen rows
            style=move || {
                let translate_y = index.get() * 36;
                format!(
                    "height: 36px; transform: translateY({translate_y}px); content-visibility: auto; contain-intrinsic-size: auto 36px;",
                )
            }
        >
            {children()}
        </div>
    }
}

/* ========================================================== */
/*                     ✨ CONSTANTS ✨                        */
/* ========================================================== */

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */
