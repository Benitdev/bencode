//! Apple's Accessibility API for window management and native controls.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use serde_json::{Value, json};

use super::{Grant, input};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFStringRef,
        out: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, name: CFStringRef, value: CFTypeRef)
    -> i32;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFStringRef) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
}

const POINT: u32 = 1; // kAXValueTypeCGPoint
const SIZE: u32 = 2; // kAXValueTypeCGSize
const ELEMENT_TIMEOUT: f32 = 0.4;
const MAX_DEPTH: usize = 18;
const DEFAULT_MAX_NODES: usize = 150;
const MAX_NODES_CAP: usize = 250;
const MAX_VISITED: usize = 1200;
const MAX_WALK: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, Default)]
pub struct OutlineOptions {
    pub query: Option<String>,
    pub interactive_only: bool,
    pub limit: Option<usize>,
}

/// Asks for the Accessibility grant the window and control tools need
/// (`AXIsProcessTrusted`); true when BenCode already has it.
pub fn request_permission() -> bool {
    // SAFETY: Apple's constant key, borrowed for the dictionary's life.
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    // SAFETY: the options dictionary lives through the call; macOS shows
    // its prompt asynchronously.
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) != 0 }
}

pub struct Element(CFType);

impl Element {
    pub fn application(pid: i32) -> Result<Self> {
        // macOS answers Accessibility calls on our own process in-process,
        // on the calling thread: from the background executor that runs
        // AppKit off the main thread and aborts the app.
        if u32::try_from(pid).is_ok_and(|pid| pid == std::process::id()) {
            bail!(
                "BenCode cannot control its own windows through Accessibility. \
                 Act on another app's window, or use display coordinates."
            );
        }
        // SAFETY: queries the calling process and returns an owned AX object.
        if unsafe { AXIsProcessTrusted() } == 0 {
            bail!("Grant BenCode Accessibility access in macOS Privacy & Security, then retry.");
        }
        let value = unsafe { AXUIElementCreateApplication(pid) };
        if value.is_null() {
            bail!("macOS did not return an Accessibility object for process {pid}.");
        }
        Self::from_cf(unsafe { CFType::wrap_under_create_rule(value) })
            .context("macOS did not return an Accessibility application.")
    }

