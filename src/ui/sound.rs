/// Sound engine: procedural 8-bit style sound effects via rodio.
///
/// All sounds are generated as in-memory WAV buffers at init time.
/// Playback is fire-and-forget (non-blocking) via rodio's Sink.
///
/// Compile with `--no-default-features` or without "sound" feature
/// to disable audio entirely (the stub SoundEngine does nothing).

#[cfg(feature = "sound")]
mod inner {
    use std::io::Cursor;
    use std::sync::Arc;

    use rodio::{OutputStream, OutputStreamHandle, Sink};

    const SAMPLE_RATE: u32 = 22050;

    /// Pre-generated WAV buffers for each sound effect.
    pub struct SoundEngine {
        _stream: OutputStream,
        handle: OutputStreamHandle,
        sfx_gold: Arc<Vec<u8>>,
        sfx_dig: Arc<Vec<u8>>,
        sfx_fall: Arc<Vec<u8>>,
        sfx_die: Arc<Vec<u8>>,
        sfx_clear: Arc<Vec<u8>>,
        sfx_all_gold: Arc<Vec<u8>>,
    }

    impl SoundEngine {
        pub fn new() -> Option<Self> {
            let (stream, handle) = OutputStream::try_default().ok()?;

            // ── Generate all sound buffers ──
            let sfx_gold = Arc::new(make_wav(&gen_pickup()));
            let sfx_dig = Arc::new(make_wav(&gen_dig()));
            let sfx_fall = Arc::new(make_wav(&gen_fall()));
            let sfx_die = Arc::new(make_wav(&gen_die()));
            let sfx_clear = Arc::new(make_wav(&gen_clear()));
            let sfx_all_gold = Arc::new(make_wav(&gen_all_gold()));

            Some(SoundEngine {
                _stream: stream,
                handle,
                sfx_gold,
                sfx_dig,
                sfx_fall,
                sfx_die,
                sfx_clear,
                sfx_all_gold,
            })
        }

        fn play(&self, buf: &Arc<Vec<u8>>) {
            if let Ok(sink) = Sink::try_new(&self.handle) {
                let cursor = Cursor::new(buf.as_ref().clone());
                if let Ok(src) = rodio::Decoder::new(cursor) {
                    sink.append(src);
                    sink.detach(); // fire-and-forget
                }
            }
        }

        /// Short ascending blip for intro row reveal
        pub fn play_intro_blip(&self, row: usize, total_rows: usize) {
            // Pitch rises with row number: lower rows = lower pitch
            let ratio = row as f32 / total_rows.max(1) as f32;
            let freq = 300.0 + ratio * 800.0;
            let buf = make_wav(&gen_blip(freq, 0.035, 0.25));
            if let Ok(sink) = Sink::try_new(&self.handle) {
                let cursor = Cursor::new(buf);
                if let Ok(src) = rodio::Decoder::new(cursor) {
                    sink.append(src);
                    sink.detach();
                }
            }
        }

        pub fn play_gold(&self) { self.play(&self.sfx_gold); }
        pub fn play_dig(&self) { self.play(&self.sfx_dig); }
        pub fn play_fall(&self) { self.play(&self.sfx_fall); }
        pub fn play_die(&self) { self.play(&self.sfx_die); }
        pub fn play_clear(&self) { self.play(&self.sfx_clear); }
        pub fn play_all_gold(&self) { self.play(&self.sfx_all_gold); }
    }

    // ════════════════════════════════════════════════════════════
    //  Waveform generators — all produce Vec<f32> mono samples
    // ════════════════════════════════════════════════════════════

