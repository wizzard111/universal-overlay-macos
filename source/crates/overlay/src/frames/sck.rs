//! macOS frame timing without injection, via ScreenCaptureKit.
//!
//! macOS has no API to observe another app's present calls, but the window
//! server knows exactly when a window's content changes. We capture the
//! frontmost app's main window at 16×16 px (the GPU downscales; we never
//! touch the pixels) and count frames whose status is `Complete` — the
//! window server marks unchanged frames as `Idle`, so each Complete frame is
//! one new frame the game put on screen. Timestamps are the frames'
//! presentation times on the host clock (mach_absolute_time).
//!
//! Needs the "Screen Recording" permission (asked once).

use super::{best_stream, Frame, Status, Stream};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamFrameInfoStatus,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use std::ffi::c_void;
use std::sync::Mutex;

pub const LATENCY_S: f64 = 0.0;

/// Host clock (= mach_absolute_time) in ns; matches CMSampleBuffer PTS.
pub fn now() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

pub fn freq() -> f64 {
    1e9
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(a: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
    fn CFDictionaryGetValue(d: *const c_void, k: *const c_void) -> *const c_void;
    fn CFNumberGetValue(n: *const c_void, ty: i32, out: *mut c_void) -> u8;
}

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetSampleAttachmentsArray(sb: *const c_void, create: u8) -> *const c_void;
    fn CMSampleBufferGetPresentationTimeStamp(sb: *const c_void) -> CMTime;
}

struct State {
    stream: Stream,
    keep: i64,
    target: u32,
    active: Option<Active>,
    /// When we last asked ScreenCaptureKit for the target's window.
    last_attempt: i64,
    /// When the stream last delivered anything (new or idle frame).
    last_sample: i64,
}

/// Retry finding the game window this often while nothing is captured
/// (the game was focused before its window appeared).
const RETRY_NS: i64 = 2_000_000_000;
/// A stream that delivered nothing for this long is presumed dead (the
/// window was recreated, e.g. on a resolution or full-screen switch).
const STALE_NS: i64 = 6_000_000_000;

struct Active {
    stream: Retained<SCStream>,
    _sink: Retained<FrameSink>,
    _queue: DispatchRetained<DispatchQueue>,
}

// SCStream is thread-safe to stop from any thread.
unsafe impl Send for Active {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// SCFrameStatus of a sample buffer (0 = Complete).
unsafe fn frame_status(sb: &CMSampleBuffer) -> Option<i64> {
    let arr = CMSampleBufferGetSampleAttachmentsArray(sb as *const _ as *const c_void, 0);
    if arr.is_null() || CFArrayGetCount(arr) < 1 {
        return None;
    }
    let dict = CFArrayGetValueAtIndex(arr, 0);
    let key = SCStreamFrameInfoStatus as *const _ as *const c_void;
    let v = CFDictionaryGetValue(dict, key);
    if v.is_null() {
        return None;
    }
    let mut out: i64 = -1;
    (CFNumberGetValue(
        v,
        4, /* kCFNumberSInt64Type */
        &mut out as *mut _ as *mut c_void,
    ) != 0)
        .then_some(out)
}

define_class!(
    /// Receives captured frames on a private dispatch queue.
    #[unsafe(super(NSObject))]
    #[name = "UniversalOverlayFrameSink"]
    struct FrameSink;

    unsafe impl NSObjectProtocol for FrameSink {}

    unsafe impl SCStreamOutput for FrameSink {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sb: &CMSampleBuffer, ty: SCStreamOutputType) {
            if ty != SCStreamOutputType::Screen {
                return;
            }
            unsafe {
                let status = frame_status(sb);
                if let Some(st) = state().as_mut() {
                    st.last_sample = now();
                }
                if status != Some(0) {
                    return; // Idle / Blank / Suspended: nothing new on screen
                }
                let t = CMSampleBufferGetPresentationTimeStamp(sb as *const _ as *const c_void);
                if t.timescale <= 0 {
                    return;
                }
                let ns = (t.value as i128 * 1_000_000_000 / t.timescale as i128) as i64;
                if let Some(st) = state().as_mut() {
                    let keep = st.keep;
                    st.stream.push(ns, 1e9, keep);
                }
            }
        }
    }
);

