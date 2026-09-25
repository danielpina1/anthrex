//! [`RunWatcher`]: a raw daemon client read on its own thread, for the run e2e tests
//! (split out of `run_harness.rs` to keep it under the 600-line rule, F4).

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg, RunReply, RunsSnapshot};

use super::run_harness::connect;
use super::runtime;

/// A connection read on its own thread; every message it receives is kept in order.
pub struct RunWatcher {
    messages: Arc<Mutex<VecDeque<DaemonMsg>>>,
    all: Arc<Mutex<Vec<DaemonMsg>>>,
    /// The unix time each message of `all` was received at.
    times: Arc<Mutex<Vec<f64>>>,
    sender: tokio::sync::mpsc::UnboundedSender<ClientMsg>,
    _thread: std::thread::JoinHandle<()>,
}

impl RunWatcher {
    pub(super) fn connect(socket: &Path, first: Option<ClientMsg>) -> Self {
        let socket = socket.to_path_buf();
        let messages: Arc<Mutex<VecDeque<DaemonMsg>>> = Arc::default();
        let all: Arc<Mutex<Vec<DaemonMsg>>> = Arc::default();
        let times: Arc<Mutex<Vec<f64>>> = Arc::default();
        let (sender, mut outgoing) = tokio::sync::mpsc::unbounded_channel::<ClientMsg>();
        if let Some(first) = first {
            sender.send(first).unwrap();
        }
        let (queue, log, stamps) = (messages.clone(), all.clone(), times.clone());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            runtime().block_on(async move {
                let stream = connect(&socket).await;
                let _ = ready_tx.send(());
                let (mut rd, mut wr) = stream.into_split();
                let writer = tokio::spawn(async move {
                    while let Some(msg) = outgoing.recv().await {
                        if proto::write_frame(&mut wr, &msg).await.is_err() {
                            break;
                        }
                    }
                });
                while let Ok(Some(msg)) = proto::read_frame::<_, DaemonMsg>(&mut rd).await {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();
                    stamps.lock().unwrap().push(now);
                    log.lock().unwrap().push(msg.clone());
                    queue.lock().unwrap().push_back(msg);
                }
                writer.abort();
            });
        });
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the watcher connected");
        RunWatcher {
            messages,
            all,
            times,
            sender,
            _thread: thread,
        }
    }

    pub fn send(&self, msg: ClientMsg) {
        self.sender.send(msg).unwrap();
    }

    /// Every message received so far, in order.
    pub fn received(&self) -> Vec<DaemonMsg> {
        self.all.lock().unwrap().clone()
    }

    /// Every message received so far, in order, with the unix time it arrived at.
    pub fn received_at(&self) -> Vec<(f64, DaemonMsg)> {
        let times = self.times.lock().unwrap().clone();
        times.into_iter().zip(self.received()).collect()
    }

    /// Every `RunsSnapshot` received so far, in order.
    pub fn snapshots(&self) -> Vec<RunsSnapshot> {
        self.received()
            .into_iter()
            .filter_map(|m| match m {
                DaemonMsg::Run(RunReply::Snapshot(s)) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Waits for a message matching `pred`, taking every message before it off the
    /// queue.
    pub fn wait_for(&self, wait: Duration, pred: impl Fn(&DaemonMsg) -> bool) -> DaemonMsg {
        let deadline = Instant::now() + wait;
        loop {
            while let Some(msg) = self.messages.lock().unwrap().pop_front() {
                if pred(&msg) {
                    return msg;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no matching message within {wait:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
