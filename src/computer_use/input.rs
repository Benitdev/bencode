//! Quartz mouse and keyboard input on the logged-in macOS desktop.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use core_graphics::event::{
    CGEvent, CGEventFlags as Flags, CGEventTapLocation, CGEventType as EventType, CGMouseButton,
    EventField, KeyCode, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use serde::Deserialize;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation as _;

use super::{Grant, screen::Frames};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

pub fn request_permission() {
    // SAFETY: prompts for the calling app's event-posting permission.
    unsafe { CGRequestPostEventAccess() };
}

fn event_source() -> Result<CGEventSource> {
    // SAFETY: checks permission without posting an event.
    if !unsafe { CGPreflightPostEventAccess() } {
        bail!(
            "Grant BenCode Accessibility access in macOS Privacy & Security. Use Settings → Integrations → Computer Use to request it, then retry."
        );
    }
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| anyhow!("Could not initialize macOS input."))
}

fn post(event: &CGEvent) {
    event.post(CGEventTapLocation::HID);
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn events(self) -> (CGMouseButton, EventType, EventType, EventType) {
        match self {
            Self::Left => (
                CGMouseButton::Left,
                EventType::LeftMouseDown,
                EventType::LeftMouseUp,
                EventType::LeftMouseDragged,
            ),
            Self::Right => (
                CGMouseButton::Right,
                EventType::RightMouseDown,
                EventType::RightMouseUp,
                EventType::RightMouseDragged,
            ),
            Self::Middle => (
                CGMouseButton::Center,
                EventType::OtherMouseDown,
                EventType::OtherMouseUp,
                EventType::OtherMouseDragged,
            ),
        }
    }
}

/// Synthetic state belongs to the desktop grant. Physical input the user
/// was already holding is never included in this list.
#[derive(Default)]
pub struct Held {
    keys: Vec<u16>,
    buttons: Vec<MouseButton>,
    last_pointer: Option<CGPoint>,
}

impl Held {
    fn flags(&self, except: Option<u16>) -> Flags {
        self.keys
            .iter()
            .copied()
            .filter(|code| Some(*code) != except)
            .filter_map(modifier_flag)
            .fold(Flags::empty(), |all, flag| all | flag)
    }

    pub fn release_all(&mut self) {
        if self.buttons.is_empty() && self.keys.is_empty() {
            return;
        }
        let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
            log::error!("Could not release Computer Use input: Quartz event source unavailable");
            return;
        };
        let position = pointer(&source).ok().or(self.last_pointer);
        let flags = self.flags(None);
        for button in self.buttons.drain(..).rev() {
            let Some(position) = position else {
                continue;
            };
            let (_, _, up, _) = button.events();
            match mouse_event(&source, button, up, position, 1, flags) {
                Ok(event) => post(&event),
                Err(err) => log::error!("Could not release Computer Use mouse button: {err:#}"),
            }
        }
        while let Some(code) = self.keys.pop() {
            match keyboard_event(&source, code, false, self.flags(None), None) {
                Ok(event) => post(&event),
                Err(err) => log::error!("Could not release Computer Use key: {err:#}"),
            }
        }
        self.last_pointer = None;
    }
}

fn mouse_event(
    source: &CGEventSource,
    button: MouseButton,
    event_type: EventType,
    position: CGPoint,
    count: i64,
    flags: Flags,
) -> Result<CGEvent> {
    let event = CGEvent::new_mouse_event(source.clone(), event_type, position, button.events().0)
        .map_err(|()| anyhow!("Could not create a macOS mouse event."))?;
    event.set_flags(flags);
    event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
    Ok(event)
}