impl FrameSink {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn stop_active(a: Active) {
    unsafe { a.stream.stopCaptureWithCompletionHandler(None) };
}

/// Pick the biggest normal on-screen window of `pid` and start capturing it.
fn capture_pid(pid: u32) {
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, _err: *mut NSError| {
            let Some(content) = (unsafe { Retained::retain(content) }) else {
                return;
            };
            let windows = unsafe { content.windows() };
            let mut best: Option<(f64, Retained<SCWindow>)> = None;
            for w in windows.iter() {
                unsafe {
                    let owner = w.owningApplication().map(|a| a.processID() as u32);
                    if owner != Some(pid) || w.windowLayer() != 0 || !w.isOnScreen() {
                        continue;
                    }
                    let f = w.frame();
                    let area = f.size.width * f.size.height;
                    if area > 64.0 * 64.0 && best.as_ref().is_none_or(|(a, _)| area > *a) {
                        best = Some((area, w.clone()));
                    }
                }
            }
            let Some((_, window)) = best else { return };
            unsafe {
                let filter = SCContentFilter::initWithDesktopIndependentWindow(
                    SCContentFilter::alloc(),
                    &window,
                );
                let cfg = SCStreamConfiguration::new();
                cfg.setWidth(16);
                cfg.setHeight(16);
                cfg.setShowsCursor(false);
                cfg.setQueueDepth(3);
                // Deliver every new frame, up to the display refresh rate.
                cfg.setMinimumFrameInterval(CMTime {
                    value: 1,
                    timescale: 1000,
                    flags: CMTimeFlags::Valid,
                    epoch: 0,
                });
                let stream = SCStream::initWithFilter_configuration_delegate(
                    SCStream::alloc(),
                    &filter,
                    &cfg,
                    None,
                );
                let sink = FrameSink::new();
                let queue = DispatchQueue::new("universal-overlay.frames", None);
                let out = ProtocolObject::from_ref(&*sink);
                if stream
                    .addStreamOutput_type_sampleHandlerQueue_error(
                        out,
                        SCStreamOutputType::Screen,
                        Some(&queue),
                    )
                    .is_err()
                {
                    return;
                }
                stream.startCaptureWithCompletionHandler(None);
                let mut g = state();
                let Some(st) = g.as_mut() else {
                    stream.stopCaptureWithCompletionHandler(None);
                    return;
                };
                if st.target != pid {
                    stream.stopCaptureWithCompletionHandler(None); // target changed meanwhile
                    return;
                }
                if let Some(old) = st.active.replace(Active {
                    stream,
                    _sink: sink,
                    _queue: queue,
                }) {
                    stop_active(old);
                }
            }
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &handler,
        );
    }
}

pub struct Session;

impl Session {
    pub fn start(history_s: f64) -> Result<Session, Status> {
        if !unsafe { CGPreflightScreenCaptureAccess() } {
            unsafe { CGRequestScreenCaptureAccess() }; // shows the system prompt once
            return Err(Status::NoPermission);
        }
        *state() = Some(State {
            stream: Stream::new(1),
            keep: (history_s * 1e9) as i64,
            target: 0,
            active: None,
            last_attempt: 0,
            last_sample: 0,
        });
        Ok(Session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(a) = state().take().and_then(|s| s.active) {
            stop_active(a);
        }
    }
}

pub fn set_target(pid: u32) {
    let start = {
        let mut g = state();
        let Some(st) = g.as_mut() else { return };
        let t = now();
        if st.target != pid {
            st.target = pid;
            st.stream = Stream::new(1);
            if let Some(a) = st.active.take() {
                stop_active(a);
            }
            true
        } else if pid != 0
            && t - st.last_attempt > RETRY_NS
            && (st.active.is_none() || t - st.last_sample > STALE_NS)
        {
            if let Some(a) = st.active.take() {
                stop_active(a);
            }
            true
        } else {
            false
        }
    };
    if start && pid != 0 {
        if let Some(st) = state().as_mut() {
            st.last_attempt = now();
            st.last_sample = st.last_attempt;
        }
        capture_pid(pid);
    }
}

pub fn set_history_seconds(s: f64) {
    if let Some(st) = state().as_mut() {
        st.keep = (s * 1e9) as i64;
    }
}

pub fn snapshot(t: super::Tail) -> Option<Vec<Frame>> {
    let g = state();
    let st = g.as_ref()?;
    best_stream(std::iter::once(&st.stream), now(), 1e9).map(|s| super::tail(&s.frames, 1e9, t))
}
