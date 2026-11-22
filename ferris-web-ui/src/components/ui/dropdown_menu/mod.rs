use icons::Check;
use leptos::context::Provider;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use leptos_ui::clx;
use tw_merge::*;

use super::separator::Separator;
use crate::components::hooks::use_unique_id::use_unique_id_for;
use ferris_web_shared::url::safe_href;
use place::place;

mod place;

mod components {
    use super::*;
    clx! {DropdownMenuLabel, span, "px-2 py-1.5 text-sm font-medium data-inset:pl-8", "mb-1"}
    clx! {DropdownMenuGroup, ul, "group"}
    // `max-md:min-h-10`: a menu row is a tap target on a phone, where the
    // desktop `py-1.5` row is ~33px — below the 40px a finger needs. The
    // desktop metrics are untouched.
    clx! {DropdownMenuItem, li, "inline-flex gap-2 items-center w-full rounded-sm px-2 py-1.5 text-sm no-underline transition-colors duration-200 text-popover-foreground hover:bg-accent hover:text-accent-foreground max-md:min-h-10 [&_svg:not([class*='size-'])]:size-4"}
    clx! {DropdownMenuItemLabel, span, "min-w-0 flex-1 text-left truncate"}
    clx! {DropdownMenuItemTrailing, span, "ml-auto flex shrink-0 items-center gap-1 text-muted-foreground [&_svg:not([class*='size-'])]:size-4"}
    clx! {DropdownMenuLink, a, "w-full inline-flex gap-2 items-center"}
}

/// A row that hosts a control of its own (a toggle, a checkbox). Unlike
/// `DropdownMenuAction` it carries no `data-dropdown-close`, so the menu stays
/// open while that control is being used.
///
/// Hand-written rather than `clx!`d so it can take the same `on_click` prop:
/// see [`DropdownMenuAction`] for why the click handler cannot be an any-attr.
#[component]
pub fn DropdownMenuCustomItem(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(optional)] on_click: Option<Callback<()>>,
) -> impl IntoView {
    let class = tw_merge!(
        "flex items-center gap-2.5 w-full rounded-sm px-2 py-1.5 text-sm cursor-pointer text-popover-foreground transition-colors duration-200 hover:bg-accent hover:text-accent-foreground max-md:min-h-10 [&_svg:not([class*='size-'])]:size-4",
        class
    );
    let on_action = move |_| {
        if let Some(on_click) = &on_click {
            // `try_run`: the publisher's owner may already be gone — a click on
            // a menu that outlived its view is a no-op, not a panic.
            let _ = on_click.try_run(());
        }
    };

    view! {
        <li
            data-name="DropdownMenuCustomItem"
            class=class
            on:click=on_action
        >
            {children()}
        </li>
    }
}

pub use components::*;

/// The menu's own separator, spacing included: the shared [`Separator`] is
/// unspaced, and the rhythm between menu rows belongs to the menu.
#[component]
pub fn DropdownMenuSeparator(#[prop(optional, into)] class: String) -> impl IntoView {
    let merged_class = tw_merge!("my-1", class);

    view! { <Separator class=merged_class /> }
}

/* ========================================================== */
/*                     RADIO GROUP                            */
/* ========================================================== */

#[derive(Clone)]
struct DropdownMenuRadioContext<T: Clone + PartialEq + Send + Sync + 'static> {
    value_signal: RwSignal<T>,
}

/// A group of radio items where only one can be selected at a time.
#[component]
pub fn DropdownMenuRadioGroup<T>(
    children: Children,
    /// The signal holding the current selected value
    value: RwSignal<T>,
) -> impl IntoView
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let ctx = DropdownMenuRadioContext {
        value_signal: value,
    };

    view! {
        <Provider value=ctx>
            <ul data-name="DropdownMenuRadioGroup" role="group" class="group">
                {children()}
            </ul>
        </Provider>
    }
}

