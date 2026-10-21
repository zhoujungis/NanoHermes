//! Sub-agent task board and concurrency gate.
//!
//! The board records every sub-agent delegation (who, what, status, duration)
//! so the model can report progress and inspect past jobs. The gate bounds how
//! many sub-agents may run at once, since the agent launches one thread per
//! tool call and a turn can request several delegations.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Characters of a job result kept on the board (the full text still goes back
/// to the model as the tool result).
const RESULT_PREVIEW: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub task: String,
    pub status: JobStatus,
    pub result: String,
    /// Wall-clock time the job took, once it has finished.
    pub duration: Option<Duration>,
    started: Instant,
}

/// Thread-safe registry of sub-agent jobs.
pub struct Board {
    jobs: Mutex<Vec<Job>>,
    seq: Mutex<u64>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    pub fn new() -> Self {
        Board {
            jobs: Mutex::new(Vec::new()),
            seq: Mutex::new(0),
        }
    }

    /// Register a job as running and return its id.
    pub fn start(&self, name: &str, task: &str) -> String {
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let id = format!("agent-{seq}");
        self.jobs.lock().unwrap().push(Job {
            id: id.clone(),
            name: name.to_string(),
            task: task.to_string(),
            status: JobStatus::Running,
            result: String::new(),
            duration: None,
            started: Instant::now(),
        });
        id
    }

    pub fn finish_ok(&self, id: &str, result: &str) {
        self.finish(id, JobStatus::Done, result);
    }

    pub fn finish_err(&self, id: &str, error: &str) {
        self.finish(id, JobStatus::Failed, error);
    }

    fn finish(&self, id: &str, status: JobStatus, result: &str) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = status;
            job.duration = Some(job.started.elapsed());
            job.result = preview(result);
        }
    }

    /// Snapshot of every job, in launch order.
    pub fn list(&self) -> Vec<Job> {
        self.jobs.lock().unwrap().clone()
    }
}

fn preview(text: &str) -> String {
    let flat = text.replace('\n', " ");
    if flat.len() <= RESULT_PREVIEW {
        return flat;
    }
    let mut end = RESULT_PREVIEW;
    while end > 0 && !flat.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &flat[..end])
}

/// Limits how many sub-agents run concurrently.
pub struct Gate {
    active: Mutex<usize>,
    cv: Condvar,
    max: usize,
}

impl Gate {
    pub fn new(max: usize) -> Self {
        Gate {
            active: Mutex::new(0),
            cv: Condvar::new(),
            max: max.max(1),
        }
    }

    /// Block until a slot is free. The returned guard releases it on drop.
    pub fn acquire(&self) -> GateGuard<'_> {
        let mut active = self.active.lock().unwrap();
        while *active >= self.max {
            active = self.cv.wait(active).unwrap();
        }
        *active += 1;
        GateGuard { gate: self }
    }
}

pub struct GateGuard<'a> {
    gate: &'a Gate,
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.gate.active.lock().unwrap();
        *active = active.saturating_sub(1);
        self.gate.cv.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn board_records_jobs_in_order() {
        let board = Board::new();
        let a = board.start("reviewer", "review auth.rs");
        let b = board.start("explorer", "map the repo");

        let jobs = board.list();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, a);
        assert_eq!(jobs[0].status, JobStatus::Running);
        assert_eq!(jobs[1].name, "explorer");

        board.finish_ok(&a, "looks fine\nsecond line");
        board.finish_err(&b, "model returned 500");

        let jobs = board.list();
        assert_eq!(jobs[0].status, JobStatus::Done);
        assert_eq!(jobs[0].result, "looks fine second line");
        assert_eq!(jobs[1].status, JobStatus::Failed);
        assert_eq!(jobs[1].result, "model returned 500");
    }

    #[test]
    fn gate_caps_concurrency() {
        let gate = Arc::new(Gate::new(2));
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..6)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let concurrent = Arc::clone(&concurrent);
                let peak = Arc::clone(&peak);
                std::thread::spawn(move || {
                    let _permit = gate.acquire();
                    let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert!(
            peak.load(Ordering::SeqCst) <= 2,
            "peak was {}",
            peak.load(Ordering::SeqCst)
        );
        assert_eq!(*gate.active.lock().unwrap(), 0, "all permits released");
    }
}
