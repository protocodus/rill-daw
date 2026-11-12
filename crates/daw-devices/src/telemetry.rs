use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// Frames kept for scopes and spectra: a third of a second at 48 kHz, so a
/// spectrum resolves 6 Hz and still has a dozen bins below 100 Hz.
pub const TELEMETRY_FRAMES: usize = 16_384;
/// Device-defined live values (effective cutoff, gain reduction, LFO phase…).
/// Each device documents which index means what.
pub const TELEMETRY_VALUES: usize = 16;

/// Columns of level history: 8 s at [`HISTORY_RATE`].
pub const HISTORY_COLUMNS: usize = 512;
/// History columns per second, whatever the sample rate or block size.
pub const HISTORY_RATE: f32 = 64.0;

/// Lock-free path from the audio instance to its panel. The audio thread
/// writes, any thread reads. A torn read costs one odd frame in a drawing.
pub struct Telemetry {
    values: [AtomicU32; TELEMETRY_VALUES],
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    input: Ring,
    output: Ring,
    /// Three device-defined values per column, such as input peak, output
    /// peak and gain reduction. Written by the audio side at a fixed rate,
    /// so a drawing of it does not depend on the UI frame rate.
    history: Box<[[AtomicU32; 3]]>,
    history_count: AtomicUsize,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| AtomicU32::new(0)),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            input: Ring::default(),
            output: Ring::default(),
            history: (0..HISTORY_COLUMNS)
                .map(|_| std::array::from_fn(|_| AtomicU32::new(0)))
                .collect(),
            history_count: AtomicUsize::new(0),
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

    /// Where the next input frame goes in its ring. It moves with every
    /// recorded block, so a reader can tell a ring that is being written
    /// from one that holds what last played.
    pub fn input_position(&self) -> usize {
        self.input.cursor.load(Relaxed)
    }

    pub fn output_position(&self) -> usize {
        self.output.cursor.load(Relaxed)
    }

    /// Appends one history column; see [`HistoryMeter`].
    pub fn push_history(&self, column: [f32; 3]) {
        let count = self.history_count.load(Relaxed);
        for (slot, value) in self.history[count % HISTORY_COLUMNS].iter().zip(column) {
            slot.store(value.to_bits(), Relaxed);
        }
        self.history_count.store(count.wrapping_add(1), Relaxed);
    }

    /// Copies the most recent columns, oldest first, into `out` (at most
    /// [`HISTORY_COLUMNS`]). Columns never written are zero.
    pub fn read_history(&self, out: &mut [[f32; 3]]) {
        let count = out.len().min(HISTORY_COLUMNS);
        let end = self.history_count.load(Relaxed);
        for (index, column) in out[..count].iter_mut().enumerate() {
            let position = end.wrapping_add(HISTORY_COLUMNS - count + index) % HISTORY_COLUMNS;
            *column = std::array::from_fn(|value| {
                f32::from_bits(self.history[position][value].load(Relaxed))
            });
        }
    }

    /// Columns written so far (wrapping): moves when there is news to draw.
    pub fn history_count(&self) -> usize {
        self.history_count.load(Relaxed)
    }
}

/// Collects the largest of three values per history column and pushes each
/// finished column: call [`Self::add`] once per sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct HistoryMeter {
    column: [f32; 3],
    period: u32,
    filled: u32,
}

impl HistoryMeter {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.period = ((sample_rate / HISTORY_RATE).round() as u32).max(1);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.column = [0.0; 3];
        self.filled = 0;
    }

    #[inline]
    pub fn add(&mut self, a: f32, b: f32, c: f32, telemetry: &Telemetry) {
        for (slot, value) in self.column.iter_mut().zip([a, b, c]) {
            *slot = slot.max(value);
        }
        self.filled += 1;
        if self.filled >= self.period {
            telemetry.push_history(self.column);
            self.reset();
        }
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
        let block: Vec<f32> = (0..20_000).map(|index| index as f32).collect();
        telemetry.record_output(&block, &block);
        let mut out = [0.0; 3];
        telemetry.read_output(&mut out);
        assert_eq!(out, [19_997.0, 19_998.0, 19_999.0]);
        assert_eq!(telemetry.output_peak(), 19_999.0);
        assert_eq!(telemetry.output_position(), 20_000 % TELEMETRY_FRAMES);
        assert_eq!(telemetry.input_position(), 0);
        telemetry.set_value(0, 440.0);
        assert_eq!(telemetry.value(0), 440.0);
        assert_eq!(telemetry.value(99), 0.0);
    }

    #[test]
    fn history_keeps_the_latest_columns_at_a_fixed_rate() {
        let telemetry = Telemetry::default();
        let mut out = [[1.0; 3]; 4];
        telemetry.read_history(&mut out);
        assert_eq!(out, [[0.0; 3]; 4]);
        let mut meter = HistoryMeter::default();
        meter.prepare(48_000.0);
        // 750 samples per column; each column holds the maxima of its span.
        for n in 0..750 * 600 {
            let column = (n / 750) as f32;
            let last = n % 750 == 749;
            meter.add(
                if last { column } else { 0.0 },
                if n % 750 == 0 { 2.0 * column } else { 0.0 },
                -1.0,
                &telemetry,
            );
        }
        assert_eq!(telemetry.history_count(), 600);
        telemetry.read_history(&mut out);
        assert_eq!(out.map(|column| column[0]), [596.0, 597.0, 598.0, 599.0]);
        assert_eq!(out[3], [599.0, 1198.0, 0.0]);
        let mut all = vec![[0.0; 3]; HISTORY_COLUMNS + 10];
        telemetry.read_history(&mut all);
        assert_eq!(all[0][0], 88.0);
        assert_eq!(all[HISTORY_COLUMNS - 1][0], 599.0);
        assert_eq!(all[HISTORY_COLUMNS], [0.0; 3]);
    }
}
