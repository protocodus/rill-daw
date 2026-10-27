//! The upstream Bjorklund grouping rule, with a bounded number of steps.
use crate::Rhythm;
use anyhow::{Result, ensure};

/// Spread onsets evenly and rotate the resulting pattern to the left.
/// The first sounding step is accented (`O`), other onsets are `o`, rests `.`.
pub fn euclidean(pulses: u16, steps: u16, rotation: i32) -> Result<Rhythm> {
    ensure!(
        (1..=256).contains(&steps),
        "steps must be between 1 and 256"
    );
    ensure!(pulses <= steps, "pulses cannot exceed steps");
    let mut onsets = if pulses == 0 {
        vec![false; usize::from(steps)]
    } else if pulses == steps {
        vec![true; usize::from(steps)]
    } else {
        let mut groups = vec![vec![true]; usize::from(pulses)];
        let mut remainder = vec![vec![false]; usize::from(steps - pulses)];
        while remainder.len() > 1 {
            let count = groups.len().min(remainder.len());
            for (group, rest) in groups.iter_mut().zip(&remainder).take(count) {
                group.extend(rest);
            }
            remainder = if groups.len() > remainder.len() {
                groups.split_off(count)
            } else {
                remainder.split_off(count)
            };
        }
        groups.extend(remainder);
        groups.into_iter().flatten().collect::<Vec<_>>()
    };
    let shift = rotation.rem_euclid(i32::from(steps)) as usize;
    onsets.rotate_left(shift);
    let mut first = true;
    let pattern = onsets
        .into_iter()
        .map(|onset| {
            if !onset {
                '.'
            } else if first {
                first = false;
                'O'
            } else {
                'o'
            }
        })
        .collect();
    Ok(Rhythm {
        pattern,
        pulses,
        steps,
        rotation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tresillo_and_signed_rotation_follow_upstream_grouping() {
        assert_eq!(euclidean(3, 8, 0).unwrap().pattern, "O..o..o.");
        assert_eq!(euclidean(3, 8, 1).unwrap().pattern, "..O..o.o");
        assert_eq!(euclidean(3, 8, -1).unwrap().pattern, ".O..o..o");
        assert_eq!(euclidean(3, 8, i32::MIN).unwrap().pattern, "O..o..o.");
    }

    #[test]
    fn every_bounded_pattern_preserves_onset_count_and_one_accent() {
        for steps in 1..=256 {
            for pulses in 0..=steps {
                let rhythm = euclidean(pulses, steps, -17).unwrap();
                assert_eq!(rhythm.pattern.len(), usize::from(steps));
                assert_eq!(
                    rhythm.pattern.chars().filter(|c| *c != '.').count(),
                    usize::from(pulses)
                );
                assert_eq!(rhythm.pattern.matches('O').count(), usize::from(pulses > 0));
            }
        }
        assert!(euclidean(0, 0, 0).is_err());
        assert!(euclidean(1, 257, 0).is_err());
        assert!(euclidean(9, 8, 0).is_err());
    }
}
