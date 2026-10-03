use super::{Error, Image, MAX_IMAGE_BYTES, Section, pe};
use std::ffi::{CStr, c_char, c_void};
use std::io;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

const _: () = assert!(size_of::<MemoryInformation>() == 48);

/// `PAGE_EXECUTE_READWRITE`: pages that may be executed, read and written.
pub(super) const PAGE_EXECUTE_READWRITE: u32 = 0x40;

#[repr(C)]
struct MemoryInformation {
	base: *mut c_void,
	allocation_base: *mut c_void,
	allocation_protection: u32,
	partition_id: u16,
	region_size: usize,
	state: u32,
	protection: u32,
	kind: u32,
}

/// Copies process memory through the OS without creating borrowed references.
#[derive(Debug)]
pub struct MemoryReader;

impl MemoryReader {
	/// A reader of this process's memory.
	pub fn open() -> Result<Self, Error> {
		Ok(Self)
	}

	/// Copy a bounded readable range, rejecting overflow and partial reads.
	pub fn copy(&self, address: usize, len: usize) -> Result<Vec<u8>, Error> {
		if len > MAX_IMAGE_BYTES || address.checked_add(len).is_none() {
			return Err(Error::InvalidImage);
		}
		let mut bytes = vec![0; len];
		let mut read = 0;
		// SAFETY: Windows validates readable source memory and the owned output
		// allocation has len bytes. No references into mutable engine data exist.
		if unsafe {
			ReadProcessMemory(
				GetCurrentProcess(),
				address as *const c_void,
				bytes.as_mut_ptr().cast(),
				len,
				&mut read,
			)
		} == 0
		{
			return Err(std::io::Error::last_os_error().into());
		}
		if read != len {
			return Err(Error::InvalidImage);
		}
		Ok(bytes)
	}
}

/// A loader reference pinning a Windows module until dropped.
#[derive(Debug)]
pub struct Module(*mut c_void);

impl Module {
	/// Acquire the loaded module containing `address`.
	///
	/// # Safety
	/// The module must remain loaded until its loader reference is acquired.
	pub unsafe fn at(address: usize) -> Result<Self, Error> {
		let mut handle = std::ptr::null_mut();
		// SAFETY: FROM_ADDRESS treats the value as an address rather than UTF-16;
		// the output is valid and the caller keeps the module loaded during lookup.
		if unsafe { GetModuleHandleExW(4, address as *const u16, &mut handle) } == 0 {
			return Err(std::io::Error::last_os_error().into());
		}
		Ok(Self(handle))
	}

	/// Acquire the already loaded module named `name`, such as `engine.dll`,
	/// without loading it.
	///
	/// Fails with the error Windows reports if no module of that name is
	/// loaded, and with [`io::ErrorKind::InvalidInput`] if `name` contains a
	/// NUL.
	pub fn loaded(name: &str) -> Result<Self, Error> {
		if name.contains('\0') {
			return Err(io::Error::from(io::ErrorKind::InvalidInput).into());
		}

		let name = name.encode_utf16().chain([0]).collect::<Vec<_>>();
		let mut handle = std::ptr::null_mut();

		// SAFETY: Without flags, this only finds a module that is already
		// loaded, by its NUL-terminated name, and adds a loader reference to
		// it; the output is valid.
		if unsafe { GetModuleHandleExW(0, name.as_ptr(), &mut handle) } == 0 {
			return Err(io::Error::last_os_error().into());
		}

		Ok(Self(handle))
	}

	/// The module's live load address.
	pub fn base(&self) -> usize {
		self.0 as usize
	}
}

impl Drop for Module {
	fn drop(&mut self) {
		// SAFETY: Balances exactly the one loader reference taken when this
		// Module was made, by Module::at or loaded_symbol.
		unsafe {
			FreeLibrary(self.0);
		}
	}
}

#[link(name = "kernel32")]
unsafe extern "system" {
	fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
	fn FreeLibrary(module: *mut c_void) -> i32;
	fn GetCurrentProcess() -> *mut c_void;
	fn GetModuleHandleExA(flags: u32, name: *const c_char, module: *mut *mut c_void) -> i32;
	fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
	fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;

	fn ReadProcessMemory(
		process: *mut c_void,
		base: *const c_void,
		buffer: *mut c_void,
		len: usize,
		read: *mut usize,
	) -> i32;

	fn VirtualProtect(address: *const c_void, size: usize, protection: u32, old: *mut u32) -> i32;

	fn VirtualQuery(
		address: *const c_void,
		information: *mut MemoryInformation,
		size: usize,
	) -> usize;
}

