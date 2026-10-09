//! Where a project's terminal dock sits and how big it is (MonoCode
//! `projects/model/projectTerminal.ts`: `DockSide`, `clampDockSize`,
//! `withDockSide`), and the tab moves its strip offers.

use serde::{Deserialize, Serialize};

/// MonoCode `DockSide`: which edge of the workspace the dock is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockSide {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl DockSide {
    pub const ALL: [DockSide; 4] = [
        DockSide::Bottom,
        DockSide::Top,
        DockSide::Left,
        DockSide::Right,
    ];

    /// MonoCode `isVerticalDock`: stacked above or below the workspace, so
    /// its size is a height.
    pub fn is_vertical(self) -> bool {
        matches!(self, DockSide::Top | DockSide::Bottom)
    }

    /// MonoCode `DOCK_SIZE_DEFAULT`.
    pub fn default_size(self) -> f32 {
        if self.is_vertical() { 220.0 } else { 360.0 }
    }

    /// MonoCode `VERTICAL_MIN` / `HORIZONTAL_MIN`.
    fn min_size(self) -> f32 {
        if self.is_vertical() { 88.0 } else { 180.0 }
    }

    /// MonoCode `SIDE_ITEMS`' labels.
    pub fn label(self) -> &'static str {
        match self {
            DockSide::Bottom => "Dock Bottom",
            DockSide::Top => "Dock Top",
            DockSide::Left => "Dock Left",
            DockSide::Right => "Dock Right",
        }
    }

    /// The menu row id for this side.
    pub fn id(self) -> &'static str {
        match self {
            DockSide::Bottom => "bottom",
            DockSide::Top => "top",
            DockSide::Left => "left",
            DockSide::Right => "right",
        }
    }

    pub fn from_id(id: &str) -> Option<DockSide> {
        DockSide::ALL.into_iter().find(|side| side.id() == id)
    }
}

/// One project's dock placement and whether it is shown, saved in
/// `settings.json` (MonoCode keeps `side`, `size` and `open` on its
/// `ProjectTerminalDock`). A project without one has a hidden bottom dock.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockLayout {
    pub side: DockSide,
    pub size: f32,
    #[serde(default)]
    pub open: bool,
}

impl Default for DockLayout {
    fn default() -> Self {
        let side = DockSide::default();
        Self {
            side,
            size: side.default_size(),
            open: false,
        }
    }
}

impl DockLayout {
    /// MonoCode `withDockSide`: the size carries over, clamped for the new side.
    pub fn with_side(self, side: DockSide, viewport: (f32, f32)) -> Self {
        Self {
            side,
            size: clamp_size(side, self.size, viewport),
            ..self
        }
    }

    /// The size a sash drag from `start` to `point` gives: the dock grows
    /// as the sash moves away from its edge (MonoCode `onResizePointerMove`).
    pub fn dragged(self, start_size: f32, start: f32, point: f32, viewport: (f32, f32)) -> Self {
        let delta = point - start;
        let signed = match self.side {
            DockSide::Bottom | DockSide::Right => -delta,
            DockSide::Top | DockSide::Left => delta,
        };
        Self {
            size: clamp_size(self.side, start_size + signed, viewport),
            ..self
        }
    }
}

/// MonoCode `clampDockSize`: at least the side's minimum, at most 70% of
/// the window along the dock's axis. `viewport` is `(width, height)`.
pub fn clamp_size(side: DockSide, value: f32, viewport: (f32, f32)) -> f32 {
    let min = side.min_size();
    let span = if side.is_vertical() {
        viewport.1
    } else {
        viewport.0
    };
    let max = (span * 0.7).floor().max(min);
    if !value.is_finite() {
        return side.default_size();
    }
    value.round().clamp(min, max)
}

/// Moves the item at `from` to `to`, as a tab dropped on another tab.
pub fn reorder<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: (f32, f32) = (1400.0, 1000.0);

    #[test]
    fn dragging_away_from_the_edge_grows_the_dock() {
        let bottom = DockLayout::default();
        assert_eq!(bottom.dragged(220.0, 500.0, 400.0, VIEWPORT).size, 320.0);
        assert_eq!(bottom.dragged(220.0, 500.0, 600.0, VIEWPORT).size, 120.0);

        let top = DockLayout {
            side: DockSide::Top,
            size: 220.0,
            open: true,
        };
        assert_eq!(top.dragged(220.0, 300.0, 400.0, VIEWPORT).size, 320.0);

        let left = DockLayout {
            side: DockSide::Left,
            size: 360.0,
            open: true,
        };
        assert_eq!(left.dragged(360.0, 400.0, 500.0, VIEWPORT).size, 460.0);

        let right = DockLayout {
            side: DockSide::Right,
            size: 360.0,
            open: true,
        };
        assert_eq!(right.dragged(360.0, 1000.0, 900.0, VIEWPORT).size, 460.0);
    }

    #[test]
    fn sizes_stay_within_the_side_bounds() {
        assert_eq!(clamp_size(DockSide::Bottom, 10.0, VIEWPORT), 88.0);
        assert_eq!(clamp_size(DockSide::Bottom, 5000.0, VIEWPORT), 700.0);
        assert_eq!(clamp_size(DockSide::Left, 10.0, VIEWPORT), 180.0);
        assert_eq!(clamp_size(DockSide::Right, 5000.0, VIEWPORT), 980.0);
        // A window too small for 70% still allows the minimum.
        assert_eq!(clamp_size(DockSide::Bottom, 300.0, (100.0, 100.0)), 88.0);
        assert_eq!(clamp_size(DockSide::Top, f32::NAN, VIEWPORT), 220.0);
    }

    #[test]
    fn changing_side_keeps_the_size_when_it_fits() {
        let bottom = DockLayout {
            side: DockSide::Bottom,
            size: 600.0,
            open: true,
        };
        assert_eq!(bottom.with_side(DockSide::Left, VIEWPORT).size, 600.0);
        assert!(bottom.with_side(DockSide::Left, VIEWPORT).open);
        assert_eq!(bottom.with_side(DockSide::Top, (1400.0, 500.0)).size, 350.0);
    }

    #[test]
    fn layout_round_trips_in_monocode_terms() {
        let layout = DockLayout {
            side: DockSide::Right,
            size: 400.0,
            open: true,
        };
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(json, r#"{"side":"right","size":400.0,"open":true}"#);
        assert_eq!(serde_json::from_str::<DockLayout>(&json).unwrap(), layout);
        // Saved before docks remembered being shown: hidden.
        let old = serde_json::from_str::<DockLayout>(r#"{"side":"left","size":300.0}"#).unwrap();
        assert!(!old.open);
        assert_eq!(DockSide::from_id("top"), Some(DockSide::Top));
    }

    #[test]
    fn reorder_moves_one_item() {
        let mut items = vec![1, 2, 3, 4];
        reorder(&mut items, 0, 2);
        assert_eq!(items, [2, 3, 1, 4]);
        reorder(&mut items, 3, 0);
        assert_eq!(items, [4, 2, 3, 1]);
        reorder(&mut items, 1, 9);
        assert_eq!(items, [4, 2, 3, 1]);
    }
}
