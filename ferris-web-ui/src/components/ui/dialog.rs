use icons::X;
use leptos::context::Provider;
use leptos::ev;
use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::*;

use super::button::{Button, ButtonSize, ButtonVariant};
use crate::components::hooks::focus_trap::FOCUS_TRAP_JS;
use crate::components::hooks::use_scroll_lock;
use crate::components::hooks::use_unique_id::use_unique_id_for;

mod components {
    use super::*;
    clx! {DialogBody, div, "flex flex-col gap-4"}
    clx! {DialogHeader, div, "flex flex-col gap-2 pe-12 text-center sm:text-left"}
    clx! {DialogTitle, h3, "text-lg leading-none font-semibold"}
    clx! {DialogDescription, p, "ferris-dialog__description text-sm"}
    // `w-full!` is deliberate: `Button` carries `w-fit` in its base classes, so
    // a stacked footer needs the important flag to make the phone buttons span
    // the dialog instead of hugging their label at different widths.
    clx! {DialogFooter, footer, "flex flex-col-reverse gap-2 max-sm:[&>*]:w-full! sm:flex-row sm:justify-end"}
}

pub use components::*;

/// Events a controlled [`DialogContent`] dispatches on its own panel so the
/// inline script can ask for a state change instead of painting one.
const OPEN_EVENT: &str = "ferris-dialog-open";
const CLOSE_EVENT: &str = "ferris-dialog-close";

/* ========================================================== */
/*                     ✨ FUNCTIONS ✨                        */
/* ========================================================== */

#[derive(Clone)]
struct DialogContext {
    target_id: String,
}

/// Returns the dialog trigger ID when called inside a `<Dialog>` subtree.
/// Use this to wire custom elements (e.g. AttachmentTrigger) as dialog triggers.
pub fn use_dialog_trigger_id() -> Option<String> {
    use_context::<DialogContext>().map(|c| c.target_id.clone())
}

#[component]
pub fn Dialog(children: Children, #[prop(optional, into)] class: String) -> impl IntoView {
    let dialog_target_id = use_unique_id_for("dialog");

    let ctx = DialogContext {
        target_id: dialog_target_id,
    };

    // `contents`: the dialog itself draws only fixed-position nodes, so the
    // wrapper must not add a flex/grid box (a zero-height item still adds a
    // gap to its parent).
    let merged_class = tw_merge!("contents", class);

    view! {
        <Provider value=ctx>
            <div class=merged_class data-name="__Dialog">
                {children()}
            </div>
        </Provider>
    }
}

#[component]
pub fn DialogTrigger(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = ButtonVariant::Outline)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Default)] size: ButtonSize,
) -> impl IntoView {
    let ctx = expect_context::<DialogContext>();
    let trigger_id = format!("trigger_{}", ctx.target_id);

    view! {
        <Button
            class=class
            attr:id=trigger_id
            attr:tabindex="0"
            attr:data-dialog-trigger=ctx.target_id
            variant=variant
            size=size
        >
            {children()}
        </Button>
    }
}

