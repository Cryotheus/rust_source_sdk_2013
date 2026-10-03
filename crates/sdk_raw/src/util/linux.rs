use super::{Error, Image, MAX_IMAGE_BYTES, Section, u16_at, u32_at, word_at};
use std::ffi::{CStr, OsStr, c_char, c_int, c_void};
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

/// `dlopen`'s flag to only find a library that is already loaded.
const RTLD_NOLOAD: c_int = 4;

/// `dlopen`'s flag to resolve every symbol before returning.
const RTLD_NOW: c_int = 2;

#[repr(C)]
struct DlInfo {
	name: *const c_char,
	base: *mut c_void,
	symbol: *const c_char,
	address: *mut c_void,
}

pub(super) struct LoadSegment {
	pub address: usize,
	pub len: usize,
	pub executable: bool,
	pub writable: bool,
}

impl LoadSegment {
	pub fn contains(&self, address: usize, len: usize, executable: bool, writable: bool) -> bool {
		(!executable || self.executable)
			&& (!writable || self.writable)
			&& address >= self.address
			&& address
				.checked_add(len)
				.is_some_and(|end| end <= self.address + self.len)
	}
}

/// Copies process memory through the kernel without creating Rust references
/// to objects owned or mutated by native code.
pub struct MemoryReader {
	memory: File,
}

impl MemoryReader {
	pub fn open() -> Result<Self, Error> {
		Ok(Self {
			memory: File::open("/proc/self/mem")?,
		})
	}

	/// Returns an owned, bounded copy. Unmapped addresses produce an I/O error.
	pub fn copy(&self, address: usize, len: usize) -> Result<Vec<u8>, Error> {
		if len > MAX_IMAGE_BYTES || address.checked_add(len).is_none() {
			return Err(Error::InvalidImage);
		}
		let mut bytes = vec![0; len];
		self.memory.read_exact_at(&mut bytes, address as u64)?;
		Ok(bytes)
	}
}

/// Owned identity of a loaded module. This does not pin the module in memory.
pub struct Module {
	base: usize,
	path: PathBuf,
}

impl Module {
	/// Finds the module containing `address` and copies its loader filename.
	///
	/// # Safety
	/// The caller must prevent this module from unloading during this call,
	/// including while the loader-owned filename is copied.
	pub unsafe fn at(address: usize) -> Result<Self, Error> {
		let mut info = std::mem::MaybeUninit::<DlInfo>::uninit();
		// SAFETY: dladdr accepts an arbitrary address and initializes info on
		// success. The caller keeps the module and loader filename alive.
		if unsafe { dladdr(address as *const c_void, info.as_mut_ptr()) } == 0 {
			return Err(Error::InvalidImage);
		}
		// SAFETY: A successful dladdr initialized every field.
		let info = unsafe { info.assume_init() };
		if info.name.is_null() || info.base.is_null() {
			return Err(Error::InvalidImage);
		}
		// SAFETY: dladdr returned a terminated filename which stays live for
		// this copy under the caller's module-lifetime guarantee.
		let name = unsafe { CStr::from_ptr(info.name) };
		Ok(Self {
			base: info.base as usize,
			path: PathBuf::from(OsStr::from_bytes(name.to_bytes())),
		})
	}

	pub fn base(&self) -> usize {
		self.base
	}

	pub fn path(&self) -> &Path {
		&self.path
	}
}

#[link(name = "dl")]
unsafe extern "C" {
	fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
	fn dlclose(handle: *mut c_void) -> c_int;
	fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
	fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// Checks current process mapping permissions. The result is a snapshot;
/// callers must separately ensure the address remains live before calling it.
pub fn is_executable(address: usize) -> bool {
	let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
		return false;
	};
	maps.lines().any(|line| {
		let mut fields = line.split_whitespace();
		let (Some(range), Some(permissions)) = (fields.next(), fields.next()) else {
			return false;
		};
		if permissions.as_bytes().get(2) != Some(&b'x') {
			return false;
		}
		let Some((start, end)) = range.split_once('-') else {
			return false;
		};
		let (Ok(start), Ok(end)) = (
			usize::from_str_radix(start, 16),
			usize::from_str_radix(end, 16),
		) else {
			return false;
		};
		(start..end).contains(&address)
	})
}

/// Snapshots all readable PT_LOAD segments of a little-endian x86-64 ET_DYN
/// module, including its executable code. `address` must lie in readable code.
///
/// # Safety
/// The caller must keep the module containing `address` loaded and its image
/// mappings unchanged throughout this call. The returned bytes are owned.
pub unsafe fn load(address: usize) -> Result<Image, Error> {
	// SAFETY: The caller keeps the module and loader filename alive.
	let module = unsafe { Module::at(address) }?;
	let memory = MemoryReader::open()?;
	let header = memory.copy(module.base(), 64)?;
	let programs = program_headers(&memory, module.base(), &header)?;
	let segments = load_segments(module.base(), &programs)?;
	if !segments
		.iter()
		.any(|segment| segment.contains(address, 1, true, false))
	{
		return Err(Error::InvalidImage);
	}
	let sections = segments
		.into_iter()
		.map(|segment| {
			Ok(Section {
				address: segment.address,
				bytes: memory.copy(segment.address, segment.len)?,
				executable: segment.executable,
				writable: segment.writable,
			})
		})
		.collect::<Result<Vec<_>, Error>>()?;
	Ok(Image {
		base: module.base(),
		sections,
	})
}

