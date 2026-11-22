use icons::Check;
use leptos::portal::Portal;
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::button::{Button, ButtonVariant};

const TRIGGER_ID_QUALIFIER: &str = "command__trigger";

mod components {
    use super::*;
    clx! {CommandHeader, div, "ferris-command__header flex flex-col gap-2 text-center hidden sm:text-left"} // sr-only
    clx! {CommandTitle, h2, "ferris-command__title text-lg font-semibold leading-none"}
    clx! {CommandDescription, p, "ferris-command__description text-sm"}
    // `min-h-0` on phones lets the list shrink inside the capped dialog (a
    // fixed 320px minimum would push the footer out of an 85dvh surface);
    // `max-md:max-h-[55dvh]` bounds it so the input and the footer are always
    // visible while the list itself scrolls.
    clx! {CommandList, div, "ferris-command__list overflow-y-auto overflow-x-hidden min-h-0 max-h-[300px] max-md:max-h-[55dvh] md:min-h-80 scroll-py-1 no__scrollbar scroll-pt-2 scroll-pb-1.5"}
    clx! {CommandGroup, div, "ferris-command__group overflow-hidden p-1"}
    // TODO. This one should turn auto in a tag if we pass href (later, not urgent).
    clx! {CommandItemLink, a, "ferris-command__item-link relative flex cursor-default hover:cursor-pointer items-center gap-2 px-2 py-1.5 text-sm outline-hidden select-none data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 h-9 rounded-md border font-medium"}
    clx! {CommandGroupLabel, div, "ferris-command__group-label px-2 py-1.5 text-xs font-medium"}
    clx! {CommandFooter, footer, "ferris-command__footer flex gap-4 items-center px-4 h-10 text-xs font-medium rounded-b-xl border-t"}
}

pub use components::*;

/* ========================================================== */
/*                     ✨ CONTEXT ✨                          */
/* ========================================================== */

#[derive(Clone)]

struct CommandDialogContext {
    dialog_id: String,
}

#[derive(Clone, Copy)]
struct CommandContext {
    search_query_signal: RwSignal<String>,
    should_filter: bool,
}

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

mod dialog;
mod input;
mod item;

pub use dialog::{CommandDialog, CommandDialogProvider, CommandDialogTrigger};
pub use input::{CommandEmpty, CommandInput};
pub use item::CommandItem;

/* ========================================================== */
/*                     ✨ COMMAND ROOT ✨                     */
/* ========================================================== */

