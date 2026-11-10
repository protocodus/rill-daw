use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// Frames kept for scopes and spectra.
pub const TELEMETRY_FRAMES: usize = 4096;
/// Device-defined live values (effective cutoff, gain reduction, LFO phase…).
/// Each device documents which index means what.
pub const TELEMETRY_VALUES: usize = 8;

/// Lock-free path from the audio instance to its panel. The audio thread
/// writes, any thread reads. A torn read costs one odd frame in a drawing.
pub struct Telemetry {
    values: [AtomicU32; TELEMETRY_VALUES],
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    input: Ring,
    output: Ring,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| AtomicU32::new(0)),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            input: Ring::default(),
            output: Ring::default(),
        }
    }
}

impl Telemetry {
    pub fn set_value(&self, index: usize, value: f32) {
        if let Some(slot) = self.values.get(index) {
            slot.store(value.to_bits(), Relaxed);
        }
    }

    pub fn value(&self, index: usize) -> f32 {
        self.values
            .get(index)
            .map_or(0.0, |slot| f32::from_bits(slot.load(Relaxed)))
    }

    /// Records one processed block: mono input and output into the rings and
    /// their peaks. Call with the input before processing for effects.
    pub fn record_input(&self, left: &[f32], right: &[f32]) {
        self.input_peak
            .store(self.input.push(left, right).to_bits(), Relaxed);
    }

    pub fn record_output(&self, left: &[f32], right: &[f32]) {
        self.output_peak
            .store(self.output.push(left, right).to_bits(), Relaxed);
    }

    /// Peak of the last recorded block.
    pub fn input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak.load(Relaxed))
    }

    pub fn output_peak(&self) -> f32 {
        f32::from_bits(self.output_peak.load(Relaxed))
    }

    /// Copies the most recent frames, oldest first, into `out` (at most
    /// [`TELEMETRY_FRAMES`]).
    pub fn read_input(&self, out: &mut [f32]) {
        self.input.read(out);
    }

    pub fn read_output(&self, out: &mut [f32]) {
        self.output.read(out);
    }
}

struct Ring {
    frames: Box<[AtomicU32]>,
    cursor: AtomicUsize,
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            frames: (0..TELEMETRY_FRAMES).map(|_| AtomicU32::new(0)).collect(),
            cursor: AtomicUsize::new(0),
        }
    }
}

impl Ring {
    fn push(&self, left: &[f32], right: &[f32]) -> f32 {
        let mut cursor = self.cursor.load(Relaxed);
        let mut peak = 0.0_f32;
        for (left, right) in left.iter().zip(right) {
            peak = peak.max(left.abs()).max(right.abs());
            self.frames[cursor].store((0.5 * (left + right)).to_bits(), Relaxed);
            cursor = (cursor + 1) % TELEMETRY_FRAMES;
        }
        self.cursor.store(cursor, Relaxed);
        peak
    }

    fn read(&self, out: &mut [f32]) {
        let count = out.len().min(TELEMETRY_FRAMES);
        let end = self.cursor.load(Relaxed);
        let start = (end + TELEMETRY_FRAMES - count) % TELEMETRY_FRAMES;
        for (index, value) in out[..count].iter_mut().enumerate() {
            *value = f32::from_bits(self.frames[(start + index) % TELEMETRY_FRAMES].load(Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_reads_latest_frames_in_order() {
        let telemetry = Telemetry::default();
        let block: Vec<f32> = (0..5000).map(|index| index as f32).collect();
        telemetry.record_output(&block, &block);
        let mut out = [0.0; 3];
        telemetry.read_output(&mut out);
        assert_eq!(out, [4997.0, 4998.0, 4999.0]);
        assert_eq!(telemetry.output_peak(), 4999.0);
        telemetry.set_value(0, 440.0);
        assert_eq!(telemetry.value(0), 440.0);
        assert_eq!(telemetry.value(99), 0.0);
    }
}
