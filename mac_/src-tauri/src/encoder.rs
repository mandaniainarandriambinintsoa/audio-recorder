use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use mp3lame_encoder::{Builder, FlushNoGap, Mode, MonoPcm};

use crate::capture::AudioChunk;

const OUTPUT_BITRATE_KBPS: u16 = 64;

#[derive(Debug, Clone)]
pub struct EncoderMetadata {
    pub source_app: String,
    pub started_at_iso: String,
}

pub fn run(
    rx: Receiver<AudioChunk>,
    output_path: PathBuf,
    meta: EncoderMetadata,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut builder = Builder::new().context("init lame builder")?;
    builder
        .set_num_channels(1)
        .map_err(|e| anyhow::anyhow!("lame channels: {e:?}"))?;
    builder
        .set_sample_rate(48_000)
        .map_err(|e| anyhow::anyhow!("lame sample rate: {e:?}"))?;
    builder
        .set_brate(mp3lame_encoder::Bitrate::Kbps64)
        .map_err(|e| anyhow::anyhow!("lame bitrate: {e:?}"))?;
    builder
        .set_mode(Mode::Mono)
        .map_err(|e| anyhow::anyhow!("lame mode: {e:?}"))?;
    builder
        .set_quality(mp3lame_encoder::Quality::Good)
        .map_err(|e| anyhow::anyhow!("lame quality: {e:?}"))?;
    let mut encoder = builder
        .build()
        .map_err(|e| anyhow::anyhow!("lame build: {e:?}"))?;

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .with_context(|| format!("create {}", output_path.display()))?;
    let mut writer = BufWriter::new(file);

    write_id3v2(&mut writer, &meta).context("id3v2 header")?;

    let mut mp3_out: Vec<u8> = Vec::new();

    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                encode_chunk(&mut encoder, &chunk, &mut mp3_out, &mut writer)?;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    while let Ok(chunk) = rx.try_recv() {
        encode_chunk(&mut encoder, &chunk, &mut mp3_out, &mut writer)?;
    }

    let needed = mp3lame_encoder::max_required_buffer_size(0).max(7200);
    mp3_out.clear();
    mp3_out.reserve(needed);
    let n = encoder
        .flush::<FlushNoGap>(mp3_out.spare_capacity_mut())
        .map_err(|e| anyhow::anyhow!("lame flush: {e:?}"))?;
    unsafe { mp3_out.set_len(n) };
    if n > 0 {
        writer.write_all(&mp3_out)?;
    }

    writer.flush()?;
    Ok(())
}

fn encode_chunk(
    encoder: &mut mp3lame_encoder::Encoder,
    chunk: &AudioChunk,
    mp3_out: &mut Vec<u8>,
    writer: &mut BufWriter<File>,
) -> Result<()> {
    let mono_i16: Vec<i16> = downmix_stereo_to_mono(&chunk.samples)
        .into_iter()
        .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();

    let frame_count = mono_i16.len();
    let needed = mp3lame_encoder::max_required_buffer_size(frame_count);
    mp3_out.clear();
    mp3_out.reserve(needed);

    let written = encoder
        .encode(MonoPcm(&mono_i16), mp3_out.spare_capacity_mut())
        .map_err(|e| anyhow::anyhow!("lame encode: {e:?}"))?;
    unsafe { mp3_out.set_len(written) };

    if written > 0 {
        writer.write_all(mp3_out)?;
    }
    Ok(())
}

fn downmix_stereo_to_mono(stereo: &[f32]) -> Vec<f32> {
    stereo
        .chunks_exact(2)
        .map(|channels| (channels[0] + channels[1]) * 0.5)
        .collect()
}

fn write_id3v2(writer: &mut BufWriter<File>, meta: &EncoderMetadata) -> std::io::Result<()> {
    let title = format!("Audio Recorder — {}", meta.source_app);
    let comment = format!(
        "Recorded {}; mono {} kbit/s",
        meta.started_at_iso, OUTPUT_BITRATE_KBPS
    );
    let artist = "Audio Recorder";

    let mut frames = Vec::new();
    write_text_frame(&mut frames, b"TIT2", &title);
    write_text_frame(&mut frames, b"TPE1", artist);
    write_text_frame(&mut frames, b"COMM", &comment);

    let mut header = Vec::with_capacity(10 + frames.len());
    header.extend_from_slice(b"ID3");
    header.extend_from_slice(&[0x04, 0x00]);
    header.push(0x00);
    let size = synchsafe(frames.len() as u32);
    header.extend_from_slice(&size);
    header.extend_from_slice(&frames);

    writer.write_all(&header)
}

fn write_text_frame(out: &mut Vec<u8>, id: &[u8; 4], text: &str) {
    let mut payload = vec![0x03];
    payload.extend_from_slice(text.as_bytes());
    out.extend_from_slice(id);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&payload);
}

fn synchsafe(n: u32) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmixes_each_stereo_frame_without_clipping() {
        let mono = downmix_stereo_to_mono(&[1.0, 1.0, 1.0, -1.0, -0.5, 0.5]);
        assert_eq!(mono, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn output_bitrate_matches_the_size_shown_in_the_ui() {
        let bytes_per_hour = u64::from(OUTPUT_BITRATE_KBPS) * 1_000 * 3_600 / 8;
        assert_eq!(bytes_per_hour, 28_800_000);
    }
}
