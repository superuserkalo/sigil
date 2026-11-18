//! Speed and time left for a running download: a smoothed bytes a second and the window that
//! flags a crawl. Pure: the caller passes the bytes on disk and the time since the transfer
//! started, so tests step the clock instead of sleeping.
//!
//! The smoothing is the one Grimoire uses for its queue readout: a sample window of a quarter
//! second, then the newest reading counts for three tenths of the number. The slow notice takes
//! Deadlock Mod Manager's thresholds (under 150 KB/s for 8 s; Grimoire's 1 MB/s flags ordinary
//! home connections) with Grimoire's gap rule: the window starts over after [`SLOW_GAP`]
//! without readings, so a pause is never counted as a crawl. A row nearly done is not flagged
//! (DMM: it finishes before an explanation helps).

use std::time::Duration;

/// A download under this many bytes a second is crawling. GameBanana's file servers slow down
/// under load, and players report that as a bug in the app.
pub const SLOW_BPS: u64 = 150 * 1024;

/// The smoothed speed has to stay under [`SLOW_BPS`] this long before a row is flagged.
pub const SLOW_HOLD: Duration = Duration::from_secs(8);

/// No readings for this long means a pause or a stall, so the slow window starts over instead
/// of spanning the gap.
pub const SLOW_GAP: Duration = Duration::from_secs(3);

/// The least time between two speed samples. Progress reaches the queue several times a second
/// and the readout does not need to move that often.
pub const SPEED_SAMPLE: Duration = Duration::from_millis(250);

/// Rolling transfer figures for one active row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Rate {
    /// Bytes on disk and elapsed time of the previous speed sample.
    anchor: Option<(u64, u64)>,
    /// Elapsed time of the last reading, including one too close to sample.
    seen: Option<u64>,
    /// Smoothed bytes a second, once a sample landed.
    bps: Option<u64>,
    /// Elapsed time the current slow window started, while the speed is under the threshold.
    slow_since: Option<u64>,
    /// True once the slow window has lasted the full hold.
    slow: bool,
}

impl Rate {
    /// Feed one reading: the bytes on disk now and the time since the transfer started.
    ///
    /// A reading inside [`SPEED_SAMPLE`] only moves the anchor check, so a fast socket cannot
    /// jitter the number on every chunk. A reading that goes backwards means the file started
    /// over, and a long silence means a pause or a stall; both start the figures again.
    pub(super) fn observe(&mut self, done: u64, elapsed: Duration) {
        let now = millis(elapsed);
        if self.anchor.is_some_and(|(bytes, _)| done < bytes) {
            *self = Rate::default();
        }
        if self
            .seen
            .is_some_and(|last| now.saturating_sub(last) > millis(SLOW_GAP))
        {
            // Do not carry a speed or a slow window across a pause.
            *self = Rate::default();
        }
        self.seen = Some(now);
        let Some((anchor_bytes, anchor_ms)) = self.anchor else {
            self.anchor = Some((done, now));
            return;
        };
        let span = now.saturating_sub(anchor_ms);
        if span < millis(SPEED_SAMPLE) {
            return;
        }
        self.anchor = Some((done, now));
        let rate = done.saturating_sub(anchor_bytes).saturating_mul(1000) / span;
        let smoothed = match self.bps {
            // Light smoothing: the newest reading counts for three tenths. One burst or one
            // stall moves the number, but only by a little.
            Some(previous) => ((previous as u128 * 7 + rate as u128 * 3) / 10) as u64,
            None => rate,
        };
        self.bps = Some(smoothed);
        if smoothed >= SLOW_BPS {
            self.slow_since = None;
            self.slow = false;
            return;
        }
        let since = *self.slow_since.get_or_insert(now);
        self.slow = now.saturating_sub(since) >= millis(SLOW_HOLD);
    }

    /// The smoothed speed, once a sample landed.
    pub(super) fn bps(&self) -> Option<u64> {
        self.bps
    }

    /// True when the crawl lasted long enough to be worth telling the player about.
    pub(super) fn slow(&self) -> bool {
        self.slow
    }
}

fn millis(elapsed: Duration) -> u64 {
    elapsed.as_millis().min(u64::MAX as u128) as u64
}
