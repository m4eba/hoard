use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use tokio::sync::Notify;

use crate::{
    auth::Auth,
    config::Config,
    db::Db,
    events::{Events, Tasks},
    imports::Imports,
    publish::Publish,
    ytdlp::Ytdlp,
};

pub struct AppState {
    pub cfg: Config,
    pub db: Db,
    pub events: Events,
    pub tasks: Tasks,
    pub auth: Auth,
    pub imports: Imports,
    pub ytdlp: Ytdlp,
    /// fiffly's signing key and pending authorization codes.
    pub publish: Publish,
    /// Wakes the probe and thumbnail worker when new media arrive.
    pub work: Notify,
    /// Roots being scanned, so a second request does not start a second walk.
    pub scanning: Mutex<HashSet<i64>>,
    /// Wakes the storyboard worker when its queue gains work.
    pub storyboard_wake: Notify,
    /// Media whose storyboard is being built right now.
    pub storyboards: Mutex<HashSet<i64>>,
    /// Media being moved, so two moves cannot race over the same file.
    pub moving: Mutex<HashSet<i64>>,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(cfg: Config, db: Db) -> App {
        let events = Events::new();
        Arc::new(Self {
            tasks: Tasks::new(events.clone()),
            auth: Auth::new(cfg.password.clone()),
            imports: Imports::new(cfg.downloads),
            ytdlp: Ytdlp::new(&cfg),
            publish: Publish::default(),
            work: Notify::new(),
            storyboard_wake: Notify::new(),
            scanning: Default::default(),
            storyboards: Default::default(),
            moving: Default::default(),
            events,
            cfg,
            db,
        })
    }
}
