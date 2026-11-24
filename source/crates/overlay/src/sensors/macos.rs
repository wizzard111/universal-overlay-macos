//! macOS sensors without root:
//! * CPU load: Mach host_statistics(HOST_CPU_LOAD_INFO) deltas.
//! * RAM: "Memory Used" like Activity Monitor = app + wired + compressed.
//! * GPU: IOKit IOAccelerator "PerformanceStatistics" — "Device Utilization %"
//!   and "In use system memory" / "vramUsedBytes".
//!
//! CPU/GPU clocks, temperatures and power need root (powermetrics) or
//! private SMC access and are not provided.

use super::Snapshot;
use std::ffi::{c_char, c_void, CString};
use uo_common::sensor::SensorId;

type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFAllocatorDefault: CFTypeRef;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
    fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFNumberGetValue(n: CFTypeRef, ty: i32, out: *mut c_void) -> u8;
    fn CFRelease(cf: CFTypeRef);
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingServices(port: u32, matching: CFTypeRef, it: *mut u32) -> i32;
    fn IOIteratorNext(it: u32) -> u32;
    fn IORegistryEntryCreateCFProperty(
        e: u32,
        key: CFTypeRef,
        alloc: CFTypeRef,
        opts: u32,
    ) -> CFTypeRef;
    fn IOObjectRelease(o: u32) -> i32;
}

extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    fn host_statistics64(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
}

const HOST_CPU_LOAD_INFO: i32 = 3;
const HOST_VM_INFO64: i32 = 4;
const UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT64: i32 = 4;

#[repr(C)]
#[derive(Default)]
struct VmStats64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}

fn cfstr(s: &str) -> CFTypeRef {
    let c = CString::new(s).unwrap_or_default();
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), UTF8) }
}

fn sysctl_u64(name: &str) -> Option<u64> {
    let c = CString::new(name).ok()?;
    let mut v: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let r = unsafe {
        libc::sysctlbyname(
            c.as_ptr(),
            &mut v as *mut _ as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (r == 0).then_some(v)
}

pub struct Sensors {
    last_cpu: Option<(u64, u64)>,
    total_mem: Option<u64>,
    page: u64,
    keys: [CFTypeRef; 4],
}

impl Sensors {
    pub fn new() -> Self {
        Self {
            last_cpu: None,
            total_mem: sysctl_u64("hw.memsize"),
            page: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64,
            keys: [
                cfstr("PerformanceStatistics"),
                cfstr("Device Utilization %"),
                cfstr("In use system memory"),
                cfstr("vramUsedBytes"),
            ],
        }
    }

    pub fn gpu_name(&self) -> String {
        "IOAccelerator".into()
    }

    fn cpu(&mut self, s: &mut Snapshot) {
        let mut ticks = [0u32; 4]; // user, system, idle, nice
        let mut count = 4u32;
        if unsafe {
            host_statistics(
                mach_host_self(),
                HOST_CPU_LOAD_INFO,
                ticks.as_mut_ptr() as *mut i32,
                &mut count,
            )
        } != 0
        {
            return;
        }
        let idle = ticks[2] as u64;
        let total = ticks.iter().map(|&t| t as u64).sum::<u64>();
        if let Some((pi, pt)) = self.last_cpu.replace((idle, total)) {
            let dt = total.wrapping_sub(pt);
            if dt > 0 {
                let busy = 1.0 - idle.wrapping_sub(pi) as f64 / dt as f64;
                s.set(SensorId::CpuLoad, Some((busy * 100.0).clamp(0.0, 100.0)));
            }
        }
    }

    fn ram(&self, s: &mut Snapshot) {
        let mut vm = VmStats64::default();
        let mut count = (std::mem::size_of::<VmStats64>() / 4) as u32;
        if unsafe {
            host_statistics64(
                mach_host_self(),
                HOST_VM_INFO64,
                &mut vm as *mut _ as *mut i32,
                &mut count,
            )
        } != 0
        {
            return;
        }
        let app = vm.internal_page_count.saturating_sub(vm.purgeable_count) as u64;
        let used = (app + vm.wire_count as u64 + vm.compressor_page_count as u64) * self.page;
        s.set(SensorId::RamUsed, Some(used as f64 / (1024.0 * 1024.0)));
        if let Some(t) = self.total_mem {
            s.set(SensorId::RamLoad, Some(used as f64 * 100.0 / t as f64));
        }
    }

    fn gpu(&self, s: &mut Snapshot) {
        unsafe {
            let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
            let mut it = 0u32;
            if IOServiceGetMatchingServices(0, matching, &mut it) != 0 {
                return;
            }
            let num = |d: CFTypeRef, k: CFTypeRef| -> Option<f64> {
                let v = CFDictionaryGetValue(d, k);
                if v.is_null() || CFGetTypeID(v) != CFNumberGetTypeID() {
                    return None;
                }
                let mut out: i64 = 0;
                (CFNumberGetValue(v, CF_NUMBER_SINT64, &mut out as *mut _ as *mut c_void) != 0)
                    .then_some(out as f64)
            };
            loop {
                let e = IOIteratorNext(it);
                if e == 0 {
                    break;
                }
                let stats =
                    IORegistryEntryCreateCFProperty(e, self.keys[0], kCFAllocatorDefault, 0);
                IOObjectRelease(e);
                if stats.is_null() {
                    continue;
                }
                if CFGetTypeID(stats) == CFDictionaryGetTypeID() {
                    if let Some(u) = num(stats, self.keys[1]) {
                        s.set(SensorId::GpuLoad, Some(u));
                    }
                    if let Some(b) = num(stats, self.keys[2]).or_else(|| num(stats, self.keys[3])) {
                        s.set(SensorId::GpuVram, Some(b / (1024.0 * 1024.0)));
                    }
                }
                CFRelease(stats);
            }
            IOObjectRelease(it);
        }
    }

    pub fn poll(&mut self, s: &mut Snapshot, want: super::Want) {
        if want.cpu {
            self.cpu(s);
        }
        if want.ram {
            self.ram(s);
        }
        if want.gpu {
            self.gpu(s);
        }
    }
}

impl Default for Sensors {
    fn default() -> Self {
        Self::new()
    }
}
