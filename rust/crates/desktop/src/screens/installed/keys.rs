//! Cheap fingerprints of what Installed's poll works from, so a refresh that would come out
//! the same is skipped. Poll runs on every frame that animates and on every wake, and the
//! refreshes walk every mod, the library and the notes store: at 300 mods they were about a
//! tenth of all main-thread time while anything moved. Each key hashes only the fields its
//! refresh reads, with no lookups between the lists, so it costs one pass over each.

use std::hash::{Hash, Hasher};

use crate::app::State;

/// A fast non-cryptographic hasher (the Fx fold): these keys only tell "changed" from "the
/// same", and SipHash over every field of every mod cost more than the checks it guards.
#[derive(Default)]
struct Fold(u64);

impl Hasher for Fold {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
    fn write_u16(&mut self, n: u16) {
        self.write_u64(u64::from(n));
    }
    fn write_u8(&mut self, n: u8) {
        self.write_u64(u64::from(n));
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

fn finish(f: impl FnOnce(&mut Fold)) -> u64 {
    let mut h = Fold::default();
    f(&mut h);
    h.finish()
}

/// The rows on disk: slot, size and whether each is on.
pub fn addons(s: &State) -> u64 {
    finish(|h| {
        s.addons.addons.len().hash(h);
        for a in &s.addons.addons {
            (a.slot, a.size_bytes, a.enabled).hash(h);
        }
    })
}

/// The library entries: what a mod's id, name and preview come from.
pub fn library(s: &State) -> u64 {
    finish(|h| {
        s.installed.library.mods.len().hash(h);
        for m in &s.installed.library.mods {
            (m.slot, m.size_bytes, m.gamebanana_id).hash(h);
            m.thumbnail_url.hash(h);
        }
    })
}

/// The notes entries' load order marks.
pub fn always_first(s: &State) -> u64 {
    finish(|h| {
        s.installed.notes.extras.mods.len().hash(h);
        for e in &s.installed.notes.extras.mods {
            (e.slot, e.size_bytes, e.always_first).hash(h);
        }
    })
}

/// The requirement pages read so far, in any order.
pub fn requirements(s: &State) -> u64 {
    s.installed.gates.known.iter().fold(s.installed.gates.known.len() as u64, |sum, (id, (at, list))| {
        sum.wrapping_add(finish(|h| (id, at, list.len()).hash(h)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_moves_with_what_it_covers() {
        let mut s = crate::data::init_state();
        crate::bench_scenes::installed_stress_of(&mut s, 40);
        let (a, l, f) = (addons(&s), library(&s), always_first(&s));
        s.addons.addons[3].enabled = !s.addons.addons[3].enabled;
        assert_ne!(addons(&s), a);
        s.installed.library.mods[2].thumbnail_url = Some("https://images.gamebanana.com/x.webp".into());
        assert_ne!(library(&s), l);
        let first = !s.installed.notes.extras.mods[1].always_first;
        s.installed.notes.extras.mods[1].always_first = first;
        assert_ne!(always_first(&s), f);
    }
}
