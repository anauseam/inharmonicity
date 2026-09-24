//! # Audio capture and the analysis host
//!
//! The CPAL input stream, whose callback DC-blocks each sample into a lock-free
//! ring buffer, and [`spawn_analysis_thread`], which runs an [`AudioPipeline`] on
//! its own thread. A host with its own audio thread calls
//! [`AudioPipeline::push_audio`] instead.

use anyhow::{Result, anyhow};
use cpal::SupportedStreamConfigRange;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use crate::FrameOutput;
use crate::pipeline::{
    AudioPipeline, CaptureSender, PipelineHandle, PipelinePorts, ProfileSender, StrobeSender,
};
use crate::worker::{CurveJob, WorkerJob, WorkerOutput};

/// The standard analysis window size, in samples.
pub const WINDOW_SIZE: usize = 2048;

/// The expanded analysis window size for extracting exact bass fundamental frequencies.
pub const BASS_WINDOW_SIZE: usize = WINDOW_SIZE * 4; // 8192 samples

/// Hop size between overlapping frames: half a window.
pub const HOP_SIZE: usize = WINDOW_SIZE / 2; // 1024 samples

/// Frames the DSP produces per second, one per hop: ≈ 43.07 Hz.
pub const HOP_RATE_HZ: f32 = SAMPLE_RATE as f32 / HOP_SIZE as f32;

/// Capacity of the ring buffer between the capture callback and the analysis
/// thread, in samples: ≈ 371 ms, headroom for a scheduling spike.
pub const RING_BUFFER_CAPACITY: usize = WINDOW_SIZE * 8;

/// The sample rate the pipeline is dimensioned for, in Hz. Buffer sizes, windows
/// and every gate's timing derive from it, so it is a constant of the design.
// Plumbing the negotiated rate through is planned; see TODO.md.
pub const SAMPLE_RATE: u32 = 44_100;

/// The consumer half of the audio ring buffer, so callers need not import
/// `ringbuf`.
pub type AudioConsumer = ringbuf::HeapCons<f32>;

/// DC-blocking filter coefficient. For `y[n] = x[n] − x[n−1] + α·y[n−1]` the
/// −3 dB corner is `(1−α)·fs/2π`: 35 Hz, above A0. It costs −4.2 dB at A0's
/// fundamental, −2.4 dB at E1, and −0.4 dB from A2 up.
// Do not raise α to put the corner below A0: the CFAR reference cells sit in this
// band, so the coarse readout worsens (availability 93.3 → 87.4 %, |e| 0.70 →
// 1.85 ¢), and the bass fundamental recovered stays under mask_peaks' −30 dB gate.
// The lever is the filter's order.
// report 0019
const DC_BLOCK_ALPHA: f32 = 0.995;

// ─── Shared CPAL Stream Setup ────────────────────────────────────────────────

/// Opens the default input device at [`SAMPLE_RATE`], DC-blocking every sample
/// into a ring buffer of `capacity` samples. The callback neither allocates nor
/// blocks: a full buffer drops samples. Returns the running stream, the consumer
/// and the negotiated rate in Hz.
///
/// # Errors
/// If there is no input device, or no mono `f32` configuration at [`SAMPLE_RATE`].
pub fn open_input_stream(capacity: usize) -> Result<(cpal::Stream, AudioConsumer, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?;

    eprintln!("Using audio input device: {:?}", device.description()?);

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, SAMPLE_RATE).ok_or_else(|| {
        anyhow!("No mono f32 input configuration supporting {SAMPLE_RATE} Hz was found")
    })?;

    // `try_with_sample_rate`, not `with_sample_rate`, which panics on a range
    // that misses the rate: a guard in case `find_supported_config` loosens.
    let config = supported_config
        .try_with_sample_rate(SAMPLE_RATE)
        .ok_or_else(|| anyhow!("Input configuration does not support {SAMPLE_RATE} Hz"))?;

    let sample_rate_val = config.sample_rate();
    let config: cpal::StreamConfig = config.into();

    eprintln!("Selected sample rate: {} Hz", sample_rate_val);

    let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

    let rb = HeapRb::<f32>::new(capacity);
    let (mut producer, consumer) = rb.split();

    let mut dc_prev_x: f32 = 0.0;
    let mut dc_prev_y: f32 = 0.0;

    let stream = device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                let filtered = dc_block(sample, &mut dc_prev_x, &mut dc_prev_y);

                // Dropping beats blocking: this is the real-time callback.
                let _ = producer.try_push(filtered);
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    Ok((stream, consumer, sample_rate_val))
}