/// A radio item that shows a checkmark when selected.
#[component]
pub fn DropdownMenuRadioItem<T>(
    children: Children,
    /// The value this item represents
    value: T,
    #[prop(optional, into)] class: String,
) -> impl IntoView
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let ctx = expect_context::<DropdownMenuRadioContext<T>>();

    let value_for_check = value.clone();
    let value_for_click = value;
    let is_selected = move || ctx.value_signal.get() == value_for_check;

    let merged_class = tw_merge!(
        "group inline-flex gap-2 items-center w-full rounded-sm pl-2 pr-2 py-1.5 text-sm cursor-pointer no-underline transition-colors duration-200 text-popover-foreground hover:bg-accent hover:text-accent-foreground max-md:min-h-10 [&_svg:not([class*='size-'])]:size-4",
        class
    );

    view! {
        <li
            data-name="DropdownMenuRadioItem"
            class=merged_class
            role="menuitemradio"
            aria-checked=move || is_selected().to_string()
            data-dropdown-close="true"
            on:click=move |_| {
                ctx.value_signal.set(value_for_click.clone());
            }
        >
            {children()}
            <Check class="ml-auto opacity-0 size-4 text-muted-foreground group-aria-checked:opacity-100" />
        </li>
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuActionVariant {
    #[default]
    Default,
    Destructive,
    Muted,
}

/// An action item in a dropdown menu (no checkmark, just triggers an action).
///
/// `on_click` is a prop rather than an `on:click=` any-attr on purpose. An
/// any-attr spread onto a component is re-applied to the component's elements on
/// every rebuild *without the previous listener being removed*
/// (`AnyViewWithAttrs::rebuild`), so the row would accumulate listeners, each one
/// still holding the closure of an earlier render.
#[component]
pub fn DropdownMenuAction(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] href: Option<String>,
    #[prop(optional)] on_click: Option<Callback<()>>,
    #[prop(default = DropdownMenuActionVariant::default())] variant: DropdownMenuActionVariant,
    #[prop(optional)] disabled: bool,
) -> impl IntoView {
    let _ctx = expect_context::<DropdownMenuContext>();

    let variant_class = match variant {
        DropdownMenuActionVariant::Default => {
            "text-popover-foreground hover:bg-accent hover:text-accent-foreground"
        }
        DropdownMenuActionVariant::Destructive => {
            "text-destructive hover:bg-destructive/10 hover:text-destructive"
        }
        DropdownMenuActionVariant::Muted => {
            "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
        }
    };

    // A disabled item must be present but inert: no click, no navigation, and
    // the visual affordance comes from the `disabled:` Tailwind variants.
    let state_class = if disabled {
        "disabled:opacity-50 disabled:pointer-events-none"
    } else {
        ""
    };
    // A disabled or unsafe destination must not navigate. A sanitized value
    // that fails the allow-list falls back to the button branch below.
    let href = if disabled { None } else { href };
    let href = href.and_then(|href| safe_href(&href));

    let class = tw_merge!(
        "inline-flex gap-2 items-center w-full text-sm text-left transition-colors duration-200 focus:outline-none focus-visible:outline-none max-md:min-h-10 [&_svg:not([class*='size-'])]:size-4",
        variant_class,
        state_class,
        class
    );

    if let Some(href) = href {
        // Render as <a> tag when href is provided: a link navigates, so there is
        // nothing to run on click.
        view! {
            <a data-name="DropdownMenuAction" class=class href=href data-dropdown-close="true">
                {children()}
            </a>
        }
        .into_any()
    } else {
        // Render as <button> tag when no href
        let on_action = move |_| {
            if !disabled {
                if let Some(on_click) = &on_click {
                    // See `DropdownMenuCustomItem` above: `try_run`, not `run`.
                    let _ = on_click.try_run(());
                }
            }
        };
        view! {
            <button
                type="button"
                data-name="DropdownMenuAction"
                class=class
                data-dropdown-close="true"
                aria-disabled=disabled
                disabled=disabled
                on:click=on_action
            >
                {children()}
            </button>
        }
        .into_any()
    }
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuAlign {
    #[default]
    Start,
    StartOuter,
    End,
    EndOuter,
    Center,
}

#[derive(Clone)]
struct DropdownMenuContext {
    target_id: String,
    align: DropdownMenuAlign,
    /// The single source of truth for the menu. The panel is always in the DOM;
    /// this flag is what decides whether it is shown.
    open: RwSignal<bool>,
}

#[component]
pub fn DropdownMenu(
    children: Children,
    #[prop(default = DropdownMenuAlign::default())] align: DropdownMenuAlign,
) -> impl IntoView {
    let dropdown_target_id = use_unique_id_for("dropdown");

    let ctx = DropdownMenuContext {
        target_id: dropdown_target_id,
        align,
        open: RwSignal::new(false),
    };

    let root = NodeRef::<Div>::new();
    // The trigger and the panel are siblings inside `root`, so a press that
    // misses both of them is the only thing that counts as "outside". This
    // listens on `mousedown` while the trigger toggles on `click`, so pressing
    // one trigger also dismisses whichever menu was open.
    let outside = window_event_listener(leptos::ev::mousedown, move |ev: web_sys::MouseEvent| {
        if !ctx.open.get_untracked() {
            return;
        }
        let (Some(Some(root)), Some(target)) = (root.try_get_untracked(), ev.target()) else {
            return;
        };
        let Ok(target) = target.dyn_into::<web_sys::Node>() else {
            return;
        };
        if !root.contains(Some(&target)) {
            ctx.open.set(false);
        }
    });
    on_cleanup(move || outside.remove());

    let escape = ctx.clone();
    let _escape = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            escape.open.set(false);
        }
    });
    on_cleanup(move || _escape.remove());

    let closer = ctx.clone();
    view! {
        <Provider value=ctx>
            <div
                node_ref=root
                data-name="DropdownMenu"
                on:click=move |ev: web_sys::MouseEvent| {
                    // Dismiss the menu once an item marked `data-dropdown-close`
                    // has been pressed. Its own handler already ran (this listens
                    // on the bubbling phase), and a control rendered inside a
                    // `DropdownMenuCustomItem` carries no marker, which is what
                    // keeps the menu open for it.
                    let Some(target) = ev.target() else { return };
                    let Ok(target) = target.dyn_into::<web_sys::Element>() else { return };
                    if target.closest("[data-dropdown-close]").ok().flatten().is_some() {
                        closer.open.set(false);
                    }
                }
            >
                {children()}
            </div>
        </Provider>
    }
}