    /// Simple sine blip at given frequency and duration
    fn gen_blip(freq: f32, duration: f32, volume: f32) -> Vec<f32> {
        let n = (SAMPLE_RATE as f32 * duration) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 1.0 - (i as f32 / n as f32); // linear fade out
                (t * freq * 2.0 * std::f32::consts::PI).sin() * env * volume
            })
            .collect()
    }

    /// Gold pickup: quick ascending arpeggio C6→E6→G6
    fn gen_pickup() -> Vec<f32> {
        let notes = [1047.0_f32, 1319.0, 1568.0]; // C6, E6, G6
        let note_dur = 0.045;
        let mut samples = Vec::new();
        for &freq in &notes {
            let n = (SAMPLE_RATE as f32 * note_dur) as usize;
            for i in 0..n {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 1.0 - (i as f32 / n as f32).powf(0.5);
                // Square-ish wave (sine + 3rd harmonic) for retro feel
                let wave = (t * freq * 2.0 * std::f32::consts::PI).sin() * 0.7
                    + (t * freq * 3.0 * 2.0 * std::f32::consts::PI).sin() * 0.3;
                samples.push(wave * env * 0.25);
            }
        }
        samples
    }

    /// Dig: short noise burst with descending pitch
    fn gen_dig() -> Vec<f32> {
        let duration = 0.12;
        let n = (SAMPLE_RATE as f32 * duration) as usize;
        let mut rng: u32 = 12345;
        (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let freq = 200.0 + (1.0 - t) * 300.0; // descending
                let ti = i as f32 / SAMPLE_RATE as f32;
                let tone = (ti * freq * 2.0 * std::f32::consts::PI).sin();
                // Simple LCG noise
                rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
                let noise = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let env = (1.0 - t).powf(0.8);
                (tone * 0.4 + noise * 0.6) * env * 0.3
            })
            .collect()
    }

    /// Fall start: short descending whistle
    fn gen_fall() -> Vec<f32> {
        let duration = 0.15;
        let n = (SAMPLE_RATE as f32 * duration) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let freq = 600.0 - t * 400.0; // 600Hz → 200Hz
                let ti = i as f32 / SAMPLE_RATE as f32;
                let env = (1.0 - t).powf(0.6);
                (ti * freq * 2.0 * std::f32::consts::PI).sin() * env * 0.25
            })
            .collect()
    }

    /// Death: sad descending tone
    fn gen_die() -> Vec<f32> {
        let notes = [440.0_f32, 370.0, 311.0, 261.0]; // A4→F#4→Eb4→C4
        let note_dur = 0.12;
        let mut samples = Vec::new();
        for &freq in &notes {
            let n = (SAMPLE_RATE as f32 * note_dur) as usize;
            for i in 0..n {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 1.0 - (i as f32 / n as f32) * 0.3;
                let wave = (t * freq * 2.0 * std::f32::consts::PI).sin();
                samples.push(wave * env * 0.3);
            }
        }
        // Final fade
        let fade_len = samples.len() / 4;
        let total = samples.len();
        for i in (total - fade_len)..total {
            let ratio = (total - i) as f32 / fade_len as f32;
            samples[i] *= ratio;
        }
        samples
    }

    /// Stage clear: victory ascending fanfare
    fn gen_clear() -> Vec<f32> {
        let notes = [523.0_f32, 659.0, 784.0, 1047.0]; // C5→E5→G5→C6
        let note_dur = 0.1;
        let mut samples = Vec::new();
        for &freq in &notes {
            let n = (SAMPLE_RATE as f32 * note_dur) as usize;
            for i in 0..n {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 1.0 - (i as f32 / n as f32) * 0.3;
                let wave = (t * freq * 2.0 * std::f32::consts::PI).sin() * 0.6
                    + (t * freq * 2.0 * 2.0 * std::f32::consts::PI).sin() * 0.3
                    + (t * freq * 3.0 * 2.0 * std::f32::consts::PI).sin() * 0.1;
                samples.push(wave * env * 0.3);
            }
        }
        // Sustain the last note
        let last_freq = 1047.0_f32;
        let n = (SAMPLE_RATE as f32 * 0.25) as usize;
        for i in 0..n {
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = 1.0 - (i as f32 / n as f32);
            let wave = (t * last_freq * 2.0 * std::f32::consts::PI).sin();
            samples.push(wave * env * 0.3);
        }
        samples
    }

    /// All gold collected: triumphant two-note chime
    fn gen_all_gold() -> Vec<f32> {
        let pairs = [(784.0_f32, 0.08), (1047.0, 0.15)]; // G5, C6
        let mut samples = Vec::new();
        for &(freq, dur) in &pairs {
            let n = (SAMPLE_RATE as f32 * dur) as usize;
            for i in 0..n {
                let t = i as f32 / SAMPLE_RATE as f32;
                let env = 1.0 - (i as f32 / n as f32).powf(0.5);
                let wave = (t * freq * 2.0 * std::f32::consts::PI).sin() * 0.7
                    + (t * freq * 2.0 * 2.0 * std::f32::consts::PI).sin() * 0.3;
                samples.push(wave * env * 0.3);
            }
        }
        samples
    }

    // ════════════════════════════════════════════════════════════
    //  WAV encoder — wraps f32 samples into a valid WAV buffer
    // ════════════════════════════════════════════════════════════

    fn make_wav(samples: &[f32]) -> Vec<u8> {
        let num_channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let byte_rate = SAMPLE_RATE * (num_channels as u32) * (bits_per_sample as u32) / 8;
        let block_align = num_channels * bits_per_sample / 8;
        let data_size = samples.len() as u32 * 2; // 16-bit = 2 bytes per sample
        let file_size = 36 + data_size;

        let mut buf = Vec::with_capacity(44 + data_size as usize);

        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        buf.extend_from_slice(&1u16.to_le_bytes());  // PCM format
        buf.extend_from_slice(&num_channels.to_le_bytes());
        buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());

        for &s in samples {
            let clamped = s.max(-1.0).min(1.0);
            let val = (clamped * 32767.0) as i16;
            buf.extend_from_slice(&val.to_le_bytes());
        }

        buf
    }
}

// ════════════════════════════════════════════════════════════
//  Public API — compiles to no-ops when sound feature is off
// ════════════════════════════════════════════════════════════

#[cfg(feature = "sound")]
pub use inner::SoundEngine;

#[cfg(not(feature = "sound"))]
pub struct SoundEngine;

#[cfg(not(feature = "sound"))]
impl SoundEngine {
    pub fn new() -> Option<Self> { Some(SoundEngine) }
    pub fn play_intro_blip(&self, _row: usize, _total: usize) {}
    pub fn play_gold(&self) {}
    pub fn play_dig(&self) {}
    pub fn play_fall(&self) {}
    pub fn play_die(&self) {}
    pub fn play_clear(&self) {}
    pub fn play_all_gold(&self) {}
}
