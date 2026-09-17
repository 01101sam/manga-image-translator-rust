use std::collections::VecDeque;

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::sync::watch;

pub const QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFull {
    pub capacity: usize,
}

pub struct Queue {
    inner: Mutex<VecDeque<String>>,
    notify: Notify,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
        }
    }

    pub fn try_push(&self, job_id: String) -> Result<(), QueueFull> {
        let mut g = self.inner.lock();
        if g.len() >= QUEUE_CAPACITY {
            return Err(QueueFull {
                capacity: QUEUE_CAPACITY,
            });
        }
        g.push_back(job_id);
        drop(g);
        self.notify.notify_one();
        Ok(())
    }

    pub fn remove(&self, job_id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(i) = g.iter().position(|id| id == job_id) {
            g.remove(i);
            true
        } else {
            false
        }
    }

    pub fn drain(&self) -> Vec<String> {
        let mut g = self.inner.lock();
        g.drain(..).collect()
    }

    pub fn position(&self, job_id: &str) -> Option<usize> {
        self.inner
            .lock()
            .iter()
            .position(|id| id == job_id)
    }

    pub fn wake_all(&self) {
        self.notify.notify_waiters();
    }

    fn pop_front(&self) -> Option<String> {
        self.inner.lock().pop_front()
    }

    pub async fn recv(&self, stop: &mut watch::Receiver<bool>) -> Option<String> {
        loop {
            if *stop.borrow() {
                return None;
            }
            if let Some(id) = self.pop_front() {
                return Some(id);
            }
            let notified = self.notify.notified();
            if *stop.borrow() {
                return None;
            }
            if let Some(id) = self.pop_front() {
                return Some(id);
            }
            tokio::select! {
                _ = notified => {}
                _ = stop.changed() => {
                    if *stop.borrow() {
                        return None;
                    }
                }
            }
        }
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_then_pop_fifo() {
        let q = Queue::new();
        q.try_push("a".into()).unwrap();
        q.try_push("b".into()).unwrap();
        q.try_push("c".into()).unwrap();
        assert_eq!(q.pop_front().as_deref(), Some("a"));
        assert_eq!(q.pop_front().as_deref(), Some("b"));
        assert_eq!(q.pop_front().as_deref(), Some("c"));
        assert_eq!(q.pop_front(), None);
    }

    #[test]
    fn full_queue_rejects_at_capacity() {
        let q = Queue::new();
        for i in 0..QUEUE_CAPACITY {
            q.try_push(format!("j{i}")).unwrap();
        }
        let err = q.try_push("overflow".into()).unwrap_err();
        assert_eq!(
            err,
            QueueFull {
                capacity: QUEUE_CAPACITY
            }
        );
        assert_eq!(q.pop_front().as_deref(), Some("j0"));
    }
}