/// Makes the instruction fetches of this process see the code changed in
/// `len` bytes at `address`.
pub(super) fn flush_instruction_cache(address: *const c_void, len: usize) -> io::Result<()> {
	// SAFETY: The process pseudo-handle is valid, and flushing reads no
	// memory through `address`.
	if unsafe { FlushInstructionCache(GetCurrentProcess(), address, len) } == 0 {
		return Err(io::Error::last_os_error());
	}

	Ok(())
}

/// Whether the OS currently reports committed executable memory at `address`.
pub fn is_executable(address: usize) -> bool {
	let mut information = MaybeUninit::<MemoryInformation>::uninit();
	// SAFETY: VirtualQuery queries without dereferencing the address. The output
	// uses the verified Windows x64 MEMORY_BASIC_INFORMATION layout.
	if unsafe {
		VirtualQuery(
			address as *const c_void,
			information.as_mut_ptr(),
			size_of::<MemoryInformation>(),
		)
	} != size_of::<MemoryInformation>()
	{
		return false;
	}
	// SAFETY: Successful VirtualQuery initialized the complete structure.
	let information = unsafe { information.assume_init() };
	information.state == 0x1000
		&& information.protection & 0x101 == 0
		&& information.protection & 0xf0 != 0
}

pub(super) unsafe fn load(address: usize) -> Result<Image, Error> {
	// SAFETY: Forwarded keep-loaded guarantee from Image::load.
	let module = unsafe { Module::at(address)? };
	let image = snapshot(&module)?;
	if !image.executable(address) {
		return Err(Error::InvalidImage);
	}
	Ok(image)
}

/// The address of the export `name` of the already loaded library `library`,
/// such as `tier0.dll`, or `None` if no library of that name is loaded or it
/// does not export `name`. This never loads a library.
///
/// The library is kept loaded during the lookup only. The address is usable
/// as a native pointer only while the library stays loaded, and only with the
/// type the library exports it with.
pub fn loaded_symbol(library: &CStr, name: &CStr) -> Option<NonNull<c_void>> {
	let mut handle = std::ptr::null_mut();

	// SAFETY: Without flags, this only finds a module that is already loaded,
	// by name, and adds a loader reference to it; the output is valid.
	if unsafe { GetModuleHandleExA(0, library.as_ptr(), &mut handle) } == 0 {
		return None;
	}

	// Releases the reference when dropped, after the lookup.
	let module = Module(handle);

	// SAFETY: The reference keeps the module loaded during the lookup.
	NonNull::new(unsafe { GetProcAddress(module.0, name.as_ptr()) })
}

/// Sets the protection of the pages holding `len` bytes at `address` to
/// `protection`, a `PAGE_*` value, returning the previous protection of the
/// first.
///
/// # Safety
///
/// The pages must belong to a module or allocation that stays mapped for the
/// call, and nothing may rely on their previous protection while it is
/// changed, such as code executing from them that the new protection forbids.
pub(super) unsafe fn protect(
	address: *const c_void,
	len: usize,
	protection: u32,
) -> io::Result<u32> {
	let mut old = 0;

	// SAFETY: As the caller promises; `old` is a valid output.
	if unsafe { VirtualProtect(address, len, protection, &mut old) } == 0 {
		return Err(io::Error::last_os_error());
	}

	Ok(old)
}

/// Snapshots the image of the module that `module` keeps loaded.
pub(super) fn snapshot(module: &Module) -> Result<Image, Error> {
	let memory = MemoryReader::open()?;
	let base = module.base();
	let headers = pe::Headers::read(|offset, len| {
		memory.copy(base.checked_add(offset).ok_or(Error::InvalidImage)?, len)
	})?;
	base.checked_add(headers.size).ok_or(Error::InvalidImage)?;
	let mut sections = Vec::new();
	for section in headers.sections {
		let start = base
			.checked_add(section.offset)
			.ok_or(Error::InvalidImage)?;
		sections.push(Section {
			address: start,
			bytes: memory.copy(start, section.len)?,
			executable: section.executable,
			writable: section.writable,
		});
	}
	Ok(Image { base, sections })
}
