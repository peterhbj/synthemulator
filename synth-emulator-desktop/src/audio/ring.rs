use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// SPSC lock-free ring of f32 samples (input thread → audio thread).
pub struct AudioRing {
    buf: Vec<AtomicU32>,
    mask: usize,
    write: AtomicUsize,
    read: AtomicUsize,
}

impl AudioRing {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(64);
        let mut buf = Vec::with_capacity(cap);
        for _ in 0..cap {
            buf.push(AtomicU32::new(0));
        }
        Self {
            buf,
            mask: cap - 1,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    pub fn occupancy(&self) -> usize {
        let w = self.write.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Acquire);
        w.wrapping_sub(r)
    }

    pub fn push(&self, x: f32) {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        if w.wrapping_sub(r) >= self.mask {
            return;
        }
        self.buf[w & self.mask].store(x.to_bits(), Ordering::Relaxed);
        self.write.store(w.wrapping_add(1), Ordering::Release);
    }

    pub fn pop(&self) -> Option<f32> {
        let r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        if r == w {
            return None;
        }
        let bits = self.buf[r & self.mask].load(Ordering::Relaxed);
        self.read.store(r.wrapping_add(1), Ordering::Release);
        Some(f32::from_bits(bits))
    }

    /// Pop the latest guitar sample, dropping backlog so USB clock drift
    /// cannot turn the ring into an extra delay pedal.
    pub fn pop_live(&self, slack: usize) -> Option<f32> {
        let slack = slack.max(32);
        let r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        let occ = w.wrapping_sub(r);
        if occ == 0 {
            return None;
        }
        let r = if occ > slack {
            w.wrapping_sub(slack)
        } else {
            r
        };
        let bits = self.buf[r & self.mask].load(Ordering::Relaxed);
        self.read.store(r.wrapping_add(1), Ordering::Release);
        Some(f32::from_bits(bits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_live_drops_backlog() {
        let ring = AudioRing::new(2048);
        for i in 0..1000 {
            ring.push(i as f32);
        }
        assert_eq!(ring.occupancy(), 1000);
        let y = ring.pop_live(64).unwrap();
        assert!((y - (1000 - 64) as f32).abs() < 0.5, "got {y}");
        assert!(ring.occupancy() <= 64);
    }

    #[test]
    fn pop_live_empty() {
        let ring = AudioRing::new(64);
        assert!(ring.pop_live(32).is_none());
    }
}
