//! Seeded draws of a reverb's times: every seed gives another set of delay
//! lengths of the same spread and the same mean, so another room of the
//! same size.
use super::nearest_prime;
use crate::Random;

/// `N` values over `low`–`high`, one in each of `N` equal strata and never
/// at a stratum's edge, in rising order: evenly spread whatever the draw.
pub fn stratified<const N: usize>(random: &mut Random, low: f32, high: f32) -> [f32; N] {
    std::array::from_fn(|index| {
        low + (high - low) * (index as f32 + 0.1 + 0.8 * random.unit()) / N as f32
    })
}

/// The line lengths of a feedback delay network in seconds: spread
/// geometrically over a ratio of 1:2.9, one per stratum, with the mean
/// exactly `mean_seconds`.
pub fn line_seconds<const N: usize>(random: &mut Random, mean_seconds: f32) -> [f32; N] {
    let ratios = stratified::<N>(random, 0.0, 1.0).map(|place| 0.55 * (1.6_f32 / 0.55).powf(place));
    let scale = mean_seconds * N as f32 / ratios.iter().sum::<f32>();
    ratios.map(|ratio| ratio * scale)
}

/// Each time as a prime number of samples (at least 2, at most `longest`),
/// no two alike: a repeated prime moves to the next larger unused one, so
/// the echoes of two lines never coincide.
pub fn prime_lengths<const N: usize>(
    seconds: &[f32; N],
    sample_rate: f32,
    longest: u32,
) -> [u32; N] {
    let mut lengths = [0; N];
    for index in 0..N {
        let wanted = (seconds[index] * sample_rate)
            .round()
            .clamp(2.0, longest.max(2) as f32) as u32;
        let mut length = nearest_prime(wanted);
        while lengths[..index].contains(&length) {
            length += 1;
            while nearest_prime(length) != length {
                length += 1;
            }
        }
        lengths[index] = length;
    }
    lengths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stratified_values_fill_their_strata() {
        for seed in 1..=32 {
            let values = stratified::<8>(&mut Random::stream(seed, 0, 0), 2.0, 10.0);
            for (index, value) in values.iter().enumerate() {
                let low = 2.0 + index as f32;
                assert!(*value >= low + 0.099 && *value <= low + 0.901, "{values:?}");
            }
        }
    }

    #[test]
    fn line_lengths_keep_their_mean_and_spread_for_every_seed() {
        let mut sets = Vec::new();
        for seed in 1..=32 {
            let seconds = line_seconds::<16>(&mut Random::stream(seed, 1, 0), 0.02);
            let mean = seconds.iter().sum::<f32>() / 16.0;
            assert!((mean / 0.02 - 1.0).abs() < 1e-5, "{mean}");
            assert!(seconds.windows(2).all(|pair| pair[0] < pair[1]));
            let spread = seconds[15] / seconds[0];
            assert!((2.2..2.91).contains(&spread), "{spread}");
            sets.push(seconds);
        }
        assert!(sets.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn lengths_are_distinct_primes() {
        let seconds = [0.001_f32; 6];
        let lengths = prime_lengths(&seconds, 48_000.0, 4000);
        assert_eq!(lengths, [47, 53, 59, 61, 67, 71]);
        let lengths = prime_lengths(&[0.5, 0.0], 48_000.0, 1000);
        assert_eq!(lengths, [997, 2]);
    }
}
