//! Finding the mod that breaks the game, by halving the enabled set.
//!
//! A mod that crashes Deadlock or makes a match unplayable is easy to spot when it was the last
//! one added, and slow to find by hand in a library of forty. This is the search Grimoire and
//! Deadlock Mod Manager run: turn all suspects off, then turn half of them on and start the game.
//! Broken means the culprit is in the half that was on; fine (or "not sure") means it is in the
//! other half. A few launches find it, and every launch is the player's, so the state is a value
//! the screen can hold and redraw (`Bisect`).
//!
//! The search is pure: it never touches the game folder. The caller switches the set
//! [`Bisect::suspects`] names on, records the player's answer with [`Bisect::judge`], and reads
//! [`Bisect::suspects`] again for the next launch. `culprits` keeps what the search found and
//! `original` is the set that was on before, for Restore.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// How many launches a search over `n` suspects is expected to take.
pub fn estimate(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut rounds = 0usize;
    let mut size = n;
    while size > 1 {
        size = size.div_ceil(2);
        rounds += 1;
    }
    rounds + 2
}

/// What the player reported after a launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Still broken: the culprit is in the set that was on.
    Broken,
    /// Works now: the culprit is in the set that was off.
    Works,
    /// The game did not start, or the problem came and went. Treated as Works.
    Unsure,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Broken => "Still broken",
            Verdict::Works => "Works now",
            Verdict::Unsure => "Not sure",
        }
    }
}

/// One launch and what it said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    /// The suspects that were on for this launch, ascending.
    pub suspects: Vec<u16>,
    pub verdict: Verdict,
}

/// Where the search stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Turn [`Bisect::suspects`] on (and everything else off), then launch.
    Testing,
    /// One culprit found; the search over the rest of the pool stopped here.
    Found,
    /// Nothing left to test.
    Done,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bisect {
    /// Every slot that was on when the search started, ascending.
    pub original: Vec<u16>,
    /// Suspects left, ascending. Never includes a culprit.
    pub pool: Vec<u16>,
    /// Slots this search blamed, in the order it found them.
    pub culprits: Vec<u16>,
    /// The number of pool entries known to be fine.
    pub low: usize,
    /// The number of pool entries the culprit is within.
    pub high: usize,
    /// A launch came back broken, so the window is known to hold the culprit.
    pub narrowed: bool,
    /// The first launch has every suspect off: it checks that one of them is the cause.
    pub sanity: bool,
    pub rounds: Vec<Round>,
}

impl Bisect {
    pub fn new(original: &[u16], pool: &[u16]) -> Self {
        let mut pool = pool.to_vec();
        pool.sort_unstable();
        pool.dedup();
        let mut original = original.to_vec();
        original.sort_unstable();
        let high = pool.len();
        let sanity = !pool.is_empty();
        Bisect {
            original,
            pool,
            culprits: Vec::new(),
            low: 0,
            high,
            narrowed: false,
            sanity,
            rounds: Vec::new(),
        }
    }

    pub fn state(&self) -> State {
        if !self.culprits.is_empty() {
            State::Found
        } else if self.pool.is_empty() || (self.low >= self.high && !self.sanity) {
            State::Done
        } else {
            State::Testing
        }
    }

    /// The suspects to have on for the next launch. Empty once a culprit is found or the pool
    /// is empty.
    pub fn suspects(&self) -> Vec<u16> {
        match self.state() {
            // The first launch has everything off, so the search knows a suspect is the cause.
            State::Testing if self.sanity => Vec::new(),
            State::Testing => {
                let high = self.high.min(self.pool.len());
                let mid = (self.low + high) / 2;
                let end = mid.max(self.low + 1).min(high);
                self.pool[self.low.min(end)..end].to_vec()
            }
            State::Found | State::Done => Vec::new(),
        }
    }

    /// Every suspect the round has not cleared, for the "all off" first launch.
    pub fn pool_slots(&self) -> &[u16] {
        &self.pool
    }

    /// Record the answer and move on. Returns the state afterwards.
    pub fn judge(&mut self, verdict: Verdict) -> State {
        let suspects = self.suspects();
        if suspects.is_empty() {
            if self.sanity {
                // Everything off still breaks: none of these mods is the cause.
                self.rounds.push(Round {
                    suspects: Vec::new(),
                    verdict,
                });
                self.sanity = false;
                if verdict == Verdict::Broken {
                    self.pool.clear();
                    self.low = 0;
                    self.high = 0;
                }
            }
            return self.state();
        }
        self.rounds.push(Round {
            suspects: suspects.clone(),
            verdict,
        });
        let tested = suspects.len();
        self.sanity = false;
        match verdict {
            Verdict::Broken => {
                self.high = self.low + tested;
                self.narrowed = true;
            }
            Verdict::Works | Verdict::Unsure => self.low += tested,
        }
        // A broken launch narrowed the window to one suspect: that one is the culprit. When
        // every launch so far worked, the last suspect still has to be tried on.
        if self.narrowed
            && self.high <= self.pool.len()
            && self.high.saturating_sub(self.low) == 1
        {
            let culprit = self.pool[self.high - 1];
            self.culprits.push(culprit);
            // Whatever came before it is cleared; the search stopped here.
            self.pool.truncate(self.high - 1);
            self.low = 0;
            self.high = 0;
            self.narrowed = false;
        }
        self.state()
    }

