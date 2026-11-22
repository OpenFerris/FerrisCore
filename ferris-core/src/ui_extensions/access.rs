//! Read accessors over the UI registry (stable owner order) and lookups.

use super::registry::UI;
use super::specs::{AppUi, DashboardSpec, StyleSpec, UiExtensionSpec, WebsiteSpec, WidgetDefSpec};
use super::validation::resolve_widget_in;

/// Whether any app has registered a UI set yet.
pub fn has_ui_extensions() -> bool {
    UI.is_registered()
}

/// Every registered UI set, in the stable mount order of Fase 4 (§8).
///
/// Apps are sorted by their `owner` tag and each app's own declarations keep
/// their order — so slots, dashboards and the wire feed mount identically
/// no matter which order plugins happened to register in (limitación 11).
/// The write path still decides registration-time ties (a second `default`
/// dashboard is demoted on arrival, in registration order).
pub fn registered_ui() -> Vec<AppUi> {
    let mut apps = UI.snapshot();
    apps.sort_by(|a, b| a.owner.cmp(&b.owner));
    apps
}

/// Every registered slot extension, in the stable owner order of
/// [`registered_ui`] (owner tags stripped).
pub fn registered_extensions() -> Vec<UiExtensionSpec> {
    registered_ui()
        .into_iter()
        .flat_map(|ui| ui.extensions)
        .collect()
}

/// Every registered dashboard, in the stable owner order of
/// [`registered_ui`] (owner tags stripped).
pub fn registered_dashboards() -> Vec<DashboardSpec> {
    registered_ui()
        .into_iter()
        .flat_map(|ui| ui.dashboards)
        .collect()
}

/// Every registered widget definition with its owner, in the stable owner
/// order of [`registered_ui`].
#[must_use]
pub fn registered_widgets() -> Vec<(String, WidgetDefSpec)> {
    registered_ui()
        .into_iter()
        .flat_map(|ui| {
            let owner = ui.owner;
            ui.widgets.into_iter().map(move |w| (owner.clone(), w))
        })
        .collect()
}

/// Resolve a widget reference: `owner.name` selects that owner's definition,
/// a bare `name` is looked up under `default_owner` (the dashboard's app).
#[must_use]
pub fn find_widget(reference: &str, default_owner: &str) -> Option<(String, WidgetDefSpec)> {
    let apps = registered_ui();
    resolve_widget_in(&apps, reference, default_owner)
}

/// Resolve a whole list of widget references against **one** snapshot of the
/// registry.
///
/// This is the batch form of [`find_widget`], so normalizing a dashboard
/// resolves N references with one clone of the catalog instead of N.
#[must_use]
pub fn find_widgets<'a>(
    references: impl IntoIterator<Item = &'a str>,
    default_owner: &str,
) -> Vec<Option<(String, WidgetDefSpec)>> {
    let apps = registered_ui();
    references
        .into_iter()
        .map(|reference| resolve_widget_in(&apps, reference, default_owner))
        .collect()
}

/// Every registered website declaration, in the stable owner order of
/// [`registered_ui`] (owner tags stripped).
pub fn registered_websites() -> Vec<WebsiteSpec> {
    registered_ui()
        .into_iter()
        .filter_map(|ui| ui.website)
        .collect()
}

/// Whether an extension is declared for `slot` and (optionally) `doctype`.
///
/// The host's views use this to decide whether a mount point has anything to
/// render. Doctype-aware slots match extensions whose `doctype` is `None`
/// (applies to every doctype) or equals `doctype`; slot-wide extensions match
/// on the slot alone.
pub fn extension_for(slot: &str, doctype: Option<&str>) -> Option<UiExtensionSpec> {
    registered_extensions().into_iter().rev().find(|ext| {
        ext.slot == slot
            && match &ext.doctype {
                None => true,
                Some(have) => doctype.is_some_and(|wanted| wanteq(have, wanted)),
            }
    })
}

/// Case-insensitive doctype equality: `DocTypes` are capitalized identifiers but
/// routes and user input often arrive lowercase.
///
/// `eq_ignore_ascii_case` rather than lowercasing both sides: this one runs per
/// extension per mounted slot and must not allocate.
const fn wanteq(have: &str, wanted: &str) -> bool {
    have.eq_ignore_ascii_case(wanted)
}

/// Resolve one widget instance's appearance against its definition: the
/// instance's own style wins field by field, and whatever it leaves unset falls
/// back to the definition's declared default.
///
/// This is the **one** place the widget-level precedence is decided, on the spec
/// side, so the wire projection in `ferris-api` and any other consumer cannot
/// disagree about which level wins. `StyleSpec::overlay` on the contract type is
/// the same rule; keeping the *call* here (over the registry's `WidgetDefSpec`)
/// means a caller never has to remember that the instance is the more specific
/// level. It returns `None` when neither level declares anything, which is the
/// common case and must stay allocation-free.
///
/// [`StyleSpec::overlay`]: crate::ui_extensions::StyleSpec::overlay
#[must_use]
pub fn widget_style(
    instance_style: Option<&StyleSpec>,
    definition: &WidgetDefSpec,
) -> Option<StyleSpec> {
    match (instance_style, definition.style.as_ref()) {
        (None, None) => None,
        (Some(instance), None) => Some(instance.clamped()),
        (None, Some(base)) => Some(base.clamped()),
        (Some(instance), Some(base)) => Some(instance.overlay(base).clamped()),
    }
}

/// Resolve one widget reference's declared style: the definition looked up by
/// reference (bare names resolve against `default_owner`), or `None` when the
/// reference resolves to no definition.
///
/// The lookup twin of [`widget_style`], for a caller that has only the reference
/// (a stored instance, a feed) and not the definition in hand.
#[must_use]
pub fn widget_style_by_reference(
    reference: &str,
    default_owner: &str,
    instance_style: Option<&StyleSpec>,
) -> Option<StyleSpec> {
    find_widget(reference, default_owner).and_then(|(_, def)| widget_style(instance_style, &def))
}