fn pointer(source: &CGEventSource) -> Result<CGPoint> {
    CGEvent::new(source.clone())
        .map(|event| event.location())
        .map_err(|()| anyhow!("Could not read the current pointer position."))
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum MouseAction {
    Move,
    Click {
        #[serde(default)]
        button: MouseButton,
        count: Option<i64>,
    },
    Down {
        #[serde(default)]
        button: MouseButton,
    },
    Up {
        #[serde(default)]
        button: MouseButton,
    },
    Drag {
        #[serde(default)]
        button: MouseButton,
        to_screenshot_id: Option<u64>,
        to_x: f64,
        to_y: f64,
    },
    Scroll {
        #[serde(default)]
        delta_x: i32,
        #[serde(default)]
        delta_y: i32,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mouse {
    screenshot_id: Option<u64>,
    x: Option<f64>,
    y: Option<f64>,
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(flatten)]
    action: MouseAction,
}

impl Mouse {
    pub fn parse(args: Value) -> Result<Self> {
        let mouse: Self = serde_json::from_value(args)?;
        match (mouse.screenshot_id, mouse.x, mouse.y) {
            (Some(_), Some(_), Some(_)) => {}
            (None, None, None) if matches!(mouse.action, MouseAction::Up { .. }) => {}
            _ => bail!("Supply screenshotId, x and y together. Only up can omit the position."),
        }
        match mouse.action {
            MouseAction::Click {
                count: Some(count), ..
            } if !(1..=3).contains(&count) => {
                bail!("Click count must be between 1 and 3.");
            }
            MouseAction::Scroll { delta_x, delta_y }
                if !(-2000..=2000).contains(&delta_x)
                    || !(-2000..=2000).contains(&delta_y)
                    || (delta_x == 0 && delta_y == 0) =>
            {
                bail!("Scroll needs a nonzero delta between -2000 and 2000.");
            }
            _ => {}
        }
        flags(mouse.modifiers.iter().map(String::as_str))?;
        Ok(mouse)
    }

    pub fn perform(self, turn: u64, frames: &Frames, held: &mut Held, grant: &Grant) -> Result<()> {
        if matches!(&self.action, MouseAction::Up { button } if !held.buttons.contains(button)) {
            return Ok(());
        }
        let source = event_source()?;
        let position = match self.screenshot_id.zip(self.x.zip(self.y)) {
            Some((id, (x, y))) => frames.point(turn, id, x, y)?,
            None => pointer(&source)?,
        };
        let flags = flags(self.modifiers.iter().map(String::as_str))? | held.flags(None);
        let mouse =
            |button, kind, point, count| mouse_event(&source, button, kind, point, count, flags);
        let move_event = |where_to| match held.buttons.last().copied() {
            Some(button) => mouse(button, button.events().3, where_to, 1),
            None => mouse(MouseButton::Left, EventType::MouseMoved, where_to, 0),
        };

        grant.check()?;
        match self.action {
            MouseAction::Move => post(&move_event(position)?),
            MouseAction::Down { button } => {
                if !held.buttons.contains(&button) {
                    let event = mouse(button, button.events().1, position, 1)?;
                    held.buttons.push(button);
                    post(&event);
                }
            }
            MouseAction::Up { button } => {
                post(&mouse(button, button.events().2, position, 1)?);
                held.buttons.retain(|held| *held != button);
            }
            MouseAction::Click { button, count } => {
                check_button_free(&held.buttons, button)?;
                let (_, down, up, _) = button.events();
                let count = count.unwrap_or(1);
                for click in 1..=count {
                    grant.check()?;
                    let press = mouse(button, down, position, click)?;
                    let release = mouse(button, up, position, click)?;
                    post(&press);
                    post(&release);
                    if click < count {
                        std::thread::sleep(Duration::from_millis(55));
                    }
                }
            }
            MouseAction::Drag {
                button,
                to_screenshot_id,
                to_x,
                to_y,
            } => {
                check_button_free(&held.buttons, button)?;
                let id = to_screenshot_id
                    .or(self.screenshot_id)
                    .expect("validated drag source");
                let target = frames.point(turn, id, to_x, to_y)?;
                drag(position, target, button, mouse, grant)?;
                held.last_pointer = Some(target);
                return Ok(());
            }
            MouseAction::Scroll { delta_x, delta_y } => {
                post(&move_event(position)?);
                // Quartz puts the scroll event at the newly moved pointer.
                std::thread::sleep(Duration::from_millis(25));
                grant.check()?;
                let wheel = CGEvent::new_scroll_event(
                    source.clone(),
                    ScrollEventUnit::PIXEL,
                    2,
                    -delta_y,
                    -delta_x,
                    0,
                )
                .map_err(|()| anyhow!("Could not create a macOS scroll event."))?;
                wheel.set_flags(flags);
                post(&wheel);
            }
        }
        held.last_pointer = Some(position);
        Ok(())
    }
}

fn check_button_free(buttons: &[MouseButton], button: MouseButton) -> Result<()> {
    if buttons.contains(&button) {
        bail!("That mouse button is already held. Use move and up to finish the gesture.");
    }
    Ok(())
}

fn drag(
    origin: CGPoint,
    target: CGPoint,
    button: MouseButton,
    mouse: impl Fn(MouseButton, EventType, CGPoint, i64) -> Result<CGEvent>,
    grant: &Grant,
) -> Result<()> {
    let (_, down, up, dragged) = button.events();
    let press = mouse(button, down, origin, 1)?;
    let release = mouse(button, up, origin, 1)?;
    // Prepare the release at every point before pressing so an interrupted
    // drag always releases at the last position reached.
    let steps = (1..=16)
        .map(|step| {
            let progress = f64::from(step) / 16.0;
            let point = CGPoint::new(
                origin.x + (target.x - origin.x) * progress,
                origin.y + (target.y - origin.y) * progress,
            );
            Ok((
                mouse(button, dragged, point, 1)?,
                mouse(button, up, point, 1)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    grant.check()?;
    post(&press);
    let mut last_release = &release;
    for (movement, release) in &steps {
        if grant.check().is_err() {
            break;
        }
        post(movement);
        last_release = release;
        std::thread::sleep(Duration::from_millis(15));
    }
    post(last_release);
    grant.check()
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    #[default]
    Press,
    Down,
    Up,
    Repeat,
}

pub struct Chord {
    code: u16,
    flags: Flags,
    action: KeyAction,
}

impl Chord {
    pub fn parse(key: &str, action: KeyAction) -> Result<Self> {
        if key.len() > 80 {
            bail!("A key combination must be at most 80 bytes.");
        }
        let mut parts = key.split('+');
        let key = parts
            .next_back()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let code = key_code(&key)
            .ok_or_else(|| anyhow!("Unknown key {key:?}. Try Return, ArrowDown or Cmd+Space."))?;
        let flags = flags(parts.map(str::trim))?;
        if action != KeyAction::Press && !flags.is_empty() {
            bail!("Hold, release and repeat take one key. Hold modifier keys separately.");
        }
        Ok(Self {
            code,
            flags,
            action,
        })
    }

    pub fn perform(self, held: &mut Held, grant: &Grant) -> Result<()> {
        let pressed = held.keys.contains(&self.code);
        if (pressed && self.action == KeyAction::Down) || (!pressed && self.action == KeyAction::Up)
        {
            return Ok(());
        }
        if pressed && self.action == KeyAction::Press {
            bail!("This key is already held. Use repeat or release it before pressing it again.");
        }
        if self.action == KeyAction::Repeat && (!pressed || modifier_flag(self.code).is_some()) {
            bail!("Repeat needs a held non-modifier key.");
        }
        let source = event_source()?;
        let flags = held.flags(None) | self.flags;
        grant.check()?;
        match self.action {
            KeyAction::Press => key_press(&source, self.code, flags, None)?,
            KeyAction::Down => {
                let event = keyboard_event(&source, self.code, true, flags, None)?;
                held.keys.push(self.code);
                post(&event);
            }
            KeyAction::Up => {
                post(&keyboard_event(
                    &source,
                    self.code,
                    false,
                    held.flags(Some(self.code)),
                    None,
                )?);
                held.keys.retain(|code| *code != self.code);
            }
            KeyAction::Repeat => {
                let event = keyboard_event(&source, self.code, true, flags, None)?;
                event.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, 1);
                post(&event);
            }
        }
        Ok(())
    }
}

fn modifier_flag(code: u16) -> Option<Flags> {
    Some(match code {
        KeyCode::COMMAND | KeyCode::RIGHT_COMMAND => Flags::CGEventFlagCommand,
        KeyCode::SHIFT | KeyCode::RIGHT_SHIFT => Flags::CGEventFlagShift,
        KeyCode::OPTION | KeyCode::RIGHT_OPTION => Flags::CGEventFlagAlternate,
        KeyCode::CONTROL | KeyCode::RIGHT_CONTROL => Flags::CGEventFlagControl,
        KeyCode::FUNCTION => Flags::CGEventFlagSecondaryFn,
        _ => return None,
    })
}

fn flags<'a>(mut modifiers: impl Iterator<Item = &'a str>) -> Result<Flags> {
    modifiers.try_fold(Flags::empty(), |flags, modifier| {
        let flag = match modifier.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" => Flags::CGEventFlagCommand,
            "ctrl" | "control" => Flags::CGEventFlagControl,
            "opt" | "option" | "alt" => Flags::CGEventFlagAlternate,
            "shift" => Flags::CGEventFlagShift,
            "fn" => Flags::CGEventFlagSecondaryFn,
            _ => bail!("Unknown modifier {modifier:?}."),
        };
        if flags.contains(flag) {
            bail!("Duplicate modifier {modifier:?}.");
        }
        Ok(flags | flag)
    })
}

fn key_code(key: &str) -> Option<u16> {
    // ANSI virtual key codes (the physical keys, independent of input source).
    const ANSI: [&str; 52] = [
        "a", "s", "d", "f", "h", "g", "z", "x", "c", "v", "", "b", "q", "w", "e", "r", "y", "t",
        "1", "2", "3", "4", "6", "5", "=", "9", "7", "-", "8", "0", "]", "o", "u", "[", "i", "p",
        "", "l", "j", "'", "k", ";", "\\", ",", "/", "n", "m", ".", "", "", "`", "",
    ];
    const FUNCTION: [u16; 20] = [
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
        KeyCode::F13,
        KeyCode::F14,
        KeyCode::F15,
        KeyCode::F16,
        KeyCode::F17,
        KeyCode::F18,
        KeyCode::F19,
        KeyCode::F20,
    ];
    Some(match key {
        "cmd" | "command" | "meta" => KeyCode::COMMAND,
        "shift" => KeyCode::SHIFT,
        "alt" | "option" | "opt" => KeyCode::OPTION,
        "ctrl" | "control" => KeyCode::CONTROL,
        "fn" => KeyCode::FUNCTION,
        "rightcmd" | "rightcommand" => KeyCode::RIGHT_COMMAND,
        "rightshift" => KeyCode::RIGHT_SHIFT,
        "rightalt" | "rightoption" => KeyCode::RIGHT_OPTION,
        "rightctrl" | "rightcontrol" => KeyCode::RIGHT_CONTROL,
        "return" | "enter" => KeyCode::RETURN,
        "tab" => KeyCode::TAB,
        "space" => KeyCode::SPACE,
        "escape" | "esc" => KeyCode::ESCAPE,
        "delete" | "backspace" => KeyCode::DELETE,
        "forwarddelete" => KeyCode::FORWARD_DELETE,
        "home" => KeyCode::HOME,
        "end" => KeyCode::END,
        "pageup" => KeyCode::PAGE_UP,
        "pagedown" => KeyCode::PAGE_DOWN,
        "arrowleft" | "left" => KeyCode::LEFT_ARROW,
        "arrowright" | "right" => KeyCode::RIGHT_ARROW,
        "arrowup" | "up" => KeyCode::UP_ARROW,
        "arrowdown" | "down" => KeyCode::DOWN_ARROW,
        "equal" | "plus" => 0x18,
        "minus" => 0x1b,
        key if key.len() > 1 && key.starts_with('f') => {
            let n: usize = key[1..].parse().ok()?;
            *FUNCTION.get(n.checked_sub(1)?)?
        }
        key if key.len() == 1 => ANSI.iter().position(|candidate| *candidate == key)? as u16,
        _ => return None,
    })
}

fn keyboard_event(
    source: &CGEventSource,
    code: u16,
    down: bool,
    mut flags: Flags,
    text: Option<&str>,
) -> Result<CGEvent> {
    let event = CGEvent::new_keyboard_event(source.clone(), code, down)
        .map_err(|()| anyhow!("Could not create a macOS keyboard event."))?;
    if let Some(flag) = modifier_flag(code) {
        event.set_type(EventType::FlagsChanged);
        if down {
            flags |= flag;
        }
    }
    event.set_flags(flags);
    if let Some(text) = text {
        event.set_string(text);
    }
    Ok(event)
}

fn key_press(source: &CGEventSource, code: u16, flags: Flags, text: Option<&str>) -> Result<()> {
    let down = keyboard_event(source, code, true, flags, text)?;
    let up = keyboard_event(source, code, false, flags, text)?;
    post(&down);
    post(&up);
    Ok(())
}

pub fn type_text(text: &str, held: &Held, grant: &Grant) -> Result<()> {
    if !held.keys.is_empty() {
        bail!("Release held keys before typing text. Use computer_key for modified keystrokes.");
    }
    let source = event_source()?;
    let mut chunk = String::new();
    let mut units = 0;
    for grapheme in text.graphemes(true) {
        let special = match grapheme {
            "\r\n" | "\r" | "\n" => Some(KeyCode::RETURN),
            "\t" => Some(KeyCode::TAB),
            _ => None,
        };
        let size = grapheme.encode_utf16().count();
        if special.is_some() || units + size > 20 {
            flush(&source, grant, &mut chunk)?;
            units = 0;
        }
        if let Some(code) = special {
            grant.check()?;
            key_press(&source, code, Flags::empty(), None)?;
        } else if size > 20 {
            // A single extended grapheme may be longer than an event can carry.
            for ch in grapheme.chars() {
                grant.check()?;
                key_press(
                    &source,
                    0,
                    Flags::empty(),
                    Some(ch.encode_utf8(&mut [0; 4])),
                )?;
            }
        } else {
            chunk.push_str(grapheme);
            units += size;
        }
    }
    flush(&source, grant, &mut chunk)
}

fn flush(source: &CGEventSource, grant: &Grant, chunk: &mut String) -> Result<()> {
    if !chunk.is_empty() {
        grant.check()?;
        key_press(source, 0, Flags::empty(), Some(chunk))?;
        chunk.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn translates_key_chords_without_accepting_invalid_modifiers() {
        let chord = Chord::parse("Cmd+Shift+P", KeyAction::Press).unwrap();
        assert_eq!(chord.code, 0x23);
        assert_eq!(
            chord.flags,
            Flags::CGEventFlagCommand | Flags::CGEventFlagShift
        );
        assert_eq!(
            Chord::parse("F12", KeyAction::Press).unwrap().code,
            KeyCode::F12
        );
        assert!(Chord::parse("Cmd+Cmd+P", KeyAction::Press).is_err());
        assert!(Chord::parse("Fn+", KeyAction::Press).is_err());
    }

    #[test]
    fn rejects_invalid_gestures_before_requesting_desktop_access() {
        assert!(
            Mouse::parse(json!({
                "action":"drag", "screenshotId":1, "x":4, "y":5, "toX":6, "toY":7
            }))
            .is_ok()
        );
        assert!(
            Mouse::parse(json!({
                "action":"click", "screenshotId":1, "x":4, "y":5, "count":4
            }))
            .is_err()
        );
        assert!(
            Mouse::parse(json!({
                "action":"scroll", "screenshotId":1, "x":4, "y":5
            }))
            .is_err()
        );
    }

    #[test]
    fn accepts_cross_display_drags_and_releasing_without_coordinates() {
        let drag = Mouse::parse(json!({
            "action": "drag", "screenshotId": 1, "x": 4, "y": 5,
            "toScreenshotId": 2, "toX": 40, "toY": 50
        }))
        .unwrap();
        assert!(matches!(
            drag.action,
            MouseAction::Drag {
                to_screenshot_id: Some(2),
                ..
            }
        ));
        assert!(Mouse::parse(json!({"action": "up", "button": "right"})).is_ok());
        assert!(Mouse::parse(json!({"action": "down", "button": "right"})).is_err());
    }

    #[test]
    fn held_modifiers_are_reflected_in_quartz_events_and_released_individually() {
        assert!(Chord::parse("Cmd+P", KeyAction::Down).is_err());
        assert_eq!(
            Chord::parse("Shift", KeyAction::Down).unwrap().code,
            KeyCode::SHIFT
        );
        let held = Held {
            keys: vec![KeyCode::SHIFT, KeyCode::RIGHT_SHIFT, KeyCode::COMMAND],
            ..Held::default()
        };
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();
        let shift_up = keyboard_event(
            &source,
            KeyCode::SHIFT,
            false,
            held.flags(Some(KeyCode::SHIFT)),
            None,
        )
        .unwrap();
        assert!(matches!(shift_up.get_type(), EventType::FlagsChanged));
        assert!(shift_up.get_flags().contains(Flags::CGEventFlagShift));
        let command_up = keyboard_event(
            &source,
            KeyCode::COMMAND,
            false,
            held.flags(Some(KeyCode::COMMAND)),
            None,
        )
        .unwrap();
        assert!(!command_up.get_flags().contains(Flags::CGEventFlagCommand));
    }
}
