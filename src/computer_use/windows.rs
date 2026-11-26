//! Window Server metadata and the matching native Accessibility windows.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use core_foundation::ConcreteCFType;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::geometry::{CGPoint, CGRect};
use core_graphics::window as quartz;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp::bridge::ToolReply;

use super::{Grant, ax};

pub struct Window {
    pub id: u32,
    pub pid: i32,
    pub bounds: CGRect,
    pub app: String,
    pub title: String,
    pub on_screen: bool,
    pub alpha: f64,
}

pub fn list(include_hidden: bool, pid: Option<i32>) -> Result<ToolReply> {
    let windows: Vec<_> = read(include_hidden)?
        .into_iter()
        .filter(|window| pid.is_none_or(|pid| window.pid == pid))
        .take(200)
        .map(|window| {
            json!({
                "windowId": window.id,
                "pid": window.pid,
                "app": window.app,
                "title": window.title,
                "onScreen": window.on_screen,
                "x": window.bounds.origin.x,
                "y": window.bounds.origin.y,
                "width": window.bounds.size.width,
                "height": window.bounds.size.height,
            })
        })
        .collect();
    Ok(ToolReply::text(json!({ "windows": windows }).to_string()))
}

pub fn find(id: u32) -> Result<Window> {
    // Quartz can query just one id, including a window that has been minimized.
    let windows = quartz::copy_window_info(quartz::kCGWindowListOptionIncludingWindow, id)
        .context("Could not query the macOS Window Server.")?;
    windows
        .iter()
        .filter_map(|value| decode_window(*value))
        .find(|window| window.id == id)
        .context("The window is unavailable. Call computer_windows for current window IDs.")
}

/// Frontmost on-screen window. When `skip_self` is true, skips BenCode's own
/// windows so read-only inspection reaches the active target app right away.
pub fn frontmost(skip_self: bool) -> Result<Window> {
    let self_pid = std::process::id() as i32;
    let windows = read(false)?;
    if skip_self {
        windows
            .into_iter()
            .find(|w| w.pid != self_pid)
            .context("No external on-screen window was found. Open an app or call computer_windows.")
    } else {
        let front = windows
            .into_iter()
            .next()
            .context("No on-screen window was found. Open an app or call computer_windows.")?;
        if front.pid == self_pid {
            bail!(
                "BenCode is currently in front. Focus the target window with computer_window (action='focus') first, or pass its windowId for non-pointer Accessibility actions."
            );
        }
        Ok(front)
    }
}

/// Ensures `point` is not covered by another visible normal window in front of `target_id`.
pub(super) fn ensure_point_visible(target_id: u32, point: CGPoint) -> Result<()> {
    let windows = read(false)?;
    check_point_unoccluded(&windows, target_id, point)
}

fn check_point_unoccluded(windows: &[Window], target_id: u32, point: CGPoint) -> Result<()> {
    for window in windows {
        if window.id == target_id {
            return Ok(());
        }
        if window.alpha <= 0.0 {
            continue;
        }
        if rect_contains(window.bounds, point) {
            let owner = if window.app.is_empty() {
                "another window"
            } else {
                window.app.as_str()
            };
            bail!(
                "That point is covered by {owner}. Focus the target window with computer_window (action='focus') first."
            );
        }
    }
    bail!(
        "The target window is not visible on the current Space. Focus it with computer_window (action='focus') first."
    )
}

fn rect_contains(rect: CGRect, point: CGPoint) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x < rect.origin.x + rect.size.width
        && point.y < rect.origin.y + rect.size.height
}

fn read(include_hidden: bool) -> Result<Vec<Window>> {
    let visibility = if include_hidden {
        quartz::kCGWindowListOptionAll
    } else {
        quartz::kCGWindowListOptionOnScreenOnly
    };
    let windows = quartz::copy_window_info(
        visibility | quartz::kCGWindowListExcludeDesktopElements,
        quartz::kCGNullWindowID,
    )
    .context("Could not query the macOS Window Server.")?;
    Ok(windows
        .iter()
        .filter_map(|value| decode_window(*value))
        .collect())
}

