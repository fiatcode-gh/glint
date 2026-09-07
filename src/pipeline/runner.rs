//! Running a pipeline description and watching what it pushes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gstreamer as gst;
use gstreamer::prelude::*;

/// Long enough for a live source to drain, short enough that a wedged
/// pipeline does not hang its caller. Only the bound's existence is a
/// contract; the number is not.
const BUS_TIMEOUT: gst::ClockTime = gst::ClockTime::from_seconds(5);

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("the pipeline description could not be built: {0}")]
    Parse(String),
    #[error("the pipeline failed: {0}")]
    Pipeline(String),
    #[error("the pipeline did not reach the end of its stream within {0} seconds")]
    Timeout(u64),
    #[error("the pipeline has no element named {0}")]
    MissingElement(&'static str),
}

/// Keeps the failing element's own words: "internal data stream error" alone
/// has never helped anyone find the element that produced it.
fn bus_failure(message: &gst::Message) -> RunnerError {
    match message.view() {
        gst::MessageView::Error(error) => RunnerError::Pipeline(format!(
            "{}: {}",
            error
                .src()
                .map(|src| src.path_string().to_string())
                .unwrap_or_else(|| "pipeline".to_string()),
            error.error()
        )),
        _ => RunnerError::Pipeline("the pipeline failed without a message".to_string()),
    }
}

/// One second's worth of the pipeline, as the daemon reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub bytes_per_second: u64,
    pub dropped_frames: u64,
}

pub struct Runner {
    pipeline: gst::Pipeline,
    bytes: Arc<AtomicU64>,
    rate: gst::Element,
}

impl Runner {
    pub fn start(description: &str) -> Result<Runner, RunnerError> {
        gst::init().ok();
        let pipeline = gst::parse::launch(description)
            .map_err(|e| RunnerError::Parse(e.to_string()))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| RunnerError::Parse("the description is not a pipeline".to_string()))?;

        let rate = pipeline
            .by_name("rate")
            .ok_or(RunnerError::MissingElement("rate"))?;
        let mux = pipeline
            .by_name("mux")
            .ok_or(RunnerError::MissingElement("mux"))?;
        let src = mux
            .static_pad("src")
            .ok_or(RunnerError::MissingElement("mux"))?;

