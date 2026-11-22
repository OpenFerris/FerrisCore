//! Viewport placement for fixed dropdown panels.
//!
//! Kept apart from the component definitions: it is the only part of the
//! primitive that needs `web_sys` geometry, and a panel has to be measured
//! after mount because its trigger box is not known during SSR.

use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::{DropdownMenuAlign, DropdownMenuPosition};

/// Gap between the trigger and the panel.
const PANEL_GAP: f64 = 4.0;
/// Keeps a panel this far from the viewport edge on both axes.
const VIEWPORT_MARGIN: f64 = 8.0;

/// A viewport-space rectangle, as `getBoundingClientRect` reports one.
#[derive(Clone, Copy)]
struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    fn width(&self) -> f64 {
        self.right - self.left
    }
    fn height(&self) -> f64 {
        self.bottom - self.top
    }

    fn union(&self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Rectangle of an element; a zero-sized one for anything that is not an element.
    fn of(node: &web_sys::Node) -> Self {
        let Some(element) = node.dyn_ref::<web_sys::Element>() else {
            return Self::ZERO;
        };
        let rect = element.get_bounding_client_rect();
        Self {
            left: rect.left(),
            top: rect.top(),
            right: rect.right(),
            bottom: rect.bottom(),
        }
    }
}

/// The box a dropdown has to align to.
///
/// A trigger rendered with `as_child` wraps its control in a `display: contents`
/// span, which draws no box of its own: measured directly it is a zero-sized
/// rectangle sitting at the viewport origin, which pins the panel to the
/// top-left corner of the screen. Its children are what actually draw, so those
/// are measured instead.
fn measured_box(node: &web_sys::Node) -> Rect {
    let own = Rect::of(node);
    if own.width() != 0.0 || own.height() != 0.0 {
        return own;
    }

    let children = node.child_nodes();
    (0..children.length())
        .filter_map(|index| children.item(index))
        .map(|child| Rect::of(&child))
        .filter(|child_box| child_box.width() != 0.0 || child_box.height() != 0.0)
        .reduce(|a, b| a.union(b))
        .unwrap_or(own)
}

/// Places the fixed panel against its trigger and keeps it inside the viewport.
/// Sized with `offset_*` because the closed `scale-95` transform would skew a
/// `getBoundingClientRect` measurement.
pub(super) fn place(
    panel: NodeRef<Div>,
    id: &str,
    align: DropdownMenuAlign,
    position: DropdownMenuPosition,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(Some(node)) = panel.try_get_untracked() else {
        return;
    };
    let panel: &web_sys::HtmlElement = &node;
    let Ok(Some(root)) = panel.closest(r#"[data-name="DropdownMenu"]"#) else {
        return;
    };
    let Ok(Some(trigger)) = root.query_selector(&format!(r#"[data-dropdown-trigger="{id}"]"#))
    else {
        return;
    };

    let trigger_box = measured_box(&trigger);
    let panel_width = f64::from(panel.offset_width());
    let panel_height = f64::from(panel.offset_height());
    let viewport_width = window
        .inner_width()
        .ok()
        .and_then(|w| w.as_f64())
        .unwrap_or_default();
    let viewport_height = window
        .inner_height()
        .ok()
        .and_then(|h| h.as_f64())
        .unwrap_or_default();

    let below = trigger_box.bottom + PANEL_GAP;
    let (top, origin_y) = match position {
        DropdownMenuPosition::Top => (trigger_box.top - panel_height - PANEL_GAP, "bottom"),
        DropdownMenuPosition::Bottom => (below, "top"),
        DropdownMenuPosition::Auto => {
            if below + panel_height <= viewport_height || trigger_box.top < panel_height {
                (below, "top")
            } else {
                (trigger_box.top - panel_height - PANEL_GAP, "bottom")
            }
        }
    };

    let left = match align {
        DropdownMenuAlign::Start => trigger_box.left,
        DropdownMenuAlign::StartOuter => trigger_box.left - panel_width,
        DropdownMenuAlign::Center => trigger_box.left + (trigger_box.width() - panel_width) / 2.0,
        DropdownMenuAlign::End | DropdownMenuAlign::EndOuter => trigger_box.right - panel_width,
    };

    // `HtmlElement::style` is called through its full path because the
    // `ElementExt` blanket impl also offers a `style` attribute builder.
    // These land on the same attribute as the panel's `pointer-events` rule, and
    // have to win over it: this runs from a deferred `Effect`, which is flushed
    // after the attribute is applied.
    let style = web_sys::HtmlElement::style(panel);
    // Start/End menus are floored at the trigger's width so they feel attached
    // to their button. A centred panel (the module header) is `w-max` and sizes
    // to its own content instead: its trigger spans the whole header, so
    // flooring it at that width would stretch the menu across the sidebar.
    if align != DropdownMenuAlign::Center {
        style
            .set_property("min-width", &format!("{}px", trigger_box.width()))
            .ok();
    }
    style
        .set_property(
            "top",
            &format!("{}px", keep_inside(top, panel_height, viewport_height)),
        )
        .ok();
    style
        .set_property(
            "left",
            &format!("{}px", keep_inside(left, panel_width, viewport_width)),
        )
        .ok();
    style
        .set_property("transform-origin", &format!("center {origin_y}"))
        .ok();
}

fn keep_inside(offset: f64, panel_size: f64, viewport: f64) -> f64 {
    let max = (viewport - panel_size - VIEWPORT_MARGIN).max(VIEWPORT_MARGIN);
    offset.clamp(VIEWPORT_MARGIN, max)
}
