//! What the server tells open browsers about: background tasks, finished
//! thumbnails, import progress. One broadcast channel feeds every SSE stream.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::imports::Job;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// A media item changed in a way a grid shows: a new thumbnail, mostly.
    #[serde(rename_all = "camelCase")]
    Media { id: i64, thumb_url: Option<String> },
    #[serde(rename_all = "camelCase")]
    Storyboard { id: i64, ready: bool },
    Task { task: TaskInfo },
    Job { job: Job },
    /// Roots or folders changed; the sidebar should refetch.
    Library,
}

#[derive(Clone)]
pub struct Events(broadcast::Sender<Event>);

impl Events {
    pub fn new() -> Self {
        Self(broadcast::channel(1024).0)
    }
    pub fn send(&self, event: Event) {
        // No subscribers is the normal state with no browser open.
        let _ = self.0.send(event);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.subscribe()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: u64,
    pub kind: &'static str,
    pub label: String,
    pub done: u64,
    pub total: u64,
    /// running | done | failed
    pub state: &'static str,
    pub detail: String,
}

/// Long-running work the activity indicator shows: scans, moves, storyboards.
#[derive(Clone)]
pub struct Tasks {
    inner: Arc<Mutex<HashMap<u64, (TaskInfo, Option<Instant>)>>>,
    next: Arc<AtomicU64>,
    events: Events,
}

/// How long a finished task stays listed, so a quick one is still seen.
const KEEP_FINISHED: Duration = Duration::from_secs(30);

impl Tasks {
    pub fn new(events: Events) -> Self {
        Self { inner: Default::default(), next: Arc::new(AtomicU64::new(1)), events }
    }

    pub fn start(&self, kind: &'static str, label: impl Into<String>, total: u64) -> Task {
        let info = TaskInfo {
            id: self.next.fetch_add(1, Ordering::Relaxed),
            kind,
            label: label.into(),
            done: 0,
            total,
            state: "running",
            detail: String::new(),
        };
        self.inner.lock().unwrap().insert(info.id, (info.clone(), None));
        self.events.send(Event::Task { task: info.clone() });
        Task { tasks: self.clone(), id: info.id, last_sent: Mutex::new(Instant::now()) }
    }

    pub fn list(&self) -> Vec<TaskInfo> {
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, (_, finished)| finished.is_none_or(|at| at.elapsed() < KEEP_FINISHED));
        let mut list: Vec<_> = map.values().map(|(t, _)| t.clone()).collect();
        list.sort_by_key(|t| t.id);
        list
    }

    fn update(&self, id: u64, force: bool, f: impl FnOnce(&mut TaskInfo)) -> Option<TaskInfo> {
        let mut map = self.inner.lock().unwrap();
        let (info, finished) = map.get_mut(&id)?;
        f(info);
        if info.state != "running" {
            *finished = Some(Instant::now());
        }
        force.then(|| info.clone())
    }
}

/// A handle on one running task. Dropping it without finishing marks it done,
/// so an early return can never leave a spinner running forever.
pub struct Task {
    tasks: Tasks,
    id: u64,
    last_sent: Mutex<Instant>,
}

impl Task {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Progress is throttled: a scan of thousands of files would otherwise send
    /// thousands of frames to every open tab.
    pub fn progress(&self, done: u64, total: u64, detail: impl Into<String>) {
        let detail = detail.into();
        let due = {
            let mut last = self.last_sent.lock().unwrap();
            let due = last.elapsed() > Duration::from_millis(400);
            if due {
                *last = Instant::now();
            }
            due
        };
        if let Some(info) = self.tasks.update(self.id, due, |t| {
            t.done = done;
            t.total = total;
            t.detail = detail;
        }) {
            self.tasks.events.send(Event::Task { task: info });
        }
    }

    pub fn finish(self, detail: impl Into<String>) {
        self.end("done", detail.into());
    }

    pub fn fail(self, detail: impl Into<String>) {
        self.end("failed", detail.into());
    }

    fn end(&self, state: &'static str, detail: String) {
        if let Some(info) = self.tasks.update(self.id, true, |t| {
            if t.state == "running" {
                t.state = state;
                t.detail = detail;
                if state == "done" {
                    t.done = t.total;
                }
            }
        }) {
            self.tasks.events.send(Event::Task { task: info });
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let running = self.tasks.inner.lock().unwrap().get(&self.id).is_some_and(|(t, _)| t.state == "running");
        if running {
            self.end("done", String::new());
        }
    }
}