#[component]
pub fn DropdownMenuTrigger(
    children: Children,
    #[prop(optional, into)] class: String,
    /// Render children directly instead of wrapping in a button.
    /// Use when the child is already a button (e.g. InputGroupButton) to avoid nested buttons.
    #[prop(optional)]
    as_child: bool,
) -> impl IntoView {
    let ctx = expect_context::<DropdownMenuContext>();

    let state = move || if ctx.open.get() { "open" } else { "closed" };
    let expanded = move || ctx.open.get().to_string();
    let toggle = move |_| ctx.open.update(|open| *open = !*open);

    if as_child {
        return view! {
            <span
                data-name="DropdownMenuTrigger"
                data-dropdown-trigger=ctx.target_id.clone()
                data-state=state
                aria-haspopup="menu"
                aria-expanded=expanded
                class="group contents"
                on:click=toggle
            >
                {children()}
            </span>
        }
        .into_any();
    }

    let button_class = tw_merge!(
        "px-4 py-2 h-9 inline-flex justify-center items-center text-sm font-medium whitespace-nowrap rounded-md transition-colors w-fit max-md:h-10 focus:outline-none focus:ring-1 focus:ring-ring focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&_svg:not([class*='size-'])]:size-4  border bg-input border-input hover:bg-accent hover:text-accent-foreground",
        class
    );

    view! {
        <button
            type="button"
            class=button_class
            data-name="DropdownMenuTrigger"
            data-dropdown-trigger=ctx.target_id
            data-state=state
            aria-haspopup="menu"
            aria-expanded=expanded
            tabindex="0"
            on:click=toggle
        >
            {children()}
        </button>
    }
    .into_any()
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuPosition {
    #[default]
    Auto,
    Top,
    Bottom,
}

#[component]
pub fn DropdownMenuContent(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = DropdownMenuPosition::default())] position: DropdownMenuPosition,
    /// Animate the panel with a scale transition.
    ///
    /// Turn it off for a menu that hosts another `DropdownMenu`: a `transform`
    /// establishes a containing block for the nested panel's `position: fixed`,
    /// so its viewport coordinates would be measured from this menu instead of
    /// the viewport and the second menu would open offset by the first one's
    /// position.
    #[prop(default = true)]
    animated: bool,
) -> impl IntoView {
    let ctx = expect_context::<DropdownMenuContext>();

    let base_classes = if animated {
        "z-[70] p-1 rounded-xl border bg-card shadow-lg h-fit fixed max-w-[calc(100vw-1rem)] transition-all duration-200 data-[state=closed]:opacity-0 data-[state=closed]:scale-95 data-[state=open]:opacity-100 data-[state=open]:scale-100 max-md:max-h-[calc(100dvh-1rem)] max-md:overflow-y-auto max-md:overscroll-contain"
    } else {
        "z-[70] p-1 rounded-xl border bg-card shadow-lg h-fit fixed max-w-[calc(100vw-1rem)] transition-colors duration-200 data-[state=closed]:opacity-0 data-[state=open]:opacity-100 max-md:max-h-[calc(100dvh-1rem)] max-md:overflow-y-auto max-md:overscroll-contain"
    };
    // A centred panel (the module header) sizes to its content; `place`
    // floors only the Start/End menus at the trigger's width. A percentage
    // floor cannot be used here: a fixed element resolves percentages against
    // the viewport, so it would span the screen.
    let width_class = match ctx.align {
        DropdownMenuAlign::Center => "w-max",
        _ => "w-56",
    };

    let class = tw_merge!(width_class, base_classes, class);

    let align_for_style = match ctx.align {
        DropdownMenuAlign::Start => "start",
        DropdownMenuAlign::StartOuter => "start-outer",
        DropdownMenuAlign::End => "end",
        DropdownMenuAlign::EndOuter => "end-outer",
        DropdownMenuAlign::Center => "center",
    };

    let position_for_style = match position {
        DropdownMenuPosition::Auto => "auto",
        DropdownMenuPosition::Top => "top",
        DropdownMenuPosition::Bottom => "bottom",
    };

    let panel = NodeRef::<Div>::new();
    let id = ctx.target_id.clone();
    let id_on_resize = id.clone();
    // The panel is `position: fixed`, so it has to be placed against the
    // trigger's box — numbers only a browser has, hence the effect. It also
    // re-places on resize, which is when an `align`-based position goes stale.
    Effect::new(move |_| {
        if ctx.open.get() {
            place(panel, &id, ctx.align, position);
        }
    });
    let resize = window_event_listener(leptos::ev::resize, move |_| {
        if ctx.open.get_untracked() {
            place(panel, &id_on_resize, ctx.align, position);
        }
    });
    on_cleanup(move || resize.remove());

    view! {
        <div
            node_ref=panel
            data-name="DropdownMenuContent"
            class=class
            id=ctx.target_id
            data-target="target__dropdown"
            role="menu"
            data-state=move || if ctx.open.get() { "open" } else { "closed" }
            data-align=align_for_style
            data-position=position_for_style
            // Property-level, not the whole `style` attribute: `place` writes
            // `top`/`left`/`min-width` on this same element, and a whole-attribute
            // binding erased them when the panel closed — it jumped and its width
            // collapsed mid-transition.
            style:pointer-events=move || if ctx.open.get() { "auto" } else { "none" }
        >
            {children()}
        </div>


    }
}

mod submenu;

pub use submenu::*;
