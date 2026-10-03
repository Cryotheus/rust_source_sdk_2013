//! Run-time type information that C++ compilers emit for polymorphic
//! classes: discovery of primary vtables in owned image snapshots, and checks
//! of a live object's class.
//!
//! The live readers check a polymorphic object's class before code relies on
//! a layout of the engine's that no public header declares.

#[cfg(any(target_os = "windows", test))]
use super::u32_at;

use super::{Image, is_executable, word_at};
use std::ffi::{CStr, c_char, c_void};

#[cfg(any(target_os = "windows", test))]
const _: () = {
	use std::mem::offset_of;

	assert!(size_of::<CompleteObjectLocator>() == 24);
	assert!(offset_of!(CompleteObjectLocator, signature) == 0);
	assert!(offset_of!(CompleteObjectLocator, offset) == 4);
	assert!(offset_of!(CompleteObjectLocator, constructor_displacement) == 8);
	assert!(offset_of!(CompleteObjectLocator, type_descriptor) == 12);
	assert!(offset_of!(CompleteObjectLocator, class_descriptor) == 16);
	assert!(offset_of!(CompleteObjectLocator, this) == 20);
};

/// Where an MSVC `_TypeDescriptor`'s decorated name starts: after its vtable
/// pointer and the undecorated name the runtime caches.
#[cfg(any(target_os = "windows", test))]
const TYPE_DESCRIPTOR_NAME_OFFSET: usize = 2 * size_of::<*const c_void>();

/// MSVC's `_RTTICompleteObjectLocator` for 64-bit images, whose references
/// are relative to the image's base.
///
/// MSVC stores the address of a vtable's locator in the slot before the
/// vtable's first.
#[doc(alias = "_RTTICompleteObjectLocator")]
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CompleteObjectLocator {
	/// [`Self::SIGNATURE`] for 64-bit images.
	pub signature: u32,

	/// The subobject's offset in its complete object.
	pub offset: u32,

	/// `cdOffset`, the constructor displacement offset, which this module
	/// does not read.
	pub constructor_displacement: u32,

	/// The image offset of the complete object's `_TypeDescriptor`, which
	/// holds its decorated class name.
	pub type_descriptor: u32,

	/// The image offset of the complete object's
	/// `_RTTIClassHierarchyDescriptor`.
	pub class_descriptor: u32,

	/// The locator's own offset in the image.
	pub this: u32,
}

#[cfg(any(target_os = "windows", test))]
impl CompleteObjectLocator {
	/// The [`signature`](Self::signature) of a locator in a 64-bit image.
	pub const SIGNATURE: u32 = 1;

	/// Reads a locator from the start of `bytes`, as it lies in an image.
	fn parse(bytes: &[u8]) -> Option<Self> {
		use std::mem::offset_of;

		Some(Self {
			signature: u32_at(bytes, offset_of!(Self, signature))?,
			offset: u32_at(bytes, offset_of!(Self, offset))?,
			constructor_displacement: u32_at(bytes, offset_of!(Self, constructor_displacement))?,
			type_descriptor: u32_at(bytes, offset_of!(Self, type_descriptor))?,
			class_descriptor: u32_at(bytes, offset_of!(Self, class_descriptor))?,
			this: u32_at(bytes, offset_of!(Self, this))?,
		})
	}
}