    fn from_cf(value: CFType) -> Option<Self> {
        // SAFETY: only AX objects are passed to the messaging APIs.
        if value.type_of() != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(value.as_CFTypeRef(), ELEMENT_TIMEOUT) };
        Some(Self(value))
    }

    fn attribute(&self, name: &str) -> Option<CFType> {
        let name = CFString::new(name);
        let mut value = std::ptr::null();
        // SAFETY: `out` receives an owned CF object if the call succeeds.
        let error = unsafe {
            AXUIElementCopyAttributeValue(
                self.0.as_CFTypeRef(),
                name.as_concrete_TypeRef(),
                &mut value,
            )
        };
        (error == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    pub fn text(&self, name: &str) -> Option<String> {
        self.attribute(name)?
            .downcast_into::<CFString>()
            .map(|text| text.to_string())
    }

    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.attribute(name)?
            .downcast_into::<CFBoolean>()
            .map(bool::from)
    }

    pub fn child(&self, name: &str) -> Option<Self> {
        Self::from_cf(self.attribute(name)?)
    }

    fn array(&self, name: &str) -> Option<CFArray> {
        self.attribute(name)?.downcast_into::<CFArray>()
    }

    pub fn windows(&self) -> Vec<Self> {
        self.array("AXWindows").map_or_else(Vec::new, |array| {
            array
                .iter()
                .filter_map(|value| element_ref(*value))
                .collect()
        })
    }

    pub fn bounds(&self) -> Option<CGRect> {
        let position = self.geometry("AXPosition", POINT)?;
        let size = self.geometry("AXSize", SIZE)?;
        Some(CGRect::new(&position, &size))
    }

    fn geometry<T: Default>(&self, name: &str, kind: u32) -> Option<T> {
        let value = self.attribute(name)?;
        let mut result = T::default();
        // SAFETY: the caller pairs CGPoint/CGSize with its AX kind.
        (value.type_of() == unsafe { AXValueGetTypeID() }
            && unsafe {
                AXValueGetValue(value.as_CFTypeRef(), kind, (&mut result as *mut T).cast())
            } != 0)
            .then_some(result)
    }

    fn set(&self, name: &str, value: &impl TCFType) -> Result<()> {
        let attr = CFString::new(name);
        // SAFETY: both typed CF objects live through the call.
        let error = unsafe {
            AXUIElementSetAttributeValue(
                self.0.as_CFTypeRef(),
                attr.as_concrete_TypeRef(),
                value.as_CFTypeRef(),
            )
        };
        ax_result(error, name)
    }

    pub fn set_bool(&self, name: &str, enabled: bool) -> Result<()> {
        self.set(name, &CFBoolean::from(enabled))
    }

    pub fn set_position(&self, x: f64, y: f64) -> Result<()> {
        self.set_geometry("AXPosition", POINT, &CGPoint::new(x, y))
    }

    pub fn set_size(&self, width: f64, height: f64) -> Result<()> {
        self.set_geometry("AXSize", SIZE, &CGSize::new(width, height))
    }

    fn set_geometry<T>(&self, name: &str, kind: u32, geometry: &T) -> Result<()> {
        // SAFETY: the caller pairs the native struct with its AX kind.
        let value = unsafe { AXValueCreate(kind, (geometry as *const T).cast()) };
        if value.is_null() {
            bail!("Could not create a macOS window geometry value.");
        }
        self.set(name, &unsafe { CFType::wrap_under_create_rule(value) })
    }

    pub fn action(&self, action: &str) -> Result<()> {
        let attr = CFString::new(action);
        // SAFETY: the AX object and action live through this call.
        let error =
            unsafe { AXUIElementPerformAction(self.0.as_CFTypeRef(), attr.as_concrete_TypeRef()) };
        ax_result(error, action)
    }

    pub fn outline(&self, options: &OutlineOptions, grant: &Grant) -> Result<Value> {
        let limit = options
            .limit
            .unwrap_or(DEFAULT_MAX_NODES)
            .clamp(1, MAX_NODES_CAP);
        let query = options
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_lowercase);
        let mut outline = Outline {
            grant,
            deadline: Instant::now() + MAX_WALK,
            limit,
            query,
            interactive_only: options.interactive_only,
            visited: 0,
            nodes: Vec::new(),
            truncated: false,
        };
        outline.walk(self)?;
        Ok(json!({
            "elements": outline.nodes,
            "count": outline.nodes.len(),
            "truncated": outline.truncated,
        }))
    }

    pub fn control(
        &self,
        reference: &str,
        verb: Control,
        text: Option<&str>,
        window_id: u32,
        window_bounds: CGRect,
        grant: &Grant,
    ) -> Result<()> {
        let (path, fingerprint) = reference
            .split_once(':')
            .context("Use an element ref from computer_ui outline.")?;
        let mut current = Self(self.0.clone());
        if !path.is_empty() {
            let path: Vec<isize> = path
                .split('.')
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()?;
            if path.len() > MAX_DEPTH {
                bail!("Invalid element reference. Request a fresh outline.");
            }
            for index in path {
                grant.check()?;
                if index < 0 {
                    bail!("Invalid element reference.");
                }
                current = current
                    .array("AXChildren")
                    .and_then(|children| children.get(index).and_then(|value| element_ref(*value)))
                    .context("The element changed. Request a fresh outline.")?;
            }
        }
        if fingerprint != current.description().fingerprint() {
            bail!("The element changed. Request a fresh outline.");
        }
        grant.check()?;
        match verb {
            Control::Press => current.action("AXPress"),
            Control::Focus => current.set_bool("AXFocused", true),
            Control::Increment => current.action("AXIncrement"),
            Control::Decrement => current.action("AXDecrement"),
            Control::SetText => {
                if current.text("AXSubrole").as_deref() == Some("AXSecureTextField") {
                    bail!("This is a secure text field. Use normal keyboard input.");
                }
                current.set(
                    "AXValue",
                    &CFString::new(text.context("setText requires text.")?),
                )
            }
            Control::Click | Control::DoubleClick => {
                let bounds = current
                    .bounds()
                    .context("This element has no on-screen bounds to click.")?;
                let center = element_center(bounds, window_bounds)?;
                super::windows::ensure_point_visible(window_id, center)?;
                let count = if matches!(verb, Control::DoubleClick) {
                    2
                } else {
                    1
                };
                grant.input(|held| input::click_point(center, count, held, grant))
            }
        }
    }

    /// What the control holds (a field's text, a checkbox's state, a
    /// slider's number); never a secure field's.
    fn value(&self, role: &str) -> Option<Value> {
        if matches!(
            role,
            "AXWindow" | "AXScrollArea" | "AXSplitGroup" | "AXToolbar"
        ) {
            return None;
        }
        if matches!(role, "AXTextField" | "AXTextArea")
            && self.text("AXSubrole").as_deref() == Some("AXSecureTextField")
        {
            return None;
        }
        let value = self.attribute("AXValue")?;
        if let Some(text) = value.downcast::<CFString>() {
            let text: String = text.to_string().chars().take(250).collect();
            return (!text.is_empty()).then(|| Value::String(text));
        }
        if let Some(flag) = value.downcast::<CFBoolean>() {
            return Some(Value::Bool(flag.into()));
        }
        let number = value.downcast::<CFNumber>()?;
        if let Some(int) = number.to_i64() {
            return Some(json!(int));
        }
        number.to_f64().map(|float| json!(float))
    }

    fn description(&self) -> Description {
        let role = self.text("AXRole").unwrap_or_default();
        let label = self
            .text("AXTitle")
            .filter(|text| !text.is_empty())
            .or_else(|| self.text("AXDescription"))
            .unwrap_or_default();
        let identifier = self.text("AXIdentifier").unwrap_or_default();
        Description {
            role,
            label,
            identifier,
        }
    }
}

