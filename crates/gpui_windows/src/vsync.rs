use std::{
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use gpui_util::ResultExt;
use smallvec::SmallVec;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DWM_TIMING_INFO, DwmFlush, DwmGetCompositionTimingInfo},
    System::Performance::QueryPerformanceFrequency,
};

use crate::SafeHwnd;

#[derive(Clone)]
pub(crate) struct FrameRequestSender(mpsc::Sender<FrameRequest>);

pub(crate) struct FrameRequestReceiver {
    receiver: mpsc::Receiver<FrameRequest>,
    first_request: Option<FrameRequest>,
}

#[derive(Clone)]
pub(crate) struct FrameRequester {
    hwnd: SafeHwnd,
    state: Arc<FrameRequestState>,
    sender: FrameRequestSender,
}

struct FrameRequestState {
    queued: AtomicBool,
    closed: AtomicBool,
}

pub(crate) struct FrameRequest {
    hwnd: SafeHwnd,
    state: Arc<FrameRequestState>,
}

pub(crate) fn frame_request_channel() -> (FrameRequestSender, FrameRequestReceiver) {
    let (sender, receiver) = mpsc::channel();
    (
        FrameRequestSender(sender),
        FrameRequestReceiver {
            receiver,
            first_request: None,
        },
    )
}

impl FrameRequestSender {
    pub(crate) fn requester_for(&self, hwnd: SafeHwnd) -> FrameRequester {
        FrameRequester {
            hwnd,
            state: Arc::new(FrameRequestState {
                queued: AtomicBool::new(false),
                closed: AtomicBool::new(false),
            }),
            sender: self.clone(),
        }
    }
}

impl FrameRequester {
    pub(crate) fn request(&self) {
        if self.state.closed.load(Ordering::Acquire)
            || self.state.queued.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let request = FrameRequest {
            hwnd: self.hwnd,
            state: self.state.clone(),
        };
        if self.sender.0.send(request).is_err() {
            self.state.queued.store(false, Ordering::Release);
        }
    }

    pub(crate) fn close(&self) {
        self.state.closed.store(true, Ordering::Release);
    }
}

impl FrameRequest {
    pub(crate) fn hwnd_if_open(&self) -> Option<SafeHwnd> {
        (!self.state.closed.load(Ordering::Acquire)).then_some(self.hwnd)
    }
}

impl FrameRequestReceiver {
    pub(crate) fn wait(&mut self) -> bool {
        match self.receiver.recv() {
            Ok(request) => {
                self.first_request = Some(request);
                true
            }
            Err(_) => false,
        }
    }

    pub(crate) fn take_requested_windows(&mut self) -> SmallVec<[FrameRequest; 4]> {
        self.first_request
            .take()
            .into_iter()
            .chain(self.receiver.try_iter())
            .filter(|request| {
                request.state.queued.store(false, Ordering::Release);
                !request.state.closed.load(Ordering::Acquire)
            })
            .collect()
    }
}

static QPC_TICKS_PER_SECOND: LazyLock<u64> = LazyLock::new(|| {
    let mut frequency = 0;
    // On systems that run Windows XP or later, the function will always succeed and
    // will thus never return zero.
    unsafe { QueryPerformanceFrequency(&mut frequency).unwrap() };
    frequency as u64
});

const VSYNC_INTERVAL_THRESHOLD: Duration = Duration::from_millis(1);
const DEFAULT_VSYNC_INTERVAL: Duration = Duration::from_micros(16_666); // ~60Hz

pub(crate) struct VSyncProvider {
    interval: Duration,
    f: Box<dyn Fn() -> bool>,
}

impl VSyncProvider {
    pub(crate) fn new() -> Self {
        let interval = get_dwm_interval()
            .context("Failed to get DWM interval")
            .log_err()
            .unwrap_or(DEFAULT_VSYNC_INTERVAL);
        let f = Box::new(|| unsafe { DwmFlush().is_ok() });
        Self { interval, f }
    }

    pub(crate) fn wait_for_vsync(&self) {
        let vsync_start = Instant::now();
        let wait_succeeded = (self.f)();
        let elapsed = vsync_start.elapsed();
        // DwmFlush and DCompositionWaitForCompositorClock returns very early
        // instead of waiting until vblank when the monitor goes to sleep or is
        // unplugged (nothing to present due to desktop occlusion). We use 1ms as
        // a threshold for the duration of the wait functions and fallback to
        // Sleep() if it returns before that. This could happen during normal
        // operation for the first call after the vsync thread becomes non-idle,
        // but it shouldn't happen often.
        if !wait_succeeded || elapsed < VSYNC_INTERVAL_THRESHOLD {
            log::trace!("VSyncProvider::wait_for_vsync() took less time than expected");
            std::thread::sleep(self.interval);
        }
    }
}

