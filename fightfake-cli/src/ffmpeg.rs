//! Thin wrappers around ffmpeg / ffprobe subprocesses.
//!
//! ffmpeg must be on PATH.  Install via `apt install ffmpeg` / `brew install ffmpeg`.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

// ── Probe ─────────────────────────────────────────────────────────────────────

/// Returns `(width, height, fps_num, fps_den, num_frames)`.
///
/// Uses ffprobe JSON output; falls back to counting raw YUV frames if
/// `nb_frames` is unavailable (e.g. for transport streams).
pub fn probe_video(input: &Path) -> Result<(usize, usize, u64, u64, usize)> {
    let out = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,nb_frames",
            "-of", "json",
            input.to_str().unwrap_or(""),
        ])
        .output()
        .context("ffprobe not found — install ffmpeg")?;

    if !out.status.success() {
        bail!(
            "ffprobe failed on {}:\n{}",
            input.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let json: serde_json::Value = serde_json::from_slice(&out.stdout)
        .context("failed to parse ffprobe output")?;
    let stream = json["streams"]
        .get(0)
        .ok_or_else(|| anyhow::anyhow!("no video stream found in {}", input.display()))?;

    let width: usize = stream["width"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("no width in ffprobe output"))? as usize;
    let height: usize = stream["height"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("no height in ffprobe output"))? as usize;

    let fps_str = stream["r_frame_rate"]
        .as_str()
        .unwrap_or("30/1");
    let (fps_num, fps_den) = parse_fraction(fps_str).unwrap_or((30, 1));

    let num_frames: usize = stream["nb_frames"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            // Fallback: count frames by decoding headers only.
            count_frames_fallback(input).unwrap_or(0)
        });

    if num_frames == 0 {
        bail!("could not determine number of frames in {}", input.display());
    }

    Ok((width, height, fps_num, fps_den, num_frames))
}

fn count_frames_fallback(input: &Path) -> Result<usize> {
    let out = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-count_frames",
            "-select_streams", "v:0",
            "-show_entries", "stream=nb_read_frames",
            "-of", "default=nokey=1:noprint_wrappers=1",
            input.to_str().unwrap_or(""),
        ])
        .output()?;
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.trim().parse().unwrap_or(0))
}

fn parse_fraction(s: &str) -> Option<(u64, u64)> {
    let mut parts = s.splitn(2, '/');
    let n: u64 = parts.next()?.trim().parse().ok()?;
    let d: u64 = parts.next().unwrap_or("1").trim().parse().ok()?;
    Some((n, d.max(1)))
}

// ── Decode ────────────────────────────────────────────────────────────────────

/// Decode `input` video to raw planar YUV 4:2:0.
///
/// Width × height must be multiples of 16 (Eva constraint).
pub fn ffmpeg_decode_to_yuv(
    input: &Path,
    output_yuv: &Path,
    width: usize,
    height: usize,
) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i", input.to_str().unwrap_or(""),
            "-vf", &format!("scale={width}:{height}"),
            "-pix_fmt", "yuv420p",
            "-f", "rawvideo",
            output_yuv.to_str().unwrap_or(""),
        ])
        .status()
        .context("ffmpeg not found — install ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg decode failed for {}", input.display());
    }
    Ok(())
}

// ── Encode ────────────────────────────────────────────────────────────────────

/// Re-encode a raw planar YUV 4:2:0 file to H.264 MP4.
pub fn ffmpeg_encode_from_yuv(
    input_yuv: &Path,
    output_mp4: &Path,
    width: usize,
    height: usize,
    fps_num: u64,
    fps_den: u64,
) -> Result<()> {
    let framerate = format!("{fps_num}/{fps_den}");
    let size = format!("{width}x{height}");

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "rawvideo",
            "-pix_fmt", "yuv420p",
            "-s", &size,
            "-r", &framerate,
            "-i", input_yuv.to_str().unwrap_or(""),
            "-c:v", "libx264",
            "-preset", "fast",
            "-crf", "18",
            "-movflags", "+faststart",
            output_mp4.to_str().unwrap_or(""),
        ])
        .status()
        .context("ffmpeg not found — install ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg encode failed for {}", input_yuv.display());
    }
    Ok(())
}
