//! Background worker threads with "latest wins" slots: submitting a job
//! for a slot that already has a pending job replaces it. Results come
//! back through a channel the UI drains each frame; a waker (the GUI's
//! repaint request) is called after each result.

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

type Waker = Arc<dyn Fn() + Send + Sync>;

struct Queue<J> {
    jobs: VecDeque<(String, J)>,
    running: usize,
    closed: bool,
}

struct Shared<J> {
    q: Mutex<Queue<J>>,
    cv: Condvar,
}

pub struct Worker<J: Send + 'static, R: Send + 'static> {
    shared: Arc<Shared<J>>,
    rx: Receiver<(String, R)>,
    threads: Vec<JoinHandle<()>>,
}

impl<J: Send + 'static, R: Send + 'static> Worker<J, R> {
    /// Starts `threads` threads running `f` on submitted jobs.
    pub fn new(
        name: &str,
        threads: usize,
        f: impl Fn(J) -> R + Send + Sync + 'static,
        waker: Waker,
    ) -> Worker<J, R> {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue {
                jobs: VecDeque::new(),
                running: 0,
                closed: false,
            }),
            cv: Condvar::new(),
        });
        let (tx, rx) = channel::<(String, R)>();
        let f = Arc::new(f);
        let mut handles = Vec::new();
        for i in 0..threads.max(1) {
            let shared = Arc::clone(&shared);
            let tx: Sender<(String, R)> = tx.clone();
            let f = Arc::clone(&f);
            let waker = Arc::clone(&waker);
            let spawned = std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || loop {
                    let job = {
                        // A poisoned lock means a job panicked; the queue
                        // itself is still consistent.
                        let mut q = shared.q.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if q.closed {
                                return;
                            }
                            if let Some(j) = q.jobs.pop_front() {
                                q.running += 1;
                                break j;
                            }
                            q = shared.cv.wait(q).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let (slot, j) = job;
                    let r = f(j);
                    {
                        let mut q = shared.q.lock().unwrap_or_else(|e| e.into_inner());
                        q.running -= 1;
                    }
                    if tx.send((slot, r)).is_err() {
                        return;
                    }
                    waker();
                });
            if let Ok(h) = spawned {
                handles.push(h);
            }
        }
        Worker {
            shared,
            rx,
            threads: handles,
        }
    }

    /// Queues `job` for `slot`, replacing a pending job of the same slot.
    pub fn submit(&self, slot: &str, job: J) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.jobs.retain(|(s, _)| s != slot);
        q.jobs.push_back((slot.to_string(), job));
        drop(q);
        self.shared.cv.notify_one();
    }

    /// Whether a job for `slot` is waiting (not yet started).
    pub fn is_pending(&self, slot: &str) -> bool {
        let q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.jobs.iter().any(|(s, _)| s == slot)
    }

    /// Jobs waiting plus running.
    pub fn busy(&self) -> usize {
        let q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.jobs.len() + q.running
    }

    /// Drops the waiting jobs whose slot `gone` says is no longer wanted;
    /// returns how many.
    pub fn cancel_where(&self, gone: impl Fn(&str) -> bool) -> usize {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        let n = q.jobs.len();
        q.jobs.retain(|(s, _)| !gone(s));
        n - q.jobs.len()
    }

    /// Drops all waiting jobs.
    pub fn clear(&self) {
        let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
        q.jobs.clear();
    }

    /// Finished results so far.
    pub fn results(&self) -> Vec<(String, R)> {
        self.rx.try_iter().collect()
    }

    /// Whether worker threads could be started (false if the OS refused).
    pub fn has_threads(&self) -> bool {
        !self.threads.is_empty()
    }
}

impl<J: Send + 'static, R: Send + 'static> Drop for Worker<J, R> {
    fn drop(&mut self) {
        {
            let mut q = self.shared.q.lock().unwrap_or_else(|e| e.into_inner());
            q.closed = true;
            q.jobs.clear();
        }
        self.shared.cv.notify_all();
        // Running jobs finish on their own; the threads are not joined so
        // a long kicad-cli run never blocks closing the window.
        self.threads.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn wait_for<R: Send + 'static, J: Send + 'static>(
        w: &Worker<J, R>,
        n: usize,
    ) -> Vec<(String, R)> {
        let t = Instant::now();
        let mut out = Vec::new();
        while out.len() < n && t.elapsed() < Duration::from_secs(10) {
            out.extend(w.results());
            std::thread::sleep(Duration::from_millis(2));
        }
        out
    }

    #[test]
    fn runs_jobs_and_wakes() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w2 = Arc::clone(&wakes);
        let w = Worker::new(
            "t",
            2,
            |x: u32| x * 2,
            Arc::new(move || {
                w2.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(w.has_threads());
        for i in 0..5 {
            w.submit(&format!("s{i}"), i);
        }
        let mut r = wait_for(&w, 5);
        r.sort();
        assert_eq!(
            r.iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![0, 2, 4, 6, 8]
        );
        assert_eq!(wakes.load(Ordering::SeqCst), 5);
        assert_eq!(w.busy(), 0);
    }

    #[test]
    fn cancel_drops_matching_waiting_jobs() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = Arc::clone(&gate);
        let w = Worker::new(
            "t",
            1,
            move |x: u32| {
                let (m, cv) = &*g;
                let mut open = m.lock().unwrap();
                while !*open {
                    open = cv.wait(open).unwrap();
                }
                x
            },
            Arc::new(|| {}),
        );
        w.submit("busy", 0);
        let t = Instant::now();
        while w.is_pending("busy") && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        w.submit("thumb:a", 1);
        w.submit("sheet:a", 2);
        w.submit("thumb:b", 3);
        assert_eq!(w.cancel_where(|s| s.ends_with(":a")), 2);
        {
            let (m, cv) = &*gate;
            *m.lock().unwrap() = true;
            cv.notify_all();
        }
        let r = wait_for(&w, 2);
        assert_eq!(r, vec![("busy".to_string(), 0), ("thumb:b".to_string(), 3)]);
    }

    #[test]
    fn latest_job_per_slot_wins() {
        // One thread blocked on a gate, so later submissions queue up.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = Arc::clone(&gate);
        let w = Worker::new(
            "t",
            1,
            move |x: u32| {
                let (m, cv) = &*g;
                let mut open = m.lock().unwrap();
                while !*open {
                    open = cv.wait(open).unwrap();
                }
                x
            },
            Arc::new(|| {}),
        );
        w.submit("first", 0);
        // Wait until "first" runs.
        let t = Instant::now();
        while w.is_pending("first") && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        for v in 1..=5 {
            w.submit("slot", v);
        }
        assert!(w.is_pending("slot"));
        assert_eq!(w.busy(), 2);
        {
            let (m, cv) = &*gate;
            *m.lock().unwrap() = true;
            cv.notify_all();
        }
        let r = wait_for(&w, 2);
        assert_eq!(r, vec![("first".to_string(), 0), ("slot".to_string(), 5)]);
    }
}
