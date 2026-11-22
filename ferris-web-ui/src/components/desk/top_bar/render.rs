//! The bar itself: the dumb dropdown, the per-action renderer and the
//! [`TopBar`] component that assembles breadcrumbs, title, status and the
//! generative action slot.

use leptos::prelude::*;
use leptos_router::components::A;

use icons::{ChevronsUpDown, Maximize2, Minimize2};

use crate::components::ui::badge::Badge;
use crate::components::ui::breadcrumb::{Crumb, build_breadcrumb_view};
use crate::components::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::components::ui::dropdown_menu::{
    DropdownMenu, DropdownMenuAction, DropdownMenuActionVariant, DropdownMenuAlign,
    DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator,
    DropdownMenuTrigger,
};
use crate::components::ui::icon::Icon;
use tw_merge::IntoTailwindClass;

use super::registry::{
    TopBarActions, use_top_bar_actions, use_top_bar_chrome, use_top_bar_status, use_top_bar_title,
};
use super::specs::{ButtonSpec, DropdownItem, DropdownSpec, SelectionMenuSpec, TopBarAction};

/// The shell's mobile drawer control, provided by the Desk so the top bar can
/// offer a menu button on a phone (where the rail is a drawer).
///
/// `available` is false while the device preference hides the drawer; the
/// button is then not drawn at all, instead of being a control that does
/// nothing.
#[derive(Clone, Copy)]
pub struct TopBarMobileMenu {
    pub open: RwSignal<bool>,
    pub available: Signal<bool>,
}

// ---------------------------------------------------------------------------
// Dumb dropdown
// ---------------------------------------------------------------------------

/// A plain dropdown: label + items. Delegates its open state to
/// [`DropdownMenu`] and knows nothing about the registry, indices, or selection.
#[component]
fn TopBarDropdown(spec: DropdownSpec) -> impl IntoView {
    let DropdownSpec {
        icon,
        items,
        label,
        aria_label,
        selectable,
    } = spec;

    // The children closure captures its copy of the label by move, and `view!`
    // builds it before the attributes, so the two can't share one variable.
    let text = label.clone();
    let accessible_name = aria_label.unwrap_or(label);
    // An icon-only dropdown ("⋯") is the same square as the other icon buttons
    // in the bar: with a labelled size, the empty label span still took the
    // base gap and the icon-only padding, so the trigger sat wider than the
    // buttons beside it (and, being last, looked like it carried a stray gap).
    let has_label = !text.trim().is_empty();
    let size = if has_label {
        ButtonSize::Sm
    } else {
        ButtonSize::IconSm
    };

    view! {
        <DropdownMenu align=DropdownMenuAlign::End>
            <DropdownMenuTrigger as_child=true>
                <button
                    type="button"
                    class=move || {
                        crate::components::ui::button::ButtonClass {
                            variant: ButtonVariant::Outline,
                            size,
                        }
                        .with_class("")
                    }
                    aria-label=accessible_name.clone()
                >
                    <Icon name=icon />
                    {has_label.then(|| view! { <span class="max-md:sr-only">{text}</span> })}
                    {selectable.then(|| view! { <ChevronsUpDown /> })}
                </button>
            </DropdownMenuTrigger>

            <DropdownMenuContent>
                <DropdownMenuGroup>
                    {items.into_iter().map(render_dropdown_item).collect_view()}
                </DropdownMenuGroup>
            </DropdownMenuContent>
        </DropdownMenu>
    }
}