#[component]
pub fn Command(
    children: Children,
    #[prop(into, optional)] class: String,
    /// When false, disables client-side filtering (use for server-side search).
    /// Default: true (client-side filtering enabled).
    #[prop(default = true)]
    should_filter: bool,
) -> impl IntoView {
    let dialog_context = use_context::<CommandDialogContext>();
    let search_query_signal = RwSignal::new(String::new());
    let command_context = CommandContext {
        search_query_signal,
        should_filter,
    };

    provide_context(command_context);

    let merged_class = tw_merge!(
        "ferris-command flex overflow-hidden flex-col w-full h-full rounded-none",
        class
    );

    let script_content = if let Some(ctx) = dialog_context {
        // Dialog version with context
        let dialog_id = ctx.dialog_id;
        let backdrop_id = format!("{dialog_id}__{TRIGGER_ID_QUALIFIER}");
        format!(
            r#"
            (function() {{
                const setupCommandKeyboard = () => {{
                    const FIRST_INDEX = 0;
                    const dialog = document.querySelector('#{dialog_id}');
                    const backdrop = document.querySelector('#{backdrop_id}');
                    const command_list = dialog?.querySelector('[data-name="CommandList"]');
                    const command_input = dialog?.querySelector('[data-name="CommandInput"]');
                    const command_items = command_list?.querySelectorAll('[data-name="CommandItemLink"]');
                    const command_groups = command_list?.querySelectorAll('[data-name="CommandGroup"]');

                    if (!command_items || command_items.length === 0 || !command_input) {{
                        // Elements not ready yet, try again shortly
                        setTimeout(setupCommandKeyboard, 50);
                        return;
                    }}

                    let index = FIRST_INDEX;

                    // Get visible items only (also skip items inside a hidden group)
                    const getVisibleItems = () => {{
                        return Array.from(command_items).filter(item => {{
                            if (item.style.display === 'none') return false;
                            const group = item.closest('[data-name="CommandGroup"]');
                            return !group || group.style.display !== 'none';
                        }});
                    }};

                    const select = (i) => {{
                        const visibleItems = getVisibleItems();
                        if (visibleItems.length === 0) return;

                        command_items.forEach(item => item.setAttribute('aria-selected', 'false'));
                        if (visibleItems[i]) {{
                            visibleItems[i].setAttribute('aria-selected', 'true');
                            visibleItems[i].scrollIntoView({{ block: 'nearest', behavior: 'smooth' }});
                        }}
                    }};

                    // Filter items based on search query
                    const filterItems = (query) => {{
                        const searchQuery = query.toLowerCase().trim();

                        command_items.forEach(item => {{
                            const text = item.textContent.toLowerCase();
                            if (searchQuery === '' || text.includes(searchQuery)) {{
                                item.style.display = '';
                            }} else {{
                                item.style.display = 'none';
                            }}
                        }});

                        // Hide empty groups
                        command_groups.forEach(group => {{
                            const groupItems = group.querySelectorAll('[data-name="CommandItemLink"]');
                            const hasVisibleItems = Array.from(groupItems).some(item => item.style.display !== 'none');
                            group.style.display = hasVisibleItems ? '' : 'none';
                        }});

                        // Reset selection to first visible item
                        index = FIRST_INDEX;
                        select(FIRST_INDEX);
                    }};

                    // Listen to input changes
                    command_input.addEventListener('input', (e) => {{
                        filterItems(e.target.value);
                    }});

                    // Close dialog function
                    // NOTE: We DON'T handle body scroll restoration here.
                    // That's handled by CommandDialog's closeDialog function.
                    // We just trigger the dialog close, and the backdrop click handler will do the rest.
                    const closeDialog = () => {{
                        backdrop.click();
                    }};

                    // Add click handlers to all command items to close dialog
                    command_items.forEach((item) => {{
                        item.addEventListener('click', () => {{
                            closeDialog();
                        }});
                    }});

                    document.addEventListener('keydown', (e) => {{
                        // Only handle keyboard navigation if dialog is open
                        if (dialog?.getAttribute('data-state') !== 'open') return;

                        const visibleItems = getVisibleItems();
                        if (visibleItems.length === 0) return;

                        if (e.key === 'ArrowDown') {{
                            e.preventDefault();
                            if (index < visibleItems.length - 1) select(++index);
                        }} else if (e.key === 'ArrowUp') {{
                            e.preventDefault();
                            if (index > FIRST_INDEX) select(--index);
                            else command_list.scrollTo({{ top: 0, behavior: 'smooth' }});
                        }} else if (e.key === 'Enter') {{
                            e.preventDefault();
                            visibleItems[index]?.click();
                        }}
                    }});

                    // Initialize selection only when dialog opens
                    const observer = new MutationObserver((mutations) => {{
                        mutations.forEach((mutation) => {{
                            if (mutation.attributeName === 'data-state') {{
                                if (dialog.getAttribute('data-state') === 'open') {{
                                    // Reset search input and filter
                                    command_input.value = '';
                                    filterItems('');
                                    index = FIRST_INDEX;
                                    select(FIRST_INDEX);
                                    // Focus the input when dialog opens (needed only on click the Trigger)
                                    setTimeout(() => command_input.focus(), 0);
                                }}
                            }}
                        }});
                    }});

                    observer.observe(dialog, {{ attributes: true }});
                }};

                // Try to setup immediately, or wait for DOMContentLoaded
                if (document.readyState === 'loading') {{
                    document.addEventListener('DOMContentLoaded', setupCommandKeyboard);
                }} else {{
                    setupCommandKeyboard();
                }}
            }})();
            "#
        )
    } else {
        // Standalone version without dialog context - always active keyboard navigation
        r#"
        (function() {
            const setupCommand = () => {
                const FIRST_INDEX = 0;
                const command_list = document.querySelector('[data-name="CommandList"]');
                const command_input = document.querySelector('[data-name="CommandInput"]');
                const command_items = command_list?.querySelectorAll('[data-name="CommandItemLink"]');
                const command_groups = command_list?.querySelectorAll('[data-name="CommandGroup"]');

                if (!command_items || command_items.length === 0) {
                    // Elements not ready yet, try again shortly
                    setTimeout(setupCommand, 50);
                    return;
                }

                let index = FIRST_INDEX;

                // Get visible items only
                const getVisibleItems = () => {
                    return Array.from(command_items).filter(item => item.style.display !== 'none');
                };

                const select = (i) => {
                    const visibleItems = getVisibleItems();
                    if (visibleItems.length === 0) return;

                    command_items.forEach(item => item.setAttribute('aria-selected', 'false'));
                    if (visibleItems[i]) {
                        visibleItems[i].setAttribute('aria-selected', 'true');
                        visibleItems[i].scrollIntoView({ block: 'nearest', behavior: 'smooth' });
                    }
                };

                // Filter items based on search query
                const filterItems = (query) => {
                    const searchQuery = query.toLowerCase().trim();

                    command_items.forEach(item => {
                        const text = item.textContent.toLowerCase();
                        if (searchQuery === '' || text.includes(searchQuery)) {
                            item.style.display = '';
                        } else {
                            item.style.display = 'none';
                        }
                    });

                    // Hide empty groups
                    if (command_groups) {
                        command_groups.forEach(group => {
                            const groupItems = group.querySelectorAll('[data-name="CommandItemLink"]');
                            const hasVisibleItems = Array.from(groupItems).some(item => item.style.display !== 'none');
                            group.style.display = hasVisibleItems ? '' : 'none';
                        });
                    }

                    // Reset selection to first visible item
                    index = FIRST_INDEX;
                    select(FIRST_INDEX);
                };

                // Listen to input changes if input exists
                if (command_input) {
                    command_input.addEventListener('input', (e) => {
                        filterItems(e.target.value);
                    });
                }

                // Initialize first item as selected
                select(FIRST_INDEX);

                document.addEventListener('keydown', (e) => {
                    const visibleItems = getVisibleItems();
                    if (visibleItems.length === 0) return;

                    if (e.key === 'ArrowDown') {
                        e.preventDefault();
                        if (index < visibleItems.length - 1) select(++index);
                    } else if (e.key === 'ArrowUp') {
                        e.preventDefault();
                        if (index > FIRST_INDEX) select(--index);
                        else command_list.scrollTo({ top: 0, behavior: 'smooth' });
                    } else if (e.key === 'Enter') {
                        e.preventDefault();
                        visibleItems[index]?.click();
                    }
                });
            };

            // Try to setup immediately, or wait for DOMContentLoaded
            if (document.readyState === 'loading') {
                document.addEventListener('DOMContentLoaded', setupCommand);
            } else {
                setupCommand();
            }
        })();
        "#
        .to_string()
    };

    view! {
        <style>
            r#"
            /* Hide CommandEmpty when there are visible items */
            [data-name="CommandList"]:has([data-name="CommandItem"][style*="flex"]) [data-name="CommandEmpty"] {
                display: none;
            }
            "#
        </style>

        <div data-name="Command" class=merged_class tabindex="-1">
            // <label style="position: absolute; width: 1px; height: 1px; padding: 0px; margin: -1px; overflow: hidden; clip: rect(0px, 0px, 0px, 0px); white-space: nowrap; border-width: 0px;"></label>
            {children()}
        </div>

        <script nonce=crate::csp_nonce()>{script_content}</script>
    }
}