#[derive(Clone, Copy, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Control {
    Press,
    Focus,
    Increment,
    Decrement,
    SetText,
    Click,
    DoubleClick,
}

fn element_center(bounds: CGRect, window_bounds: CGRect) -> Result<CGPoint> {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        bail!("This element is hidden or has zero size.");
    }
    let center = CGPoint::new(
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    let outside = center.x < window_bounds.origin.x - 2.0
        || center.y < window_bounds.origin.y - 2.0
        || center.x > window_bounds.origin.x + window_bounds.size.width + 2.0
        || center.y > window_bounds.origin.y + window_bounds.size.height + 2.0;
    if outside {
        bail!("This element is scrolled outside the visible window.");
    }
    Ok(center)
}

fn is_ignored_subtree(role: &str) -> bool {
    matches!(role, "AXScrollBar" | "AXValueIndicator" | "AXSplitter")
}

fn is_pure_chrome_container(role: &str) -> bool {
    matches!(
        role,
        "AXWindow" | "AXSplitGroup" | "AXScrollArea" | "AXLayoutArea" | "AXToolbar"
    )
}

fn is_container_role(role: &str) -> bool {
    is_pure_chrome_container(role)
        || matches!(
            role,
            "AXGroup"
                | "AXSection"
                | "AXTabGroup"
                | "AXList"
                | "AXOutline"
                | "AXTable"
                | "AXColumn"
                | "AXBrowser"
                | "AXSheet"
                | "AXDrawer"
                | "AXPopover"
                | "AXWebArea"
                | "AXGenericElement"
                | "AXPane"
                | "AXBox"
        )
}

fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "AXButton"
            | "AXTextField"
            | "AXTextArea"
            | "AXSearchField"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXPopUpButton"
            | "AXComboBox"
            | "AXSlider"
            | "AXMenuButton"
            | "AXMenuItem"
            | "AXMenuBarItem"
            | "AXTab"
            | "AXLink"
            | "AXDisclosureTriangle"
            | "AXIncrementor"
            | "AXColorWell"
            | "AXDateField"
            | "AXSwitch"
            | "AXSegmentedControl"
            | "AXRow"
            | "AXOutlineRow"
            | "AXCell"
    )
}

fn is_list_item_role(role: &str) -> bool {
    matches!(role, "AXRow" | "AXOutlineRow" | "AXCell")
}

/// When `interactiveOnly` is set, keep actionable controls plus static text
/// labels inside list/outline rows (macOS view-based tables and SwiftUI Lists
/// leave `AXRow`/`AXCell` unlabeled and store row text in child `AXStaticText`).
fn keep_in_interactive_only(role: &str, in_list_item: bool) -> bool {
    is_interactive_role(role) || (in_list_item && role == "AXStaticText")
}

fn matches_query(query: &str, desc: &Description, value: Option<&Value>) -> bool {
    let contains = |haystack: &str| haystack.to_lowercase().contains(query);
    contains(&desc.label)
        || contains(&desc.role)
        || contains(&desc.identifier)
        || value.is_some_and(|v| match v {
            Value::String(s) => contains(s),
            other => contains(&other.to_string()),
        })
}

fn element_ref(value: CFTypeRef) -> Option<Element> {
    if value.is_null() {
        return None;
    }
    // SAFETY: the caller borrowed this value from a live CFArray.
    Element::from_cf(unsafe { CFType::wrap_under_get_rule(value) })
}

fn ax_result(error: i32, operation: &str) -> Result<()> {
    if error != 0 {
        bail!("The target app could not perform {operation} (macOS Accessibility error {error}).");
    }
    Ok(())
}

struct Description {
    role: String,
    label: String,
    identifier: String,
}

impl Description {
    fn fingerprint(&self) -> String {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hash = DefaultHasher::new();
        (&self.role, &self.label, &self.identifier).hash(&mut hash);
        format!("{:016x}", hash.finish())
    }
}

struct Outline<'a> {
    grant: &'a Grant,
    deadline: Instant,
    limit: usize,
    query: Option<String>,
    interactive_only: bool,
    visited: usize,
    nodes: Vec<Value>,
    truncated: bool,
}

