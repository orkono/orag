//! x86-64 instructions and the CPU's brand string, read with CPUID on any
//! operating system. `is_x86_feature_detected!` is not used: in a binary
//! built with `+avx2` (Windows) it answers `true` at compile time, which is
//! exactly the answer that cannot be trusted here.

use serde::Serialize;

/// The instructions the shipped Windows x86-64 binary is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct X86Features {
    pub avx2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub bmi2: bool,
}

impl X86Features {
    /// The required instructions this CPU lacks, in the floor's order.
    pub fn missing(&self) -> Vec<&'static str> {
        [
            ("avx2", self.avx2),
            ("fma", self.fma),
            ("f16c", self.f16c),
            ("bmi2", self.bmi2),
        ]
        .into_iter()
        .filter(|(_, present)| !present)
        .map(|(name, _)| name)
        .collect()
    }
}

/// ECX of leaf 1 and EBX of leaf 7 (subleaf 0), and whether the OS saves
/// the AVX (YMM) registers (XCR0 bits 1 and 2): AVX2, FMA and F16C are only
/// usable when it does.
#[cfg_attr(not(any(target_arch = "x86_64", test)), allow(dead_code))]
pub(super) fn decode(leaf1_ecx: u32, leaf7_ebx: u32, os_saves_ymm: bool) -> X86Features {
    let bit = |value: u32, n: u32| value & (1 << n) != 0;
    let avx = bit(leaf1_ecx, 28) && os_saves_ymm;
    X86Features {
        avx2: avx && bit(leaf7_ebx, 5),
        fma: avx && bit(leaf1_ecx, 12),
        f16c: avx && bit(leaf1_ecx, 29),
        bmi2: bit(leaf7_ebx, 8),
    }
}

/// The 48-byte brand string of leaves 0x8000_0002..4 (EAX, EBX, ECX, EDX
/// each), trimmed; `None` when empty.
#[cfg_attr(not(any(target_arch = "x86_64", test)), allow(dead_code))]
pub(super) fn brand_from(registers: &[[u32; 4]; 3]) -> Option<String> {
    let bytes: Vec<u8> = registers
        .iter()
        .flatten()
        .flat_map(|word| word.to_le_bytes())
        .take_while(|&b| b != 0)
        .collect();
    let text = String::from_utf8_lossy(&bytes);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

#[cfg(target_arch = "x86_64")]
pub(super) fn features() -> super::CpuFeatures {
    use std::arch::x86_64::__cpuid_count;
    // Leaf 7 only when the CPU has it (the maximum leaf is in leaf 0).
    let leaf1 = __cpuid_count(1, 0);
    let leaf7 = if __cpuid_count(0, 0).eax >= 7 {
        __cpuid_count(7, 0).ebx
    } else {
        0
    };
    let osxsave = leaf1.ecx & (1 << 27) != 0;
    let saves_ymm = osxsave && xcr0() & 0b110 == 0b110;
    super::CpuFeatures::X86(decode(leaf1.ecx, leaf7, saves_ymm))
}

/// XCR0, read only after CPUID said the OS enabled XSAVE (OSXSAVE).
#[cfg(target_arch = "x86_64")]
fn xcr0() -> u64 {
    #[target_feature(enable = "xsave")]
    unsafe fn read() -> u64 {
        // SAFETY: the caller checked OSXSAVE, so XGETBV is available.
        unsafe { std::arch::x86_64::_xgetbv(0) }
    }
    // SAFETY: only called when CPUID leaf 1 reports OSXSAVE.
    unsafe { read() }
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn features() -> super::CpuFeatures {
    super::CpuFeatures::NotApplicable
}

#[cfg(target_arch = "x86_64")]
pub(super) fn brand() -> Option<String> {
    use std::arch::x86_64::__cpuid;
    // The brand leaves exist only up to the extended maximum.
    if __cpuid(0x8000_0000).eax < 0x8000_0004 {
        return None;
    }
    let regs = |leaf: u32| {
        let r = __cpuid(leaf);
        [r.eax, r.ebx, r.ecx, r.edx]
    };
    brand_from(&[regs(0x8000_0002), regs(0x8000_0003), regs(0x8000_0004)])
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn brand() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const AVX: u32 = 1 << 28;
    const FMA: u32 = 1 << 12;
    const F16C: u32 = 1 << 29;
    const AVX2: u32 = 1 << 5;
    const BMI2: u32 = 1 << 8;

    #[test]
    fn a_haswell_has_the_floor_and_an_older_cpu_does_not() {
        let haswell = decode(AVX | FMA | F16C, AVX2 | BMI2, true);
        assert!(haswell.missing().is_empty(), "{haswell:?}");
        let sandy_bridge = decode(AVX, 0, true);
        assert_eq!(sandy_bridge.missing(), ["avx2", "fma", "f16c", "bmi2"]);
        let no_bmi2 = decode(AVX | FMA | F16C, AVX2, true);
        assert_eq!(no_bmi2.missing(), ["bmi2"]);
    }

    #[test]
    fn avx_instructions_need_the_os_to_save_ymm_registers() {
        let features = decode(AVX | FMA | F16C, AVX2 | BMI2, false);
        assert_eq!(features.missing(), ["avx2", "fma", "f16c"]);
    }

    #[test]
    fn the_brand_string_is_trimmed_and_stops_at_nul() {
        let text = b"  Intel(R) Core(TM) i7-4770 CPU @ 3.40GHz\0\0\0\0\0\0\0";
        let mut words = [[0u32; 4]; 3];
        for (i, chunk) in text.chunks(4).enumerate() {
            let mut b = [0u8; 4];
            b[..chunk.len()].copy_from_slice(chunk);
            words[i / 4][i % 4] = u32::from_le_bytes(b);
        }
        assert_eq!(
            brand_from(&words).as_deref(),
            Some("Intel(R) Core(TM) i7-4770 CPU @ 3.40GHz")
        );
        assert_eq!(brand_from(&[[0; 4]; 3]), None);
    }
}
