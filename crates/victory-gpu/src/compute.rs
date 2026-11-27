//! The title's compute queue on the card's compute engine
//! (`VICTORY_ASYNC_COMPUTE`; off by default).
//!
//! Without it every host list goes to the one direct queue, the title's
//! compute-queue submissions (queue kind 2, `{COMPUTE, 0, pipe 1, queue
//! 0}`) among them. With it the host sides of the title's compute lists
//! (list type 2) are COMPUTE lists, and their submissions run on a host
//! COMPUTE queue: the engine Task Manager calls "Compute".
//!
//! What is synchronised, and with what (all on the card, no CPU holds):
//!
//! - The submission fence stays one line of numbers (`Host::submitted`):
//!   value N on it still means "submission N and every one before it has
//!   run", which the flights, the relay, the copy-back and the copy lanes
//!   (`crate::queue2`) all rely on. A compute submission's number is
//!   signalled by the compute queue; a later direct submission's number is
//!   signalled by the direct queue only behind a `Wait` for the last
//!   compute number ([`Compute::before_signal`]).
//! - A compute submission's lists start behind everything sent to the
//!   direct queue before them: the direct queue signals [`Compute::pre`]
//!   and the compute queue waits for it. The direct-type lists of the
//!   submission (texture preludes, textures uploaded ahead, our card
//!   copies' sends) run on the direct queue in their place, with a wait each
//!   way between them and the compute lists ([`Compute::send`]).
//! - Mode 1 (`VICTORY_ASYNC_COMPUTE=1`): a direct submission after a
//!   compute one waits for it before its lists, so no two queues ever
//!   touch a resource at once (the debug layer's rule for writes on two
//!   queues, `crate::queue2`). The work is the same, on another engine.
//! - Mode 2 (`=2`, a measurement): the wait stands before the direct
//!   submission's signal only, so its lists run beside the compute work.
//!   Nothing checks that they touch different resources: what this gains
//!   is the most that tracking those would.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;

/// `VICTORY_ASYNC_COMPUTE`: 0 (default) off, 1 on, 2 on with direct work
/// beside the compute work (see the module text).
pub(crate) fn mode() -> u32 {
    static MODE: OnceLock<u32> = OnceLock::new();
    *MODE.get_or_init(|| {
        let mode = std::env::var("VICTORY_ASYNC_COMPUTE")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0)
            .min(2);
        if mode > 0 {
            tracing::info!(mode, "the title's compute queue runs on a host compute queue (VICTORY_ASYNC_COMPUTE)");
        }
        mode
    })
}

/// Whether the title's compute lists record into host COMPUTE lists: on,
/// and the host compute queue could be made.
pub(crate) fn on() -> bool {
    mode() > 0 && crate::host::host().is_some_and(|h| get(&h.device).is_some())
}

/// The host compute queue and its fences.
pub(crate) struct Compute {
    pub(crate) queue: ID3D12CommandQueue,
    /// Signalled by the direct queue, waited for by the compute queue.
    pre: ID3D12Fence,
    pre_value: AtomicU64,
    /// Signalled by the compute queue inside a submission, waited for by
    /// the direct queue (a direct-type list behind a compute one).
    mid: ID3D12Fence,
    mid_value: AtomicU64,
    /// The last submission number the compute queue signals, and the last
    /// one the direct queue has been told to wait for.
    last: AtomicU64,
    waited: AtomicU64,
    /// Per frame line: compute submissions, compute lists, waits of the
    /// direct queue for the compute queue (before lists, before signals).
    pub(crate) stats: [AtomicU64; 4],
}

// SAFETY: D3D12 queues and fences are free-threaded.
unsafe impl Send for Compute {}
unsafe impl Sync for Compute {}

static COMPUTE: OnceLock<Option<Compute>> = OnceLock::new();

/// The compute queue, made at the first ask (`None` when off or if it
/// cannot be made).
pub(crate) fn get(device: &ID3D12Device) -> Option<&'static Compute> {
    if mode() == 0 {
        return None;
    }
    COMPUTE
        .get_or_init(|| {
            // SAFETY: plain object creation.
            let made = unsafe {
                (|| -> windows::core::Result<Compute> {
                    let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                        Type: D3D12_COMMAND_LIST_TYPE_COMPUTE,
                        ..Default::default()
                    })?;
                    let _ = queue.SetName(windows::core::w!("victory title compute queue"));
                    Ok(Compute {
                        queue,
                        pre: device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?,
                        pre_value: AtomicU64::new(0),
                        mid: device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?,
                        mid_value: AtomicU64::new(0),
                        last: AtomicU64::new(0),
                        waited: AtomicU64::new(0),
                        stats: Default::default(),
                    })
                })()
            };
            match made {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(error = %e, "no host compute queue: the title's compute lists run on the direct queue");
                    None
                }
            }
        })
        .as_ref()
}

/// The compute queue if it has been made (no device at hand).
pub(crate) fn made() -> Option<&'static Compute> {
    COMPUTE.get().and_then(Option::as_ref)
}

/// Which queue a list of a submission runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum On {
    Direct,
    Compute,
}

