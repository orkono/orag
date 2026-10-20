//! Windows: `GlobalMemoryStatusEx` for RAM, the processor-core records for
//! physical cores, and DXGI for the GPU. `dxgi.dll` is loaded at run time
//! from System32 (not imported), so the binary's import table keeps only
//! the system DLLs `scripts/platform-floors.sh` allows, and a Windows
//! without DXGI just reports no GPU.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{FreeLibrary, LUID};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GlobalMemoryStatusEx, MEMORYSTATUSEX, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows_sys::Win32::System::Threading::{ALL_PROCESSOR_GROUPS, GetActiveProcessorCount};
use windows_sys::core::{GUID, HRESULT};

use super::parse::{best_gpu, dxgi_adapter};
use super::{Gpu, StaticFacts};

/// More adapters than this are not read.
const MAX_ADAPTERS: u32 = 16;

pub fn static_facts() -> StaticFacts {
    // SAFETY: GetActiveProcessorCount has no preconditions; 0 is failure.
    let logical = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
    StaticFacts {
        total_ram_bytes: memory_status().map(|m| m.ullTotalPhys),
        logical_cores: (logical > 0).then_some(logical),
        physical_cores: physical_cores(),
        // The CPUID brand string (super::x86), on every x86-64 OS.
        cpu_model: None,
        gpu: best_gpu(adapters()),
    }
}

pub fn available_ram_bytes() -> Option<u64> {
    memory_status().map(|m| m.ullAvailPhys)
}

fn memory_status() -> Option<MEMORYSTATUSEX> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is writable and its `dwLength` set, as the call requires.
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status)
}

/// One `RelationProcessorCore` record per physical core.
fn physical_cores() -> Option<u32> {
    let mut len = 0u32;
    // SAFETY: a null buffer with length 0 asks for the size only.
    unsafe {
        GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &mut len)
    };
    if len == 0 {
        return None;
    }
    // u64 words: the records need 8-byte alignment.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    let records = buffer
        .as_mut_ptr()
        .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // SAFETY: `buffer` is writable for at least `len` bytes and aligned.
    if unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, records, &mut len) } == 0 {
        return None;
    }
    let bytes = buffer.as_ptr().cast::<u8>();
    let (mut offset, mut cores) = (0usize, 0u32);
    while offset + std::mem::size_of::<u32>() * 2 <= len as usize {
        // SAFETY: `offset` stays inside the `len` bytes the call filled, and
        // each record starts with its relationship and its own size.
        let size = unsafe {
            let record = bytes
                .add(offset)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
            (*record).Size as usize
        };
        if size == 0 {
            break;
        }
        cores += 1;
        offset += size;
    }
    (cores > 0).then_some(cores)
}

/// `DXGI_ADAPTER_DESC1`.
#[repr(C)]
struct AdapterDesc1 {
    description: [u16; 128],
    vendor_id: u32,
    device_id: u32,
    sub_sys_id: u32,
    revision: u32,
    dedicated_video_memory: usize,
    dedicated_system_memory: usize,
    shared_system_memory: usize,
    adapter_luid: LUID,
    flags: u32,
}

/// `IID_IDXGIFactory1`.
const IID_IDXGI_FACTORY1: GUID = GUID::from_u128(0x770aae78_f26f_4dba_a829_253c83d1b387);
const DXGI_ERROR_NOT_FOUND: HRESULT = 0x887A_0002_u32 as HRESULT;

type CreateFactory1 = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;
type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;
type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> HRESULT;

/// Vtable slots: IUnknown (3) + IDXGIObject (4) come first; then
/// IDXGIFactory has 5 and IDXGIFactory1 starts with EnumAdapters1; an
/// IDXGIAdapter has 3 and IDXGIAdapter1 adds GetDesc1.
const RELEASE: usize = 2;
const FACTORY1_ENUM_ADAPTERS1: usize = 12;
const ADAPTER1_GET_DESC1: usize = 10;

/// The method in slot `index` of the COM object `object`.
///
/// # Safety
/// `object` must be a live COM interface whose vtable has `index + 1` slots
/// and slot `index` must have the signature `F`.
unsafe fn method<F: Copy>(object: *mut c_void, index: usize) -> F {
    // SAFETY: per the contract, `object` points at its vtable pointer.
    unsafe {
        let vtable = *object.cast::<*const *const c_void>();
        std::mem::transmute_copy(&*vtable.add(index))
    }
}

/// Releases a COM object on drop.
struct Com(*mut c_void);

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a live interface this guard owns one reference to.
        unsafe { method::<Release>(self.0, RELEASE)(self.0) };
    }
}

/// Frees the loaded `dxgi.dll` on drop (after every object it made).
struct Library(windows_sys::Win32::Foundation::HMODULE);

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the module was loaded by `adapters` and is freed once.
        unsafe { FreeLibrary(self.0) };
    }
}

fn adapters() -> Vec<Gpu> {
    let name: Vec<u16> = "dxgi.dll".encode_utf16().chain([0]).collect();
    // SAFETY: `name` is NUL-terminated; System32 only, never the app folder.
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Vec::new();
    }
    let library = Library(module);
    // SAFETY: the name is a NUL-terminated ASCII string.
    let Some(create) =
        (unsafe { GetProcAddress(library.0, c"CreateDXGIFactory1".as_ptr().cast()) })
    else {
        return Vec::new();
    };
    // SAFETY: CreateDXGIFactory1 has this signature.
    let create: CreateFactory1 = unsafe { std::mem::transmute(create) };
    let mut factory = std::ptr::null_mut();
    // SAFETY: valid IID and out pointer.
    if unsafe { create(&IID_IDXGI_FACTORY1, &mut factory) } < 0 || factory.is_null() {
        return Vec::new();
    }
    let factory = Com(factory);
    let found = (0..MAX_ADAPTERS)
        .map_while(|index| {
            let mut adapter = std::ptr::null_mut();
            // SAFETY: `factory` is an IDXGIFactory1; slot 12 is EnumAdapters1.
            let hr = unsafe {
                method::<EnumAdapters1>(factory.0, FACTORY1_ENUM_ADAPTERS1)(
                    factory.0,
                    index,
                    &mut adapter,
                )
            };
            (hr != DXGI_ERROR_NOT_FOUND && hr >= 0 && !adapter.is_null()).then(|| Com(adapter))
        })
        .filter_map(|adapter| {
            // SAFETY: zeroed is a valid DXGI_ADAPTER_DESC1 (plain data).
            let mut desc: AdapterDesc1 = unsafe { std::mem::zeroed() };
            // SAFETY: an adapter from EnumAdapters1 is an IDXGIAdapter1.
            let hr =
                unsafe { method::<GetDesc1>(adapter.0, ADAPTER1_GET_DESC1)(adapter.0, &mut desc) };
            (hr >= 0).then(|| {
                dxgi_adapter(
                    &desc.description,
                    desc.dedicated_video_memory as u64,
                    desc.flags,
                )
            })?
        })
        .collect();
    drop(factory);
    drop(library);
    found
}
