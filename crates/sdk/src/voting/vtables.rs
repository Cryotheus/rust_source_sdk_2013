//! Finds primary vote-issue vtables using compiler RTTI in an owned snapshot.
//! No entity layouts, file offsets, or executable byte signatures are used.

use super::VoteHookTargetError;
use std::ffi::c_void;
use std::ops::Range;
use std::ptr::NonNull;

struct Segment {
	address: usize,
	bytes: Vec<u8>,
}

pub(super) struct Image {
	base: usize,
	data: Vec<Segment>,
	code: Vec<Range<usize>>,
}

fn invalid() -> VoteHookTargetError {
	VoteHookTargetError::InvalidImage
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
	Some(u16::from_le_bytes(
		bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
	))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
	Some(u32::from_le_bytes(
		bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
	))
}

fn word_at(bytes: &[u8], offset: usize) -> Option<usize> {
	Some(usize::from_le_bytes(
		bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
	))
}

impl Image {
	pub(super) fn load(factory: usize) -> Result<Self, VoteHookTargetError> {
		platform::load(factory)
	}

	pub(super) fn find(&self, class: &str, slot: usize) -> Option<NonNull<*mut c_void>> {
		if self.base == 0 {
			return None;
		}
		#[cfg(target_os = "windows")]
		let candidates = self.msvc(class, slot);
		#[cfg(target_os = "linux")]
		let candidates = self.itanium(class, slot);
		let mut candidates = candidates.into_iter();
		let one = candidates.next()?;
		if candidates.next().is_some() {
			return None;
		}
		NonNull::new(one as *mut *mut c_void)
	}

	fn matches(&self, bytes: &[u8], alignment: usize) -> Vec<usize> {
		self.data
			.iter()
			.flat_map(|section| {
				section
					.bytes
					.windows(bytes.len())
					.enumerate()
					.filter_map(|(offset, candidate)| {
						let address = section.address + offset;
						(address % alignment == 0 && candidate == bytes).then_some(address)
					})
			})
			.collect()
	}

	fn read(&self, address: usize, len: usize) -> Option<&[u8]> {
		self.data.iter().find_map(|section| {
			let offset = address.checked_sub(section.address)?;
			section.bytes.get(offset..offset.checked_add(len)?)
		})
	}

	fn valid_table(&self, address: usize, slot: usize) -> bool {
		let Some(bytes) = slot
			.checked_add(1)
			.and_then(|n| n.checked_mul(8))
			.and_then(|size| self.read(address, size))
		else {
			return false;
		};
		let Some(function) = word_at(bytes, slot * 8) else {
			return false;
		};
		// SourceHook/KHook can retain a trampoline in this slot after our
		// handler is removed, or another plugin may have installed one first.
		// RTTI still identifies the original class and slot ABI. Require actual
		// executable memory, but do not require that trampoline to live inside
		// the game's image. Only the game's vtable address is returned/retained.
		self.code.iter().any(|range| range.contains(&function)) || platform::is_executable(function)
	}

	#[cfg(any(target_os = "windows", test))]
	fn msvc(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();
		for name in self.matches(format!(".?AV{class}@@\0").as_bytes(), 1) {
			let Some(descriptor) = name.checked_sub(16) else {
				continue;
			};
			let Some(relative) = descriptor
				.checked_sub(self.base)
				.and_then(|n| u32::try_from(n).ok())
			else {
				continue;
			};
			for reference in self.matches(&relative.to_le_bytes(), 4) {
				let Some(locator) = reference.checked_sub(12) else {
					continue;
				};
				let Some(bytes) = self.read(locator, 24) else {
					continue;
				};
				// 64-bit MSVC complete-object locator: signature=1, primary
				// subobject offset=0, no construction displacement, self RVA.
				if u32_at(bytes, 0) != Some(1)
					|| u32_at(bytes, 4) != Some(0)
					|| u32_at(bytes, 8) != Some(0)
					|| u32_at(bytes, 20).map(|n| self.base + n as usize) != Some(locator)
				{
					continue;
				}
				for pointer in self.matches(&locator.to_le_bytes(), 8) {
					let table = pointer + 8;
					if self.valid_table(table, slot) {
						tables.push(table);
					}
				}
			}
		}
		tables.sort_unstable();
		tables.dedup();
		tables
	}

	#[cfg(any(target_os = "linux", test))]
	fn itanium(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();
		for name in self.matches(format!("{}{class}\0", class.len()).as_bytes(), 1) {
			for name_pointer in self.matches(&name.to_le_bytes(), 8) {
				let Some(type_info) = name_pointer.checked_sub(8) else {
					continue;
				};
				for reference in self.matches(&type_info.to_le_bytes(), 8) {
					// Itanium primary address point follows offset-to-top=0
					// and the pointer to the class's type_info object.
					let Some(prefix) = reference.checked_sub(8).and_then(|p| self.read(p, 8))
					else {
						continue;
					};
					let table = reference + 8;
					if word_at(prefix, 0) == Some(0) && self.valid_table(table, slot) {
						tables.push(table);
					}
				}
			}
		}
		tables.sort_unstable();
		tables.dedup();
		tables
	}
}

