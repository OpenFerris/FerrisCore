use super::*;
use crate::components::hooks::focus_trap::FOCUS_TRAP_JS;

#[component]
pub fn CommandDialogProvider(children: Children, #[prop(into)] id: String) -> impl IntoView {
    let context = CommandDialogContext { dialog_id: id };

    provide_context(context);

    children()
}

#[component]
pub fn CommandDialogTrigger(
    children: Children,
    #[prop(into, optional)] class: String,
) -> impl IntoView {
    let context = expect_context::<CommandDialogContext>();
    let trigger_id = format!("{TRIGGER_ID_QUALIFIER}__{}", context.dialog_id);

    view! {
        <Button attr:data-name="CommandDialogTrigger" class=class variant=ButtonVariant::Outline attr:id=trigger_id>
            {children()}
        </Button>
    }
}

#[component]
pub fn CommandDialog(children: ChildrenFn, #[prop(into, optional)] class: String) -> impl IntoView {
    let context = expect_context::<CommandDialogContext>();
    // The palette is a fixed centred surface: it must never be taller than the
    // visible viewport (`dvh` accounts for mobile browser chrome) and it clips
    // its own corners, while the list inside does the scrolling so the search
    // field stays on screen.
    let merged_class = tw_merge!(
        "ferris-command__dialog grid fixed z-100 gap-4 p-2 w-full max-h-[85dvh] overflow-hidden bg-clip-padding rounded-xl border border-none ring-4 shadow-2xl sm:max-w-lg top-[50%] left-[50%] max-w-[calc(100vw-1rem)] translate-x-[-50%] translate-y-[-50%] transition-all duration-200 data-[state=closed]:opacity-0 data-[state=closed]:scale-95 data-[state=open]:opacity-100 data-[state=open]:scale-100",
        class
    );

    let dialog_id = context.dialog_id;
    let backdrop_id = format!("{dialog_id}__{TRIGGER_ID_QUALIFIER}");
    let trigger_id = format!("{TRIGGER_ID_QUALIFIER}__{dialog_id}");
    let focus_trap_js = FOCUS_TRAP_JS;

    let script_content = format!(
        r#"
        {focus_trap_js}
        (function() {{
            const KEY_HANDLER_KEY = '__commandDialog_{dialog_id}_keyHandler';
            const CLICK_HANDLER_KEY = '__commandDialog_{dialog_id}_clickHandler';
            const BACKDROP_HANDLER_KEY = '__commandDialog_{dialog_id}_backdropHandler';

            const setupDialog = () => {{
                const dialog = document.querySelector('#{dialog_id}');
                const backdrop = document.querySelector('#{backdrop_id}');

                if (!dialog || !backdrop) {{
                    setTimeout(setupDialog, 50);
                    return;
                }}

                // Remove old listeners if they exist (for SPA navigation)
                if (window[KEY_HANDLER_KEY]) {{
                    document.removeEventListener('keydown', window[KEY_HANDLER_KEY]);
                }}
                if (window[CLICK_HANDLER_KEY]) {{
                    document.removeEventListener('click', window[CLICK_HANDLER_KEY]);
                }}
                if (window[BACKDROP_HANDLER_KEY]) {{
                    document.removeEventListener('click', window[BACKDROP_HANDLER_KEY]);
                }}

                // The palette is modal: Tab cycles inside it and the page behind
                // is inert while it is open, restored on close.
                const trap = window.__ferrisFocusTrap;
                let trapped = false;
                const syncFocus = () => {{
                    const isOpen = dialog.getAttribute('data-state') === 'open';
                    if (isOpen && !trapped) {{
                        trapped = true;
                        trap.enter(dialog);
                    }} else if (!isOpen && trapped) {{
                        trapped = false;
                        trap.leave(dialog);
                    }}
                }};
                new MutationObserver(syncFocus).observe(dialog, {{
                    attributes: true,
                    attributeFilter: ['data-state'],
                }});
                syncFocus();

                // Click handler using event delegation (works after SPA navigation)
                const clickHandler = (e) => {{
                    const openBtn = e.target.closest('#{trigger_id}');
                    if (!openBtn) return;

                    const currentDialog = document.querySelector('#{dialog_id}');
                    const currentBackdrop = document.querySelector('#{backdrop_id}');
                    if (!currentDialog || !currentBackdrop) return;

                    window.ScrollLock.lock();
                    currentDialog.setAttribute('data-state', 'open');
                    currentBackdrop.setAttribute('data-state', 'open');
                    currentDialog.style.pointerEvents = 'auto';
                    currentBackdrop.style.pointerEvents = 'auto';
                    currentDialog.querySelector('[data-name="CommandInput"]')?.focus();
                }};

                // Backdrop click handler using event delegation
                const backdropHandler = (e) => {{
                    const clickedBackdrop = e.target.closest('#{backdrop_id}');
                    if (!clickedBackdrop) return;

                    const currentDialog = document.querySelector('#{dialog_id}');
                    const currentBackdrop = document.querySelector('#{backdrop_id}');
                    if (!currentDialog || !currentBackdrop) return;

                    currentDialog.setAttribute('data-state', 'closed');
                    currentBackdrop.setAttribute('data-state', 'closed');
                    currentDialog.style.pointerEvents = 'none';
                    currentBackdrop.style.pointerEvents = 'none';
                    window.ScrollLock.unlock(100);
                }};

                // Global keyboard listener for Cmd+K, Ctrl+K, or /
                const keyHandler = (e) => {{
                    const currentDialog = document.querySelector('#{dialog_id}');
                    const currentBackdrop = document.querySelector('#{backdrop_id}');
                    if (!currentDialog || !currentBackdrop) return;

                    const openDialog = () => {{
                        window.ScrollLock.lock();
                        currentDialog.setAttribute('data-state', 'open');
                        currentBackdrop.setAttribute('data-state', 'open');
                        currentDialog.style.pointerEvents = 'auto';
                        currentBackdrop.style.pointerEvents = 'auto';
                        currentDialog.querySelector('[data-name="CommandInput"]')?.focus();
                    }};

                    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {{
                        e.preventDefault();
                        if (currentDialog.getAttribute('data-state') !== 'open') openDialog();
                    }}
                    else if (e.key === '/' && currentDialog.getAttribute('data-state') !== 'open') {{
                        const el = document.activeElement;
                        const tag = el?.tagName;
                        if (tag === 'INPUT' || tag === 'TEXTAREA' || el?.isContentEditable) return;
                        e.preventDefault();
                        openDialog();
                    }}
                    else if (e.key === 'Escape' && currentDialog.getAttribute('data-state') === 'open') {{
                        e.preventDefault();
                        currentDialog.setAttribute('data-state', 'closed');
                        currentBackdrop.setAttribute('data-state', 'closed');
                        currentDialog.style.pointerEvents = 'none';
                        currentBackdrop.style.pointerEvents = 'none';
                        window.ScrollLock.unlock(100);
                    }}
                }};

                // Store handler references and add listeners
                window[KEY_HANDLER_KEY] = keyHandler;
                window[CLICK_HANDLER_KEY] = clickHandler;
                window[BACKDROP_HANDLER_KEY] = backdropHandler;
                document.addEventListener('keydown', keyHandler);
                document.addEventListener('click', clickHandler);
                document.addEventListener('click', backdropHandler);
            }};

            if (document.readyState === 'loading') {{
                document.addEventListener('DOMContentLoaded', setupDialog);
            }} else {{
                setupDialog();
            }}
        }})();
        "#
    );

    view! {
        <CommandDialogPortal dialog_id=dialog_id backdrop_id=backdrop_id class=merged_class>
            {children()}
        </CommandDialogPortal>

        <script nonce=crate::csp_nonce()>{script_content}</script>
    }
}

// Renders backdrop and dialog using Leptos Portal to body
#[component]
fn CommandDialogPortal(
    children: ChildrenFn,
    #[prop(into)] dialog_id: String,
    #[prop(into)] backdrop_id: String,
    #[prop(into)] class: String,
) -> impl IntoView {
    let backdrop_id = StoredValue::new(backdrop_id);
    let dialog_id = StoredValue::new(dialog_id);
    let class = StoredValue::new(class);

    view! {
        <Portal>
            <div
                data-name="CommandDialogBackdrop"
                id=backdrop_id.get_value()
                data-ferris-backdrop=""
                class="ferris-overlay fixed inset-0 transition-opacity duration-200 pointer-events-none z-60 data-[state=closed]:opacity-0 data-[state=open]:opacity-100"
                data-state="closed"
            />

            <div
                data-name="CommandDialog"
                class=class.get_value()
                id=dialog_id.get_value()
                data-state="closed"
                role="dialog"
                aria-modal="true"
                tabindex="-1"
                style="pointer-events: none;"
            >
                {children()}
            </div>
        </Portal>
    }
}

/* ========================================================== */
