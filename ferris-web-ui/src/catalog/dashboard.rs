//! Dashboard builder: the board the tiles sit on and the tile itself.

/// The board: the surface behind the grid. Its background, radius and inner
/// padding are the configured ones (`--ferris-board-*`, written by the Desk
/// from the resolved `DashboardStyle`); the geometry of each tile stays in the
/// inline style, because it is data, not a design decision.
pub const DASHBOARD_BOARD: &str = "ferris-dashboard-board";

/// A tile in the dashboard grid. The class owns the tile's motion (the
/// geometry itself is inline style — a data-dependent box cannot be a class)
/// and the configured appearance (`--ferris-tile-*`): move/resize transitions,
/// the no-lag override while the pointer drags it (`data-dragging`), the
/// entrance animation and the reduced-motion opt-out.
pub const DASHBOARD_TILE: &str = "ferris-dashboard-tile";

/// The tile in edit mode: the seam for the header strip. The builder adds no
/// surface of its own — the configured background, border, radius and shadow
/// stay exactly as they are — and only squares the top edge so the strip sits
/// flush on the tile.
pub const DASHBOARD_TILE_EDITING: &str = "ferris-dashboard-tile--editing";

/// The strip above a tile in edit mode (handle, reference, buttons): its
/// surface and bottom rule.
pub const DASHBOARD_TILE_CHROME: &str = "ferris-dashboard-tile__chrome";

/// The resize corner of a tile in edit mode: its surface and colour.
pub const DASHBOARD_TILE_RESIZE: &str = "ferris-dashboard-tile__resize";
