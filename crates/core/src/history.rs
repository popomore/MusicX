use anyhow::{Context, Result};
use musicx_protocol::tr;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::protocol::{HistoryProgress, track_key};
use crate::{protocol::Track, storage::state_dir};

const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60);
const FILE: &str = "playback-history.jsonl";
static HISTORY: Mutex<Option<Active>> = Mutex::new(None);

pub use crate::protocol::HistoryEntry as Entry;

// The start record's byte offset identifies one play, even when the same
// track starts twice in a second. This log is append-only.
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    play_offset: u64,
    progress: HistoryProgress,
}

struct Active {
    offset: u64,
    progress: Option<HistoryProgress>,
    saved: Option<HistoryProgress>,
    last_write: Instant,
}

impl Active {
    fn new(offset: u64, progress: Option<HistoryProgress>) -> Self {
        Self {
            offset,
            progress,
            saved: progress,
            last_write: Instant::now(),
        }
    }

    fn checkpoint(&mut self) -> Result<()> {
        if let Some(progress) = self.progress
            && self.progress != self.saved
        {
            self.last_write = Instant::now();
            append(&Checkpoint {
                play_offset: self.offset,
                progress,
            })?;
            self.saved = Some(progress);
        }
        Ok(())
    }
}

fn append(value: &impl Serialize) -> Result<u64> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .mode(0o600)
        .open(state_dir()?.join(FILE))
        .context(tr!("Could not open listening history"))?;
    let mut offset = file.metadata()?.len();
    if offset > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        // Keep a previous partial write from swallowing this complete record.
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
            offset += 1;
        }
    }
    file.write_all(&line)?;
    Ok(offset)
}

pub fn record(track: Track, progress: HistoryProgress) -> Result<()> {
    let mut active = HISTORY
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Listening history lock unavailable")))?;
    // A failed start write must never leave the previous play as the update target.
    *active = None;
    let entry = Entry {
        track,
        played_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        progress: Some(progress),
    };
    *active = Some(Active::new(append(&entry)?, entry.progress));
    Ok(())
}

pub fn update(mut progress: HistoryProgress, flush: bool) -> Result<()> {
    let mut active = HISTORY
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Listening history lock unavailable")))?;
    if let Some(active) = active.as_mut() {
        progress.completed |= active.progress.is_some_and(|previous| previous.completed)
            && progress.duration > 0.0
            && progress.position >= progress.duration - 0.12;
        let just_completed =
            progress.completed && !active.progress.is_some_and(|previous| previous.completed);
        active.progress = Some(progress);
        if flush || just_completed || active.last_write.elapsed() >= CHECKPOINT_INTERVAL {
            active.checkpoint()?;
        }
    }
    Ok(())
}

fn open() -> Result<Option<File>> {
    match File::open(state_dir()?.join(FILE)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn read_recent(mut reader: impl BufRead, limit: usize) -> Result<VecDeque<(u64, Entry)>> {
    let mut entries = VecDeque::with_capacity(limit);
    let mut line = Vec::new();
    let mut offset = 0;
    loop {
        let size = reader.read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }
        if let Ok(entry) = serde_json::from_slice::<Entry>(&line) {
            if entries.len() == limit {
                entries.pop_front();
            }
            entries.push_back((offset, entry));
        } else if let Ok(checkpoint) = serde_json::from_slice::<Checkpoint>(&line)
            && let Some((_, entry)) = entries
                .iter_mut()
                .rev()
                .find(|(id, _)| *id == checkpoint.play_offset)
        {
            entry.progress = Some(checkpoint.progress);
        }
        offset += size as u64;
        line.clear();
    }
    Ok(entries)
}

/// Reassociate a local session handoff with its existing history entry.
pub fn restore(track: &Track) -> Result<bool> {
    let mut active = HISTORY
        .lock()
        .map_err(|_| anyhow::anyhow!(tr!("Listening history lock unavailable")))?;
    let Some(file) = open()? else {
        return Ok(false);
    };
    let Some((offset, entry)) = read_recent(BufReader::new(file), 1)?
        .pop_back()
        .filter(|(_, entry)| track_key(&entry.track) == track_key(track))
    else {
        return Ok(false);
    };
    *active = Some(Active::new(offset, entry.progress));
    Ok(true)
}

pub fn recent(limit: u64) -> Result<Vec<Entry>> {
    let (file, active) = {
        let active = HISTORY
            .lock()
            .map_err(|_| anyhow::anyhow!(tr!("Listening history lock unavailable")))?;
        let Some(file) = open()? else {
            return Ok(Vec::new());
        };
        let len = file.metadata()?.len();
        (
            file.take(len),
            active.as_ref().map(|a| (a.offset, a.progress)),
        )
    };
    let mut entries = read_recent(BufReader::new(file), limit.clamp(1, 100) as usize)?;
    if let Some((offset, progress)) = active
        && let Some((_, entry)) = entries.iter_mut().rev().find(|(id, _)| *id == offset)
    {
        entry.progress = progress;
    }
    Ok(entries.into_iter().rev().map(|(_, entry)| entry).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_update_only_the_matching_play_and_keep_legacy_rows() {
        let old = br#"{"id":1,"title":"same","artist":"artist","duration":90,"played_at_unix":1}"#;
        let mut log = Vec::new();
        log.extend_from_slice(old);
        log.push(b'\n');
        let second = log.len() as u64;
        log.extend_from_slice(old);
        log.extend_from_slice(b"\n{\xff\n");
        let progress = HistoryProgress {
            position: 30.0,
            duration: 90.0,
            completed: false,
        };
        serde_json::to_writer(
            &mut log,
            &Checkpoint {
                play_offset: second,
                progress,
            },
        )
        .unwrap();
        log.push(b'\n');
        let rows = read_recent(log.as_slice(), 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].1.progress.is_none());
        assert!(rows[1].1.progress == Some(progress));
        assert_eq!(read_recent(log.as_slice(), 1).unwrap().len(), 1);
    }
}
