use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::components::constants::PAGINATION;

/// Buffer rows to render above/below viewport for smooth scrolling
const BUFFER_ROWS: usize = 5;

/// Virtual scroll state containing the visible range
#[derive(Clone, Copy)]
pub struct VirtualScrollState {
    /// First visible row index
    pub start_index: Memo<usize>,
    /// Last visible row index (exclusive)
    pub end_index: Memo<usize>,
    /// Total height of the virtual container in pixels
    pub total_height: Signal<usize>,
}

/// Get the virtual scroll context from a parent VirtualizedGrid.
/// Returns None if used outside of a VirtualizedGrid.
pub fn use_virtual_scroll_context() -> Option<VirtualScrollState> {
    use_context::<VirtualScrollState>()
}

/// Hook for virtual scrolling in data grids.
///
/// Only renders rows that are visible in the viewport plus a small buffer,
/// dramatically improving performance for large datasets.
///
/// # Arguments
/// * `container_ref` - NodeRef to the scrollable container element
/// * `total_rows` - Signal containing the total number of rows
///
/// # Returns
/// * `VirtualScrollState` with start/end indices and total height
///
/// # Example
/// ```ignore
/// let container_ref = NodeRef::<html::Div>::new();
/// let total_rows = Signal::derive(move || data.get().len());
/// let virtual_scroll = use_virtual_scroll(container_ref, total_rows);
///
/// // Only render visible rows
/// <For
///     each=move || (virtual_scroll.start_index.get()..virtual_scroll.end_index.get())
///     key=|&i| i
///     children=move |i| { ... }
/// />
/// ```
pub fn use_virtual_scroll(
    container_ref: NodeRef<html::Div>,
    total_rows: Signal<usize>,
) -> VirtualScrollState {
    let scroll_top_signal = RwSignal::new(0usize);
    let container_height_signal = RwSignal::new(600usize); // Default height

    // Track if component is still mounted to prevent accessing disposed signals
    // Using Arc<AtomicBool> for thread-safe (Send + Sync) mounted flag
    let is_mounted = Arc::new(AtomicBool::new(true));
    let is_mounted_for_cleanup = Arc::clone(&is_mounted);

    // The listener is owned here, not leaked with `forget`: the list mounts a
    // fresh table whenever the loading skeleton replaces it, and a forgotten
    // listener would keep the old element and its signals alive for the whole
    // session. `LocalStorage` lets the non-`Send` DOM closure live in the
    // reactive arena while the copyable handle crossed into the cleanup
    // callback, whose signature is `Send + Sync`.
    type ScrollListener = (web_sys::HtmlDivElement, Closure<dyn Fn()>);
    let listener = StoredValue::new_local(None::<ScrollListener>);

    // Cleanup: mark as unmounted and detach the listener from the element.
    let listener_for_cleanup = listener;
    on_cleanup(move || {
        is_mounted_for_cleanup.store(false, Ordering::SeqCst);
        let taken = listener_for_cleanup
            .try_update_value(Option::take)
            .flatten();
        if let Some((element, handler)) = taken {
            let _ = element
                .remove_event_listener_with_callback("scroll", handler.as_ref().unchecked_ref());
        }
    });

    // Update scroll position and container height on scroll
    let is_mounted_for_effect = Arc::clone(&is_mounted);
    let listener_for_effect = listener;
    Effect::new(move || {
        if !is_mounted_for_effect.load(Ordering::SeqCst) {
            return;
        }
        if let Some(el) = container_ref.get() {
            // The effect can re-run if the ref is replaced. Detach the previous
            // handler first so a rerun cannot leave two listeners on one node.
            let old = listener_for_effect.try_update_value(Option::take).flatten();
            if let Some((old_element, old_handler)) = old {
                let _ = old_element.remove_event_listener_with_callback(
                    "scroll",
                    old_handler.as_ref().unchecked_ref(),
                );
            }

            // Measure height after browser layout completes using requestAnimationFrame.
            // Without this, the Effect runs before CSS layout is finished, causing
            // client_height() to return 0 or an incorrect value. This resulted in
            // only ~30 rows being rendered on initial load until the user scrolled.
            // requestAnimationFrame ensures we measure after the paint cycle.
            let is_mounted_for_raf = Arc::clone(&is_mounted_for_effect);
            let measure_height = Closure::once_into_js(move || {
                // Check if still mounted before accessing signals
                if !is_mounted_for_raf.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(el) = container_ref.get_untracked() {
                    container_height_signal.set(el.client_height().max(0) as usize);
                }
            });

            let window = leptos::prelude::window();
            let _ = window.request_animation_frame(measure_height.unchecked_ref());

            // Set up scroll listener with mounted check
            let is_mounted_for_handler = Arc::clone(&is_mounted_for_effect);
            let scroll_handler = Closure::wrap(Box::new(move || {
                // Check if still mounted before accessing signals
                if !is_mounted_for_handler.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(el) = container_ref.get_untracked() {
                    scroll_top_signal.set(el.scroll_top().max(0) as usize);
                    container_height_signal.set(el.client_height().max(0) as usize);
                }
            }) as Box<dyn Fn()>);

            let _ = el.add_event_listener_with_callback(
                "scroll",
                scroll_handler.as_ref().unchecked_ref(),
            );

            listener_for_effect.update_value(|slot| *slot = Some((el, scroll_handler)));
        }
    });

    let start_index = Memo::new(move |_| {
        let scroll_top = scroll_top_signal.get();
        let start = scroll_top / PAGINATION::ROW_HEIGHT;
        start.saturating_sub(BUFFER_ROWS)
    });

    let end_index = Memo::new(move |_| {
        let scroll_top = scroll_top_signal.get();
        let container_height = container_height_signal.get();
        let total = total_rows.get();

        let visible_rows = (container_height / PAGINATION::ROW_HEIGHT) + 1;
        let start = scroll_top / PAGINATION::ROW_HEIGHT;
        let end = start + visible_rows + BUFFER_ROWS * 2;

        end.min(total)
    });

    let total_height = Signal::derive(move || total_rows.get() * PAGINATION::ROW_HEIGHT);

    VirtualScrollState {
        start_index,
        end_index,
        total_height,
    }
}