impl Image {
	#[cfg(any(target_os = "linux", test))]
	fn itanium(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();

		for name in self.matches(format!("{}{class}\0", class.len()).as_bytes(), 1) {
			for name_pointer in self.matches(&name.to_le_bytes(), 8) {
				let Some(type_info) = name_pointer.checked_sub(8) else {
					continue;
				};

				for reference in self.matches(&type_info.to_le_bytes(), 8) {
					// The primary address point follows offset-to-top=0 and the
					// pointer to the class's type_info object.
					let Some(prefix) = reference.checked_sub(8).and_then(|p| self.rtti_read(p, 8))
					else {
						continue;
					};
					let Some(table) = reference.checked_add(8) else {
						continue;
					};

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

	#[cfg(any(target_os = "windows", test))]
	fn msvc(&self, class: &str, slot: usize) -> Vec<usize> {
		let mut tables = Vec::new();

		for name in self.matches(format!(".?AV{class}@@\0").as_bytes(), 1) {
			let Some(descriptor) = name.checked_sub(TYPE_DESCRIPTOR_NAME_OFFSET) else {
				continue;
			};
			let Some(relative) = descriptor
				.checked_sub(self.base)
				.and_then(|n| u32::try_from(n).ok())
			else {
				continue;
			};

			for reference in self.matches(&relative.to_le_bytes(), 4) {
				let Some(locator) = reference
					.checked_sub(std::mem::offset_of!(CompleteObjectLocator, type_descriptor))
				else {
					continue;
				};
				let Some(located) = self
					.rtti_read(locator, size_of::<CompleteObjectLocator>())
					.and_then(CompleteObjectLocator::parse)
				else {
					continue;
				};

				// MSVC x64 complete-object locator: signature=1, primary
				// subobject offset=0, no construction displacement, self RVA.
				if located.signature != CompleteObjectLocator::SIGNATURE
					|| located.offset != 0
					|| located.constructor_displacement != 0
					|| self.base.checked_add(located.this as usize) != Some(locator)
				{
					continue;
				}

				for pointer in self.matches(&locator.to_le_bytes(), 8) {
					let Some(table) = pointer.checked_add(8) else {
						continue;
					};
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

	/// Finds one primary vtable for a global, unqualified C++ class name.
	/// Uses MSVC x64 RTTI on Windows and Itanium RTTI on Linux. Secondary and
	/// ambiguous tables are rejected, as are tables lacking an executable entry
	/// at `slot`. An entry may point to a hook trampoline outside this image.
	///
	/// The returned address is metadata from the snapshot. It does not keep the
	/// module loaded, establish a function signature, or authorize dereferencing
	/// the address. Callers must establish those guarantees before using it.
	pub fn primary_vtable(&self, class: &str, slot: usize) -> Option<usize> {
		if self.base == 0 || class.is_empty() || class.as_bytes().contains(&0) {
			return None;
		}

		#[cfg(target_os = "windows")]
		let candidates = self.msvc(class, slot);

		#[cfg(target_os = "linux")]
		let candidates = self.itanium(class, slot);

		let mut candidates = candidates.into_iter();
		let one = candidates.next()?;
		candidates.next().is_none().then_some(one)
	}

	// RTTI and vtable records must stay wholly inside a data snapshot, even
	// when a matched reference is adjacent to an executable section.
	fn rtti_read(&self, address: usize, len: usize) -> Option<&[u8]> {
		address.checked_add(len)?;
		self.sections
			.iter()
			.filter(|section| !section.executable)
			.find_map(|section| {
				section.address.checked_add(section.bytes.len())?;
				let offset = address.checked_sub(section.address)?;
				section.bytes.get(offset..offset.checked_add(len)?)
			})
	}

	fn valid_table(&self, address: usize, slot: usize) -> bool {
		let Some(bytes) = slot
			.checked_add(1)
			.and_then(|n| n.checked_mul(8))
			.and_then(|size| self.rtti_read(address, size))
		else {
			return false;
		};
		let Some(function) = slot
			.checked_mul(8)
			.and_then(|offset| word_at(bytes, offset))
		else {
			return false;
		};

		// Hook managers can retain trampolines after removing a handler, or
		// another plugin may have hooked the slot first. RTTI still identifies
		// the original table. Do not require its function to be inside the image.
		self.executable(function) || is_executable(function)
	}
}

/// The offset of the subobject `object` points to in its complete object,
/// and the complete object's decorated class name, from MSVC's run-time type
/// information.
///
/// # Safety
///
/// `object` must point to a live polymorphic subobject, whose vtable the
/// compiler emitted with run-time type information for the target's ABI. The
/// module that emitted it must stay loaded for `'a`, which the name borrows.
#[cfg(target_os = "windows")]
pub unsafe fn dynamic_type<'a>(object: *const c_void) -> Option<(isize, &'a CStr)> {
	// SAFETY: A polymorphic subobject starts with its vtable pointer, and MSVC
	// stores the locator's address in the slot before the vtable's first.
	let locator = unsafe { object.cast::<*const *const c_void>().read().sub(1).read() }
		.cast::<CompleteObjectLocator>();

	if locator.is_null() || !locator.is_aligned() {
		return None;
	}

	// SAFETY: The compiler emitted the locator with the vtable.
	let located = unsafe { locator.read() };

	if located.signature != CompleteObjectLocator::SIGNATURE {
		return None;
	}

	let image = locator.addr().checked_sub(located.this as usize)?;

	let name = image
		.checked_add(located.type_descriptor as usize)?
		.checked_add(TYPE_DESCRIPTOR_NAME_OFFSET)?;

	// SAFETY: As above, and the name is a string in the image, which the
	// caller keeps loaded for `'a`.
	Some((located.offset as isize, unsafe {
		CStr::from_ptr(locator.cast::<c_char>().with_addr(name))
	}))
}

/// The offset of the subobject `object` points to in its complete object,
/// and the complete object's mangled class name, from the Itanium ABI's
/// run-time type information: the vtable stores the offset from the
/// subobject to its complete object two slots before its first, and the
/// `std::type_info` one slot before.
///
/// # Safety
///
/// `object` must point to a live polymorphic subobject, whose vtable the
/// compiler emitted with run-time type information for the target's ABI. The
/// module that emitted it must stay loaded for `'a`, which the name borrows.
#[cfg(not(target_os = "windows"))]
pub unsafe fn dynamic_type<'a>(object: *const c_void) -> Option<(isize, &'a CStr)> {
	// SAFETY: As the caller promises, per the Itanium ABI's vtable layout.
	let (offset_to_top, type_info) = unsafe {
		let vtable = object.cast::<*const isize>().read();

		(
			vtable.sub(2).read(),
			vtable.sub(1).read() as *const *const c_char,
		)
	};

	if type_info.is_null() {
		return None;
	}

	// SAFETY: A `std::type_info` stores its vtable pointer, then its name.
	let name = unsafe { type_info.add(1).read() };

	// SAFETY: The name is a string in the image, which the caller keeps
	// loaded for `'a`.
	(!name.is_null()).then(|| (-offset_to_top, unsafe { CStr::from_ptr(name) }))
}

/// Whether `decorated`, a class name as [`dynamic_type`] returns it, names
/// the class `class`, which MSVC decorates as `.?AVName@@`.
#[cfg(target_os = "windows")]
pub fn matches_class_name(decorated: &[u8], class: &str) -> bool {
	decorated
		.strip_prefix(b".?AV")
		.and_then(|name| name.strip_suffix(b"@@"))
		.is_some_and(|name| name == class.as_bytes())
}

/// Whether `decorated`, a class name as [`dynamic_type`] returns it, names
/// the class `class`, which the Itanium ABI mangles as its length, then the
/// name.
#[cfg(not(target_os = "windows"))]
pub fn matches_class_name(decorated: &[u8], class: &str) -> bool {
	let length = class.len().to_string();

	decorated
		.strip_prefix(length.as_bytes())
		.is_some_and(|name| name == class.as_bytes())
}

/// Where the subobject `object` points to sits in its complete object, if
/// that object's class is named `class`, such as `CGameClient`.
///
/// # Safety
///
/// `object` must point to a live polymorphic subobject, whose vtable the
/// compiler emitted with run-time type information for the target's ABI, in
/// a module that stays loaded for the call.
pub unsafe fn subobject_offset(object: *const c_void, class: &str) -> Option<isize> {
	// SAFETY: As the caller promises; the name is not kept past the call.
	let (offset, name) = unsafe { dynamic_type(object) }?;

	matches_class_name(name.to_bytes(), class).then_some(offset)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::util::Section;

	const BASE: usize = 0x10000;

	fn fixture() -> Image {
		Image {
			base: BASE,
			sections: vec![
				Section {
					address: BASE,
					bytes: vec![0; 1024],
					executable: false,
					writable: false,
				},
				Section {
					address: 0x20000,
					bytes: vec![0; 0x1000],
					executable: true,
					writable: false,
				},
			],
		}
	}

	#[test]
	fn itanium_excludes_secondary_and_truncated_tables() {
		let mut image = fixture();
		image.sections[0].bytes[0x100..0x10d].copy_from_slice(b"10CKickIssue\0");
		word(&mut image, 0x188, BASE + 0x100);
		word(&mut image, 0x208, BASE + 0x180);
		word(&mut image, 0x210 + 9 * 8, 0x20010);
		assert_eq!(image.itanium("CKickIssue", 9), [BASE + 0x210]);
		word(&mut image, 0x200, usize::MAX - 7);
		assert!(image.itanium("CKickIssue", 9).is_empty());
		word(&mut image, 0x200, 0);
		image.sections[0].bytes.truncate(0x210 + 9 * 8);
		assert!(image.itanium("CKickIssue", 9).is_empty());
	}

	#[test]
	fn malformed_locator_and_slot_arithmetic_cannot_wrap() {
		let mut image = fixture();
		assert!(!image.valid_table(BASE, usize::MAX));
		image.base = usize::MAX - 2047;
		image.sections[0].address = image.base;
		image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
		for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, u32::MAX)] {
			image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		assert!(image.msvc("CKickIssue", 8).is_empty());
	}

	#[test]
	fn msvc_requires_unique_primary_locator_and_executable_slot() {
		let mut image = fixture();
		image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
		for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, 0x180)] {
			image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
		}
		word(&mut image, 0x200, BASE + 0x180);
		word(&mut image, 0x208 + 8 * 8, 0x20010);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x208]);
		word(&mut image, 0x280, BASE + 0x180);
		word(&mut image, 0x288 + 8 * 8, 0x20020);
		assert_eq!(image.msvc("CKickIssue", 8).len(), 2);
		#[cfg(target_os = "windows")]
		assert!(image.primary_vtable("CKickIssue", 8).is_none());
		word(&mut image, 0x208 + 8 * 8, 0x30000);
		assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x288]);
	}

	#[test]
	fn names_follow_the_abi() {
		#[cfg(target_os = "windows")]
		{
			assert!(matches_class_name(b".?AVCGameClient@@", "CGameClient"));
			assert!(!matches_class_name(b".?AVCGameClientX@@", "CGameClient"));
			assert!(!matches_class_name(b"11CGameClient", "CGameClient"));
		}

		#[cfg(not(target_os = "windows"))]
		{
			assert!(matches_class_name(b"11CGameClient", "CGameClient"));
			assert!(!matches_class_name(b"12CGameClientX", "CGameClient"));
			assert!(!matches_class_name(b".?AVCGameClient@@", "CGameClient"));
		}
	}

	#[test]
	fn retained_hook_trampolines_may_be_outside_the_game_image() {
		unsafe extern "C" fn trampoline() {}
		let mut image = fixture();
		word(&mut image, 0x100 + 8 * 8, trampoline as *const () as usize);
		assert!(image.valid_table(BASE + 0x100, 8));
		// An ordinary data allocation is never a valid replacement for code.
		let data = [0_u8; 32];
		word(&mut image, 0x100 + 8 * 8, data.as_ptr() as usize);
		assert!(!image.valid_table(BASE + 0x100, 8));
	}

	#[test]
	fn vtable_records_cannot_use_executable_snapshot_bytes() {
		let mut image = fixture();
		image.sections[1].bytes[8 * 8..9 * 8].copy_from_slice(&0x20010_usize.to_le_bytes());
		assert!(!image.valid_table(0x20000, 8));
		word(&mut image, 0x100 + 8 * 8, 0x20010);
		assert!(image.valid_table(BASE + 0x100, 8));
	}

	fn word(image: &mut Image, offset: usize, value: usize) {
		image.sections[0].bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
	}
}
