//! One tiny persisted preference; no stream URLs or per-track probe caches.
use crate::protocol::{AudioQuality, Quality};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::{OnceLock, RwLock};

fn preference() -> &'static RwLock<Quality> {
    static VALUE: OnceLock<RwLock<Quality>> = OnceLock::new();
    VALUE.get_or_init(|| {
        RwLock::new(
            crate::protocol::data_dir()
                .and_then(|p| std::fs::read(p.join("audio-quality.json")).ok())
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
        )
    })
}
pub fn preferred() -> Quality {
    *preference().read().unwrap_or_else(|e| e.into_inner())
}
pub fn set_preferred(quality: Quality) -> Result<()> {
    let mut value = preference()
        .write()
        .map_err(|_| anyhow::anyhow!("音质设置不可用"))?;
    crate::netease::save_private("audio-quality.json", &serde_json::to_string(&quality)?)
        .context("无法保存音质设置")?;
    *value = quality;
    Ok(())
}
fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
pub fn metadata(requested: Quality, item: &Value, plugin: bool) -> AudioQuality {
    let bitrate = if plugin {
        number(&item["bitRate"])
            .or_else(|| number(&item["bitrate"]))
            .filter(|n| *n > 0)
            .map(|n| if n < 10000 { n * 1000 } else { n })
    } else {
        number(&item["br"])
    }
    .filter(|n| *n > 0 && *n <= 100_000_000);
    let level = item["quality"]
        .as_str()
        .or_else(|| item["level"].as_str())
        .filter(|v| {
            matches!(
                *v,
                "standard"
                    | "higher"
                    | "high"
                    | "exhigh"
                    | "lossless"
                    | "hires"
                    | "jyeffect"
                    | "sky"
                    | "vivid"
                    | "jymaster"
            )
        })
        .map(str::to_owned);
    let format = item["type"]
        .as_str()
        .or_else(|| item["format"].as_str())
        .map(str::to_ascii_lowercase)
        .filter(|v| {
            matches!(
                v.as_str(),
                "mp3" | "flac" | "aac" | "m4a" | "ogg" | "opus" | "wav" | "alac" | "aiff"
            )
        })
        .map(|v| v.to_ascii_uppercase());
    AudioQuality {
        requested,
        level,
        format,
        bitrate_bps: bitrate,
        sample_rate_hz: number(&item["sampleRate"])
            .or_else(|| number(&item["sr"]))
            .filter(|n| (8000..=768000).contains(n))
            .map(|n| n as u32),
        bit_depth: number(&item["bitDepth"])
            .filter(|n| (8..=64).contains(n))
            .map(|n| n as u16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_downgrade_is_not_relabelled_as_requested_lossless() {
        let audio = metadata(
            Quality::Lossless,
            &json!({"level":"standard","type":"mp3","br":128000}),
            false,
        );
        assert_eq!(audio.requested, Quality::Lossless);
        assert_eq!(audio.summary(), "标准 · MP3 · 128 kbps");
        let unknown = metadata(Quality::Hires, &json!({}), true);
        assert_eq!(unknown.summary(), "来源未提供音质参数");
    }
    #[test]
    fn plugin_rate_units_and_partial_metadata() {
        for rate in [320, 320000] {
            let audio = metadata(Quality::High, &json!({"bitRate":rate,"type":"mp3"}), true);
            assert_eq!(audio.bitrate_bps, Some(320000));
            assert_eq!(audio.level, None);
        }
        let audio = metadata(
            Quality::Hires,
            &json!({"level":"hires","type":"flac","sr":96000,"bitDepth":24}),
            false,
        );
        assert_eq!(audio.summary(), "Hi-Res · FLAC · 96 kHz · 24 bit");
        assert_eq!(Quality::Hires.plugin_quality(), "lossless");
    }
}
