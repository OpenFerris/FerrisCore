//! The document key, the registry-to-contract shims and the one placement rule
//! every dashboard backing shares.
//!
//! `normalize_widgets` and `sanitize_widgets` are `pub(crate)` on purpose: a
//! document-format dashboard ([`crate::dashboard_layout_doc`]) saves and renders
//! through the very same functions, so the two backings cannot lay out the same
//! widget list differently.

use ferris_core::ui_extensions::{GridSizeSpec, StyleSpec, find_widgets};
use ferris_schema::contract::{GridSize, Sizing, WidgetInstance, layout};

/// The identity of one dashboard inside one app: `"{owner}:{dashboard}"`.
///
/// This is the *dashboard's* name, not a row's: since API-10 the saved rows are
/// per user ([`layout_name`] appends the user), so the same dashboard addressed
/// by two people addresses two rows while keeping one identity for logs, error
/// messages and the existence check.
pub(super) fn doc_name(owner: &str, dashboard: &str) -> String {
    format!("{owner}:{dashboard}")
}

/// The document name one user's saved layout of one dashboard lives under:
/// `"{owner}:{dashboard}:{user}"`.
///
/// The layout is per user — a dashboard arrangement is the person's own desk,
/// not a piece of furniture every user shares — so the user belongs in the
/// primary key. The user is kept as written (`trim`med) because it is the
/// caller's resolved id: two spellings of the same account never reach here,
/// the auth layer resolves the account first.
pub(super) fn layout_name(owner: &str, dashboard: &str, user: &str) -> String {
    format!("{}:{}", doc_name(owner, dashboard), user.trim())
}

/// The registry footprint hint as the contract's shape.
///
/// `ferris-core` cannot name `contract::GridSize` (it does not depend on this
/// crate), so the two look-alike structs are converted here, once. Take it as a
/// value and let the call sites `.map(grid_size)`.
pub(super) const fn grid_size(size: GridSizeSpec) -> GridSize {
    GridSize {
        span: size.span,
        rows: size.rows,
    }
}

/// A registry-declared widget style as the contract's shape.
///
/// The structural twin of [`ferris_core::ui_extensions::StyleSpec`] exists only
/// because the registry sits below this crate (feature `native`); the two are
/// kept field-for-field compatible by design, so this is a plain shim. `None`
/// stays `None`: an instance that declares no style keeps inherited defaults.
pub(super) fn style_spec(style: StyleSpec) -> ferris_schema::contract::StyleSpec {
    ferris_schema::contract::StyleSpec {
        background: style.background,
        background_custom: style.background_custom,
        border_color: style.border_color,
        border_width: style.border_width,
        border_radius: style.border_radius,
        padding: style.padding,
        shadow: style.shadow,
    }
}

/// Normalize a widget list in place against the global catalog: clamp the
/// geometry, fill absent cells, and resolve overlaps — with **one** snapshot of
/// the registry, so a dashboard does not clone it once per widget.
///
/// `pub(crate)` for [`crate::dashboard_layout_doc`]: a document-format
/// dashboard displays its bound field through this very rule, so the two
/// backings cannot lay out the same list differently.
pub(crate) fn normalize_widgets(owner: &str, widgets: &mut Vec<WidgetInstance>) {
    let hints: Vec<(String, Sizing)> =
        find_widgets(widgets.iter().map(|widget| widget.widget.as_str()), owner)
            .into_iter()
            .zip(widgets.iter())
            .map(|(found, widget)| {
                let sizing = found.map_or_else(
                    || GridSize::resolve("", None, None),
                    |(_, def)| {
                        GridSize::resolve(
                            &def.kind,
                            def.size.map(grid_size),
                            def.min.map(grid_size),
                        )
                    },
                );
                (widget.widget.clone(), sizing)
            })
            .collect();

    layout::normalize(widgets, |reference| {
        hints
            .iter()
            .find(|(name, _)| name == reference)
            .map_or_else(|| GridSize::resolve("", None, None), |(_, sizing)| *sizing)
    });
}

/// Normalize a client-provided list to what may be saved: only references that
/// resolve in the global catalog survive, duplicate instance ids collapse to
/// the first occurrence, and the shared layout rule fills in and clamps the
/// geometry.
///
/// `pub(crate)` for [`crate::dashboard_layout_doc`]: a document-format
/// dashboard saves through this same filter, so "which widgets are valid" is
/// decided in exactly one place for both backings.
pub(crate) fn sanitize_widgets(owner: &str, widgets: &[WidgetInstance]) -> Vec<WidgetInstance> {
    // Resolve the catalog once for the whole list instead of once per widget.
    let resolved = find_widgets(widgets.iter().map(|widget| widget.widget.as_str()), owner);
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<WidgetInstance> = widgets
        .iter()
        .zip(resolved)
        .filter(|(w, found)| {
            !w.id.trim().is_empty() && !w.widget.trim().is_empty() && found.is_some()
        })
        .filter(|(w, _)| seen.insert(w.id.trim().to_string()))
        .map(|(w, _)| w.clone())
        .collect();
    // The subtitle is free text from the editor: a whitespace-only value is
    // "no subtitle", not a blank line. The definition's description is the
    // fallback, resolved by the Desk.
    for widget in &mut kept {
        widget.subtitle = widget
            .subtitle
            .take()
            .map(|subtitle| subtitle.trim().to_string())
            .filter(|subtitle| !subtitle.is_empty());
    }
    normalize_widgets(owner, &mut kept);
    kept
}
