use leptos::prelude::*;
use tw_merge::tw_merge;

/* ========================================================== */
/*                       Context                              */
/* ========================================================== */

#[derive(Clone)]
struct TabsCtx {
    selected: RwSignal<String>,
    /// Prefix of the `tab`/`panel` element ids. Unique per tab group on the page.
    base: String,
}

#[derive(Clone, Copy)]
struct TabsListCtx {
    variant: TabsVariant,
}

/* ========================================================== */
/*                       Enums                                */
/* ========================================================== */

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsVariant {
    #[default]
    Default,
    Line,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[component]
pub fn Tabs(
    #[prop(into, optional)] class: String,
    #[prop(into, optional)] default_value: String,
    /// Prefix for the tab/panel ids. Two groups on one page must pass different
    /// values; the WAI-ARIA `aria-controls`/`aria-labelledby` pair needs
    /// document-unique ids to point at the right panel.
    #[prop(into, optional)]
    id: String,
    #[prop(default = TabsOrientation::Horizontal)] orientation: TabsOrientation,
    children: Children,
) -> impl IntoView {
    let selected = RwSignal::new(default_value);
    let base = if id.is_empty() {
        "tabs".to_string()
    } else {
        id
    };
    provide_context(TabsCtx { selected, base });

    let is_horizontal = orientation == TabsOrientation::Horizontal;
    let class = tw_merge!(
        "group/tabs flex gap-2",
        if is_horizontal {
            "flex-col"
        } else {
            "flex-row"
        },
        class
    );

    view! {
        <div class=class data-name="Tabs" data-orientation=if is_horizontal { "Horizontal" } else { "Vertical" }>
            {children()}
        </div>
    }
}

#[component]
pub fn TabsList(
    #[prop(into, optional)] class: String,
    #[prop(default = TabsVariant::Default)] variant: TabsVariant,
    children: Children,
) -> impl IntoView {
    provide_context(TabsListCtx { variant });

    let is_line = variant == TabsVariant::Line;
    // On a phone a many-tab strip scrolls horizontally instead of wrapping (a
    // wrapped row changes the control's height and moves the panel below it).
    // `overflow-x-auto` also clips at the padding edge, so the Line variant's
    // underline (`bottom:-5px`) gets `pb-2` of room and is not cut off.
    let horizontal_scroll = "max-md:max-w-full max-md:flex-nowrap max-md:overflow-x-auto max-md:[scrollbar-width:none] max-md:[&::-webkit-scrollbar]:hidden";
    let class = tw_merge!(
        "group/tabs-list inline-flex w-fit items-center justify-center rounded-lg p-[3px] text-muted-foreground",
        horizontal_scroll,
        // Orientation via group-data on parent Tabs element (works — Tailwind scans static strings)
        "group-data-[orientation=Horizontal]/tabs:h-8 group-data-[orientation=Horizontal]/tabs:max-md:h-11",
        "group-data-[orientation=Vertical]/tabs:h-fit group-data-[orientation=Vertical]/tabs:flex-col",
        if is_line {
            "gap-1 bg-transparent rounded-none p-0 max-md:pb-2"
        } else {
            "bg-muted"
        },
        class
    );

    view! {
        <div
            class=class
            data-name="TabsList"
            data-variant=if is_line { "Line" } else { "Default" }
            role="tablist"
        >
            {children()}
        </div>
    }
}

#[component]
pub fn TabsTrigger(
    #[prop(into)] value: String,
    #[prop(into, optional)] class: String,
    children: Children,
) -> impl IntoView {
    let ctx = expect_context::<TabsCtx>();
    let variant = expect_context::<TabsListCtx>().variant;
    let is_line = variant == TabsVariant::Line;

    let slug = tab_slug(&value);
    let trigger_id = format!("{}-tab-{slug}", ctx.base);
    let panel_id = format!("{}-panel-{slug}", ctx.base);
    let val = value.clone();
    let is_active = Memo::new(move |_| ctx.selected.get() == val);

    // Static classes that Tailwind can scan (group-data selectors reference parent data attrs)
    let base = tw_merge!(
        "relative inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center gap-1.5 rounded-md border border-transparent px-1.5 py-0.5 text-sm font-medium whitespace-nowrap transition-all cursor-pointer select-none max-md:px-3 max-md:min-h-10",
        "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none",
        "disabled:pointer-events-none disabled:opacity-50",
        // Vertical layout via group-data on parent
        "group-data-[orientation=Vertical]/tabs:w-full group-data-[orientation=Vertical]/tabs:justify-start",
        // After pseudo-element (underline indicator)
        "after:absolute after:bg-foreground after:transition-opacity",
        "group-data-[orientation=Horizontal]/tabs:after:inset-x-0 group-data-[orientation=Horizontal]/tabs:after:bottom-[-5px] group-data-[orientation=Horizontal]/tabs:after:h-0.5",
        "group-data-[orientation=Vertical]/tabs:after:inset-y-0 group-data-[orientation=Vertical]/tabs:after:-right-1 group-data-[orientation=Vertical]/tabs:after:w-0.5",
        class
    );

    view! {
        <button
            type="button"
            role="tab"
            id=trigger_id
            aria-controls=panel_id
            aria-selected=move || is_active.get().to_string()
            tabindex=move || if is_active.get() { "0" } else { "-1" }
            class=move || {
                let active = is_active.get();
                tw_merge!(
                    &base,
                    // Active state applied reactively (CSS attr selectors not generated by Tailwind)
                    if active { "text-foreground" } else { "text-foreground/60 hover:text-foreground" },
                    // Default variant: active gets white pill + shadow
                    if !is_line && active { "bg-background shadow-sm" } else { "" },
                    // Line variant: show underline when active
                    if is_line && active { "after:opacity-100" } else { "after:opacity-0" },
                )
            }
            data-name="TabsTrigger"
            data-value=value.clone()
            data-state=move || if is_active.get() { "Active" } else { "Inactive" }
            on:click=move |_| ctx.selected.set(value.clone())
            on:keydown={
                // WAI-ARIA tab keyboard model: arrows move between tabs,
                // Home/End jump to the first/last.
                #[cfg(target_arch = "wasm32")]
                {
                    let ctx = ctx.clone();
                    move |ev: leptos::ev::KeyboardEvent| move_tab_focus(&ev, &ctx)
                }
                #[cfg(not(target_arch = "wasm32"))]
                move |_ev: leptos::ev::KeyboardEvent| {}
            }
        >
            {children()}
        </button>
    }
}

#[component]
pub fn TabsContent(
    #[prop(into)] value: String,
    #[prop(into, optional)] class: String,
    children: Children,
) -> impl IntoView {
    let ctx = expect_context::<TabsCtx>();
    let slug = tab_slug(&value);
    let panel_id = format!("{}-panel-{slug}", ctx.base);
    let trigger_id = format!("{}-tab-{slug}", ctx.base);
    let val = value;
    let is_active = Memo::new(move |_| ctx.selected.get() == val);

    view! {
        <div
            data-name="TabsContent"
            role="tabpanel"
            id=panel_id
            aria-labelledby=trigger_id
            tabindex="0"
            class=move || {
                tw_merge!(
                    "flex-1 text-sm outline-none",
                // Orientation via group-data on parent
                "group-data-[orientation=Horizontal]/tabs:mt-2 group-data-[orientation=Vertical]/tabs:ml-4",
                &class,
                if is_active.get() { "" } else { "hidden" }
                )
            }
        >
            {children()}
        </div>
    }
}

/// A DOM-id-safe slug of a tab value: `sales orders` → `sales-orders`.
#[must_use]
pub fn tab_slug(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Move focus/selection between the sibling tabs of `ev`'s tablist.
#[cfg(target_arch = "wasm32")]
fn move_tab_focus(ev: &leptos::ev::KeyboardEvent, ctx: &TabsCtx) {
    use wasm_bindgen::JsCast;

    let Some(target) = ev
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let Some(list) = target.parent_element() else {
        return;
    };
    let Ok(tabs) = list.query_selector_all("[role='tab']") else {
        return;
    };
    let count = tabs.length();
    if count == 0 {
        return;
    }

    let index = (0..count).find(|index| {
        tabs.item(*index)
            .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
            .is_some_and(|element| element == target)
    });
    let Some(index) = index else {
        return;
    };

    let next = match ev.key().as_str() {
        "ArrowRight" | "ArrowDown" => (index + 1) % count,
        "ArrowLeft" | "ArrowUp" => (index + count - 1) % count,
        "Home" => 0,
        "End" => count - 1,
        _ => return,
    };
    ev.prevent_default();

    let Some(element) = tabs
        .item(next)
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    if let Some(value) = element.get_attribute("data-value") {
        ctx.selected.set(value);
    }
    let _ = element.focus();
}

#[cfg(test)]
mod tests {
    use super::tab_slug;

    #[test]
    fn a_tab_value_becomes_a_dom_safe_slug() {
        assert_eq!(tab_slug("General"), "general");
        assert_eq!(tab_slug("Sales Orders"), "sales-orders");
        assert_eq!(tab_slug("  "), "");
        assert_eq!(tab_slug("a.b/c"), "a-b-c");
    }
}