/// Render one [`DropdownItem`] — a divider, a link (`href`) or an action.
fn render_dropdown_item(item: DropdownItem) -> AnyView {
    let DropdownItem {
        label,
        icon,
        destructive,
        disabled,
        separator,
        href,
        on_click,
    } = item;

    // A divider is not a row: it is the line between two groups, so it is not
    // wrapped in `DropdownMenuItem` (that wrapper carries the item padding).
    if separator {
        return view! { <DropdownMenuSeparator /> }.into_any();
    }

    if let Some(href) = href {
        view! {
            <DropdownMenuItem>
                <DropdownMenuAction class="gap-2" href=href disabled=disabled>
                    <Icon name=icon />
                    {label}
                </DropdownMenuAction>
            </DropdownMenuItem>
        }
        .into_any()
    } else {
        // `on_click` goes through the prop, never a spread `on:click=`: an
        // any-attr listener is re-applied on every rebuild without removing the
        // previous one, so it accumulates closures from past renders (the
        // anti-pattern `DropdownMenuAction` warns about). The prop replaces the
        // button's handler cleanly; old listeners died holding a disposed
        // callback and clicking "Delete" panicked.
        view! {
            <DropdownMenuItem>
                <DropdownMenuAction
                    class="gap-2"
                    variant=if destructive {
                        DropdownMenuActionVariant::Destructive
                    } else {
                        DropdownMenuActionVariant::Default
                    }
                    on_click=on_click
                    disabled=disabled
                >
                    <Icon name=icon />
                    {label}
                </DropdownMenuAction>
            </DropdownMenuItem>
        }
        .into_any()
    }
}

// ---------------------------------------------------------------------------
// Action rendering
// ---------------------------------------------------------------------------

