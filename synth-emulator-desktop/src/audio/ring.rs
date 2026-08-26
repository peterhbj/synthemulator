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
}
