//! Bounded HTTP workers keep Promise.all genuinely concurrent without sharing a VM.
use rquickjs::{Ctx, Function};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Instant,
};

type Work = Box<dyn FnOnce() -> String + Send>;
type Slot = Arc<Mutex<Option<String>>>;
struct Pending {
    result: Slot,
    aborted: Arc<AtomicBool>,
}
struct Task {
    work: Work,
    slot: Slot,
    cancelled: Arc<AtomicBool>,
    aborted: Arc<AtomicBool>,
}
impl Task {
    fn run(self) {
        if self.cancelled.load(Ordering::Acquire) || self.aborted.load(Ordering::Acquire) {
            return;
        }
        let result = (self.work)();
        if !self.cancelled.load(Ordering::Acquire) && !self.aborted.load(Ordering::Acquire) {
            *self.slot.lock().unwrap() = Some(result);
        }
    }
}

fn workers() -> &'static SyncSender<Task> {
    static WORKERS: OnceLock<SyncSender<Task>> = OnceLock::new();
    WORKERS.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Task>(64);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..6 {
            let receiver = receiver.clone();
            std::thread::spawn(move || {
                loop {
                    let task = receiver.lock().unwrap().recv();
                    let Ok(task) = task else {
                        break;
                    };
                    task.run();
                }
            });
        }
        sender
    })
}

pub(crate) struct Bindings {
    cancelled: Arc<AtomicBool>,
}

impl Drop for Bindings {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Bindings {
    pub(crate) fn install(
        ctx: &Ctx<'_>,
        client: reqwest::blocking::Client,
        grants: Vec<String>,
        deadline: Instant,
    ) -> rquickjs::Result<Self> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let slots = Arc::new(Mutex::new((0u32, BTreeMap::<u32, Pending>::new())));
        let pending = slots.clone();
        let cancellation = cancelled.clone();
        ctx.globals().set(
            "__fetchStart",
            Function::new(ctx.clone(), move |input: String| {
                let mut pending = pending.lock().unwrap();
                if input.len() > 2 * 1024 * 1024
                    || pending.1.len() >= 16
                    || Instant::now() >= deadline
                {
                    return json!({"error":"Plugin request limit reached"}).to_string();
                }
                pending.0 += 1;
                let id = pending.0;
                let slot = Arc::new(Mutex::new(None));
                let aborted = Arc::new(AtomicBool::new(false));
                let client = client.clone();
                let grants = grants.clone();
                let task = Task {
                    slot: slot.clone(),
                    cancelled: cancellation.clone(),
                    aborted: aborted.clone(),
                    work: Box::new(move || {
                        crate::mygo::fetch(&client, &input, &grants, deadline)
                            .unwrap_or_else(|error| json!({"error":error.to_string()}))
                            .to_string()
                    }),
                };
                if workers().try_send(task).is_err() {
                    return json!({"error":"Plugin request queue is busy"}).to_string();
                }
                pending.1.insert(
                    id,
                    Pending {
                        result: slot,
                        aborted,
                    },
                );
                json!({"id":id}).to_string()
            })?,
        )?;
        let cancellations = slots.clone();
        ctx.globals().set(
            "__fetchPoll",
            Function::new(ctx.clone(), move |id: u32| -> Option<String> {
                let mut slots = slots.lock().unwrap();
                let Some(slot) = slots.1.get(&id) else {
                    return Some(json!({"error":"Unknown plugin request"}).to_string());
                };
                let result = slot.result.lock().unwrap().take();
                if result.is_some() {
                    slots.1.remove(&id);
                }
                result
            })?,
        )?;
        ctx.globals().set(
            "__fetchCancel",
            Function::new(ctx.clone(), move |id: u32| {
                if let Some(pending) = cancellations.lock().unwrap().1.remove(&id) {
                    pending.aborted.store(true, Ordering::Release);
                }
            })?,
        )?;
        Ok(Self { cancelled })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Condvar;
    #[test]
    fn http_jobs_can_run_in_parallel_and_queued_cancelled_work_is_skipped() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (sent, received) = mpsc::channel();
        let mut slots = Vec::new();
        for _ in 0..6 {
            let gate = gate.clone();
            let sent = sent.clone();
            let slot = Arc::new(Mutex::new(None));
            slots.push(slot.clone());
            workers()
                .send(Task {
                    slot,
                    cancelled: Arc::new(AtomicBool::new(false)),
                    aborted: Arc::new(AtomicBool::new(false)),
                    work: Box::new(move || {
                        sent.send(()).unwrap();
                        let (lock, condition) = &*gate;
                        drop(
                            condition
                                .wait_while(lock.lock().unwrap(), |open| !*open)
                                .unwrap(),
                        );
                        "done".into()
                    }),
                })
                .unwrap();
        }
        let started = (0..6).all(|_| {
            received
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok()
        });
        let skipped = Arc::new(AtomicBool::new(false));
        let ran = skipped.clone();
        Task {
            slot: Arc::new(Mutex::new(None)),
            cancelled: Arc::new(AtomicBool::new(true)),
            aborted: Arc::new(AtomicBool::new(false)),
            work: Box::new(move || {
                ran.store(true, Ordering::Release);
                "bad".into()
            }),
        }
        .run();
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert!(started, "all six jobs must start before any is released");
        let until = Instant::now() + std::time::Duration::from_secs(2);
        while slots.iter().any(|s| s.lock().unwrap().is_none()) && Instant::now() < until {
            std::thread::yield_now();
        }
        assert!(
            slots
                .iter()
                .all(|s| s.lock().unwrap().as_deref() == Some("done"))
        );
        assert!(!skipped.load(Ordering::Acquire));
    }
}