#[component]
pub fn DialogContent(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = true)] show_close_button: bool,
    #[prop(default = true)] close_on_backdrop_click: bool,
    #[prop(default = "Dialog")] data_name_prefix: &'static str,
    /// Drive the dialog from a signal instead of a trigger element.
    ///
    /// A dialog opened from a menu row has nowhere to host a
    /// [`DialogTrigger`], so the row only ever gets to set a signal. The
    /// close button, the backdrop and ESC write back to it, which leaves the
    /// state with a single owner.
    #[prop(optional)]
    open: Option<RwSignal<bool>>,
) -> impl IntoView {
    let ctx = expect_context::<DialogContext>();
    // On a phone the panel keeps a 1rem gutter (`100vw - 1rem`) and scrolls
    // internally inside 90dvh, so the close button and the footer stay
    // reachable even when the browser chrome is visible (`dvh` shrinks with
    // it, `vh` does not). From `md` up the previous metrics are restored
    // verbatim so the desktop look is unchanged.
    let merged_class = tw_merge!(
        // "flex flex-col gap-4", // TODO 🐛 Bug when I try to have this.. Using DialogBody instead.
        "ferris-dialog__content relative border rounded-xl shadow-lg p-4 sm:p-6 w-[calc(100vw-1rem)] max-w-md max-h-[90dvh] overflow-y-auto overscroll-contain md:max-h-[85vh] fixed top-[50%] left-[50%] translate-x-[-50%] translate-y-[-50%] z-100 transition-all duration-200 data-[state=closed]:opacity-0 data-[state=closed]:scale-95 data-[state=open]:opacity-100 data-[state=open]:scale-100",
        class
    );

    let backdrop_data_name = format!("{data_name_prefix}Backdrop");
    let content_data_name = format!("{data_name_prefix}Content");

    let target_id_clone = ctx.target_id.clone();
    let backdrop_id = format!("{}_backdrop", ctx.target_id);
    let target_id_for_script = ctx.target_id.clone();
    let backdrop_id_for_script = backdrop_id.clone();
    let backdrop_behavior = if close_on_backdrop_click {
        "auto"
    } else {
        "manual"
    };

    let controlled = open.is_some();

    // A controlled dialog owns `data-state` and `pointer-events`: the inline
    // script asks for a state change instead of painting one, so it must not
    // also be the writer here. An untriggered dialog derives a constant and
    // stays entirely script-driven.
    //
    // They are bound as attributes rather than looked up by id on purpose:
    // `use_unique_id_for` counts process-globally, so the id the client
    // computes while hydrating never matches the one in the server HTML.
    let data_state = Signal::derive(move || {
        match open {
            Some(open) if open.get() => "open",
            _ => "closed",
        }
        .to_string()
    });
    let pointer_events = Signal::derive(move || {
        match open {
            Some(open) if open.get() => "pointer-events: auto;",
            _ => "pointer-events: none;",
        }
        .to_string()
    });

    if let Some(open) = open {
        Effect::new(move |_| {
            if open.get() {
                use_scroll_lock::lock();
            } else {
                use_scroll_lock::unlock(200);
            }
        });
    }

    // A controlled dialog paints its own state, but the backdrop's
    // `pointer-events` used to stay `none` forever: the panel could only be
    // dismissed with the close button or Esc, never by tapping outside. An
    // uncontrolled dialog is driven by the inline script, which writes this
    // property itself; leaving the static value alone there keeps that path
    // working.
    let backdrop_pointer = Signal::derive(move || match open {
        Some(open) if open.get() => "pointer-events: auto;",
        _ => "pointer-events: none;",
    });

    let focus_trap_js = FOCUS_TRAP_JS;

    view! {
        <div
            data-name=backdrop_data_name
            id=backdrop_id
            data-ferris-backdrop=""
            class="ferris-overlay fixed inset-0 transition-opacity duration-200 pointer-events-none z-60 data-[state=closed]:opacity-0 data-[state=open]:opacity-100"
            data-state=data_state
            style=backdrop_pointer
        />

        <div
            data-name=content_data_name
            class=merged_class
            id=ctx.target_id
            data-target="target__dialog"
            data-state=data_state
            data-backdrop=backdrop_behavior
            role="dialog"
            aria-modal="true"
            tabindex="-1"
            style=pointer_events
            on:ferris-dialog-open=move |_: ev::CustomEvent| {
                if let Some(open) = open { open.set(true) }
            }
            on:ferris-dialog-close=move |_: ev::CustomEvent| {
                if let Some(open) = open { open.set(false) }
            }
        >
            <button
                type="button"
                class=tw_merge!(
                    "ferris-dialog__close absolute right-3 top-3 inline-flex size-8 items-center justify-center rounded-md focus:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 sm:right-4 sm:top-4 max-md:size-10 [&_svg:not([class*='size-'])]:size-4",
                    if show_close_button { "" } else { "hidden" },
                )
                data-dialog-close=target_id_clone
                aria-label=crate::i18n::t("ui.dialog.close")
            >
                <span class="hidden">{crate::i18n::t("ui.dialog.close_hidden")}</span>
                <X />
            </button>

            {children()}
        </div>

        <script nonce=crate::csp_nonce()>
            {format!(
                r#"
                {focus_trap_js}
                (function() {{
                    const controlled = {controlled};
                    const setupDialog = () => {{
                        const dialog = document.querySelector('#{target_id_for_script}');
                        const backdrop = document.querySelector('#{backdrop_id_for_script}');

                        if (!dialog || !backdrop) {{
                            setTimeout(setupDialog, 50);
                            return;
                        }}

                        // Focus trap and inert background follow `data-state`,
                        // so the controlled (signal) and script-driven paths
                        // share one enter/leave edge.
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

                        const openDialog = () => {{
                            if (controlled) {{
                                dialog.dispatchEvent(new Event('{OPEN_EVENT}'));
                                return;
                            }}

                            // Lock scrolling
                            if (window.ScrollLock) window.ScrollLock.lock();

                            dialog.setAttribute('data-state', 'open');
                            backdrop.setAttribute('data-state', 'open');
                            dialog.style.pointerEvents = 'auto';
                            backdrop.style.pointerEvents = 'auto';
                        }};

                        const closeDialog = () => {{
                            if (controlled) {{
                                dialog.dispatchEvent(new Event('{CLOSE_EVENT}'));
                                return;
                            }}

                            dialog.setAttribute('data-state', 'closed');
                            backdrop.setAttribute('data-state', 'closed');
                            dialog.style.pointerEvents = 'none';
                            backdrop.style.pointerEvents = 'none';

                            // Unlock scrolling after animation
                            if (window.ScrollLock) window.ScrollLock.unlock(200);
                        }};

                        // The panel's own dismissals are wired as soon as it is in the
                        // DOM: a controlled dialog has no trigger element to wait for.
                        if (!dialog.hasAttribute('data-initialized')) {{
                            dialog.setAttribute('data-initialized', 'true');

                            const closeButtons = dialog.querySelectorAll('[data-dialog-close]');
                            closeButtons.forEach(btn => {{
                                btn.addEventListener('click', closeDialog);
                            }});

                            // Close on backdrop click (if data-backdrop="auto")
                            backdrop.addEventListener('click', () => {{
                                if (dialog.getAttribute('data-backdrop') === 'auto') {{
                                    closeDialog();
                                }}
                            }});

                            // Handle ESC key to close
                            document.addEventListener('keydown', (e) => {{
                                if (e.key === 'Escape' && dialog.getAttribute('data-state') === 'open') {{
                                    e.preventDefault();
                                    closeDialog();
                                }}
                            }});
                        }}

                        const trigger = document.querySelector('[data-dialog-trigger="{target_id_for_script}"]');
                        if (trigger) {{
                            if (!trigger.hasAttribute('data-dialog-bound')) {{
                                trigger.setAttribute('data-dialog-bound', 'true');
                                trigger.addEventListener('click', openDialog);
                            }}
                        }} else if (!controlled) {{
                            // No trigger in the DOM yet: come back for it.
                            setTimeout(setupDialog, 50);
                        }}
                    }};

                    if (document.readyState === 'loading') {{
                        document.addEventListener('DOMContentLoaded', setupDialog);
                    }} else {{
                        setupDialog();
                    }}
                }})();
                "#,
            )}
        </script>
    }
}

#[component]
pub fn DialogClose(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = ButtonVariant::Outline)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Default)] size: ButtonSize,
) -> impl IntoView {
    let ctx = expect_context::<DialogContext>();

    view! {
        <Button
            class=class
            attr:data-dialog-close=ctx.target_id
            attr:aria-label=crate::i18n::t("ui.dialog.close")
            variant=variant
            size=size
        >
            {children()}
        </Button>
    }
}

#[component]
pub fn DialogAction(
    children: Children,
    #[prop(optional, into)] class: String,
    #[prop(default = ButtonVariant::Default)] variant: ButtonVariant,
    #[prop(default = ButtonSize::Default)] size: ButtonSize,
) -> impl IntoView {
    let ctx = expect_context::<DialogContext>();

    view! {
        <Button
            class=class
            attr:data-dialog-close=ctx.target_id
            variant=variant
            size=size
        >
            {children()}
        </Button>
    }
}
