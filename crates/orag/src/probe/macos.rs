//! macOS: sysctl for RAM, cores and the CPU name, the Mach VM statistics
//! for available memory. The GPU of an Apple Silicon Mac is its Metal GPU
//! (`Machine::metal`); IOKit is not read, so `gpu` stays empty.

use std::ffi::CString;

use super::StaticFacts;

/// A sysctl value of fixed size (an integer).
fn sysctl_number<T: Copy + Default>(name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let mut value = T::default();
    let mut len = std::mem::size_of::<T>();
    // SAFETY: `name` is NUL-terminated, `value` is writable for `len` bytes,
    // and no new value is set.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len == std::mem::size_of::<T>()).then_some(value)
}

/// A sysctl string, without its NUL.
fn sysctl_string(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut buf = [0u8; 256];
    let mut len = buf.len();
    // SAFETY: as in `sysctl_number`, with `buf` writable for `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&buf[..len.min(buf.len())]);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn count(name: &str) -> Option<u32> {
    sysctl_number::<i32>(name).and_then(|n| u32::try_from(n).ok())
}

pub fn static_facts() -> StaticFacts {
    StaticFacts {
        total_ram_bytes: sysctl_number::<u64>("hw.memsize"),
        logical_cores: count("hw.logicalcpu"),
        physical_cores: count("hw.physicalcpu"),
        cpu_model: sysctl_string("machdep.cpu.brand_string"),
        gpu: None,
    }
}

/// Free and inactive pages: what the system can hand out without swapping
/// (about what Activity Monitor calls available).
pub fn available_ram_bytes() -> Option<u64> {
    // SAFETY: `stats` is writable for `count` natural_t words, which the
    // kernel fills at most; the host port needs no clean-up.
    let stats = unsafe {
        let mut stats = std::mem::zeroed::<libc::vm_statistics64>();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        #[allow(deprecated)] // the libc binding points to the mach2 crate
        let host = libc::mach_host_self();
        let rc = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&raw mut stats).cast(),
            &mut count,
        );
        if rc != libc::KERN_SUCCESS {
            return None;
        }
        stats
    };
    // SAFETY: sysconf has no preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    available_from(stats.free_count, stats.inactive_count, page)
}

/// Available bytes from the VM statistics. XNU's `host_statistics64`
/// reports `free_count` with the speculative pages already in it, so
/// `speculative_count` is not added again.
fn available_from(free: u32, inactive: u32, page: u64) -> Option<u64> {
    (u64::from(free) + u64::from(inactive)).checked_mul(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speculative_pages_are_counted_once_inside_free() {
        // 100 free (30 of them speculative, already inside free_count) and
        // 50 inactive 16 KiB pages.
        assert_eq!(available_from(100, 50, 16_384), Some(150 * 16_384));
        assert_eq!(available_from(0, 0, 16_384), Some(0));
        assert_eq!(available_from(u32::MAX, u32::MAX, u64::MAX), None);
    }
}
