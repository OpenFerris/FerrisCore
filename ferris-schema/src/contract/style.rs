//! Appearance and layout configuration for widgets and dashboards.
//!
//! Everything a user (or an app) can configure about *how* a widget or a board
//! looks — background, border, radius, padding, shadow, the gap between tiles —
//! as opposed to *what* it shows (`params`) or *where* it sits (`layout`).
//!
//! Three levels of the same shape, each one optional and layered:
//!
//! 1. the Desk's global default ([`DashboardStyle`] on the global settings row),
//! 2. a dashboard's own override ([`DashboardConfig`], shared across users),
//! 3. a widget instance's own style ([`StyleSpec`]), over its definition's
//!    declared default ([`WidgetDef::style`]).
//!
//! Every field is `Option`, and every field that is absent means "inherit from
//! the level below". That is what makes the feature purely additive: an app that
//! declares no style at all keeps working, because the Desk resolves a default
//! for every field it did not receive.
//!
//! ## Colours are tokens, never literals
//!
//! [`StyleSpec::background`] (and its siblings) name a **semantic token**
//! (`"card"`, `"muted"`, `"accent"`, …) — a value the theme already defines, so
//! light/dark and runtime themes keep working. A user who genuinely wants a
//! colour outside the palette uses the matching `*_custom` field, which the Desk
//! injects as a **CSS custom property** on the element (`--ferris-widget-bg`)
//! and a rule in `components.css` consumes with the token as fallback. The wire
//! carries the string; the Desk decides how to apply it, exactly as the theme
//! mechanism does. This keeps the "no raw colours in classes" rule (AGENTS.md
//! §5) intact: the literal never reaches a class name.

use serde::{Deserialize, Serialize};

/// Upper bounds for the numeric style fields. Clamped by the server on save the
/// same way the grid geometry is, so a hand-crafted payload cannot store a
/// padding that breaks the layout.
pub const MAX_BORDER_WIDTH: u8 = 8;
/// Radius ceiling, in pixels: past this a widget is a pill, not a card.
pub const MAX_RADIUS: u8 = 32;
/// Padding ceiling, in pixels, of a widget's inner content.
pub const MAX_PADDING: u8 = 64;
/// Gap ceiling, in pixels, between two dashboard tiles.
pub const MAX_GAP: u8 = 64;

/// Appearance of one widget (an instance, or a definition's default).
///
/// All fields optional: an absent field inherits from the level below, and a
/// widget with no style at all renders exactly like one before this feature.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StyleSpec {
    /// Semantic token for the background (`"card"`, `"muted"`, `"accent"`,
    /// `"background"`, …). A name, not a colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Free-form background colour (any CSS colour string), used only when
    /// [`StyleSpec::background`] is absent. The Desk injects it as
    /// `--ferris-widget-bg`, so it never becomes a class name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_custom: Option<String>,
    /// Semantic token for the border colour; `"none"` removes the border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,
    /// Border width, in pixels (`0` = no border). Clamped to
    /// [`MAX_BORDER_WIDTH`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_width: Option<u8>,
    /// Corner radius, in pixels. Clamped to [`MAX_RADIUS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u8>,
    /// Inner padding, in pixels. Clamped to [`MAX_PADDING`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u8>,
    /// Semantic elevation token: `"none"`, `"xs"`, `"sm"`, `"md"`, `"lg"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
}

