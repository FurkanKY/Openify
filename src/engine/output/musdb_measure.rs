//! Offline karaoke measurement on decoded MUSDB18 clips.
//!
//! The mask is computed once on the mix and then applied to each isolated
//! stem, so the table says how much each instrument group is reduced by
//! exactly the gain the listener would hear. This is a local tool: the test is
//! `#[ignore]`d, so neither `cargo test` nor CI runs it, and it returns early
//! when the clip folder is missing.
//!
//! ```text
//! OPENIFY_MUSDB7=<folder with one sub-folder per song> \
//!     cargo test --release musdb_measure -- --ignored --nocapture
//! ```
//!
//! Each song folder holds 32-bit float stereo 44.1 kHz `mix`, `vocals`,
//! `bass`, `drums` and `other` WAV files.

use super::karaoke::{Karaoke, KaraokeCtl, TraceFrame};
use super::{BLOCK, RATE};
use std::{
    fs,
    path::{Path, PathBuf},
};

const DEFAULT_ROOT: &str = r"C:\Users\furka\Desktop\Projects\Openify\target\musdb7\wav";
const STEMS: [&str; 4] = ["vocals", "bass", "drums", "other"];

/// Reads a 32-bit float stereo WAV as interleaved samples.
fn read_wav(path: &Path) -> Vec<f32> {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&bytes[..4], b"RIFF", "{}", path.display());
    assert_eq!(&bytes[8..12], b"WAVE", "{}", path.display());
    let format = u16::from_le_bytes([bytes[20], bytes[21]]);
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    assert_eq!((format, channels), (3, 2), "{}", path.display());
    let data = bytes
        .windows(4)
        .position(|w| w == b"data")
        .unwrap_or_else(|| panic!("{}: no data chunk", path.display()));
    bytes[data + 8..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// Runs the real-time path over the mix; returns the output and the mask of
/// every analysis frame.
fn run_mix(mix: &[f32], amount: f32) -> (Vec<f32>, Vec<TraceFrame>) {
    let ctl = KaraokeCtl::new();
    ctl.set(true, amount);
    let mut karaoke = Karaoke::new();
    karaoke.record_mask_trace();
    karaoke.poll(&ctl);
    let mut out = mix.to_vec();
    let frames = out.len() / 2;
    for start in (0..frames).step_by(BLOCK) {
        let n = (frames - start).min(BLOCK);
        karaoke.process(&mut out[start * 2..(start + n) * 2], n);
    }
    (out, karaoke.take_mask_trace())
}

/// Energy change in dB, ignoring the first half second (engage glide).
fn change_db(input: &[f32], output: &[f32]) -> f64 {
    let skip = RATE as usize;
    let energy = |x: &[f32]| x[skip..].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
    10.0 * (energy(output) / energy(input)).log10()
}

/// Linear-interpolated percentile of an ascending slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let at = (sorted.len() - 1) as f64 * p;
    let (lo, hi) = (at.floor() as usize, at.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (at - lo as f64)
}

#[test]
#[ignore = "needs the decoded MUSDB18 clips; run locally with --ignored"]
fn measure_musdb7_stems() {
    let root = std::env::var_os("OPENIFY_MUSDB7")
        .map_or_else(|| PathBuf::from(DEFAULT_ROOT), PathBuf::from);
    let Ok(entries) = fs::read_dir(&root) else {
        eprintln!("MUSDB clips not found at {}; skipping", root.display());
        return;
    };
    let mut songs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("mix.wav").is_file())
        .collect();
    songs.sort();
    if songs.is_empty() {
        eprintln!("no songs under {}; skipping", root.display());
        return;
    }

    // Columns: vocals, bass, drums, other, accompaniment (bass + drums + other
    // summed), whole mix.
    const NAMES: [&str; 6] = ["vocals", "bass", "drums", "other", "accomp", "mix"];
    let mut changes: [Vec<f64>; 6] = Default::default();
    let (mut clipped, mut peak_gain) = (0usize, 0.0f32);
    for song in &songs {
        let mix = read_wav(&song.join("mix.wav"));
        let (mix_out, trace) = run_mix(&mix, 1.0);

        let mut replay = mix.clone();
        Karaoke::new().process_with_mask_trace(&mut replay, &trace);
        let error = mix_out
            .iter()
            .zip(&replay)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            error < 2.0e-4,
            "mask replay differs by {error}: {}",
            song.display()
        );

        let mut accomp = vec![0.0f32; mix.len()];
        for (i, stem) in STEMS.iter().enumerate() {
            let input = read_wav(&song.join(format!("{stem}.wav")));
            if i > 0 {
                accomp.iter_mut().zip(&input).for_each(|(a, v)| *a += v);
            }
            let mut output = input.clone();
            Karaoke::new().process_with_mask_trace(&mut output, &trace);
            changes[i].push(change_db(&input, &output));
        }
        let mut output = accomp.clone();
        Karaoke::new().process_with_mask_trace(&mut output, &trace);
        changes[4].push(change_db(&accomp, &output));
        changes[5].push(change_db(&mix, &mix_out));

        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let (peak_in, peak_out) = (peak(&mix), peak(&mix_out));
        clipped += usize::from(peak_out > 1.0 && peak_in <= 1.0);
        peak_gain = peak_gain.max(peak_out - peak_in);
    }

    eprintln!(
        "MUSDB18 {} songs, amount 1.0, dB change (p10 / median / p90)",
        songs.len()
    );
    for (changes, name) in changes.iter_mut().zip(NAMES) {
        changes.sort_by(f64::total_cmp);
        eprintln!(
            "{name:>7}: p10 {:6.2}  median {:6.2}  p90 {:6.2}",
            percentile(changes, 0.1),
            percentile(changes, 0.5),
            percentile(changes, 0.9)
        );
    }
    eprintln!(
        "mix peaks pushed above 1.0 by karaoke: {clipped} songs, largest peak rise {peak_gain:.3}"
    );
}
