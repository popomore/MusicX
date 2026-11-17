//! Per-field last-writer registers with durable tombstones and Lamport ordering.
//! A device publishes its own replica; file arrival order never decides the result.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub type Snapshot = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub clock: u64,
    pub device: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Register {
    pub stamp: Stamp,
    pub value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replica {
    pub schema: u32,
    pub device: String,
    pub clock: u64,
    pub records: BTreeMap<String, Register>,
}

impl Replica {
    pub fn new(device: String) -> Self {
        Self {
            schema: 1,
            device,
            clock: 0,
            records: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported iCloud sync format");
        ensure!(
            valid_device(&self.device),
            "Invalid iCloud device identifier"
        );
        ensure!(
            self.records.len() <= 400_000,
            "iCloud sync data is too large"
        );
        for (key, register) in &self.records {
            ensure!(
                key.len() <= 16_384
                    && (key.starts_with("p/") || key.starts_with("s/") || key.starts_with("h/")),
                "Invalid iCloud record"
            );
            ensure!(
                valid_device(&register.stamp.device)
                    && register.stamp.clock > 0
                    && register.stamp.clock <= self.clock,
                "Invalid iCloud record clock"
            );
        }
        Ok(())
    }

    pub fn capture(&mut self, baseline: &Snapshot, current: &Snapshot) -> Result<()> {
        let changed: Vec<_> = baseline
            .keys()
            .chain(current.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|key| baseline.get(*key) != current.get(*key))
            .cloned()
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("iCloud sync clock exhausted"))?;
        let stamp = Stamp {
            clock: self.clock,
            device: self.device.clone(),
        };
        for key in changed {
            self.records.insert(
                key.clone(),
                Register {
                    stamp: stamp.clone(),
                    value: current.get(&key).cloned(),
                },
            );
        }
        Ok(())
    }

    pub fn merge(&mut self, other: &Self) -> Result<()> {
        other.validate()?;
        for (key, incoming) in &other.records {
            match self.records.get(key) {
                Some(existing) if existing.stamp == incoming.stamp => {
                    ensure!(
                        existing.value == incoming.value,
                        "Conflicting iCloud record with the same revision"
                    );
                }
                Some(existing) if existing.stamp > incoming.stamp => {}
                _ => {
                    self.records.insert(key.clone(), incoming.clone());
                }
            }
        }
        self.clock = self.clock.max(other.clock);
        self.validate()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.records
            .iter()
            .filter_map(|(key, value)| value.value.as_ref().map(|v| (key.clone(), v.clone())))
            .collect()
    }
}

pub fn valid_device(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

/// Replay an interrupted apply, retaining edits made after its input was captured.
pub fn recover(before: &Snapshot, after: &Snapshot, current: &Snapshot) -> Snapshot {
    let mut recovered = after.clone();
    for key in before.keys().chain(current.keys()).collect::<BTreeSet<_>>() {
        if before.get(key) != current.get(key) {
            match current.get(key) {
                Some(value) => {
                    recovered.insert(key.clone(), value.clone());
                }
                None => {
                    recovered.remove(key);
                }
            }
        }
    }
    recovered
}