// ─── Standalone Host Extension ───────────────────────────────────────────────

/// Where audio samples come from.
pub enum AudioSource {
    /// The default input device.
    Default,
    /// A ring buffer the caller feeds, such as a plugin host or a test.
    External {
        /// The consumer end of a ring buffer already being fed by an external source.
        consumer: AudioConsumer,
        /// The sample rate of the external audio source in Hz.
        sample_rate: u32,
    },
}

/// A frontend's handle on a running analysis host: its frames, the shared
/// state and every channel. Dropping it stops the analysis thread before the
/// stream.
pub struct HostHandle {
    /// The freshest per-hop frame (lossy). An `Option`, so a frontend can take it.
    pub frame_rx: Option<triple_buffer::Output<FrameOutput>>,
    /// The pipeline's shared atomic state.
    pub pipeline_handle: PipelineHandle,
    /// Worker → UI results (crossing #5): a `KeyMeasurement` per capture, a
    /// `CurveBundle` per recompute.
    pub worker_rx: crossbeam_channel::Receiver<WorkerOutput>,
    /// UI → Worker jobs (crossing #6), sent through the typed methods.
    worker_job_tx: crossbeam_channel::Sender<WorkerJob>,
    /// UI → DSP template updates (crossing #4).
    pub profiles: ProfileSender,
    /// UI → DSP strobe reference updates (crossing #4).
    pub strobe_refs: StrobeSender,
    /// UI → DSP capture-lifecycle commands (crossing #4).
    pub capture_commands: CaptureSender,
    /// Keeps the CPAL stream alive; `None` for an external source.
    _stream: Option<cpal::Stream>,
    thread_handle: Option<JoinHandle<()>>,
}

impl HostHandle {
    /// Enqueues a curve job for the Worker (crossing #6) without blocking.
    /// Returns `false` if the slot is full or the Worker has gone.
    pub fn send_curve_job(&self, job: CurveJob) -> bool {
        self.worker_job_tx.try_send(WorkerJob::Curve(job)).is_ok()
    }

    /// Points capture dumps at `dir`, or nowhere when `None` (crossing #6), from
    /// the Worker's next capture. Returns `false` if the slot is full; unlike a
    /// curve job, a dropped change must be retried, or captures land under the
    /// wrong instrument.
    pub fn send_dump_dir(&self, dir: Option<std::path::PathBuf>) -> bool {
        self.worker_job_tx
            .try_send(WorkerJob::SetDumpDir(dir))
            .is_ok()
    }

    /// Signals the analysis thread to stop and joins it; dropping the handle
    /// does the same.
    pub fn stop(&mut self) {
        self.pipeline_handle
            .atomics
            .shutdown
            .store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            eprintln!("[HOST] Waiting for analysis thread to finish...");
            let _ = handle.join();
            eprintln!("[HOST] Analysis thread finished.");
        }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        if self.thread_handle.is_some() {
            self.stop();
        }
    }
}

// `HostHandle` holds a `JoinHandle` and `cpal::Stream`, which are not `Debug`.
impl std::fmt::Debug for HostHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostHandle")
            .field("has_stream", &self._stream.is_some())
            .field(
                "thread_alive",
                &self
                    .thread_handle
                    .as_ref()
                    .is_some_and(|h| !h.is_finished()),
            )
            .finish()
    }
}