/// Render a single [`TopBarAction`] as the matching control.
#[component]
fn TopBarActionView(action: TopBarAction) -> impl IntoView {
    match action {
        TopBarAction::Button(ButtonSpec {
            label,
            aria_label,
            icon,
            variant,
            disabled,
            on_click,
        }) => {
            let text = label.clone();
            let accessible_name = aria_label.unwrap_or(label.clone());
            let button_variant = variant;
            // Icon-only actions (undo/redo, prev/next, refresh, …) are square
            // icon buttons; a labelled one keeps the small size. Rendering an
            // empty label span at `Sm` used to add the base gap and the larger
            // horizontal padding to every icon-only button, so they never
            // lined up with the icon buttons from `Button`.
            let has_label = !text.trim().is_empty();
            let button_size = if has_label {
                ButtonSize::Sm
            } else {
                ButtonSize::IconSm
            };
            view! {
                <button
                    type="button"
                    class=move || {
                        let base = crate::components::ui::button::ButtonClass {
                            variant: button_variant,
                            size: button_size,
                        };
                        base.with_class("")
                    }
                    aria-label=accessible_name.clone()
                    disabled=disabled
                    on:click=move |_| {
                        // A disposed callback is a handler that outlived its
                        // view: no-op, never a panic (see module docs).
                        let _ = on_click.try_run(());
                    }
                >
                    {if let Some(icon_name) = icon {
                        view! { <Icon name=icon_name /> }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {has_label
                        .then(|| view! { <span class="max-md:sr-only">{text}</span> })}
                </button>
            }
            .into_any()
        }

        TopBarAction::Dropdown(spec) => view! {
            <TopBarDropdown spec=spec />
        }
        .into_any(),

        TopBarAction::FullScreenToggle(spec) => {
            let label = if spec.active {
                "Salir de pantalla completa"
            } else {
                "Pantalla completa"
            };
            let on_click = spec.on_click;
            let active = spec.active;
            view! {
                <Button
                    variant=ButtonVariant::Ghost
                    size=ButtonSize::IconSm
                    on:click=move |_| {
                        // Disposed callback = a click on a view already gone.
                        let _ = on_click.try_run(());
                    }
                    attr:aria-label=label
                    attr:title=label
                >
                    {if active {
                        view! { <Minimize2 class="size-4" /> }.into_any()
                    } else {
                        view! { <Maximize2 class="size-4" /> }.into_any()
                    }}
                </Button>
            }
            .into_any()
        }

        TopBarAction::SelectionMenu(SelectionMenuSpec {
            idle_label,
            idle_icon,
            on_idle_click,
            selection,
        }) => view! {
            {move || match selection.get() {
                Some(sel) => view! {
                    <TopBarDropdown
                        spec=DropdownSpec {
                            label: sel.label,
                            aria_label: None,
                            icon: idle_icon,
                            items: sel.items,
                            selectable: false
                        }
                    />
                }
                .into_any(),
                None => {
                    let text = idle_label.clone();
                    let on_idle_click = on_idle_click;
                    view! {
                        <button
                            type="button"
                            class=move || {
                                crate::components::ui::button::ButtonClass {
                                    variant: ButtonVariant::Default,
                                    size: ButtonSize::Sm,
                                }
                                .with_class("")
                            }
                            aria-label=idle_label.clone()
                            on:click=move |_| {
                                // A disposed callback is a handler that outlived its
                                // view: no-op, never a panic (see module docs).
                                let _ = on_idle_click.try_run(());
                            }
                        >
                            <Icon name=idle_icon />
                            <span class="max-md:sr-only">{text}</span>
                        </button>
                    }
                    .into_any()
                }
            }}
        }
        .into_any(),
    }
}

/// Renders the current view's actions in one dynamic block.
///
/// In full-screen mode only the full-screen toggle survives: the board owns the
/// screen, so its edit controls (and every other action) are hidden while the
/// bar keeps the way out.
#[component]
fn TopBarActionsSlot(actions: TopBarActions, fullscreen: Signal<bool>) -> impl IntoView {
    view! {
        {move || {
            let mut list = actions.get();
            if fullscreen.get() {
                list.retain(|action| matches!(action, TopBarAction::FullScreenToggle(_)));
            }
            if list.is_empty() {
                view! { <span></span> }.into_any()
            } else {
                list.into_iter()
                    .map(|a| view! { <TopBarActionView action=a /> }.into_any())
                    .collect_view()
                    .into_any()
            }
        }}
    }
}

// ---------------------------------------------------------------------------
// Top bar
// ---------------------------------------------------------------------------

#[component]
pub fn TopBar(
    /// Extra content after the breadcrumbs, title and the status indicator
    /// (the form's `form.toolbar.left` anchor). `ferris-web-ui` cannot know
    /// `UiSlot` (the dependency goes the other way), so the caller hands the
    /// view in.
    #[prop(optional, into)]
    leading: Option<AnyView>,
    /// Extra content at the end of the right-hand actions row. The Desk
    /// passes its OpenUI `shell.topbar.right` anchor here: `ferris-web-ui`
    /// cannot know `UiSlot` (the dependency goes the other way), so the
    /// caller hands the view in.
    #[prop(optional, into)]
    trailing: Option<AnyView>,
    /// The trail to draw, already resolved by the caller: `ferris-web-ui` does
    /// not know the route table, so it cannot tell a module label apart from a
    /// page a crumb may link to. A signal, not a `Vec`: the bar is built once
    /// per shell render but must re-derive its trail on navigation.
    #[prop(into)]
    breadcrumbs: Signal<Vec<Crumb>>,
) -> impl IntoView {
    let actions = use_top_bar_actions();
    // Optional: only the Desk shell provides a mobile drawer.
    let mobile_menu = use_context::<TopBarMobileMenu>();
    let status = use_top_bar_status();
    let title = use_top_bar_title();
    let chrome = use_top_bar_chrome();
    let fullscreen = Signal::derive(move || chrome.get().fullscreen);
    let show_title = Signal::derive(move || chrome.get().title);

    view! {
        <header class="ferris-topbar border-b sticky top-0 z-30">
            <div class="h-12 flex items-center justify-between gap-2 px-3">
                // Left, normal mode: breadcrumbs, view title, status indicator
                // and the form's anchors. Full screen hides the cluster instead
                // of unmounting it: `leading` is a one-shot view, and a dynamic
                // branch would move it out on the second render.
                <div class=move || {
                    if fullscreen.get() {
                        "hidden"
                    } else {
                        "hidden md:flex min-w-0 items-center gap-2"
                    }
                }>
                    {move || build_breadcrumb_view(breadcrumbs.get())}
                    {move || {
                        title
                            .get()
                            .map(|text| view! { <span class="min-w-0 truncate text-sm font-semibold">{text}</span> })
                    }}
                    {move || {
                        status
                            .get()
                            .map(|s| view! { <Badge variant=s.variant>{s.label}</Badge> })
                    }}
                    {leading}
                </div>

                // Left, full screen: the title, and only when the board keeps
                // it (`title_full_screen`); the toggle alone is a valid bar.
                <div class=move || {
                    if fullscreen.get() {
                        "flex min-w-0 items-center gap-2"
                    } else {
                        "hidden"
                    }
                }>
                    {move || {
                        if !show_title.get() {
                            return None;
                        }
                        title
                            .get()
                            .map(|text| view! { <span class="min-w-0 truncate text-sm font-semibold">{text}</span> })
                    }}
                </div>

                // Left, phones: the breadcrumb trail does not fit, but a phone
                // still needs to know where it is and how to go back. The
                // current crumb names the page and the nearest ancestor with a
                // real href becomes a back control — without it the only way
                // out of a form was the browser's own back button.
                <div class=move || {
                    if fullscreen.get() {
                        "hidden"
                    } else {
                        "flex min-w-0 flex-1 items-center gap-1.5 md:hidden"
                    }
                }>
                    {mobile_menu.map(|menu| view! {
                        <Show when=move || menu.available.get()>
                            <button
                                type="button"
                                aria-label="Abrir menú"
                                class="inline-flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 max-md:size-9"
                                on:click=move |_| menu.open.update(|open| *open = !*open)
                            >
                                <Icon name="Menu" class="size-4" />
                            </button>
                        </Show>
                    })}
                    {move || {
                        let crumbs = breadcrumbs.get();
                        let back = crumbs
                            .iter()
                            .rev()
                            .skip_while(|crumb| !crumb.is_current)
                            .skip(1)
                            .find_map(|crumb| crumb.href.clone());
                        let label = crumbs
                            .iter()
                            .rev()
                            .find(|crumb| crumb.is_current)
                            .map(|crumb| crumb.label.clone())
                            .or_else(|| crumbs.last().map(|crumb| crumb.label.clone()))
                            .unwrap_or_default();
                        // A document page ("TASK00001") says nothing about where
                        // it lives. When the crumb right before it links to the
                        // collection — always the DocType list for a form — the
                        // phone shows that as a small line above the name
                        // ("Task" over "TASK00001") instead of one long line
                        // that truncates to "Task · TAS…". Other trails (list,
                        // report, kanban, settings) keep just the leaf, because
                        // their preceding crumb is the module or the section,
                        // not a place the document belongs to.
                        let parent = crumbs
                            .iter()
                            .position(|crumb| crumb.is_current)
                            .filter(|pos| *pos >= 2)
                            .map(|pos| &crumbs[pos - 1])
                            .filter(|previous| previous.href.is_some())
                            .map(|previous| previous.label.clone());
                        view! {
                            {back.map(|href| view! {
                                <A
                                    href=href
                                    attr:class="inline-flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                    attr:aria-label="Volver"
                                >
                                    <Icon name="ChevronLeft" class="size-4" />
                                </A>
                            })}
                            <span class="flex min-w-0 flex-1 flex-col justify-center leading-tight">
                                {parent.map(|parent| view! {
                                    <span class="truncate text-xs text-muted-foreground">{parent}</span>
                                })}
                                <span class="truncate text-sm font-medium text-foreground">
                                    {label}
                                </span>
                            </span>
                        }
                    }}
                </div>

                // Right: the generative actions slot. Full screen filters it
                // down to the toggle, and hides the caller's extra anchors.
                // On a phone the row scrolls instead of pushing the title out
                // of the viewport or clipping the last action.
                <div class="flex min-w-0 items-center gap-1.5 max-md:max-w-[55%] max-md:overflow-x-auto max-md:overscroll-x-contain">
                    <TopBarActionsSlot actions=actions fullscreen=fullscreen />
                    <div class=move || {
                        if fullscreen.get() {
                            "hidden"
                        } else {
                            "flex items-center"
                        }
                    }>{trailing}</div>
                </div>
            </div>
        </header>
    }
}
