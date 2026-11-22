//! Submenus: the row that unfolds a nested panel, and the panel itself.
//!
//! Split out of `dropdown_menu/mod.rs` to keep that module under the
//! repository's 600-line limit; the parent re-exports every component here, so
//! `components::ui::dropdown_menu::DropdownMenuSub*` keeps resolving.

use icons::ChevronRight;
use leptos::prelude::*;
use tw_merge::tw_merge;

use super::DropdownMenuContext;

/// The open state of one submenu, provided by [`DropdownMenuSub`] and read by
/// the [`DropdownMenuSubContent`] nested directly inside it.
///
/// The state lives in Rust, not in a `group-hover/sub:` class, because a CSS
/// hover selector matches *any* ancestor: with a nested submenu (a workspace's
/// folders inside the workspaces menu) hovering the outer row would also reveal
/// every inner panel at once. A signal scoped to the nearest provider opens
/// exactly one level.
#[derive(Clone, Copy)]
struct DropdownMenuSubContext {
    open: RwSignal<bool>,
}

#[component]
pub fn DropdownMenuSub(children: Children) -> impl IntoView {
    let open = RwSignal::new(false);
    provide_context(DropdownMenuSubContext { open });

    // A submenu opened by hover can outlive the menu itself: a click that closes
    // the menu leaves the pointer over a row that is now hidden, so no
    // `mouseleave` follows. Reset on the menu's own close so the next open
    // starts closed.
    let menu_open = expect_context::<DropdownMenuContext>().open;
    Effect::new(move |_| {
        if !menu_open.get() {
            open.set(false);
        }
    });

    view! {
        <li
            data-name="DropdownMenuSubRoot"
            class="relative inline-flex items-center py-1.5 px-2 w-full text-sm no-underline rounded-sm transition-colors duration-200 cursor-pointer text-popover-foreground hover:bg-accent hover:text-accent-foreground max-md:min-h-10 [&_svg:not([class*='size-'])]:size-4"
            // Hover reveals the panel on a pointer device; a touch screen never
            // fires it, so a press also opens it. The press only *opens*: on a
            // touch screen the browser follows the tap with an emulated
            // `mouseenter` and then a `click`, so a toggle there would open and
            // immediately close the panel. `Enter`/`Space` toggle (a keyboard
            // has no hover) and `Escape` closes.
            role="menuitem"
            aria-haspopup="menu"
            aria-expanded=move || open.get().to_string()
            tabindex="0"
            on:mouseenter=move |_| open.set(true)
            on:mouseleave=move |_| open.set(false)
            on:click=move |_| open.set(true)
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    open.update(|shown| *shown = !*shown);
                } else if ev.key() == "Escape" {
                    open.set(false);
                }
            }
        >
            {children()}
        </li>
    }
}

#[component]
pub fn DropdownMenuSubTrigger(
    children: Children,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let class = tw_merge!("flex items-center justify-between w-full gap-2", class);

    view! {
        <span data-name="DropdownMenuSubTrigger" class=class>
            <span class="flex flex-1 gap-2 items-center min-w-0">{children()}</span>
            <ChevronRight class="opacity-70 size-4 shrink-0 text-muted-foreground" />
        </span>
    }
}

#[component]
pub fn DropdownMenuSubItem(
    children: Children,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let class = tw_merge!(
        "inline-flex gap-2 items-center w-full rounded-sm px-3 py-2 text-sm transition-all duration-150 ease text-popover-foreground hover:bg-accent hover:text-accent-foreground cursor-pointer hover:translate-x-[2px] max-md:min-h-10",
        class
    );

    view! {
        <li data-name="DropdownMenuSubItem" class=class data-dropdown-close="true">
            {children()}
        </li>
    }
}

/// The panel a [`DropdownMenuSub`] unfolds. Visible only while the nearest sub's
/// signal (see [`DropdownMenuSubContext`]) is set, so nesting works: an inner
/// panel stays shut until its own row is hovered, however many levels deep it
/// sits. `visibility` is deliberately not forced back to `visible` when open, so
/// an open inner panel still stays hidden while a closed outer panel is — the
/// outer's `invisible` is inherited.
#[component]
pub fn DropdownMenuSubContent(
    children: Children,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let open = use_context::<DropdownMenuSubContext>()
        .map(|ctx| ctx.open)
        .unwrap_or_else(|| RwSignal::new(false));
    let base = tw_merge!(
        // `left-full` opens the panel beside its parent row, which is wider
        // than the phone: below `md` it stacks under the row and inside the
        // menu's own width instead, and `max-w` keeps a long label from
        // pushing the page sideways on any screen.
        "absolute left-full top-0 z-[100] min-w-[160px] max-w-[calc(100vw-1rem)] rounded-xl border bg-card shadow-lg p-1 transition-all duration-200 ease-out max-md:left-0 max-md:top-full max-md:max-h-[60dvh] max-md:w-full max-md:min-w-0 max-md:overflow-y-auto max-md:overscroll-contain",
        class
    );
    // A reactive `class` string rather than a stack of `class:` bindings: the
    // slide-in class starts with a dash (`-translate-x-2`), which does not read
    // cleanly as a `class:` binding, and the assembled string also keeps the
    // closed/hidden classes together.
    let class = move || {
        if open.get() {
            format!("{base} translate-x-0")
        } else {
            format!("{base} invisible pointer-events-none opacity-0 -translate-x-2")
        }
    };

    view! {
        <ul data-name="DropdownMenuSubContent" class=class>
            {children()}
        </ul>
    }
}