#[cfg(target_os = "windows")]
mod platform {
	use super::*;
	use std::mem::MaybeUninit;
	use std::ptr;

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

	const _: () = assert!(size_of::<MemoryInformation>() == 48);

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
		fn FreeLibrary(module: *mut c_void) -> i32;
		fn GetCurrentProcess() -> *mut c_void;
		fn VirtualQuery(
			address: *const c_void,
			information: *mut MemoryInformation,
			size: usize,
		) -> usize;
		fn ReadProcessMemory(
			process: *mut c_void,
			base: *const c_void,
			buffer: *mut c_void,
			size: usize,
			read: *mut usize,
		) -> i32;
	}

	pub(super) fn is_executable(address: usize) -> bool {
		let mut information = MaybeUninit::<MemoryInformation>::uninit();
		// SAFETY: Windows queries the address without dereferencing it. The
		// output has the verified Windows x64 MEMORY_BASIC_INFORMATION layout.
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
		// SAFETY: VirtualQuery initialized the complete structure on success.
		let information = unsafe { information.assume_init() };
		information.state == 0x1000 // MEM_COMMIT
			&& information.protection & 0x101 == 0 // no GUARD/NOACCESS
			&& information.protection & 0xf0 != 0 // executable protection
	}

	struct Module(*mut c_void);
	impl Drop for Module {
		fn drop(&mut self) {
			// SAFETY: Releases exactly the loader reference acquired below.
			unsafe { FreeLibrary(self.0) };
		}
	}

	fn copy(address: usize, len: usize) -> Result<Vec<u8>, VoteHookTargetError> {
		if len > 0x40000000 || address.checked_add(len).is_none() {
			return Err(invalid());
		}
		let mut bytes = vec![0; len];
		let mut read = 0;
		// SAFETY: Windows checks the source mapping; the owned destination is
		// valid for len bytes. No references into engine-owned memory exist.
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
			return Err(invalid());
		}
		Ok(bytes)
	}

	pub(super) fn load(factory: usize) -> Result<Image, VoteHookTargetError> {
		let mut handle = ptr::null_mut();
		// SAFETY: FROM_ADDRESS treats factory as an address, not a string.
		// Server supplies a live factory; a loader reference pins its module
		// while making the snapshot. The Server contract keeps it loaded later.
		if unsafe { GetModuleHandleExW(4, factory as *const u16, &mut handle) } == 0 {
			return Err(std::io::Error::last_os_error().into());
		}
		let module = Module(handle);
		let base = module.0 as usize;
		let dos = copy(base, 64)?;
		if dos[..2] != *b"MZ" {
			return Err(invalid());
		}
		let pe = u32_at(&dos, 60).ok_or_else(invalid)? as usize;
		if !(64..=0x100000).contains(&pe) {
			return Err(invalid());
		}
		let coff = copy(base + pe, 24)?;
		if coff[..4] != *b"PE\0\0" || u16_at(&coff, 4) != Some(0x8664) {
			return Err(invalid());
		}
		let count = u16_at(&coff, 6).ok_or_else(invalid)? as usize;
		let optional_size = u16_at(&coff, 20).ok_or_else(invalid)? as usize;
		if !(1..=96).contains(&count) || !(112..=4096).contains(&optional_size) {
			return Err(invalid());
		}
		let optional = copy(base + pe + 24, optional_size)?;
		if u16_at(&optional, 0) != Some(0x20b) {
			return Err(invalid());
		}
		let size = u32_at(&optional, 56).ok_or_else(invalid)? as usize;
		if !(pe + 24 + optional_size + count * 40..=0x40000000).contains(&size) {
			return Err(invalid());
		}
		let sections = copy(base + pe + 24 + optional_size, count * 40)?;
		let mut image = Image {
			base,
			data: Vec::new(),
			code: Vec::new(),
		};
		for section in sections.chunks_exact(40) {
			let len = u32_at(section, 8).unwrap() as usize;
			let offset = u32_at(section, 12).unwrap() as usize;
			let flags = u32_at(section, 36).unwrap();
			if offset.checked_add(len).is_none_or(|end| end > size) {
				return Err(invalid());
			}
			if len == 0 || flags & 0x40000000 == 0 {
				continue;
			}
			let address = base + offset;
			if flags & 0x20000000 != 0 {
				image.code.push(address..address + len);
			} else {
				image.data.push(Segment {
					address,
					bytes: copy(address, len)?,
				});
			}
		}
		if !image.code.iter().any(|range| range.contains(&factory)) {
			return Err(invalid());
		}
		Ok(image)
	}
}

#[cfg(target_os = "linux")]
mod platform {
	use super::*;
	use std::ffi::{c_char, c_int};
	use std::fs::File;
	use std::os::unix::fs::FileExt;

	#[repr(C)]
	struct DlInfo {
		name: *const c_char,
		base: *mut c_void,
		symbol: *const c_char,
		address: *mut c_void,
	}