fn get_dwm_interval() -> Result<Duration> {
    let mut timing_info = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing_info) }?;
    let interval = retrieve_duration(timing_info.qpcRefreshPeriod, *QPC_TICKS_PER_SECOND);
    // Check for interval values that are impossibly low. A 29 microsecond
    // interval was seen (from a qpcRefreshPeriod of 60).
    if interval < VSYNC_INTERVAL_THRESHOLD {
        Ok(retrieve_duration(
            timing_info.rateRefresh.uiDenominator as u64,
            timing_info.rateRefresh.uiNumerator as u64,
        ))
    } else {
        Ok(interval)
    }
}

#[inline]
fn retrieve_duration(counts: u64, ticks_per_second: u64) -> Duration {
    let ticks_per_microsecond = ticks_per_second / 1_000_000;
    Duration::from_micros(counts / ticks_per_microsecond)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hwnd(value: isize) -> SafeHwnd {
        HWND(value as _).into()
    }

    fn raw(request: &FrameRequest) -> Option<HWND> {
        request.hwnd_if_open().map(|hwnd| hwnd.as_raw())
    }

    #[test]
    fn receiver_blocks_until_a_window_asks_for_a_frame() {
        let (sender, mut receiver) = frame_request_channel();
        let requester = sender.requester_for(test_hwnd(1));
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let woke = receiver.wait();
            done_sender
                .send((woke, receiver.take_requested_windows().len()))
                .unwrap();
        });

        assert_eq!(
            done_receiver.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        requester.request();
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_secs(5)),
            Ok((true, 1))
        );
    }

    #[test]
    fn requests_coalesce_until_the_vsync_thread_takes_them() {
        let (sender, mut receiver) = frame_request_channel();
        let requester = sender.requester_for(test_hwnd(1));

        requester.request();
        requester.request();
        assert!(receiver.wait());
        let requests = receiver.take_requested_windows();
        assert_eq!(requests.len(), 1);
        assert_eq!(raw(&requests[0]), Some(test_hwnd(1).as_raw()));

        requester.request();
        assert!(receiver.wait());
        assert_eq!(receiver.take_requested_windows().len(), 1);
    }

    #[test]
    fn one_vsync_serves_every_window_that_asked() {
        let (sender, mut receiver) = frame_request_channel();
        let first = sender.requester_for(test_hwnd(1));
        let second = sender.requester_for(test_hwnd(2));

        first.request();
        assert!(receiver.wait());
        second.request();
        let requests = receiver.take_requested_windows();
        assert_eq!(requests.len(), 2);
        assert_eq!(raw(&requests[0]), Some(test_hwnd(1).as_raw()));
        assert_eq!(raw(&requests[1]), Some(test_hwnd(2).as_raw()));
    }

    #[test]
    fn closed_windows_drop_pending_and_new_requests() {
        let (sender, mut receiver) = frame_request_channel();
        let requester = sender.requester_for(test_hwnd(1));

        requester.request();
        assert!(receiver.wait());
        requester.close();
        assert!(receiver.take_requested_windows().is_empty());

        requester.request();
        drop(requester);
        drop(sender);
        assert!(!receiver.wait());
    }

    #[test]
    fn closing_after_take_cancels_the_redraw() {
        let (sender, mut receiver) = frame_request_channel();
        let requester = sender.requester_for(test_hwnd(1));

        requester.request();
        assert!(receiver.wait());
        let requests = receiver.take_requested_windows();
        requester.close();
        assert_eq!(raw(&requests[0]), None);
    }

    #[test]
    fn a_reused_hwnd_does_not_inherit_a_closed_request() {
        let (sender, mut receiver) = frame_request_channel();
        let closed = sender.requester_for(test_hwnd(1));
        let reopened = sender.requester_for(test_hwnd(1));

        closed.request();
        assert!(receiver.wait());
        closed.close();
        reopened.request();
        let requests = receiver.take_requested_windows();
        assert_eq!(requests.len(), 1);
        assert!(Arc::ptr_eq(&requests[0].state, &reopened.state));
    }
}
