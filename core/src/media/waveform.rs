//! Waveform peak extraction: PCM samples reduced to a small number of peak
//! buckets that travel inside the post operation, so the player renders
//! before the audio blob has arrived.

/// Number of bars a post's waveform carries (matches the design's player).
pub const WAVEFORM_BUCKETS: usize = 18;

/// Reduces mono PCM samples to `buckets` peak values scaled 0..=255.
///
/// Peaks are normalized against the loudest bucket so quiet recordings still
/// render a readable shape; silence yields all-zero bars.
pub fn peaks(samples: &[f32], buckets: usize) -> Vec<u8> {
    if samples.is_empty() || buckets == 0 {
        return vec![0; buckets];
    }

    let chunk_len = samples.len().div_ceil(buckets);
    let mut raw = Vec::with_capacity(buckets);
    for chunk in samples.chunks(chunk_len) {
        let peak = chunk.iter().fold(0.0_f32, |max, s| max.max(s.abs()));
        raw.push(peak);
    }
    raw.resize(buckets, 0.0);

    let loudest = raw.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= f32::EPSILON {
        return vec![0; buckets];
    }
    raw.iter()
        .map(|peak| ((peak / loudest) * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_normalize_to_the_loudest_bucket() {
        // Two halves: quiet (0.25) then loud (0.5) — normalized to 128/255.
        let mut samples = vec![0.25_f32; 100];
        samples.extend(vec![0.5_f32; 100]);
        let peaks = peaks(&samples, 2);
        assert_eq!(peaks, vec![128, 255]);
    }

    #[test]
    fn silence_renders_flat() {
        assert_eq!(peaks(&vec![0.0; 500], 4), vec![0, 0, 0, 0]);
        assert_eq!(peaks(&[], 3), vec![0, 0, 0]);
    }

    #[test]
    fn short_input_pads_remaining_buckets() {
        let peaks = peaks(&[1.0], 4);
        assert_eq!(peaks[0], 255);
        assert_eq!(&peaks[1..], &[0, 0, 0]);
    }
}