	#[link(name = "dl")]
	unsafe extern "C" {
		fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
	}

	pub(super) fn is_executable(address: usize) -> bool {
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

	fn copy(memory: &File, address: usize, len: usize) -> Result<Vec<u8>, VoteHookTargetError> {
		if len > 0x40000000 || address.checked_add(len).is_none() {
			return Err(invalid());
		}
		let mut bytes = vec![0; len];
		memory.read_exact_at(&mut bytes, address as u64)?;
		Ok(bytes)
	}

	pub(super) fn load(factory: usize) -> Result<Image, VoteHookTargetError> {
		let mut info = std::mem::MaybeUninit::<DlInfo>::uninit();
		// SAFETY: dladdr validates the address and writes a complete Dl_info.
		// Server's callback contract keeps the game module loaded throughout.
		if unsafe { dladdr(factory as *const c_void, info.as_mut_ptr()) } == 0 {
			return Err(invalid());
		}
		// SAFETY: A successful dladdr initialized every field.
		let base = unsafe { info.assume_init() }.base as usize;
		let memory = File::open("/proc/self/mem")?;
		let header = copy(&memory, base, 64)?;
		if header[..7] != *b"\x7fELF\x02\x01\x01" || u16_at(&header, 18) != Some(62) {
			return Err(invalid());
		}
		let offset = word_at(&header, 32).ok_or_else(invalid)?;
		let entry_size = u16_at(&header, 54).ok_or_else(invalid)? as usize;
		let count = u16_at(&header, 56).ok_or_else(invalid)? as usize;
		if entry_size != 56 || !(1..=128).contains(&count) || offset > 0x100000 {
			return Err(invalid());
		}
		let headers = copy(&memory, base + offset, entry_size * count)?;
		let mut image = Image {
			base,
			data: Vec::new(),
			code: Vec::new(),
		};
		for header in headers.chunks_exact(entry_size) {
			let flags = u32_at(header, 4).unwrap();
			if u32_at(header, 0) != Some(1) || flags & 4 == 0 {
				continue;
			}
			let address = base
				.checked_add(word_at(header, 16).unwrap())
				.ok_or_else(invalid)?;
			let len = word_at(header, 40).unwrap();
			let end = address.checked_add(len).ok_or_else(invalid)?;
			if len == 0 {
				continue;
			}
			if flags & 1 != 0 {
				image.code.push(address..end);
			} else {
				image.data.push(Segment {
					address,
					bytes: copy(&memory, address, len)?,
				});
			}
		}
		if !image.code.iter().any(|range| range.contains(&factory)) {
			return Err(invalid());
		}
		Ok(image)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	const BASE: usize = 0x10000;

	fn fixture() -> Image {
		Image {
			base: BASE,
			data: vec![Segment {
				address: BASE,
				bytes: vec![0; 1024],
			}],
			code: vec![0x20000..0x21000],
		}
	}
	fn word(image: &mut Image, offset: usize, value: usize) {
		image.data[0].bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
	}

	#[test]
	fn msvc_requires_unique_primary_locator_and_executable_slot() {
		let mut image = fixture();
		image.data[0].bytes[0x110..0x120].copy_from_slice(b".?AVCKickIssue@@");
		for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, 0x180)] {
			image.data[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		word(&mut image, 0x200, BASE + 0x180);
		word(&mut image, 0x208 + 8 * 8, 0x20010);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x208]);
		word(&mut image, 0x280, BASE + 0x180);
		word(&mut image, 0x288 + 8 * 8, 0x20020);
		assert_eq!(image.msvc("CKickIssue", 8).len(), 2);
		word(&mut image, 0x208 + 8 * 8, 0x30000);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x288]);
	}

	#[test]
	fn itanium_excludes_secondary_and_truncated_tables() {
		let mut image = fixture();
		image.data[0].bytes[0x100..0x10d].copy_from_slice(b"10CKickIssue\0");
		word(&mut image, 0x188, BASE + 0x100);
		word(&mut image, 0x208, BASE + 0x180);
		word(&mut image, 0x210 + 9 * 8, 0x20010);
		assert_eq!(image.itanium("CKickIssue", 9), [BASE + 0x210]);
		word(&mut image, 0x200, usize::MAX - 7);
		assert!(image.itanium("CKickIssue", 9).is_empty());
		word(&mut image, 0x200, 0);
		image.data[0].bytes.truncate(0x210 + 9 * 8);
		assert!(image.itanium("CKickIssue", 9).is_empty());
	}

	#[test]
	fn retained_hook_trampolines_may_be_outside_the_game_image() {
		unsafe extern "C" fn trampoline() {}
		let mut image = fixture();
		word(&mut image, 0x100 + 8 * 8, trampoline as *const () as usize);
		assert!(image.valid_table(BASE + 0x100, 8));
		// An ordinary data allocation is never a valid replacement for code.
		let data = vec![0_u8; 32];
		word(&mut image, 0x100 + 8 * 8, data.as_ptr() as usize);
		assert!(!image.valid_table(BASE + 0x100, 8));
	}
}
