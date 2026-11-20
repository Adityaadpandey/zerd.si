//! Unattended runs: `zerd run --task task.json --headless`.
//! Nobody is there to answer, so anything that would normally ask a person is recorded here instead, and the run
//! stops with exit code 3 and the reason. Example: the agent wants to run `rm -rf build` and approval says "ask"
//! -> the question lands in BLOCKED -> the watcher sees it and ends the run.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
static BLOCKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn enable() {
    ON.store(true, Ordering::SeqCst);
}

/// Called where a prompt would have appeared. A no-op outside headless runs.
pub fn block(reason: &str) {
    if ON.load(Ordering::SeqCst) {
        BLOCKED.lock().unwrap().push(reason.to_string());
    }
}

pub fn blocked() -> Vec<String> {
    BLOCKED.lock().unwrap().clone()
}