/// Creates an [`AudioPipeline`] and runs it on a new analysis thread fed from
/// `source`, returning the frontend's [`HostHandle`]. `dump_dir` is where capture
/// dumps are written; `None` writes none.
///
/// # Errors
/// If opening the input device fails, which only [`AudioSource::Default`] can.
///
/// # Examples
/// ```no_run
/// use tuner_core::audio::{AudioSource, spawn_analysis_thread};
///
/// let mut handle = spawn_analysis_thread(AudioSource::Default, None).unwrap();
///
/// // Read the freshest visualization frame
/// let frame = handle.frame_rx.as_mut().unwrap().read();
/// println!("RMS: {}", frame.rms_ema);
///
/// // Clean shutdown
/// handle.stop();
/// ```
pub fn spawn_analysis_thread(
    source: AudioSource,
    dump_dir: Option<std::path::PathBuf>,
) -> Result<HostHandle> {
    let (pipeline, ports) = AudioPipeline::new(dump_dir);
    let PipelinePorts {
        handle: pipeline_handle,
        worker_rx,
        worker_job_tx,
        profiles,
        strobe_refs,
        capture_commands,
    } = ports;

    let (stream, consumer, sample_rate) = match source {
        AudioSource::Default => {
            let (stream, consumer, sr) = open_input_stream(RING_BUFFER_CAPACITY)?;
            (Some(stream), consumer, sr)
        }
        AudioSource::External {
            consumer,
            sample_rate,
        } => (None, consumer, sample_rate),
    };

    // The per-hop frame, DSP → UI.
    let (tri_input, tri_output) = triple_buffer::TripleBuffer::new(&FrameOutput::default()).split();

    let thread_atomics = pipeline_handle.atomics.clone();

    let thread_handle = thread::spawn(move || {
        eprintln!("[HOST] Analysis thread started.");

        let mut pipeline = pipeline;
        pipeline.engine.sample_rate = sample_rate;

        let mut consumer = consumer;
        let mut tri_input = tri_input;

        // On the stack, so the loop never allocates.
        let mut pop_buf = [0.0_f32; 512];

        #[cfg(debug_assertions)]
        let mut last_was_silence = true;

        // A start-up delay whose need is untested; TODO.md tracks it.
        std::thread::sleep(std::time::Duration::from_millis(100));

        loop {
            if thread_atomics.shutdown.load(Ordering::Relaxed) {
                eprintln!("[HOST] Received shutdown signal.");
                break;
            }

            let available = consumer.occupied_len().min(pop_buf.len());
            if available > 0 {
                consumer.pop_slice(&mut pop_buf[..available]);

                let frame_output = if let Some(res) = pipeline.push_audio(&pop_buf[..available]) {
                    res
                } else {
                    continue; // Hop boundary not reached yet
                };

                // Lossy by contract: a reader sees only the freshest frame.
                tri_input.write(frame_output.clone());

                #[cfg(debug_assertions)]
                if frame_output.is_silence != last_was_silence {
                    last_was_silence = frame_output.is_silence;
                    if last_was_silence {
                        eprintln!("[GATEKEEPER] → Silence");
                    } else {
                        eprintln!("[GATEKEEPER] → Active");
                    }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        eprintln!("[HOST] Analysis thread exiting.");
    });

    Ok(HostHandle {
        frame_rx: Some(tri_output),
        pipeline_handle,
        worker_rx,
        worker_job_tx,
        profiles,
        strobe_refs,
        capture_commands,
        _stream: stream,
        thread_handle: Some(thread_handle),
    })
}

// ─── DSP Utilities ───────────────────────────────────────────────────────────

/// One step of the DC-blocking filter ([`DC_BLOCK_ALPHA`]). `prev_x` and `prev_y`
/// carry its state and start at `0.0`.
pub(crate) fn dc_block(sample: f32, prev_x: &mut f32, prev_y: &mut f32) -> f32 {
    let y = sample - *prev_x + DC_BLOCK_ALPHA * *prev_y;
    *prev_x = sample;
    *prev_y = y;
    y
}

/// The mono `f32` configuration range covering `target_rate` whose bounds sit
/// closest to it, which prefers a dedicated range over a catch-all. Mono because
/// one filter state is carried per stream; the exact rate because every buffer
/// and timing is dimensioned for it. `None` if no range qualifies.
pub(crate) fn find_supported_config(
    configs: Vec<SupportedStreamConfigRange>,
    target_rate: u32,
) -> Option<SupportedStreamConfigRange> {
    configs
        .into_iter()
        .filter(|c| {
            c.channels() == 1
                && c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate() <= target_rate
                && target_rate <= c.max_sample_rate()
        })
        .min_by_key(|c| {
            let min_diff = (c.min_sample_rate() as i32 - target_rate as i32).abs();
            let max_diff = (c.max_sample_rate() as i32 - target_rate as i32).abs();
            min_diff.min(max_diff)
        })
}