impl StyleSpec {
    /// Whether this style declares nothing at all — the common case, and the
    /// one that must never allocate or emit a class.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.background_custom.is_none()
            && self.border_color.is_none()
            && self.border_width.is_none()
            && self.border_radius.is_none()
            && self.padding.is_none()
            && self.shadow.is_none()
    }

    /// The layered style: `self` (the more specific level) wins over `base`,
    /// field by field.
    ///
    /// This is the *one* place the precedence between a widget instance, its
    /// definition and the global default is decided, so the Desk and any future
    /// consumer cannot resolve it differently.
    #[must_use]
    pub fn overlay(&self, base: &Self) -> Self {
        Self {
            background: self.background.clone().or_else(|| base.background.clone()),
            background_custom: self
                .background_custom
                .clone()
                .or_else(|| base.background_custom.clone()),
            border_color: self
                .border_color
                .clone()
                .or_else(|| base.border_color.clone()),
            border_width: self.border_width.or(base.border_width),
            border_radius: self.border_radius.or(base.border_radius),
            padding: self.padding.or(base.padding),
            shadow: self.shadow.clone().or_else(|| base.shadow.clone()),
        }
    }

    /// The same style with every numeric field clamped to its ceiling, so a
    /// value that arrived from a client cannot break the layout. Idempotent.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            background: self.background.clone(),
            background_custom: self.background_custom.clone(),
            border_color: self.border_color.clone(),
            border_width: self.border_width.map(|n| n.min(MAX_BORDER_WIDTH)),
            border_radius: self.border_radius.map(|n| n.min(MAX_RADIUS)),
            padding: self.padding.map(|n| n.min(MAX_PADDING)),
            shadow: self.shadow.clone(),
        }
    }
}

/// Appearance and layout of a whole board: the global default of the Desk, and
/// the per-dashboard override that layers on top of it.
///
/// Unlike [`StyleSpec`], which a widget wears, this also carries the *board*
/// geometry (the gap between tiles, the board's own padding) and the chrome
/// layout flags (full-width top bar and title). It is deliberately one record
/// rather than three: a user configuring "how my board looks" answers one form,
/// and the precedence rule has one shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardStyle {
    /// Semantic token for the board's background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Free-form board background colour, used only when
    /// [`DashboardStyle::background`] is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_custom: Option<String>,
    /// Separation between two tiles, in pixels. Replaces the Desk's fixed
    /// `GAP_PX` when set. Clamped to [`MAX_GAP`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_px: Option<u8>,
    /// The board's own inner padding, in pixels. Clamped to [`MAX_PADDING`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_px: Option<u8>,
    /// The board's corner radius, in pixels. Clamped to [`MAX_RADIUS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_radius: Option<u8>,
    /// Whether the board draws its title while it is in full-screen mode.
    /// Absent means yes: full screen keeps the title unless the board opts out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_full_screen: Option<bool>,
}

impl DashboardStyle {
    /// Whether this board style declares nothing at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.background_custom.is_none()
            && self.gap_px.is_none()
            && self.padding_px.is_none()
            && self.border_radius.is_none()
            && self.title_full_screen.is_none()
    }

    /// `self` (the dashboard override) layered over `base` (the Desk's global
    /// default), field by field — the single precedence rule for a board.
    #[must_use]
    pub fn overlay(&self, base: &Self) -> Self {
        Self {
            background: self.background.clone().or_else(|| base.background.clone()),
            background_custom: self
                .background_custom
                .clone()
                .or_else(|| base.background_custom.clone()),
            gap_px: self.gap_px.or(base.gap_px),
            padding_px: self.padding_px.or(base.padding_px),
            border_radius: self.border_radius.or(base.border_radius),
            title_full_screen: self.title_full_screen.or(base.title_full_screen),
        }
    }

    /// Every numeric field clamped to its ceiling. Idempotent.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            background: self.background.clone(),
            background_custom: self.background_custom.clone(),
            gap_px: self.gap_px.map(|n| n.min(MAX_GAP)),
            padding_px: self.padding_px.map(|n| n.min(MAX_PADDING)),
            border_radius: self.border_radius.map(|n| n.min(MAX_RADIUS)),
            title_full_screen: self.title_full_screen,
        }
    }

    /// The gap between tiles, in pixels: the configured one or the Desk's
    /// default. This is the *only* place that default is written, so the
    /// board's geometry and the drag arithmetic cannot disagree.
    #[must_use]
    pub fn gap(&self) -> u8 {
        self.gap_px
            .unwrap_or(crate::contract::layout::DEFAULT_GAP_PX)
    }
}

