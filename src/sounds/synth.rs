//! The `cuelume` 0.2.2 engine MonoCode plays its cues with, rendered
//! offline: each recipe's sine and filtered-noise layers under exponential
//! envelopes, the recipe's shimmer (a filtered feedback delay), then the
//! shared output stage (×4 into a limiter). Web Audio does this live; here
//! a cue becomes a WAV once and is replayed.

/// The rate cues are rendered at.
pub const SAMPLE_RATE: u32 = 44_100;

const SOURCE_STOP_PADDING: f32 = 0.05;
const INAUDIBLE_GAIN: f32 = 0.001;
const OUTPUT_GAIN: f32 = 4.0;
/// Where Web Audio's exponential ramps start and end.
const FLOOR: f32 = 0.0001;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    Lowpass,
    Bandpass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// A sine at `frequency`, gliding exponentially to `glide_to` over
    /// `glide_time` (attack + decay when unset); `detune` in cents.
    Tone {
        frequency: f32,
        detune: f32,
        glide_to: Option<f32>,
        glide_time: Option<f32>,
    },
    /// White noise through one biquad.
    Noise {
        filter: Filter,
        frequency: f32,
        q: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub source: Source,
    pub offset: f32,
    pub attack: f32,
    pub decay: f32,
    pub peak: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shimmer {
    pub delay: f32,
    pub feedback: f32,
    pub wet: f32,
    pub lowpass: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub master_gain: f32,
    pub layers: &'static [Layer],
    pub shimmer: Option<Shimmer>,
}

const fn sine(frequency: f32, offset: f32, attack: f32, decay: f32, peak: f32) -> Layer {
    Layer {
        source: Source::Tone {
            frequency,
            detune: 0.0,
            glide_to: None,
            glide_time: None,
        },
        offset,
        attack,
        decay,
        peak,
    }
}

const fn noise(
    filter: Filter,
    frequency: f32,
    q: f32,
    offset: f32,
    attack: f32,
    decay: f32,
    peak: f32,
) -> Layer {
    Layer {
        source: Source::Noise {
            filter,
            frequency,
            q,
        },
        offset,
        attack,
        decay,
        peak,
    }
}

/// A warm, slow-swelling pad from two gently detuned sines.
pub const BLOOM: Recipe = Recipe {
    master_gain: 0.5,
    layers: &[
        sine(528.0, 0.0, 0.06, 0.32, 0.06),
        Layer {
            source: Source::Tone {
                frequency: 528.0,
                detune: 12.0,
                glide_to: None,
                glide_time: None,
            },
            offset: 0.0,
            attack: 0.06,
            decay: 0.34,
            peak: 0.05,
        },
    ],
    shimmer: Some(Shimmer {
        delay: 0.15,
        feedback: 0.2,
        wet: 0.12,
        lowpass: 2500.0,
    }),
};

/// A two-part click-clack, like a switch flipping.
pub const TOGGLE: Recipe = Recipe {
    master_gain: 0.4,
    layers: &[
        noise(Filter::Bandpass, 2200.0, 1.6, 0.0, 0.001, 0.016, 0.12),
        noise(Filter::Bandpass, 3800.0, 1.6, 0.024, 0.001, 0.02, 0.1),
    ],
    shimmer: None,
};

/// A short, warm three-note ascending confirmation.
pub const SUCCESS: Recipe = Recipe {
    master_gain: 0.5,
    layers: &[
        sine(880.0, 0.0, 0.004, 0.09, 0.06),
        sine(1108.73, 0.06, 0.004, 0.1, 0.06),
        sine(1318.51, 0.12, 0.004, 0.18, 0.07),
    ],
    shimmer: Some(Shimmer {
        delay: 0.1,
        feedback: 0.22,
        wet: 0.16,
        lowpass: 4500.0,
    }),
};

/// A fast three-step locator signal.
pub const SCAN: Recipe = Recipe {
    master_gain: 0.4,
    layers: &[
        sine(740.0, 0.0, 0.002, 0.055, 0.05),
        sine(1110.0, 0.045, 0.002, 0.055, 0.045),
        sine(1665.0, 0.09, 0.002, 0.07, 0.04),
    ],
    shimmer: Some(Shimmer {
        delay: 0.065,
        feedback: 0.16,
        wet: 0.1,
        lowpass: 4200.0,
    }),
};

/// A rising harmonic portal with a soft tail.
pub const ARRIVAL: Recipe = Recipe {
    master_gain: 0.44,
    layers: &[
        noise(Filter::Lowpass, 900.0, 0.8, 0.0, 0.05, 0.24, 0.035),
        Layer {
            source: Source::Tone {
                frequency: 220.0,
                detune: 0.0,
                glide_to: Some(440.0),
                glide_time: Some(0.32),
            },
            offset: 0.0,
            attack: 0.04,
            decay: 0.34,
            peak: 0.055,
        },
        sine(659.25, 0.12, 0.045, 0.32, 0.04),
        sine(987.77, 0.19, 0.045, 0.34, 0.032),
    ],
    shimmer: Some(Shimmer {
        delay: 0.16,
        feedback: 0.28,
        wet: 0.18,
        lowpass: 3200.0,
    }),
};

/// `sourceEnd` plus `shimmerTail`: how long the cue rings, in seconds.
fn duration(recipe: &Recipe) -> f32 {
    let end = recipe
        .layers
        .iter()
        .map(|l| l.offset + l.attack + l.decay + SOURCE_STOP_PADDING)
        .fold(0.0, f32::max);
    let tail = recipe.shimmer.map_or(0.0, |s| {
        if s.feedback <= 0.0 {
            0.0
        } else if s.feedback >= 1.0 {
            s.delay
        } else {
            s.delay * (1.0 + (INAUDIBLE_GAIN.ln() / s.feedback.ln()).ceil())
        }
    });
    end + tail
}

/// Web Audio's `exponentialRampToValueAtTime` from `from` to `to`.
fn exp_ramp(from: f32, to: f32, progress: f32) -> f32 {
    from * (to / from).powf(progress.clamp(0.0, 1.0))
}

/// A layer's gain `t` seconds after it starts: up to `peak` over `attack`,
/// down to the floor over `decay`.
fn envelope(layer: &Layer, t: f32) -> f32 {
    if t < layer.attack {
        exp_ramp(FLOOR, layer.peak, t / layer.attack)
    } else {
        exp_ramp(layer.peak, FLOOR, (t - layer.attack) / layer.decay)
    }
}

/// One biquad section (Audio EQ Cookbook, as Web Audio's `BiquadFilterNode`).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(filter: Filter, frequency: f32, q: f32) -> Self {
        let w0 = std::f32::consts::TAU * frequency / SAMPLE_RATE as f32;
        let (sin, cos) = w0.sin_cos();
        let (b, a0, a1, a2) = match filter {
            // Web Audio reads a lowpass Q in dB.
            Filter::Lowpass => {
                let alpha = sin / (2.0 * 10f32.powf(q / 20.0));
                let b1 = 1.0 - cos;
                (
                    [b1 / 2.0, b1, b1 / 2.0],
                    1.0 + alpha,
                    -2.0 * cos,
                    1.0 - alpha,
                )
            }
            Filter::Bandpass => {
                let alpha = sin / (2.0 * q);
                ([alpha, 0.0, -alpha], 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
        };
        Self {
            b: b.map(|v| v / a0),
            a: [a1 / a0, a2 / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let out = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// A small xorshift: noise needs no more, and a fixed seed keeps a cue's
/// render the same every launch.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn render_layer(layer: &Layer, master: &mut [f32], gain: f32, seed: u32) {
    let rate = SAMPLE_RATE as f32;
    let start = (layer.offset * rate) as usize;
    let length = ((layer.attack + layer.decay + SOURCE_STOP_PADDING) * rate) as usize;
    let end = (start + length).min(master.len());
    match layer.source {
        Source::Tone {
            frequency,
            detune,
            glide_to,
            glide_time,
        } => {
            let base = frequency * 2f32.powf(detune / 1200.0);
            let glide = glide_time.unwrap_or(layer.attack + layer.decay);
            let mut phase = 0.0f32;
            for (i, sample) in master[start..end].iter_mut().enumerate() {
                let t = i as f32 / rate;
                let f = glide_to.map_or(base, |to| {
                    exp_ramp(base, to * 2f32.powf(detune / 1200.0), t / glide)
                });
                *sample += phase.sin() * envelope(layer, t) * gain;
                phase = (phase + std::f32::consts::TAU * f / rate) % std::f32::consts::TAU;
            }
        }
        Source::Noise {
            filter,
            frequency,
            q,
        } => {
            let mut biquad = Biquad::new(filter, frequency, q);
            let mut noise = Noise(seed);
            for (i, sample) in master[start..end].iter_mut().enumerate() {
                let t = i as f32 / rate;
                *sample += biquad.process(noise.next()) * envelope(layer, t) * gain;
            }
        }
    }
}

/// `attachShimmer`: the master into a delay whose output is low-passed, fed
/// back at `feedback` and mixed in at `wet`.
fn shimmer(master: &[f32], shimmer: &Shimmer) -> Vec<f32> {
    let delay = ((shimmer.delay * SAMPLE_RATE as f32) as usize).max(1);
    let mut lowpass = Biquad::new(Filter::Lowpass, shimmer.lowpass, 0.0);
    // What enters the delay line: the master plus the fed-back tail.
    let mut line = vec![0.0f32; master.len()];
    let mut out = vec![0.0f32; master.len()];
    for i in 0..master.len() {
        let delayed = if i >= delay { line[i - delay] } else { 0.0 };
        let filtered = lowpass.process(delayed);
        line[i] = master[i] + filtered * shimmer.feedback;
        out[i] = filtered * shimmer.wet;
    }
    out
}

/// The output stage's `DynamicsCompressor` (threshold -8 dB, knee 6 dB,
/// ratio 12, 2 ms attack, 80 ms release), with Web Audio's automatic
/// makeup gain.
fn limit(samples: &mut [f32]) {
    const THRESHOLD: f32 = -8.0;
    const KNEE: f32 = 6.0;
    const RATIO: f32 = 12.0;
    let curve = |db: f32| {
        if db < THRESHOLD - KNEE / 2.0 {
            db
        } else if db <= THRESHOLD + KNEE / 2.0 {
            db + (1.0 / RATIO - 1.0) * (db - THRESHOLD + KNEE / 2.0).powi(2) / (2.0 * KNEE)
        } else {
            THRESHOLD + (db - THRESHOLD) / RATIO
        }
    };
    let makeup = 10f32.powf(-curve(0.0) * 0.6 / 20.0);
    let rate = SAMPLE_RATE as f32;
    let attack = (-1.0 / (0.002 * rate)).exp();
    let release = (-1.0 / (0.08 * rate)).exp();
    let mut level = 0.0f32;
    for sample in samples {
        let input = sample.abs();
        let coeff = if input > level { attack } else { release };
        level = coeff * level + (1.0 - coeff) * input;
        let db = 20.0 * level.max(1e-6).log10();
        let reduction = 10f32.powf((curve(db) - db) / 20.0);
        *sample *= reduction * makeup;
    }
}

/// The cue at `volume` (0–1), as mono samples.
pub fn render(recipe: &Recipe, volume: f32) -> Vec<f32> {
    let length = (duration(recipe) * SAMPLE_RATE as f32).ceil() as usize;
    let mut master = vec![0.0f32; length];
    let gain = recipe.master_gain * volume;
    for (ix, layer) in recipe.layers.iter().enumerate() {
        render_layer(
            layer,
            &mut master,
            gain,
            0x9E37_79B9 ^ (ix as u32 + 1).wrapping_mul(0x85EB_CA6B),
        );
    }
    let wet = recipe.shimmer.map(|s| shimmer(&master, &s));
    let mut out: Vec<f32> = master
        .iter()
        .enumerate()
        .map(|(i, dry)| (dry + wet.as_ref().map_or(0.0, |w| w[i])) * OUTPUT_GAIN)
        .collect();
    limit(&mut out);
    out
}

/// 16-bit mono PCM in a RIFF/WAVE container.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn a_cue_lasts_its_layers_plus_its_shimmer_tail() {
        // success: the last note ends at 0.12 + 0.004 + 0.18 + 0.05; the
        // tail is 0.1 × (1 + ceil(ln 0.001 / ln 0.22)) = 0.6.
        assert!((duration(&SUCCESS) - 0.954).abs() < 1e-3);
        assert!((duration(&TOGGLE) - 0.095).abs() < 1e-3);
    }

    #[test]
    fn every_cue_is_audible_and_never_clips() {
        for recipe in [BLOOM, TOGGLE, SUCCESS, SCAN, ARRIVAL] {
            let samples = render(&recipe, 0.55);
            let peak = peak(&samples);
            assert!(peak > 0.02, "too quiet: {peak}");
            assert!(peak < 1.0, "clips: {peak}");
        }
    }

    #[test]
    fn volume_scales_the_cue() {
        let loud = peak(&render(&SUCCESS, 1.0));
        let soft = peak(&render(&SUCCESS, 0.5));
        assert!(soft < loud * 0.6);
    }

    #[test]
    fn a_cue_fades_out_by_its_end() {
        let samples = render(&SCAN, 1.0);
        let tail = &samples[samples.len() - 100..];
        assert!(peak(tail) < 0.01);
    }

    #[test]
    fn wav_has_a_pcm_header_and_two_bytes_a_sample() {
        let bytes = wav(&[0.0, 0.5, -1.0]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), i16::MAX / 2);
    }
}
