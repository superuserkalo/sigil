//! One-shot background work for the library screens.
//!
//! VPK scans, the game catalog and the GameBanana feed run on a thread and report
//! back through a channel. `poll` takes the result on the UI thread, so a view
//! never waits for a disk walk or a network call.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use rune::Waker;

pub struct Job<T> {
    rx: Option<Receiver<Result<T, String>>>,
    running: bool,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Job {
            rx: None,
            running: false,
        }
    }
}

impl<T: Send + 'static> Job<T> {
    pub fn start<F>(&mut self, wake: Option<Waker>, work: F)
    where
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        if self.running {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.running = true;
        let spawned = std::thread::Builder::new()
            .name("dah-desktop-job".to_string())
            .spawn(move || {
                let result = work();
                let _ = tx.send(result);
                if let Some(wake) = wake {
                    wake();
                }
            });
        if spawned.is_err() {
            self.rx = None;
            self.running = false;
        }
    }

    /// The result once the worker is done. A dropped sender reports as an error
    /// so a screen never sits on a spinner forever.
    pub fn try_take(&mut self) -> Option<Result<T, String>> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(result) => {
                self.rx = None;
                self.running = false;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                self.running = false;
                Some(Err(
                    "The background worker stopped before reporting a result.".to_string(),
                ))
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}