impl Outline<'_> {
    fn spent(&mut self) -> bool {
        let spent = self.nodes.len() >= self.limit
            || self.visited >= MAX_VISITED
            || Instant::now() >= self.deadline;
        self.truncated |= spent;
        spent
    }

    /// Breadth-first walk that skips layout containers when they have children,
    /// so toolbars, sidebars and main content are all reached within budget.
    fn walk(&mut self, root: &Element) -> Result<()> {
        let mut queue = VecDeque::new();
        queue.push_back((Element(root.0.clone()), Vec::<usize>::new(), false));

        while let Some((element, path, in_list_item)) = queue.pop_front() {
            self.grant.check()?;
            if self.spent() {
                break;
            }
            self.visited += 1;

            let role = element.text("AXRole").unwrap_or_default();
            if is_ignored_subtree(&role) {
                continue;
            }

            let children = element.array("AXChildren");
            let has_children = children.as_ref().is_some_and(|c| !c.is_empty());
            let child_in_list_item = in_list_item || is_list_item_role(&role);

            // Enqueue children before deciding whether to emit this node so
            // skipping a container never hides its descendants.
            if let Some(children) = children {
                if path.len() >= MAX_DEPTH {
                    self.truncated |= !children.is_empty();
                } else {
                    for (index, value) in children.iter().enumerate() {
                        if let Some(child) = element_ref(*value) {
                            let mut child_path = Vec::with_capacity(path.len() + 1);
                            child_path.extend_from_slice(&path);
                            child_path.push(index);
                            queue.push_back((child, child_path, child_in_list_item));
                        }
                    }
                }
            }

            // Pure layout wrappers with children consume neither IPC nor node budget.
            if is_pure_chrome_container(&role) || (is_container_role(&role) && has_children) {
                continue;
            }

            let interactive = is_interactive_role(&role);
            if self.interactive_only && !keep_in_interactive_only(&role, in_list_item) {
                continue;
            }

            let desc = Description {
                role,
                label: element
                    .text("AXTitle")
                    .filter(|text| !text.is_empty())
                    .or_else(|| element.text("AXDescription"))
                    .unwrap_or_default(),
                identifier: element.text("AXIdentifier").unwrap_or_default(),
            };
            let value = element.value(&desc.role);

            // Drop unlabeled, valueless non-interactive leaves (spacers, decorative images).
            if !interactive && desc.label.is_empty() && value.is_none() {
                continue;
            }
            // Drop unlabeled structural rows/cells unless they hold a value.
            if is_list_item_role(&desc.role) && desc.label.is_empty() && value.is_none() {
                continue;
            }

            if let Some(query) = self.query.as_deref()
                && !matches_query(query, &desc, value.as_ref())
            {
                continue;
            }

            let path_text = path
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(".");
            let label: String = desc.label.chars().take(200).collect();
            let mut node = json!({
                "ref": format!("{path_text}:{}", desc.fingerprint()),
                "role": desc.role,
                "label": label,
            });
            if let Some(enabled) = element.boolean("AXEnabled")
                && !enabled
            {
                node["enabled"] = json!(false);
            }
            if let Some(value) = value {
                node["value"] = value;
            }
            if let Some(bounds) = element.bounds()
                && bounds.size.width > 0.0
                && bounds.size.height > 0.0
            {
                node["bounds"] = json!([
                    bounds.origin.x.round() as i64,
                    bounds.origin.y.round() as i64,
                    bounds.size.width.round() as i64,
                    bounds.size.height.round() as i64,
                ]);
            }
            self.nodes.push(node);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_center_rejects_hidden_or_offscreen_controls() {
        let win = CGRect::new(&CGPoint::new(100.0, 50.0), &CGSize::new(800.0, 600.0));
        let inside = CGRect::new(&CGPoint::new(200.0, 150.0), &CGSize::new(100.0, 40.0));
        let center = element_center(inside, win).unwrap();
        assert_eq!((center.x, center.y), (250.0, 170.0));

        let zero = CGRect::new(&CGPoint::new(200.0, 150.0), &CGSize::new(0.0, 40.0));
        assert!(element_center(zero, win).is_err());

        let scrolled_out = CGRect::new(&CGPoint::new(200.0, 800.0), &CGSize::new(100.0, 40.0));
        assert!(element_center(scrolled_out, win).is_err());
    }

    #[test]
    fn query_matching_checks_label_role_identifier_and_value_case_insensitively() {
        let desc = Description {
            role: "AXButton".into(),
            label: "Save Changes".into(),
            identifier: "btn-save".into(),
        };
        assert!(matches_query("save", &desc, None));
        assert!(matches_query("btn-save", &desc, None));
        assert!(matches_query("axbutton", &desc, None));
        assert!(matches_query("42", &desc, Some(&json!(42))));
        assert!(!matches_query("cancel", &desc, Some(&json!("ok"))));
        assert!(is_interactive_role("AXButton"));
        assert!(is_container_role("AXScrollArea"));
        assert!(is_ignored_subtree("AXScrollBar"));
        assert!(keep_in_interactive_only("AXStaticText", true));
        assert!(!keep_in_interactive_only("AXStaticText", false));
        assert!(keep_in_interactive_only("AXButton", false));
    }
}