impl Compute {
    /// Before the direct queue signals submission number `value` on
    /// `fence` (the submission fence): it waits for the last compute
    /// submission below `value` it has not waited for yet, so the number
    /// still means every submission up to it has run.
    pub(crate) fn before_signal(&self, direct: &ID3D12CommandQueue, fence: &ID3D12Fence, value: u64) {
        let last = self.last.load(Ordering::Acquire);
        if last > self.waited.load(Ordering::Relaxed) && last < value {
            // SAFETY: our queue and fence; the compute queue signals `last`
            // behind work already sent, so the wait ends.
            if unsafe { direct.Wait(fence, last) }.is_ok() {
                self.waited.store(last, Ordering::Relaxed);
                self.stats[3].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Before the direct queue runs lists of a submission: in mode 1 it
    /// waits for every compute submission sent so far.
    pub(crate) fn before_direct(&self, direct: &ID3D12CommandQueue, fence: &ID3D12Fence) {
        if mode() != 1 {
            return;
        }
        let last = self.last.load(Ordering::Acquire);
        if last > self.waited.load(Ordering::Relaxed) {
            // SAFETY: as in `before_signal`.
            if unsafe { direct.Wait(fence, last) }.is_ok() {
                self.waited.store(last, Ordering::Relaxed);
                self.stats[2].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Sends a submission's lists, each on its queue in the order given,
    /// and signals `value` on `fence` behind them all. With no compute list
    /// among them this is the direct queue's plain execute and signal (with
    /// the waits for the compute queue that the fence's order needs).
    ///
    /// The caller holds the submission lock.
    pub(crate) fn send(
        &self,
        direct: &ID3D12CommandQueue,
        fence: &ID3D12Fence,
        lists: &[(On, Option<ID3D12CommandList>)],
        value: u64,
    ) -> windows::core::Result<()> {
        let computes = lists.iter().filter(|(on, _)| *on == On::Compute).count();
        if computes == 0 {
            self.before_direct(direct, fence);
            let plain: Vec<Option<ID3D12CommandList>> = lists.iter().map(|(_, l)| l.clone()).collect();
            // SAFETY: closed direct lists; our queue and fence.
            unsafe {
                if !plain.is_empty() {
                    direct.ExecuteCommandLists(&plain);
                }
                self.before_signal(direct, fence, value);
                return direct.Signal(fence, value);
            }
        }
        self.stats[0].fetch_add(1, Ordering::Relaxed);
        self.stats[1].fetch_add(computes as u64, Ordering::Relaxed);
        // Behind every compute submission sent before (mode 1), the direct
        // lists of this one; then the compute queue stands behind all of
        // the direct queue's work so far.
        let mut at = 0;
        let mut first = true;
        // SAFETY: closed lists of the matching types; our queues and fences.
        unsafe {
            while at < lists.len() {
                let on = lists[at].0;
                let end = lists[at..].iter().position(|(o, _)| *o != on).map_or(lists.len(), |n| at + n);
                let group: Vec<Option<ID3D12CommandList>> = lists[at..end].iter().map(|(_, l)| l.clone()).collect();
                match on {
                    On::Direct => {
                        if first {
                            self.before_direct(direct, fence);
                        } else {
                            // Behind the compute lists before them.
                            let mid = self.mid_value.fetch_add(1, Ordering::Relaxed) + 1;
                            self.queue.Signal(&self.mid, mid)?;
                            direct.Wait(&self.mid, mid)?;
                        }
                        direct.ExecuteCommandLists(&group);
                    }
                    On::Compute => {
                        let pre = self.pre_value.fetch_add(1, Ordering::Relaxed) + 1;
                        direct.Signal(&self.pre, pre)?;
                        self.queue.Wait(&self.pre, pre)?;
                        self.queue.ExecuteCommandLists(&group);
                    }
                }
                first = false;
                at = end;
            }
            if lists.last().is_some_and(|(on, _)| *on == On::Compute) {
                self.queue.Signal(fence, value)?;
                self.last.store(value, Ordering::Release);
                Ok(())
            } else {
                self.before_signal(direct, fence, value);
                direct.Signal(fence, value)
            }
        }
    }
}

/// The frame line's counts, per frame over `frames` frames.
pub(crate) fn line(frames: f64) {
    let Some(c) = made() else { return };
    let v: Vec<f64> = c.stats.iter().map(|a| a.swap(0, Ordering::Relaxed) as f64 / frames.max(1.0)).collect();
    tracing::info!(
        "the host compute queue per frame (VICTORY_ASYNC_COMPUTE={}): {:.1} submissions, {:.1} lists; the direct queue \
         waited for it {:.1} times before its lists and {:.1} times before its signal",
        mode(),
        v[0],
        v[1],
        v[2],
        v[3]
    );
}

#[cfg(test)]
mod tests {
    use crate::host::compute_state;

    #[test]
    fn compute_lists_name_only_compute_states() {
        // Pixel and non-pixel shader resource: the non-pixel half stays.
        assert_eq!(compute_state(0x80 | 0x40), 0x40);
        assert_eq!(compute_state(0x80), 0x40);
        // Render target, depth write and read go (to the common state).
        assert_eq!(compute_state(0x4), 0);
        assert_eq!(compute_state(0x10), 0);
        assert_eq!(compute_state(0x20 | 0x80), 0x40);
        // What a compute list may name is kept.
        for state in [0x8, 0x40, 0x400, 0x800, 0x400 | 0x8] {
            assert_eq!(compute_state(state), state);
        }
    }

    #[test]
    fn off_by_default() {
        if std::env::var_os("VICTORY_ASYNC_COMPUTE").is_none() {
            assert_eq!(super::mode(), 0);
        }
    }
}