        let bytes = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&bytes);
        // Buffer lists are counted as well as buffers: a muxer is free to push
        // either, and counting only one of them would undercount silently.
        src.add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
            move |_, info| {
                let pushed = match info.data.as_ref() {
                    Some(gst::PadProbeData::Buffer(buffer)) => buffer.size() as u64,
                    Some(gst::PadProbeData::BufferList(list)) => {
                        list.iter().map(|buffer| buffer.size() as u64).sum()
                    }
                    _ => 0,
                };
                counter.fetch_add(pushed, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            },
        );

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| RunnerError::Pipeline(e.to_string()))?;
        Ok(Runner {
            pipeline,
            bytes,
            rate,
        })
    }

    /// Swaps the byte counter rather than reading it, so each sample reports
    /// the window since the previous one: a caller polling once a second gets
    /// a rate without keeping its own arithmetic.
    ///
    /// The two fields are not the same KIND of number. `dropped_frames` is
    /// cumulative, because videorate owns that counter and only ever counts
    /// up, so a caller printing them side by side should say which is which.
    pub fn sample(&self) -> Stats {
        Stats {
            bytes_per_second: self.bytes.swap(0, Ordering::Relaxed),
            dropped_frames: self.rate.property::<u64>("drop"),
        }
    }

    /// Sending EOS before Null is a contract, not tidiness: a muxed file cut
    /// off at Null never gets its closing tables written and will not play.
    ///
    /// Takes `&self` so the byte total stays readable after the stream ends —
    /// which is the only moment it can be compared against the finished file.
    pub fn stop(&self) -> Result<(), RunnerError> {
        let bus = self
            .pipeline
            .bus()
            .ok_or(RunnerError::MissingElement("bus"))?;

        // A pipeline that has already failed cannot be drained: its streaming
        // thread is stopped, and sending EOS into it never returns — measured
        // here, it hung indefinitely rather than timing out. A stream that has
        // already ended has nothing left to drain either. Both are collected
        // before the drain is attempted, and in one call: pop_filtered
        // DISCARDS whatever it does not match, so asking for the error alone
        // would throw away an end-of-stream that had already arrived.
        if let Some(pending) = bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos]) {
            let outcome = match pending.view() {
                gst::MessageView::Error(_) => Err(bus_failure(&pending)),
                _ => Ok(()),
            };
            let _ = self.pipeline.set_state(gst::State::Null);
            return outcome;
        }

        self.pipeline.send_event(gst::event::Eos::new());
        let outcome = match bus.timed_pop_filtered(
            BUS_TIMEOUT,
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) {
            Some(message) => match message.view() {
                gst::MessageView::Error(_) => Err(bus_failure(&message)),
                _ => Ok(()),
            },
            None => Err(RunnerError::Timeout(BUS_TIMEOUT.seconds())),
        };
        let _ = self.pipeline.set_state(gst::State::Null);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::store::tests::TempDir;

    /// `is-live=true` and no `num-buffers`, deliberately. A bounded source
    /// ends by itself, so `stop` would find the end-of-stream already waiting
    /// and never need to send one — which would quietly defuse the very
    /// contract these tests exist to pin. A live, unbounded source is also
    /// what the portal actually hands the pipeline.
    ///
    /// x264enc rather than the detected encoder: a software encoder needs no
    /// GPU render node, so the result does not depend on what the test host
    /// can open. Every description here drives videotestsrc — never a portal,
    /// a real screen, or an audio device.
    fn description(path: &std::path::Path) -> String {
        format!(
            "videotestsrc is-live=true ! \
video/x-raw,format=I420,width=320,height=240,framerate=30/1 ! \
videorate name=rate ! x264enc tune=zerolatency ! h264parse ! \
mpegtsmux name=mux ! filesink location={}",
            path.display()
        )
    }

    fn recording(tag: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new(tag);
        std::fs::create_dir_all(dir.path()).unwrap();
        let file = dir.path().join("out.ts");
        (dir, file)
    }

    /// `stop` injects end-of-stream at the source, so calling it the instant
    /// the pipeline rolls truncates the recording to nothing. Real callers
    /// record first; these tests do the same.
    fn let_it_record() {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }

    #[test]
    fn a_clean_stop_leaves_a_non_empty_file() {
        // arrange
        let (_dir, file) = recording("runner-clean-stop");
        let runner = Runner::start(&description(&file)).unwrap();
        let_it_record();
        // act
        let stopped = runner.stop();
        // assert
        assert!(stopped.is_ok(), "got: {stopped:?}");
        assert!(std::fs::metadata(&file).unwrap().len() > 0);
    }

    #[test]
    fn stopping_twice_times_out_rather_than_reporting_a_second_clean_stop() {
        // Measured, not predicted — the first guess here was that a second
        // stop would be idempotent, and it is not. The first stop consumes the
        // end-of-stream message, so the second finds an empty bus and waits
        // out the bound. Callers stop once; this pins what the second call
        // really does so it cannot drift unnoticed. It costs the bus timeout
        // in suite time, which is why it is the only test that pays it.
        // arrange
        let (_dir, file) = recording("runner-second-stop");
        let runner = Runner::start(&description(&file)).unwrap();
        let_it_record();
        // act
        let first = runner.stop();
        let second = runner.stop();
        // assert
        assert!(first.is_ok(), "got: {first:?}");
        assert!(
            matches!(second, Err(RunnerError::Timeout(_))),
            "got: {second:?}"
        );
    }

    #[test]
    fn the_probe_counts_exactly_the_bytes_the_file_receives() {
        // Decision D12. The identity this probe is supposed to satisfy is not
        // "enough bytes" but "the bytes filesink wrote", and the finished file
        // is the only honest oracle for that — a floor picked from the source
        // caps would be a guess, because lossy encoding has no lower bound
        // derivable from its input. Equality also catches over-counting,
        // double-counting and a probe on the wrong pad, which a floor cannot.
        // arrange
        let (_dir, file) = recording("runner-byte-identity");
        let runner = Runner::start(&description(&file)).unwrap();
        let_it_record();
        runner.stop().unwrap();
        // act — the first sample since start carries the whole run.
        let counted = runner.sample().bytes_per_second;
        let written = std::fs::metadata(&file).unwrap().len();
        // assert
        assert_eq!(counted, written, "counted {counted}, file holds {written}");
    }

    #[test]
    fn a_sample_after_the_stream_has_ended_reports_no_new_bytes() {
        // arrange
        let (_dir, file) = recording("runner-sample-after-stop");
        let runner = Runner::start(&description(&file)).unwrap();
        let_it_record();
        runner.stop().unwrap();
        // act — the first sample drains the window, so the second sees a
        // stopped pipeline pushing nothing.
        let drained = runner.sample();
        let after = runner.sample();
        // assert
        assert!(drained.bytes_per_second > 0, "got: {drained:?}");
        assert_eq!(after.bytes_per_second, 0, "got: {after:?}");
    }

    #[test]
    fn a_steady_source_at_a_matched_rate_drops_no_frames() {
        // videorate is in the chain for its counter, not to reshape timing: at
        // matched input and output rates it passes buffers straight through. A
        // non-zero count here would mean the capture chain is altering timing,
        // which is a finding rather than a nuisance.
        // arrange
        let (_dir, file) = recording("runner-no-drops");
        let runner = Runner::start(&description(&file)).unwrap();
        let_it_record();
        // act
        let sampled = runner.sample();
        runner.stop().unwrap();
        // assert
        assert_eq!(sampled.dropped_frames, 0, "got: {sampled:?}");
    }

    #[test]
    fn a_runtime_failure_surfaces_on_the_bus_as_a_pipeline_error() {
        // arrange — identity posts a real element error mid-flow, which is
        // the bus path. A caps mismatch would not reach it: parse::launch
        // refuses to link incompatible elements before anything runs.
        let (_dir, file) = recording("runner-bus-error");
        let failing = format!(
            "videotestsrc is-live=true ! videorate name=rate ! \
identity error-after=5 ! x264enc ! h264parse ! mpegtsmux name=mux ! \
filesink location={}",
            file.display()
        );
        let runner = Runner::start(&failing).unwrap();
        let_it_record();
        // act
        let stopped = runner.stop();
        // assert
        assert!(
            matches!(stopped, Err(RunnerError::Pipeline(_))),
            "got: {stopped:?}"
        );
    }

    #[test]
    fn a_description_that_cannot_link_is_refused_at_start() {
        // arrange — videotestsrc emits video caps audioconvert cannot take.
        let unlinkable = "videotestsrc ! videorate name=rate ! audioconvert ! \
mpegtsmux name=mux ! fakesink";
        // act
        let started = Runner::start(unlinkable);
        // assert
        assert!(
            matches!(started, Err(RunnerError::Parse(_))),
            "got: {:?}",
            started.map(|_| ())
        );
    }

    #[test]
    fn a_description_without_the_named_videorate_is_refused_at_start() {
        // The runner polls `rate` for the drop count, so a description that
        // never names it cannot deliver Task 22's stats at all.
        // arrange
        let (_dir, file) = recording("runner-no-rate");
        let without_rate = format!(
            "videotestsrc is-live=true ! x264enc ! h264parse ! \
mpegtsmux name=mux ! filesink location={}",
            file.display()
        );
        // act
        let started = Runner::start(&without_rate);
        // assert
        assert!(
            matches!(started, Err(RunnerError::MissingElement("rate"))),
            "got: {:?}",
            started.map(|_| ())
        );
    }
}
