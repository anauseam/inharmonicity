//! # Constant Overlap-Add (COLA) Module
//!
//! This module provides the lock-free, zero-allocation circular FIFO ring buffer 
//! necessary to implement the Constant Overlap-Add (COLA) audio processing architecture. 
//! By accumulating continuous audio and advancing by overlapping hop sizes, it guarantees 
//! temporal coverage and completely eliminates boundary blind-spots for transient events. 

/// Circular FIFO for overlapping audio frame analysis.
///
/// The buffer is heap-allocated once at construction via `Box<[f32]>`
/// (the std-idiomatic fixed-capacity pattern for owned DSP state).
/// All subsequent operations are allocation-free.
pub struct CircularFifo {
    buffer: Box<[f32]>,
    write_cursor: usize,
    samples_since_last_hop: usize,
}

impl CircularFifo {
    /// Creates a zeroed FIFO with the given capacity.
    /// Allocates once — no further heap operations after this call.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            write_cursor: 0,
            samples_since_last_hop: 0,
        }
    }

    /// Appends new samples, advancing the write cursor (wraps around).
    pub fn push_samples(&mut self, samples: &[f32]) {
        let mut rem = samples.len();
        let mut sample_idx = 0;
        let cap = self.buffer.len();

        while rem > 0 {
            let chunk = std::cmp::min(rem, cap - self.write_cursor);
            self.buffer[self.write_cursor..self.write_cursor + chunk]
                .copy_from_slice(&samples[sample_idx..sample_idx + chunk]);

            self.write_cursor = (self.write_cursor + chunk) % cap;
            sample_idx += chunk;
            rem -= chunk;
        }
        self.samples_since_last_hop += samples.len();
    }

    /// Returns true when at least `hop_size` new samples have arrived.
    pub fn is_hop_ready(&self, hop_size: usize) -> bool {
        self.samples_since_last_hop >= hop_size
    }

    /// Copies the most recent `window_size` samples into `out`,
    /// handling wrap-around at the circular boundary.
    /// `window_size` must be <= `self.buffer.len()`.
    pub fn read_window(&self, window_size: usize, out: &mut [f32]) {
        assert!(
            window_size <= self.buffer.len(),
            "Window size exceeds FIFO capacity"
        );
        assert!(
            out.len() >= window_size,
            "Output buffer is too small for the requested window"
        );

        let cap = self.buffer.len();
        let window_start = (self.write_cursor + cap - window_size) % cap;

        let first_chunk = std::cmp::min(window_size, cap - window_start);
        out[..first_chunk].copy_from_slice(&self.buffer[window_start..window_start + first_chunk]);

        if first_chunk < window_size {
            let second_chunk = window_size - first_chunk;
            out[first_chunk..window_size].copy_from_slice(&self.buffer[0..second_chunk]);
        }
    }

    /// Resets the hop counter after a frame has been processed.
    pub fn acknowledge_hop(&mut self, hop_size: usize) {
        if self.samples_since_last_hop >= hop_size {
            self.samples_since_last_hop -= hop_size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_read() {
        let mut fifo = CircularFifo::new(10);

        let samples: Vec<f32> = (0..5).map(|x| x as f32).collect();
        fifo.push_samples(&samples);

        assert!(fifo.is_hop_ready(3));
        assert!(!fifo.is_hop_ready(6));

        let mut window = [0.0f32; 5];
        fifo.read_window(5, &mut window[..]);
        assert_eq!(&window, &[0.0, 1.0, 2.0, 3.0, 4.0]);

        fifo.acknowledge_hop(3);
        assert!(!fifo.is_hop_ready(3));
        assert_eq!(fifo.samples_since_last_hop, 2);
    }

    #[test]
    fn test_wrap_around() {
        let mut fifo = CircularFifo::new(5);

        let samples: Vec<f32> = (1..=7).map(|x| x as f32).collect();
        fifo.push_samples(&samples);

        let mut window = [0.0f32; 5];
        fifo.read_window(5, &mut window[..]);
        assert_eq!(&window, &[3.0, 4.0, 5.0, 6.0, 7.0]);
    }
}