fn decode_window(value: *const std::ffi::c_void) -> Option<Window> {
    // SAFETY: the caller keeps the Window Server's CFArray alive.
    let row = unsafe { CFType::wrap_under_get_rule(value) }.downcast_into::<CFDictionary>()?;
    let get = |name: &str| {
        let key = CFString::new(name);
        let value = *row.find(key.as_CFTypeRef())?;
        Some(unsafe { CFType::wrap_under_get_rule(value) })
    };
    fn downcast<T: ConcreteCFType>(value: Option<CFType>) -> Option<T> {
        value?.downcast_into::<T>()
    }
    let integer = |name| downcast::<CFNumber>(get(name))?.to_i32();
    let text = |name| {
        downcast::<CFString>(get(name))
            .map(|value| value.to_string().chars().take(250).collect())
            .unwrap_or_default()
    };
    // Exclude the Dock, wallpaper and other system surfaces from selection.
    if integer("kCGWindowLayer")? != 0 {
        return None;
    }
    let bounds =
        CGRect::from_dict_representation(&downcast::<CFDictionary>(get("kCGWindowBounds"))?)?;
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    Some(Window {
        id: integer("kCGWindowNumber")? as u32,
        pid: integer("kCGWindowOwnerPID")?,
        bounds,
        app: text("kCGWindowOwnerName"),
        title: text("kCGWindowName"),
        on_screen: downcast::<CFBoolean>(get("kCGWindowIsOnscreen")).is_some_and(bool::from),
        alpha: downcast::<CFNumber>(get("kCGWindowAlpha"))
            .and_then(|value| value.to_f64())
            .unwrap_or(1.0),
    })
}

impl Window {
    fn accessibility(&self, grant: &Grant) -> Result<(ax::Element, ax::Element)> {
        let app = ax::Element::application(self.pid)?;
        let mut candidates = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        for window in app.windows().into_iter().take(100) {
            grant.check()?;
            if Instant::now() >= deadline {
                bail!("Timed out matching the native window.");
            }
            let position_match = window
                .bounds()
                .is_some_and(|bounds| close_bounds(self.bounds, bounds));
            let title_match = !self.title.is_empty()
                && window
                    .text("AXTitle")
                    .is_some_and(|title| title.chars().take(250).eq(self.title.chars()));
            candidates.push((position_match, title_match, window));
        }
        let matches: Vec<(bool, bool)> = candidates
            .iter()
            .map(|(bounds, title, _)| (*bounds, *title))
            .collect();
        // Only take a lone Accessibility window without a match when Window
        // Server confirms the app owns no other normal window (e.g. on another
        // Space or already closed), otherwise we could act on the wrong one.
        let index = resolve_window(&matches, || sole_window_for_pid(self.pid, self.id))
            .context("Could not uniquely match this window to an Accessibility window. Refresh computer_windows or use display coordinates.")?;
        Ok((app, candidates.swap_remove(index).2))
    }
}

fn sole_window_for_pid(pid: i32, id: u32) -> bool {
    read(true).is_ok_and(|windows| {
        let mut owned = windows.into_iter().filter(|window| window.pid == pid);
        owned.next().is_some_and(|window| window.id == id) && owned.next().is_none()
    })
}

fn resolve_window(matches: &[(bool, bool)], sole_app_window: impl FnOnce() -> bool) -> Option<usize> {
    match_window(matches.iter().copied())
        .or_else(|| (matches.len() == 1 && sole_app_window()).then_some(0))
}

pub(super) fn close_bounds(a: CGRect, b: CGRect) -> bool {
    // The visible Window Server frame may be rounded to integral points.
    [
        a.origin.x - b.origin.x,
        a.origin.y - b.origin.y,
        a.size.width - b.size.width,
        a.size.height - b.size.height,
    ]
    .iter()
    .all(|delta| delta.abs() <= 2.0)
}