/// The configuration a dashboard stores, shared by every user: its own style
/// override, and the id it belongs to so a stray payload cannot be applied to
/// the wrong board.
///
/// `style` is the *override* only — the effective style is this layered over the
/// Desk's global default ([`DashboardStyle::overlay`]). Keeping the override
/// distinct from the resolved value is what lets "Restaurar global" restore
/// exactly the fields this board had overridden and leave the rest alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardConfig {
    /// The dashboard id this override belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard_id: Option<String>,
    /// The board's override of the global style.
    #[serde(default, skip_serializing_if = "DashboardStyle::is_empty")]
    pub style: DashboardStyle,
}

impl DashboardConfig {
    /// Whether this config declares nothing — no override to store.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.style.is_empty()
    }

    /// This override layered over a base style: every field the override
    /// declares wins, every field it leaves absent keeps the base's value.
    ///
    /// The base is the dashboard's declared style (or, when there is none, the
    /// Desk's global default), so "unset" genuinely means "inherit" and a
    /// "Restaurar global" button can drop one field without disturbing the
    /// others.
    #[must_use]
    pub fn overlay(&self, base: &DashboardStyle) -> DashboardStyle {
        self.style.overlay(base)
    }

    /// The config with every numeric field clamped to its ceiling.
    ///
    /// The write path runs this once, at the boundary: a hand-crafted payload
    /// cannot store a padding or a gap the grid cannot draw. Field presence is
    /// preserved — an absent field stays absent (`None`), so "inherit" survives
    /// the round trip.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            dashboard_id: self.dashboard_id.clone(),
            style: self.style.clamped(),
        }
    }
}

/// The Desk's global board style: the base every board layers its own declared
/// style and saved override over.
///
/// One row shared by the whole install (`Dashboard Settings`, name `global`),
/// edited by a System Manager from a board's configuration drawer. `editable`
/// is the `Create` grant on that doctype evaluated for the caller — the write
/// gate, repeated so the Desk can hide the global scope instead of offering a
/// button that would 403.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardSettings {
    /// The global style itself.
    #[serde(default)]
    pub style: DashboardStyle,
    /// Whether the caller may edit the global style.
    #[serde(default)]
    pub editable: bool,
}

