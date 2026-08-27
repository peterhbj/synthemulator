use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// SPSC lock-free ring of f32 samples (input thread → audio thread).
///
/// USB capture and the output device can run on different clocks (GT-100
/// is 44.1 kHz, the laptop graph is 48 kHz). Without a ceiling the queue
/// grows toward capacity and adds hundreds of ms of delay. Catch up by
/// dropping oldest samples when occupancy exceeds LIVE_CEILING.
const LIVE_CEILING: usize = 2048;
const LIVE_TARGET: usize = 128;

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

    pub fn len(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    pub fn push(&self, x: f32) {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let occ = w.wrapping_sub(r);
        if occ >= LIVE_CEILING.min(self.mask) {
            let new_r = w.wrapping_sub(LIVE_TARGET.min(self.mask));
            // CAS so a concurrent pop cannot rewind the catch-up jump.
            let _ = self.read.compare_exchange(r, new_r, Ordering::Release, Ordering::Relaxed);
        } else if occ >= self.mask {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_order() {
        let r = AudioRing::new(64);
        r.push(1.0);
        r.push(2.0);
        assert_eq!(r.pop(), Some(1.0));
        assert_eq!(r.pop(), Some(2.0));
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn catch_up_drops_oldest_not_a_1024_burst() {
        let r = AudioRing::new(16_384);
        for i in 0..1024 {
            r.push(i as f32);
        }
        assert_eq!(r.len(), 1024);
        assert_eq!(r.pop(), Some(0.0));

        let r = AudioRing::new(16_384);
        for i in 0..(LIVE_CEILING + 10) {
            r.push(i as f32);
        }
        assert!(r.len() < 256, "len={}", r.len());
        let first = r.pop().unwrap();
        assert!(first > 1800.0, "first={first}");
    }
}