fn match_window(matches: impl Iterator<Item = (bool, bool)>) -> Option<usize> {
    let mut best = None;
    let mut score = 0;
    for (index, (bounds, title)) in matches.enumerate() {
        let rank = u8::from(bounds) * 2 + u8::from(title);
        if rank > score {
            best = Some(index);
            score = rank;
        } else if rank == score {
            best = None;
        }
    }
    best
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowList {
    #[serde(default)]
    pub include_hidden: bool,
    pub pid: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowAction {
    window_id: u32,
    #[serde(flatten)]
    action: WindowCommand,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum WindowCommand {
    Focus,
    Minimize,
    Restore,
    Move { x: f64, y: f64 },
    Resize { width: f64, height: f64 },
    Fullscreen { enabled: bool },
    Close,
}

impl WindowAction {
    pub fn parse(args: Value) -> Result<Self> {
        let action: Self = serde_json::from_value(args)?;
        if let WindowCommand::Resize { width, height } = action.action
            && (!(1.0..=32768.0).contains(&width) || !(1.0..=32768.0).contains(&height))
        {
            bail!("Window width and height must be between 1 and 32768 points.");
        }
        Ok(action)
    }

    pub fn perform(self, grant: &Grant) -> Result<ToolReply> {
        let target = find(self.window_id)?;
        let (app, window) = target.accessibility(grant)?;
        grant.check()?;
        match self.action {
            WindowCommand::Focus => {
                if window.boolean("AXMinimized") == Some(true) {
                    window.set_bool("AXMinimized", false)?;
                }
                grant.check()?;
                if app.boolean("AXHidden") == Some(true) {
                    app.set_bool("AXHidden", false)?;
                }
                grant.check()?;
                app.set_bool("AXFrontmost", true)?;
                grant.check()?;
                window.action("AXRaise")?;
                grant.check()?;
                // Some apps need AXMain as well, and other apps manage it themselves.
                if let Err(err) = window.set_bool("AXMain", true) {
                    log::debug!("Could not explicitly make the raised window main: {err:#}");
                }
            }
            WindowCommand::Minimize => window.set_bool("AXMinimized", true)?,
            WindowCommand::Restore => window.set_bool("AXMinimized", false)?,
            WindowCommand::Move { x, y } => window.set_position(x, y)?,
            WindowCommand::Resize { width, height } => window.set_size(width, height)?,
            WindowCommand::Fullscreen { enabled } => window.set_bool("AXFullScreen", enabled)?,
            WindowCommand::Close => window
                .child("AXCloseButton")
                .context("The window has no accessible close button.")?
                .action("AXPress")?,
        }
        Ok(ToolReply::text("Window action completed."))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiAction {
    window_id: Option<u32>,
    #[serde(flatten)]
    action: UiCommand,
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum UiCommand {
    Outline {
        query: Option<String>,
        #[serde(default)]
        interactive_only: bool,
        limit: Option<usize>,
    },
    Control {
        element: String,
        verb: ax::Control,
        text: Option<String>,
    },
}

impl UiAction {
    pub fn follows_focus(&self) -> bool {
        match self.action {
            UiCommand::Outline { .. } => false,
            UiCommand::Control { verb, .. } => {
                self.window_id.is_none()
                    || matches!(verb, ax::Control::Click | ax::Control::DoubleClick)
            }
        }
    }

    pub fn parse(args: Value) -> Result<Self> {
        let action: Self = serde_json::from_value(args)?;
        if let UiCommand::Control {
            ref element,
            ref text,
            ..
        } = action.action
            && (element.len() > 150
                || text
                    .as_ref()
                    .is_some_and(|text| text.chars().count() > 4000))
        {
            bail!("Use a current element ref and at most 4000 text characters.");
        }
        if let UiCommand::Outline {
            ref query, limit, ..
        } = action.action
        {
            if query.as_ref().is_some_and(|q| q.chars().count() > 200) {
                bail!("Query must be at most 200 characters.");
            }
            if let Some(limit) = limit
                && !(1..=250).contains(&limit)
            {
                bail!("Limit must be between 1 and 250.");
            }
        }
        Ok(action)
    }

    pub fn perform(self, grant: &Grant) -> Result<ToolReply> {
        let is_outline = matches!(self.action, UiCommand::Outline { .. });
        let target = match self.window_id {
            Some(id) => find(id)?,
            None => frontmost(is_outline)?,
        };
        let (_, window) = target.accessibility(grant)?;
        match self.action {
            UiCommand::Outline {
                query,
                interactive_only,
                limit,
            } => {
                let options = ax::OutlineOptions {
                    query,
                    interactive_only,
                    limit,
                };
                let mut result = window.outline(&options, grant)?;
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("windowId".into(), json!(target.id));
                    obj.insert("app".into(), json!(target.app));
                    obj.insert("title".into(), json!(target.title));
                    obj.insert(
                        "bounds".into(),
                        json!([
                            target.bounds.origin.x.round() as i64,
                            target.bounds.origin.y.round() as i64,
                            target.bounds.size.width.round() as i64,
                            target.bounds.size.height.round() as i64,
                        ]),
                    );
                }
                Ok(ToolReply::text(result.to_string()))
            }
            UiCommand::Control {
                element,
                verb,
                text,
            } => {
                if matches!(verb, ax::Control::Click | ax::Control::DoubleClick)
                    && !target.on_screen
                {
                    bail!(
                        "The window is not on screen. Focus it with computer_window before clicking."
                    );
                }
                window.control(
                    &element,
                    verb,
                    text.as_deref(),
                    target.id,
                    target.bounds,
                    grant,
                )?;
                Ok(ToolReply::text("Accessibility action completed."))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_matching_requires_a_unique_best_match() {
        assert_eq!(
            match_window([(true, false), (true, true), (false, true)].into_iter()),
            Some(1)
        );
        assert_eq!(match_window([(true, true), (true, true)].into_iter()), None);
        assert_eq!(match_window([(false, false)].into_iter()), None);
        assert_eq!(match_window([(false, true)].into_iter()), Some(0));
        // A lone AX window falls back only when the app has no other window.
        assert_eq!(resolve_window(&[(false, false)], || true), Some(0));
        assert_eq!(resolve_window(&[(false, false)], || false), None);
        assert_eq!(resolve_window(&[(false, true)], || false), Some(0));
    }

    #[test]
    fn decodes_window_server_metadata_without_reading_real_windows() {
        let dictionary = |values: Vec<(&str, CFType)>| {
            let pairs: Vec<_> = values
                .into_iter()
                .map(|(name, value)| (CFString::new(name), value))
                .collect();
            CFDictionary::from_CFType_pairs(&pairs)
        };
        let bounds = dictionary(vec![
            ("X", CFNumber::from(-50).as_CFType()),
            ("Y", CFNumber::from(23).as_CFType()),
            ("Width", CFNumber::from(640).as_CFType()),
            ("Height", CFNumber::from(480).as_CFType()),
        ]);
        let row = dictionary(vec![
            ("kCGWindowNumber", CFNumber::from(37).as_CFType()),
            ("kCGWindowOwnerPID", CFNumber::from(111).as_CFType()),
            ("kCGWindowLayer", CFNumber::from(0).as_CFType()),
            ("kCGWindowOwnerName", CFString::new("Preview").as_CFType()),
            ("kCGWindowName", CFString::new("Bản xem thử").as_CFType()),
            ("kCGWindowIsOnscreen", CFBoolean::true_value().as_CFType()),
            ("kCGWindowBounds", bounds.as_CFType()),
        ]);
        let decoded = decode_window(row.as_CFTypeRef()).unwrap();
        assert_eq!((decoded.id, decoded.pid), (37, 111));
        assert!(decoded.on_screen);
        assert_eq!(decoded.title, "Bản xem thử");
        assert_eq!(decoded.bounds.origin.x, -50.0);
    }

    #[test]
    fn parses_window_and_accessibility_requests() {
        for action in ["focus", "restore", "close"] {
            WindowAction::parse(json!({"windowId": 1, "action": action})).unwrap();
        }
        assert!(
            WindowAction::parse(
                json!({"windowId": 1, "action": "resize", "width": -1, "height": 45})
            )
            .is_err()
        );
        assert!(
            UiAction::parse(json!({
                "windowId": 2, "action": "control", "element": "3:deadbeef",
                "verb": "setText", "text": "Tiếng Việt"
            }))
            .is_ok()
        );
        assert!(
            UiAction::parse(json!({
                "action": "outline", "query": "Save", "interactiveOnly": true, "limit": 50
            }))
            .is_ok()
        );
        assert!(
            UiAction::parse(json!({
                "windowId": 2, "action": "control", "element": "3:deadbeef",
                "verb": "doubleClick"
            }))
            .is_ok()
        );
        assert!(
            UiAction::parse(json!({
                "action": "outline", "limit": 300
            }))
            .is_err()
        );
        // 150 Vietnamese accented characters (> 200 UTF-8 bytes) must be accepted; 201 rejected.
        let viet = "ệ".repeat(150);
        assert!(viet.len() > 200);
        assert!(UiAction::parse(json!({"action": "outline", "query": viet})).is_ok());
        assert!(
            UiAction::parse(json!({"action": "outline", "query": "ệ".repeat(201)})).is_err()
        );
    }

    #[test]
    fn point_occlusion_blocks_covered_points_and_ignores_transparent_helpers() {
        use core_graphics::geometry::CGSize;
        let win = |id, app: &str, x, y, w, h, alpha| Window {
            id,
            pid: id as i32,
            bounds: CGRect::new(&CGPoint::new(x, y), &CGSize::new(w, h)),
            app: app.into(),
            title: String::new(),
            on_screen: true,
            alpha,
        };
        let stack = vec![
            win(1, "Helper", 0.0, 0.0, 1440.0, 900.0, 0.0),
            win(2, "BenCode", 0.0, 0.0, 700.0, 800.0, 1.0),
            win(3, "Notes", 200.0, 100.0, 800.0, 600.0, 1.0),
        ];
        // Point inside Notes but covered by BenCode must be rejected.
        let err = check_point_unoccluded(&stack, 3, CGPoint::new(500.0, 300.0)).unwrap_err();
        assert!(err.to_string().contains("BenCode"), "{err:#}");
        // Exposed part of Notes (to the right of BenCode) is clickable even with transparent Helper above.
        assert!(check_point_unoccluded(&stack, 3, CGPoint::new(850.0, 300.0)).is_ok());
        // Window absent from current Space's on-screen list must be rejected.
        assert!(check_point_unoccluded(&stack, 99, CGPoint::new(850.0, 300.0)).is_err());
    }
}