    /// The slots to have on for the next launch: the culprits found plus the current suspects.
    /// Everything else the library has is switched off by the caller.
    pub fn enable_now(&self) -> Vec<u16> {
        let mut on = self.culprits.clone();
        on.extend(self.suspects());
        on.sort_unstable();
        on.dedup();
        on
    }

    /// The search reached an answer (a culprit or "none of them").
    pub fn finished(&self) -> bool {
        matches!(self.state(), State::Found | State::Done)
    }

    /// How many launches are left at most, for the dialog's line.
    pub fn launches_left(&self) -> usize {
        let left = self.high.saturating_sub(self.low);
        if left == 0 { 0 } else { left.div_ceil(2).max(1) }
    }
}

/// The slots a search starts with: everything that is on now.
pub fn candidates(enabled: &[u16]) -> BTreeSet<u16> {
    enabled.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a search where `broken` is the culprit and the answer comes from what was on.
    fn run(pool: &[u16], broken: Option<u16>) -> (Bisect, usize) {
        let mut search = Bisect::new(pool, pool);
        let mut launches = 0;
        while !search.finished() {
            let suspects = search.suspects();
            launches += 1;
            assert!(launches <= estimate(pool.len()) + 1, "took too many launches");
            // The all-off round works when the culprit is among the pool.
            let verdict = match broken {
                Some(_) if suspects.is_empty() => Verdict::Works,
                Some(slot) if suspects.contains(&slot) => Verdict::Broken,
                _ => Verdict::Works,
            };
            search.judge(verdict);
        }
        (search, launches)
    }

    #[test]
    fn finds_the_broken_mod_in_a_logarithmic_number_of_launches() {
        let pool: Vec<u16> = (1..=12).collect();
        for broken in 1..=12u16 {
            let (search, launches) = run(&pool, Some(broken));
            assert_eq!(search.culprits, vec![broken], "broken {broken}");
            assert_eq!(search.state(), State::Found);
            assert!(launches <= estimate(pool.len()), "broken {broken}: {launches} launches");
        }
        let (clean, _) = run(&pool, None);
        assert!(clean.culprits.is_empty());
        assert_eq!(clean.state(), State::Done);
    }

    #[test]
    fn one_and_two_suspects_need_a_launch_or_two() {
        let (one, launches) = run(&[7], Some(7));
        assert_eq!(one.culprits, vec![7]);
        assert_eq!(launches, 2, "everything off, then the one suspect on");
        let (two, launches) = run(&[3, 4], Some(4));
        assert_eq!(two.culprits, vec![4]);
        assert!(launches <= 3, "the all-off launch, then two halves");
        let (none, _) = run(&[3, 4], None);
        assert!(none.culprits.is_empty());
    }

    #[test]
    fn unsure_counts_as_working_and_the_rounds_are_kept() {
        let mut search = Bisect::new(&[1, 2, 3, 4], &[1, 2, 3, 4]);
        // The first launch has every suspect off.
        assert!(search.suspects().is_empty());
        search.judge(Verdict::Works);
        assert_eq!(search.suspects(), vec![1, 2]);
        search.judge(Verdict::Unsure);
        assert_eq!(search.rounds.len(), 2);
        assert_eq!(search.rounds[1].verdict, Verdict::Unsure);
        assert_eq!(search.state(), State::Testing);
        assert_eq!(search.suspects(), vec![3]);
        search.judge(Verdict::Broken);
        assert_eq!(search.culprits, vec![3]);
        assert!(search.finished());
        assert_eq!(search.enable_now(), vec![3]);
        assert_eq!(estimate(0), 0);
        assert_eq!(estimate(1), 2);
        assert_eq!(estimate(16), 6);
    }

    #[test]
    fn culprits_stay_on_while_the_rest_of_the_pool_is_tested() {
        let mut search = Bisect::new(&[1, 2, 3, 4, 5], &[1, 2, 3, 4, 5]);
        search.judge(Verdict::Works); // the all-off launch
        assert_eq!(search.suspects(), vec![1, 2]);
        search.judge(Verdict::Broken); // [1,2] broken: the culprit is in there
        assert_eq!(search.suspects(), vec![1]);
        search.judge(Verdict::Broken);
        assert_eq!(search.culprits, vec![1]);
        assert_eq!(search.enable_now(), vec![1]);
        let candidates = candidates(&[9, 3, 9]);
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn a_broken_all_off_launch_says_it_is_not_one_of_them() {
        let mut search = Bisect::new(&[1, 2, 3], &[1, 2, 3]);
        assert!(search.suspects().is_empty());
        search.judge(Verdict::Broken);
        assert!(search.culprits.is_empty());
        assert_eq!(search.state(), State::Done);
        assert!(search.enable_now().is_empty());
    }
}
