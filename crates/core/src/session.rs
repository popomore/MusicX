//! A durable listening session contains metadata, never resolved audio URLs.
use crate::protocol::{MAX_QUEUE_TRACKS, QueueContext, Status, Track};
use anyhow::{Context, Result, ensure};
use musicx_protocol::tr;
use serde::{Deserialize, Serialize};
use std::{io::ErrorKind, sync::Arc};

const FILE: &str = "playback-session.json";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Session {
    version: u32,
    pub queue: Arc<Vec<Track>>,
    pub index: Option<usize>,
    pub queue_context: Option<QueueContext>,
    pub position: f64,
    pub volume: f32,
}

impl Session {
    pub fn capture(status: &Status) -> Self {
        Self {
            version: 1,
            queue: status.queue.clone(),
            index: status.index,
            queue_context: status.queue_context.clone(),
            position: status.position,
            volume: status.volume,
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            tr!("Unsupported playback session version")
        );
        ensure!(
            self.queue.len() <= MAX_QUEUE_TRACKS
                && self.index.is_none_or(|index| index < self.queue.len())
                && self.queue.is_empty() == self.index.is_none(),
            tr!("Invalid playback queue")
        );
        ensure!(
            self.position.is_finite()
                && self.position >= 0.0
                && (0.0..=1.0).contains(&self.volume)
                && self
                    .queue
                    .iter()
                    .all(|track| track.duration.is_finite() && track.duration >= 0.0),
            tr!("Invalid playback position")
        );
        Ok(())
    }

    pub fn load() -> Result<Option<Self>> {
        let path = crate::protocol::data_dir()
            .context(tr!("User directory not found"))?
            .join(FILE);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let session: Self = serde_json::from_slice(&bytes)?;
        session.validate()?;
        Ok(Some(session))
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        crate::storage::save_private(FILE, &serde_json::to_string(self)?)
    }
}
