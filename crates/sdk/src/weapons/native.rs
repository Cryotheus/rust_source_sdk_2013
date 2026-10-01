//! Retail TF2 item generation. Resolve only the supported seven-argument
//! CItemGeneration::SpawnItem ABI; newer SDK headers add a class argument.
//! All inspection uses owned snapshots. Missing/ambiguous signatures, stripped
//! ELF symbols, or a changed file fail closed before calling native code.

use super::WeaponError;
use crate::Server;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

struct Targets {
	spawn: usize,
	singleton: usize,
	schema: usize,
	schema_offset: usize,
	definition: usize,
}

/// The caller supplies the same spawn/callback lifetime guarantees as `give`.
pub(super) unsafe fn spawn(
	server: Server<'_>,
	definition: u16,
	origin: sys::Vector,
	classname: Option<&CStr>,
) -> Result<NonNull<sys::CBaseEntity>, WeaponError> {
	let targets = platform::resolve(server.game_server_factory().as_raw() as usize)
		.ok_or(WeaponError::NativeUnavailable)?;
	type Schema = unsafe extern "C" fn() -> *mut c_void;
	type Definition = unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void;
	type Spawn = unsafe extern "C" fn(
		*mut c_void,
		i32,
		*const sys::Vector,
		*const sys::QAngle,
		i32,
		i32,
		*const c_char,
	) -> *mut sys::CBaseEntity;
	// SAFETY: The resolver verifies these functions in the callback's game
	// module. Linux uses exact mangled signatures and identical live/file code;
	// Windows verifies the native call chain and the argument setup at its
	// schema lookup. No item-view or schema layout is manufactured by Rust.
	let get_schema: Schema = unsafe { std::mem::transmute(targets.schema) };
	let get_definition: Definition = unsafe { std::mem::transmute(targets.definition) };
	let generate: Spawn = unsafe { std::mem::transmute(targets.spawn) };
	let schema = unsafe { get_schema() };
	if schema.is_null() {
		return Err(WeaponError::NativeUnavailable);
	}
	// Windows' validated SpawnItem callsite adds eight bytes to ItemSystem's
	// result; Linux resolves GetItemSchema itself, whose adjustment is zero.
	let schema = unsafe { schema.byte_add(targets.schema_offset) };
	let fallback = unsafe { get_definition(schema, -1) };
	let item = unsafe { get_definition(schema, i32::from(definition)) };
	// Unknown indices return the default item, which could otherwise create
	// an unrelated entity. Schema loading excludes all negative indices.
	if item.is_null() || item == fallback {
		return Err(WeaponError::CreationFailed);
	}
	let angles = sys::QAngle {
		x: 0.0,
		y: 0.0,
		z: 0.0,
	};
	// SAFETY: Native generation initializes the embedded CEconItemView and
	// invokes Spawn/Activate. The caller guarantees those callbacks preserve
	// Server's lifetime contract. Level one / unique quality match the native
	// GenerateItemFromDefIndex wrapper. Inputs live through this call.
	let entity = unsafe {
		generate(
			targets.singleton as *mut c_void,
			i32::from(definition),
			&origin,
			&angles,
			1,
			6,
			classname.map_or(std::ptr::null(), CStr::as_ptr),
		)
	};
	NonNull::new(entity).ok_or(WeaponError::CreationFailed)
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
#[cfg(any(target_os = "linux", test))]
fn word_at(bytes: &[u8], offset: usize) -> Option<usize> {
	Some(usize::from_le_bytes(
		bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
	))
}
fn relative(address: usize, bytes: &[u8], operand: usize) -> Option<usize> {
	let displacement = u32_at(bytes, operand)? as i32;
	address
		.checked_add(operand)?
		.checked_add(4)?
		.checked_add_signed(displacement as isize)
}

#[cfg(any(target_os = "windows", test))]
fn pattern(bytes: &[u8], expected: &[i16]) -> bool {
	bytes.len() >= expected.len()
		&& bytes
			.iter()
			.zip(expected)
			.all(|(actual, expected)| *expected < 0 || i16::from(*actual) == *expected)
}

#[cfg(target_os = "windows")]
mod platform {
	use super::*;

	struct Section {
		address: usize,
		bytes: Vec<u8>,
		executable: bool,
		writable: bool,
	}
	struct Image {
		sections: Vec<Section>,
	}
	impl Image {
		fn read(&self, address: usize, len: usize) -> Option<&[u8]> {
			self.sections.iter().find_map(|section| {
				let offset = address.checked_sub(section.address)?;
				section.bytes.get(offset..offset.checked_add(len)?)
			})
		}
		fn executable(&self, address: usize) -> bool {
			self.sections
				.iter()
				.any(|s| s.executable && (s.address..s.address + s.bytes.len()).contains(&address))
		}
		fn unique(&self, signature: &[i16]) -> Option<usize> {
			if signature.is_empty() {
				return None;
			}
			let mut found = None;
			for section in self.sections.iter().filter(|s| s.executable) {
				// All supported MSVC function entries are 16-byte aligned. This
				// also avoids treating an interior instruction sequence as a target.
				let start = section.address.wrapping_neg() & 15;
				let Some(bytes) = section.bytes.get(start..) else {
					continue;
				};
				for (index, bytes) in bytes.windows(signature.len()).step_by(16).enumerate() {
					if pattern(bytes, signature) {
						if found.is_some() {
							return None;
						}
						found = Some(section.address + start + index * 16);
					}
				}
			}
			found
		}
		fn call(&self, address: usize) -> Option<usize> {
			let bytes = self.read(address, 5)?;
			if bytes[0] != 0xe8 {
				return None;
			}
			let target = relative(address, bytes, 1)?;
			self.executable(target).then_some(target)
		}
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
		fn FreeLibrary(module: *mut c_void) -> i32;
		fn GetCurrentProcess() -> *mut c_void;
		fn ReadProcessMemory(
			process: *mut c_void,
			base: *const c_void,
			buffer: *mut c_void,
			len: usize,
			read: *mut usize,
		) -> i32;
	}
	struct Module(*mut c_void);
	impl Drop for Module {
		fn drop(&mut self) {
			// SAFETY: Balances the acquired loader reference.
			unsafe {
				FreeLibrary(self.0);
			}
		}
	}
	fn copy(address: usize, len: usize) -> Option<Vec<u8>> {
		if len > 0x10000000 || address.checked_add(len).is_none() {
			return None;
		}
		let mut bytes = vec![0; len];
		let mut read = 0;
		// SAFETY: Windows validates readable source memory; the owned output
		// allocation is len bytes. No references into mutable C++ data exist.
		let success = unsafe {
			ReadProcessMemory(
				GetCurrentProcess(),
				address as *const c_void,
				bytes.as_mut_ptr().cast(),
				len,
				&mut read,
			)
		};
		(success != 0 && read == len).then_some(bytes)
	}
	fn load(factory: usize) -> Option<Image> {
		let mut handle = std::ptr::null_mut();
		// SAFETY: FROM_ADDRESS uses the live factory as an address, not UTF-16.
		if unsafe { GetModuleHandleExW(4, factory as *const u16, &mut handle) } == 0 {
			return None;
		}
		let module = Module(handle);
		let base = module.0 as usize;
		let dos = copy(base, 64)?;
		if dos[..2] != *b"MZ" {
			return None;
		}
		let pe = u32_at(&dos, 60)? as usize;
		if !(64..=0x100000).contains(&pe) {
			return None;
		}
		let coff = copy(base.checked_add(pe)?, 24)?;
		if coff[..4] != *b"PE\0\0" || u16_at(&coff, 4)? != 0x8664 {
			return None;
		}
		let count = u16_at(&coff, 6)? as usize;
		let optional_size = u16_at(&coff, 20)? as usize;
		if !(1..=96).contains(&count) || !(112..=4096).contains(&optional_size) {
			return None;
		}
		let optional = copy(base.checked_add(pe + 24)?, optional_size)?;
		if u16_at(&optional, 0)? != 0x20b {
			return None;
		}
		let size = u32_at(&optional, 56)? as usize;
		if !(pe + 24 + optional_size + count * 40..=0x10000000).contains(&size) {
			return None;
		}
		let headers = copy(base.checked_add(pe + 24 + optional_size)?, count * 40)?;
		let mut sections = Vec::new();
		for header in headers.chunks_exact(40) {
			let offset = u32_at(header, 12)? as usize;
			let len = u32_at(header, 8)? as usize;
			let flags = u32_at(header, 36)?;
			if offset.checked_add(len)? > size {
				return None;
			}
			if len == 0 || flags & 0x40000000 == 0 {
				continue;
			}
			let address = base.checked_add(offset)?;
			sections.push(Section {
				address,
				bytes: copy(address, len)?,
				executable: flags & 0x20000000 != 0,
				writable: flags & 0x80000000 != 0,
			});
		}
		let image = Image { sections };
		image.executable(factory).then_some(image)
	}

	// Bravo retail Windows x64. Wildcards are relative call/jump operands;
	// the complete argument setup and Spawn/Activate virtual calls are fixed.
	const SPAWN: &[i16] = &[
		0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
		0x48, 0x89, 0x7c, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x49, 0x8b, 0xe9, 0x4d,
		0x8b, 0xf0, 0x8b, 0xf2, 0xe8, -1, -1, -1, -1, 0x0f, 0xb7, 0xd6, 0x48, 0x8d, 0x48, 0x08,
		0xe8, -1, -1, -1, -1,
	];
	const RANDOM: &[i16] = &[
		0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
		0x57, 0x48, 0x83, 0xec, 0x50, 0x49, 0x8b, 0xf9, 0x49, 0x8b, 0xf0, 0x48, 0x8b, 0xda, 0x48,
		0x8b, 0xe9, 0xe8, -1, -1, -1, -1,
	];
	const GIVE: &[i16] = &[
		0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x57, 0x48,
		0x81, 0xec, 0xb0, 0, 0, 0, 0x0f, 0xb6, 0x9c, 0x24, 0xf0, 0, 0, 0, 0x49, 0x8b, 0xe9, 0x45,
		0x8b, 0xf8, 0x48, 0x8b, 0xfa, 0x48, 0x8b, 0xf1,
	];
	pub(super) fn resolve(factory: usize) -> Option<Targets> {
		let image = load(factory)?;
		resolve_image(&image)
	}
	fn resolve_image(image: &Image) -> Option<Targets> {
		let spawn = image.unique(SPAWN)?;
		let random = image.unique(RANDOM)?;
		let give = image.unique(GIVE)?;
		// Independent native callers must agree on both item-generation targets.
		if image.call(random + 0x69)? != spawn || image.call(give + 0x155)? != random {
			return None;
		}
		let getter = image.call(give + 0x135)?;
		if image.call(give + 0x72)? != getter {
			return None;
		}
		let bytes = image.read(getter, 8)?;
		if !pattern(bytes, &[0x48, 0x8d, 0x05, -1, -1, -1, -1, 0xc3]) {
			return None;
		}
		let singleton = relative(getter, bytes, 3)?;
		if !image.sections.iter().any(|s| {
			s.writable
				&& singleton >= s.address
				&& singleton
					.checked_add(16)
					.is_some_and(|end| end <= s.address + s.bytes.len())
		}) {
			return None;
		}
		let tail = image.read(spawn + 0xc1, 30)?;
		if !pattern(
			tail,
			&[
				0x48, 0x8b, 0x03, 0x48, 0x8b, 0xcb, 0xff, 0x90, 0xc0, 0, 0, 0, 0x48, 0x8b, 0x03,
				0x48, 0x8b, 0xcb, 0xff, 0x90, 0x18, 0x01, 0, 0, 0x48, 0x8b, 0xc3,
			],
		) {
			return None;
		}
		let schema = image.call(spawn + 0x22)?;
		if image.call(random + 0x20)? != schema {
			return None;
		}
		let definition = image.call(spawn + 0x2e)?;
		Some(Targets {
			spawn,
			singleton,
			schema,
			schema_offset: 8,
			definition,
		})
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		#[test]
		fn signatures_reject_ambiguity_and_calls_outside_executable_sections() {
			let mut image = Image {
				sections: vec![Section {
					address: 0x1000,
					bytes: vec![0x90; 256],
					executable: true,
					writable: false,
				}],
			};
			let bytes = &mut image.sections[0].bytes;
			bytes[0x10..0x13].copy_from_slice(&[0x48, 0x89, 0xff]);
			assert_eq!(image.unique(&[0x48, -1, 0xff]), Some(0x1010));
			image.sections[0].bytes[0x20..0x23].copy_from_slice(&[0x48, 0x89, 0xff]);
			assert!(image.unique(&[0x48, -1, 0xff]).is_none());
			image.sections[0].bytes[0x30..0x35].copy_from_slice(&[0xe8, 0, 0, 0, 0]);
			assert_eq!(image.call(0x1030), Some(0x1035));
			image.sections[0].bytes[0x31..0x35].copy_from_slice(&0x1000_i32.to_le_bytes());
			assert!(image.call(0x1030).is_none());
		}
		#[test]
		#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
		fn retail_item_generation_call_chain() {
			let bytes =
				std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE"))
					.unwrap();
			let pe = u32_at(&bytes, 60).unwrap() as usize;
			let count = u16_at(&bytes, pe + 6).unwrap() as usize;
			let optional_size = u16_at(&bytes, pe + 20).unwrap() as usize;
			let base = word_at(&bytes, pe + 24 + 24).unwrap();
			let mut sections = Vec::new();
			for header in bytes[pe + 24 + optional_size..][..count * 40].chunks_exact(40) {
				let offset = u32_at(header, 12).unwrap() as usize;
				let len = u32_at(header, 8).unwrap() as usize;
				let raw_size = u32_at(header, 16).unwrap() as usize;
				let raw = u32_at(header, 20).unwrap() as usize;
				let flags = u32_at(header, 36).unwrap();
				if len == 0 || flags & 0x40000000 == 0 {
					continue;
				}
				let mut data = vec![0; len];
				let copy = raw_size.min(len);
				data[..copy].copy_from_slice(&bytes[raw..raw + copy]);
				sections.push(Section {
					address: base + offset,
					bytes: data,
					executable: flags & 0x20000000 != 0,
					writable: flags & 0x80000000 != 0,
				});
			}
			let mut image = Image { sections };
			let targets =
				resolve_image(&image).expect("retail signatures and independent call references");
			assert!(image.executable(targets.spawn));
			assert!(image.executable(targets.schema));
			assert!(image.executable(targets.definition));
			assert_eq!(targets.schema_offset, 8);
			// Matching prologues alone are insufficient: redirect the caller to
			// another executable function and require the cross-check to reject it.
			let call = image.unique(RANDOM).unwrap() + 0x69;
			let displacement =
				i32::try_from(targets.definition as isize - (call + 5) as isize).unwrap();
			let section = image
				.sections
				.iter_mut()
				.find(|section| {
					(section.address..section.address + section.bytes.len()).contains(&call)
				})
				.unwrap();
			let offset = call - section.address + 1;
			section.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
			assert!(resolve_image(&image).is_none());
		}
	}
}

#[cfg(target_os = "linux")]
mod platform {
	use super::*;
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
		fn dladdr(address: *const c_void, info: *mut DlInfo) -> i32;
	}
	struct Elf<'a> {
		bytes: &'a [u8],
		headers: Vec<&'a [u8]>,
	}
	impl<'a> Elf<'a> {
		fn new(bytes: &'a [u8]) -> Option<Self> {
			if bytes.get(..7)? != b"\x7fELF\x02\x01\x01"
				|| u16_at(bytes, 18)? != 62
				|| u16_at(bytes, 16)? != 3
			{
				return None;
			}
			let offset = word_at(bytes, 40)?;
			let size = u16_at(bytes, 58)? as usize;
			let count = u16_at(bytes, 60)? as usize;
			if size != 64 || !(1..=4096).contains(&count) {
				return None;
			}
			let headers = bytes
				.get(offset..offset.checked_add(size.checked_mul(count)?)?)?
				.chunks_exact(size)
				.collect();
			Some(Self { bytes, headers })
		}
		fn section(&self, header: &[u8]) -> Option<&'a [u8]> {
			let offset = word_at(header, 24)?;
			let len = word_at(header, 32)?;
			self.bytes.get(offset..offset.checked_add(len)?)
		}
		fn symbol(&self, name: &[u8]) -> Option<(usize, &'a [u8])> {
			let mut found = None;
			for header in &self.headers {
				if u32_at(header, 4)? != 2 || word_at(header, 56)? != 24 {
					continue;
				}
				let names = self.section(self.headers.get(u32_at(header, 40)? as usize)?)?;
				for symbol in self.section(header)?.chunks_exact(24) {
					let offset = u32_at(symbol, 0)? as usize;
					let tail = names.get(offset..)?;
					let end = tail.iter().position(|b| *b == 0)?;
					if &tail[..end] != name || symbol[4] & 15 != 2 {
						continue;
					}
					let section = self.headers.get(u16_at(symbol, 6)? as usize)?;
					if word_at(section, 8)? & 4 == 0 {
						return None;
					}
					let address = word_at(symbol, 8)?;
					let size = word_at(symbol, 16)?;
					if !(1..=65536).contains(&size) {
						return None;
					}
					let offset = address.checked_sub(word_at(section, 16)?)?;
					let body = self
						.section(section)?
						.get(offset..offset.checked_add(size)?)?;
					if found.is_some() {
						return None;
					}
					found = Some((address, body));
				}
			}
			found
		}
	}
	pub(super) fn resolve(factory: usize) -> Option<Targets> {
		let mut info = std::mem::MaybeUninit::<DlInfo>::uninit();
		// SAFETY: dladdr validates the live factory and initializes Dl_info;
		// Server guarantees the module and its filename remain live here.
		if unsafe { dladdr(factory as *const c_void, info.as_mut_ptr()) } == 0 {
			return None;
		}
		let info = unsafe { info.assume_init() };
		if info.name.is_null() {
			return None;
		}
		use std::os::unix::ffi::OsStrExt;
		let path = std::ffi::OsStr::from_bytes(unsafe { CStr::from_ptr(info.name) }.to_bytes());
		let file = File::open(path).ok()?;
		if file.metadata().ok()?.len() > 0x10000000 {
			return None;
		}
		use std::io::Read;
		let mut bytes = Vec::new();
		file.take(0x10000001).read_to_end(&mut bytes).ok()?;
		if bytes.len() > 0x10000000 {
			return None;
		}
		let elf = Elf::new(&bytes)?;
		let memory = File::open("/proc/self/mem").ok()?;
		let base = info.base as usize;
		let mut live_header = [0_u8; 64];
		memory.read_exact_at(&mut live_header, base as u64).ok()?;
		if bytes.get(..64)? != live_header {
			return None;
		}
		let offset = word_at(&bytes, 32)?;
		let entry_size = u16_at(&bytes, 54)? as usize;
		let count = u16_at(&bytes, 56)? as usize;
		if entry_size != 56 || !(1..=128).contains(&count) {
			return None;
		}
		let programs = bytes.get(offset..offset.checked_add(count.checked_mul(entry_size)?)?)?;
		let mut live_programs = vec![0; programs.len()];
		memory
			.read_exact_at(&mut live_programs, base.checked_add(offset)? as u64)
			.ok()?;
		if live_programs != programs {
			return None;
		}
		let contains = |address: usize, len: usize, permissions: u32| {
			programs.chunks_exact(56).any(|program| {
				if u32_at(program, 0) != Some(1)
					|| u32_at(program, 4).is_none_or(|flags| flags & permissions != permissions)
				{
					return false;
				}
				let Some(start) = word_at(program, 16).and_then(|offset| base.checked_add(offset))
				else {
					return false;
				};
				let Some(end) = word_at(program, 40).and_then(|len| start.checked_add(len)) else {
					return false;
				};
				address >= start && address.checked_add(len).is_some_and(|last| last <= end)
			})
		};
		if !contains(factory, 1, 5) {
			return None;
		}
		let resolve = |name: &[u8]| -> Option<(usize, &[u8])> {
			let (offset, body) = elf.symbol(name)?;
			let address = base.checked_add(offset)?;
			if !contains(address, body.len(), 5) {
				return None;
			}
			let mut live = vec![0; body.len()];
			memory.read_exact_at(&mut live, address as u64).ok()?;
			// Reject a replaced on-disk library or a detoured target. PIC text
			// has no runtime relocations in these TF2 functions.
			(live == body).then_some((address, body))
		};
		let (spawn, _) = resolve(b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKc")?;
		let (getter, body) = resolve(b"_Z14ItemGenerationv")?;
		if body.len() != 8 || body[..3] != [0x48, 0x8d, 0x05] || body[7] != 0xc3 {
			return None;
		}
		let singleton = relative(getter, body, 3)?;
		if !contains(singleton, 16, 6) {
			return None;
		}
		let mut singleton_bytes = [0_u8; 16];
		memory
			.read_exact_at(&mut singleton_bytes, singleton as u64)
			.ok()?;
		let (schema, _) = resolve(b"_Z13GetItemSchemav")?;
		let (definition, _) = resolve(b"_ZN15CEconItemSchema17GetItemDefinitionEi")?;
		Some(Targets {
			spawn,
			singleton,
			schema,
			schema_offset: 0,
			definition,
		})
	}
	#[cfg(test)]
	mod tests {
		use super::*;
		#[test]
		fn elf_parser_rejects_truncation_and_wrong_abi() {
			assert!(Elf::new(&[]).is_none());
			let mut header = vec![0; 64];
			header[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
			header[16..18].copy_from_slice(&3_u16.to_le_bytes());
			header[18..20].copy_from_slice(&62_u16.to_le_bytes());
			header[40..48].copy_from_slice(&64_usize.to_le_bytes());
			header[58..60].copy_from_slice(&64_u16.to_le_bytes());
			header[60..62].copy_from_slice(&1_u16.to_le_bytes());
			assert!(Elf::new(&header).is_none());
			header.resize(128, 0);
			assert!(Elf::new(&header).is_some());
			header[4] = 1;
			assert!(Elf::new(&header).is_none());
		}
		#[test]
		#[ignore = "set TF2_SERVER_IMAGE to an authorized unstripped retail server_srv.so for binary validation"]
		fn retail_item_generation_symbols() {
			let bytes =
				std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE"))
					.unwrap();
			let elf = Elf::new(&bytes).unwrap();
			for name in [
				b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKc".as_slice(),
				b"_Z13GetItemSchemav",
				b"_ZN15CEconItemSchema17GetItemDefinitionEi",
			] {
				assert!(elf.symbol(name).is_some());
			}
			let (getter, body) = elf.symbol(b"_Z14ItemGenerationv").unwrap();
			assert_eq!(body.len(), 8);
			assert_eq!(&body[..3], &[0x48, 0x8d, 0x05]);
			assert_eq!(body[7], 0xc3);
			assert!(relative(getter, body, 3).is_some());
			assert!(
				elf.symbol(b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKci")
					.is_none(),
				"retail ABI must exclude the SDK's extra class argument"
			);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn relative_operands_are_signed_checked_and_bounded() {
		assert_eq!(
			relative(0x1000, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1),
			Some(0xff5)
		);
		assert_eq!(relative(0, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1), None);
		assert_eq!(relative(0x1000, &[0xe8, 0, 0], 1), None);
		assert!(pattern(&[0x48, 0x89, 0xff], &[0x48, -1, 0xff]));
		assert!(!pattern(&[0x48, 0x89], &[0x48, -1, 0xff]));
	}
}