pub(super) fn load_segments(base: usize, programs: &[u8]) -> Result<Vec<LoadSegment>, Error> {
	if !programs.len().is_multiple_of(56) {
		return Err(Error::InvalidImage);
	}
	let mut segments = Vec::new();
	let mut total = 0_usize;
	for program in programs.as_chunks::<56>().0 {
		let flags = u32_at(program, 4).ok_or(Error::InvalidImage)?;
		if u32_at(program, 0) != Some(1) || flags & 4 == 0 {
			continue;
		}
		let address = base
			.checked_add(word_at(program, 16).ok_or(Error::InvalidImage)?)
			.ok_or(Error::InvalidImage)?;
		let len = word_at(program, 40).ok_or(Error::InvalidImage)?;
		address.checked_add(len).ok_or(Error::InvalidImage)?;
		total = total
			.checked_add(len)
			.filter(|total| *total <= MAX_IMAGE_BYTES)
			.ok_or(Error::InvalidImage)?;
		if len != 0 {
			segments.push(LoadSegment {
				address,
				len,
				executable: flags & 1 != 0,
				writable: flags & 2 != 0,
			});
		}
	}
	Ok(segments)
}

/// The address of the export `name` of the already loaded library `library`,
/// such as `libtier0.so`, or `None` if no library of that name is loaded or
/// it does not export `name`. This never loads a library.
///
/// The library is kept loaded during the lookup only. The address is usable
/// as a native pointer only while the library stays loaded, and only with the
/// type the library exports it with.
pub fn loaded_symbol(library: &CStr, name: &CStr) -> Option<NonNull<c_void>> {
	// SAFETY: `RTLD_NOLOAD` only finds a library that is already loaded, and
	// adds a reference to it that keeps it loaded until it is closed below.
	let handle = NonNull::new(unsafe { dlopen(library.as_ptr(), RTLD_NOW | RTLD_NOLOAD) })?;

	// SAFETY: The handle is live until closed.
	let symbol = unsafe { dlsym(handle.as_ptr(), name.as_ptr()) };

	// SAFETY: This releases only the reference `dlopen` added.
	unsafe { dlclose(handle.as_ptr()) };

	NonNull::new(symbol)
}

pub(super) fn program_headers(
	memory: &MemoryReader,
	base: usize,
	header: &[u8],
) -> Result<Vec<u8>, Error> {
	if header.get(..7) != Some(b"\x7fELF\x02\x01\x01")
		|| u16_at(header, 16) != Some(3)
		|| u16_at(header, 18) != Some(62)
	{
		return Err(Error::InvalidImage);
	}
	let offset = word_at(header, 32).ok_or(Error::InvalidImage)?;
	let entry_size = u16_at(header, 54).ok_or(Error::InvalidImage)? as usize;
	let count = u16_at(header, 56).ok_or(Error::InvalidImage)? as usize;
	if entry_size != 56 || !(1..=128).contains(&count) || offset > 0x100000 {
		return Err(Error::InvalidImage);
	}
	memory.copy(
		base.checked_add(offset).ok_or(Error::InvalidImage)?,
		entry_size * count,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn memory_reader_copies_owned_bytes_and_rejects_invalid_ranges() {
		let memory = MemoryReader::open().unwrap();
		let bytes = [1_u8, 2, 3, 4];
		assert_eq!(
			memory.copy(bytes.as_ptr() as usize, bytes.len()).unwrap(),
			bytes
		);
		assert!(matches!(
			memory.copy(usize::MAX, 2),
			Err(Error::InvalidImage)
		));
		assert!(matches!(
			memory.copy(1, MAX_IMAGE_BYTES + 1),
			Err(Error::InvalidImage)
		));
		assert!(memory.copy(0, 8).is_err());
	}

	#[test]
	fn rejects_overflowing_or_excessive_load_segments() {
		let mut program = [0_u8; 56];
		program[..4].copy_from_slice(&1_u32.to_le_bytes());
		program[4..8].copy_from_slice(&5_u32.to_le_bytes());
		program[16..24].copy_from_slice(&usize::MAX.to_le_bytes());
		program[40..48].copy_from_slice(&1_usize.to_le_bytes());
		assert!(load_segments(1, &program).is_err());
		program[16..24].copy_from_slice(&0_usize.to_le_bytes());
		program[40..48].copy_from_slice(&(MAX_IMAGE_BYTES + 1).to_le_bytes());
		assert!(load_segments(0, &program).is_err());
	}

	#[test]
	fn snapshots_code_and_data_with_load_permissions() {
		let anchor = snapshots_code_and_data_with_load_permissions as *const () as usize;
		// SAFETY: The test executable stays loaded throughout this test.
		let module = unsafe { Module::at(anchor) }.unwrap();
		assert!(module.path().is_file());
		assert!(is_executable(anchor));
		// SAFETY: The test executable stays loaded throughout this snapshot.
		let image = unsafe { load(anchor) }.unwrap();
		assert_eq!(image.base, module.base());
		assert!(
			image
				.sections
				.iter()
				.any(|section| section.executable && !section.bytes.is_empty())
		);
		assert!(image.sections.iter().any(|section| section.writable));
		assert!(image.sections.iter().any(|section| !section.executable));
	}

	#[test]
	fn symbols_are_found_only_in_loaded_libraries() {
		assert!(loaded_symbol(c"libc.so.6", c"getpid").is_some());
		assert!(loaded_symbol(c"libc.so.6", c"source_sdk_2013_raw_absent").is_none());
		assert!(loaded_symbol(c"libsource_sdk_2013_raw_absent.so", c"Msg").is_none());
	}
}
