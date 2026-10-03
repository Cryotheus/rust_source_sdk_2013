use super::{Error, Image, MAX_IMAGE_BYTES, Section, pe};
use std::ffi::{CStr, c_char, c_void};
use std::mem::MaybeUninit;
use std::ptr::NonNull;

const _: () = assert!(size_of::<MemoryInformation>() == 48);

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
	fn FreeLibrary(module: *mut c_void) -> i32;
	fn GetCurrentProcess() -> *mut c_void;
	fn GetModuleHandleExA(flags: u32, name: *const c_char, module: *mut *mut c_void) -> i32;
	fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
	fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;

	fn ReadProcessMemory(
		process: *mut c_void,
		base: *const c_void,
		buffer: *mut c_void,
		len: usize,
		read: *mut usize,
	) -> i32;

	fn VirtualQuery(
		address: *const c_void,
		information: *mut MemoryInformation,
		size: usize,
	) -> usize;
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
	let image = Image { base, sections };
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn snapshots_live_module_and_rejects_unreadable_memory() {
		let address = GetCurrentProcess as *const () as usize;
		// SAFETY: This process's linked kernel32 module remains loaded.
		let image = unsafe { Image::load(address) }.unwrap();
		assert!(image.executable(address));
		let reader = MemoryReader::open().unwrap();
		let bytes = [12_u8, 34, 56];
		assert_eq!(
			reader.copy(bytes.as_ptr() as usize, bytes.len()).unwrap(),
			bytes
		);
		assert!(reader.copy(1, 16).is_err());
		assert!(reader.copy(usize::MAX, 2).is_err());
		assert!(is_executable(address));
		assert!(!is_executable(bytes.as_ptr() as usize));
	}

	#[test]
	fn symbols_are_found_only_in_loaded_libraries() {
		assert!(loaded_symbol(c"kernel32.dll", c"GetCurrentProcess").is_some());
		assert!(loaded_symbol(c"kernel32.dll", c"source_sdk_2013_raw_absent").is_none());
		assert!(loaded_symbol(c"source_sdk_2013_raw_absent.dll", c"Msg").is_none());
	}
}
