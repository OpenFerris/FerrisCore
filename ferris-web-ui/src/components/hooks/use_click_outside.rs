use leptos::ev;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Hook to detect clicks outside a referenced element.
///
/// Registers a `mousedown` listener that calls the callback when a click occurs
/// outside the element referenced by `node_ref`.
///
/// The listener is a Leptos `window_event_listener`, whose handle removes it on
/// drop, and the hook registers that removal with `on_cleanup`. The previous
/// implementation built the `Closure` by hand and called `forget()` on it,
/// which dropped the only handle that could have removed the listener: every
/// mount of a grid added a document listener that lived until the document was
/// destroyed (UI11).
///
/// # Example
/// ```ignore
/// let container_ref = NodeRef::<Div>::new();
/// use_click_outside(container_ref, move || {
///     // Called when clicking outside the container
///     my_signal.set(None);
/// });
/// ```
pub fn use_click_outside<F>(node_ref: NodeRef<Div>, on_click_outside: F)
where
    F: Fn() + Clone + 'static,
{
    let handle = window_event_listener(ev::mousedown, move |ev: web_sys::MouseEvent| {
        // Use try_get_untracked() since we're in an event handler, not a
        // reactive context.
        if let Some(Some(element)) = node_ref.try_get_untracked()
            && let Some(target) = ev.target()
            && let Ok(target_node) = target.dyn_into::<web_sys::Node>()
            && !element.contains(Some(&target_node))
        {
            on_click_outside();
        }
    });
    on_cleanup(move || handle.remove());
}
