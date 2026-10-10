//! MonoCode `sounds.ts`: the short cues for finished turns, Inbox
//! activity, an available update, switches and Copy. MonoCode plays
//! `cuelume`'s synthesized cues through Web Audio; BenCode renders the same
//! recipes once (`synth.rs`) and plays them with `NSSound`. Whether a cue
//! may play (the switch, the project's mutes) is the app's call
//! (`app/alerts.rs`).

pub mod synth;

/// MonoCode `SoundCue`, with the `cuelume` sound each one plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    TurnFinished,
    InboxUnseen,
    UpdateAvailable,
    Switch,
    Copy,
}

impl Cue {
    fn recipe(self) -> &'static synth::Recipe {
        match self {
            Self::TurnFinished => &synth::SUCCESS,
            Self::InboxUnseen => &synth::BLOOM,
            Self::UpdateAvailable => &synth::ARRIVAL,
            Self::Switch => &synth::TOGGLE,
            Self::Copy => &synth::SCAN,
        }
    }

    const ALL: [Cue; 5] = [
        Self::TurnFinished,
        Self::InboxUnseen,
        Self::UpdateAvailable,
        Self::Switch,
        Self::Copy,
    ];
}

/// MonoCode `SOUNDS_VOLUME`: soft enough to sit in the background while a
/// turn runs in another app.
const VOLUME: f32 = 0.55;

/// The cue as a WAV, rendered on first use.
fn wav(cue: Cue) -> &'static [u8] {
    use std::sync::OnceLock;
    static WAVS: [OnceLock<Vec<u8>>; Cue::ALL.len()] = [const { OnceLock::new() }; Cue::ALL.len()];
    let ix = Cue::ALL.iter().position(|c| *c == cue).unwrap_or(0);
    WAVS[ix].get_or_init(|| synth::wav(&synth::render(cue.recipe(), VOLUME)))
}

/// Plays `cue` now. Call on the main thread.
pub fn play(cue: Cue) {
    platform::play(cue, wav(cue));
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn play(_: super::Cue, _: &[u8]) {}
}

// objc 0.2's `msg_send!` expands to a `cargo-clippy` feature check.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod platform {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};

    use super::Cue;

    thread_local! {
        /// Each cue's sound, kept while it plays (an `NSSound` stops when
        /// released). A cue played again restarts, as one voice.
        static VOICES: RefCell<HashMap<Cue, *mut Object>> = RefCell::new(HashMap::new());
    }

    pub fn play(cue: Cue, wav: &[u8]) {
        VOICES.with(|voices| {
            let mut voices = voices.borrow_mut();
            unsafe {
                let sound = match voices.get(&cue) {
                    Some(sound) => *sound,
                    None => {
                        let data: *mut Object =
                            msg_send![class!(NSData), dataWithBytes: wav.as_ptr() length: wav.len()];
                        let sound: *mut Object = msg_send![class!(NSSound), alloc];
                        let sound: *mut Object = msg_send![sound, initWithData: data];
                        if sound.is_null() {
                            log::warn!("{cue:?}: NSSound could not read the cue");
                            return;
                        }
                        voices.insert(cue, sound);
                        sound
                    }
                };
                let _: BOOL = msg_send![sound, stop];
                let played: BOOL = msg_send![sound, play];
                if played == NO {
                    log::debug!("{cue:?}: NSSound did not play");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_renders_a_wav() {
        for cue in Cue::ALL {
            let bytes = wav(cue);
            assert_eq!(&bytes[..4], b"RIFF", "{cue:?}");
            assert!(bytes.len() > 44 + 1000, "{cue:?}");
        }
    }
}