/// The write half of [`DashboardSettings`]: what `PUT /api/dashboard-settings`
/// accepts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardSettingsPayload {
    /// The global style to store. The server clamps it, exactly like a
    /// per-dashboard override.
    #[serde(default)]
    pub style: DashboardStyle,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A style with no fields at all is the common case: it must parse from an
    /// absent key, serialize to nothing, and report itself empty so the Desk
    /// emits no class.
    #[test]
    fn an_absent_style_is_empty_and_serializes_to_nothing() {
        let style: StyleSpec = serde_json::from_value(serde_json::json!({})).expect("parses");
        assert!(style.is_empty());
        assert_eq!(
            serde_json::to_value(&style).expect("serializes"),
            serde_json::json!({})
        );
    }

    /// Every field round-trips, and the wire names are the `snake_case` ones the
    /// contract documents (no rename surprises).
    #[test]
    fn a_full_style_roundtrips_with_its_wire_names() {
        let style = StyleSpec {
            background: Some("muted".to_string()),
            background_custom: Some("oklch(0.5 0.1 200)".to_string()),
            border_color: Some("border".to_string()),
            border_width: Some(2),
            border_radius: Some(12),
            padding: Some(16),
            shadow: Some("sm".to_string()),
        };
        let wire = serde_json::to_value(&style).expect("serializes");
        assert_eq!(wire["background"], "muted");
        assert_eq!(wire["border_width"], 2);
        assert_eq!(wire["border_radius"], 12);
        assert_eq!(wire["padding"], 16);
        assert_eq!(wire["shadow"], "sm");
        let back: StyleSpec = serde_json::from_value(wire).expect("parses");
        assert_eq!(back, style);
    }

    /// The precedence rule: the more specific level wins per field, and a field
    /// it does not declare falls through to the base.
    #[test]
    fn overlay_is_field_by_field_with_the_specific_level_winning() {
        let base = StyleSpec {
            background: Some("card".to_string()),
            border_width: Some(1),
            border_radius: Some(6),
            padding: Some(8),
            ..Default::default()
        };
        let instance = StyleSpec {
            background: Some("accent".to_string()),
            padding: Some(20),
            ..Default::default()
        };
        let merged = instance.overlay(&base);
        assert_eq!(
            merged.background.as_deref(),
            Some("accent"),
            "instance wins"
        );
        assert_eq!(merged.padding, Some(20), "instance wins");
        assert_eq!(merged.border_width, Some(1), "absent falls to base");
        assert_eq!(merged.border_radius, Some(6), "absent falls to base");
        // Layering an empty style over a base is the base, unchanged.
        assert_eq!(StyleSpec::default().overlay(&base), base);
    }

    /// Clamping is bounded and idempotent, exactly like the grid geometry's.
    #[test]
    fn clamping_bounds_the_numbers_and_is_idempotent() {
        let style = StyleSpec {
            border_width: Some(200),
            border_radius: Some(200),
            padding: Some(200),
            ..Default::default()
        };
        let once = style.clamped();
        assert_eq!(once.border_width, Some(MAX_BORDER_WIDTH));
        assert_eq!(once.border_radius, Some(MAX_RADIUS));
        assert_eq!(once.padding, Some(MAX_PADDING));
        assert_eq!(once.clamped(), once, "idempotent");
    }

    /// A board with no `gap_px` falls back to the Desk default — the one place
    /// that default is written.
    #[test]
    fn board_gap_defaults_to_the_contract_default() {
        assert_eq!(
            DashboardStyle::default().gap(),
            crate::contract::layout::DEFAULT_GAP_PX
        );
        let wide = DashboardStyle {
            gap_px: Some(32),
            ..Default::default()
        };
        assert_eq!(wide.gap(), 32);
    }

    /// A board override layers over the global default per field, and a config
    /// with no override is empty (so the server stores no row for it).
    #[test]
    fn dashboard_config_layers_over_the_global_default() {
        let global = DashboardStyle {
            background: Some("background".to_string()),
            gap_px: Some(12),
            title_full_screen: Some(false),
            ..Default::default()
        };
        let override_style = DashboardStyle {
            background: Some("muted".to_string()),
            gap_px: Some(24),
            ..Default::default()
        };
        let effective = override_style.overlay(&global);
        assert_eq!(effective.background.as_deref(), Some("muted"));
        assert_eq!(effective.gap_px, Some(24));
        assert_eq!(
            effective.title_full_screen,
            Some(false),
            "unset falls to the global default"
        );

        assert!(DashboardConfig::default().is_empty());
        let config = DashboardConfig {
            dashboard_id: Some("home".to_string()),
            style: override_style,
        };
        assert!(!config.is_empty());
        let wire = serde_json::to_value(&config).expect("serializes");
        assert_eq!(wire["dashboard_id"], "home");
        assert_eq!(wire["style"]["gap_px"], 24);
    }

    /// An empty style inside a config is not serialized (`skip_serializing_if`),
    /// so a dashboard that overrides nothing stores no style at all.
    #[test]
    fn an_empty_style_is_skipped_inside_a_config() {
        let config = DashboardConfig {
            dashboard_id: Some("home".to_string()),
            style: DashboardStyle::default(),
        };
        let wire = serde_json::to_value(&config).expect("serializes");
        assert!(wire.get("style").is_none(), "empty style is skipped");
    }

    /// The global settings answer defaults to "no style, not editable", and a
    /// payload with only the fields the user set round-trips without inventing
    /// the others — the server and the Desk read the same shape.
    #[test]
    fn dashboard_settings_default_and_roundtrip() {
        let settings: DashboardSettings =
            serde_json::from_value(serde_json::json!({})).expect("parses");
        assert_eq!(settings, DashboardSettings::default());
        assert!(!settings.editable);

        let payload = DashboardSettingsPayload {
            style: DashboardStyle {
                gap_px: Some(20),
                title_full_screen: Some(true),
                ..Default::default()
            },
        };
        let wire = serde_json::to_value(&payload).expect("serializes");
        assert_eq!(wire["style"]["gap_px"], 20);
        assert!(wire["style"].get("background").is_none());
        let back: DashboardSettingsPayload = serde_json::from_value(wire).expect("round-trips");
        assert_eq!(back, payload);
    }
}
