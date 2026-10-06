//! Run-time type information that C++ compilers emit for polymorphic
//! classes: discovery of primary vtables in owned image snapshots, checks of
//! a live object's class, and the records that describe classes Rust
//! implements for the engine.
//!
//! The live readers check a polymorphic object's class before code relies on
//! a layout of the engine's that no public header declares.

#[cfg(test)]
#[path = "../tests/util/rtti.rs"]
mod tests;

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

#[cfg(target_os = "windows")]
const _: () = {
	use std::mem::offset_of;

	assert!(size_of::<BaseClassDescriptor>() == 28);
	assert!(size_of::<ClassHierarchyDescriptor>() == 16);
	assert!(offset_of!(TypeDescriptor<1>, name) == TYPE_DESCRIPTOR_NAME_OFFSET);
};

/// The alignment of an MSVC `_TypeDescriptor`, which starts with its vtable
/// pointer, and so of its decorated name, [`TYPE_DESCRIPTOR_NAME_OFFSET`]
/// bytes into it: searches for names only read offsets aligned so.
#[cfg(any(target_os = "windows", test))]
const TYPE_DESCRIPTOR_ALIGNMENT: usize = align_of::<*const c_void>();

/// Where an MSVC `_TypeDescriptor`'s decorated name starts: after its vtable
/// pointer and the undecorated name the runtime caches.
#[cfg(any(target_os = "windows", test))]
const TYPE_DESCRIPTOR_NAME_OFFSET: usize = 2 * size_of::<*const c_void>();

/// MSVC's `_RTTIBaseClassDescriptor` for 64-bit images, which describes one
/// class of a hierarchy, the class itself included. Its references are
/// relative to the image's base.
#[doc(alias("_RTTIBaseClassDescriptor"))]
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct BaseClassDescriptor {
	/// The image offset of the class's [`TypeDescriptor`].
	pub type_descriptor: u32,

	/// How many bases the class has in turn.
	pub contained_bases: u32,

	/// `PMD::mdisp`, the class's offset in the object the hierarchy describes.
	pub member_displacement: i32,

	/// `PMD::pdisp`, -1 for a class that is not a virtual base.
	pub vbtable_displacement: i32,

	/// `PMD::vdisp`, the offset into the virtual base table.
	pub vbtable_offset: i32,

	/// `BCD_*` flags, such as [`Self::HAS_HIERARCHY`].
	pub attributes: u32,

	/// The image offset of the class's own [`ClassHierarchyDescriptor`], if
	/// [`Self::HAS_HIERARCHY`] is set.
	pub class_descriptor: u32,
}

#[cfg(target_os = "windows")]
impl BaseClassDescriptor {
	/// `BCD_HASPCHD`: [`class_descriptor`](Self::class_descriptor) refers to
	/// the class's hierarchy.
	#[doc(alias("BCD_HASPCHD"))]
	pub const HAS_HIERARCHY: u32 = 0x40;
}

/// MSVC's `_RTTIClassHierarchyDescriptor` for 64-bit images, which lists a
/// class and its bases. Its references are relative to the image's base.
#[doc(alias("_RTTIClassHierarchyDescriptor"))]
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct ClassHierarchyDescriptor {
	/// Always 0.
	pub signature: u32,

	/// `CHD_MULTINH` (1) for multiple inheritance and `CHD_VIRTINH` (2) for
	/// virtual inheritance.
	pub attributes: u32,

	/// The number of entries in the base class array: the class itself, then
	/// its bases.
	pub base_classes: u32,

	/// The image offset of the base class array, whose entries are the image
	/// offsets of [`BaseClassDescriptor`]s.
	pub base_class_array: u32,
}

/// The Itanium ABI's `std::type_info` for a class without bases,
/// `__cxxabiv1::__class_type_info`: its vtable and mangled name.
#[doc(alias("__class_type_info"))]
#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
#[repr(C)]
pub struct ClassTypeInfo {
	/// The address point of the C++ runtime's vtable for the class's type
	/// information, such as [`class_type_info_vtable`].
	pub vtable: *const *const c_void,

	/// The mangled name: the class name's length, then the name.
	pub name: *const c_char,
}

// SAFETY: A `ClassTypeInfo` only holds addresses, and gives no access to what
// they point to.
#[cfg(not(target_os = "windows"))]
unsafe impl Sync for ClassTypeInfo {}

/// MSVC's `_RTTICompleteObjectLocator` for 64-bit images, whose references
/// are relative to the image's base.
///
/// MSVC stores the address of a vtable's locator in the slot before the
/// vtable's first.
#[doc(alias("_RTTICompleteObjectLocator"))]
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

/// MSVC's `_TypeDescriptor`, laid out as its `std::type_info`: its vtable,
/// the undecorated name the runtime caches when asked for it, and the
/// decorated name, `N` bytes with its terminator, which casts compare.
#[doc(alias("_TypeDescriptor"))]
#[cfg(target_os = "windows")]
#[derive(Debug)]
#[repr(C)]
pub struct TypeDescriptor<const N: usize> {
	/// The `std::type_info` vtable, which casts do not read.
	pub vtable: *const c_void,

	/// The undecorated name the runtime caches, null until it is asked for.
	pub undecorated_name: *mut c_void,

	/// The decorated name, such as `.?AVName@@`, with its terminator.
	pub name: [u8; N],
}

impl Image {
	#[cfg(test)]
	fn itanium(&self, class: &str, slot: usize) -> Vec<usize> {
		self.itanium_all(&[class], slot).pop().unwrap_or_default()
	}

	/// Every Itanium primary vtable of each of `classes` with an executable
	/// entry at `slot`, sorted, in the classes' order.
	#[cfg(any(target_os = "linux", test))]
	fn itanium_all(&self, classes: &[&str], slot: usize) -> Vec<Vec<usize>> {
		let names = class_patterns(classes, |class| format!("{}{class}\0", class.len()));
		let names = self.matches_any(&names.iter().map(Vec::as_slice).collect::<Vec<_>>(), 1);

		// A class's type_info holds its vtable, then the address of its name.
		let type_infos: Vec<(usize, usize)> = self
			.references(
				&names
					.iter()
					.map(|&(name, _)| name.to_le_bytes())
					.collect::<Vec<_>>(),
			)
			.into_iter()
			.filter_map(|(name_pointer, name)| Some((name_pointer.checked_sub(8)?, names[name].1)))
			.collect();

		let mut tables = vec![Vec::new(); classes.len()];

		for (reference, type_info) in self.references(
			&type_infos
				.iter()
				.map(|&(type_info, _)| type_info.to_le_bytes())
				.collect::<Vec<_>>(),
		) {
			// The primary address point follows offset-to-top=0 and the
			// pointer to the class's type_info object.
			let Some(prefix) = reference.checked_sub(8).and_then(|p| self.rtti_read(p, 8)) else {
				continue;
			};
			let Some(table) = reference.checked_add(8) else {
				continue;
			};

			if word_at(prefix, 0) == Some(0) && self.valid_table(table, slot) {
				tables[type_infos[type_info].1].push(table);
			}
		}

		for tables in &mut tables {
			tables.sort_unstable();
			tables.dedup();
		}

		tables
	}

	#[cfg(test)]
	fn msvc(&self, class: &str, slot: usize) -> Vec<usize> {
		self.msvc_all(&[class], slot).pop().unwrap_or_default()
	}

	/// Every MSVC x64 primary vtable of each of `classes` with an executable
	/// entry at `slot`, sorted, in the classes' order.
	#[cfg(any(target_os = "windows", test))]
	fn msvc_all(&self, classes: &[&str], slot: usize) -> Vec<Vec<usize>> {
		self.msvc_tables_all(classes, slot)
			.into_iter()
			.map(|tables| {
				tables
					.into_iter()
					.filter_map(|(offset, table)| (offset == 0).then_some(table))
					.collect()
			})
			.collect()
	}

	/// Every MSVC x64 vtable of `class` with an executable entry at `slot`,
	/// sorted, with the offset in the complete object of the subobject each
	/// table belongs to.
	#[cfg(any(target_os = "windows", test))]
	fn msvc_tables(&self, class: &str, slot: usize) -> Vec<(usize, usize)> {
		self.msvc_tables_all(&[class], slot)
			.pop()
			.unwrap_or_default()
	}

	/// The tables [`Self::msvc_tables`] finds for each of `classes`, in their
	/// order.
	#[cfg(any(target_os = "windows", test))]
	fn msvc_tables_all(&self, classes: &[&str], slot: usize) -> Vec<Vec<(usize, usize)>> {
		let names = class_patterns(classes, |class| format!(".?AV{class}@@\0"));

		// Locators refer to a class's type descriptor by its offset in the image.
		let descriptors: Vec<([u8; 4], usize)> = self
			.matches_any(
				&names.iter().map(Vec::as_slice).collect::<Vec<_>>(),
				TYPE_DESCRIPTOR_ALIGNMENT,
			)
			.into_iter()
			.filter_map(|(name, class)| {
				let descriptor = name.checked_sub(TYPE_DESCRIPTOR_NAME_OFFSET)?;
				let relative = u32::try_from(descriptor.checked_sub(self.base)?).ok()?;

				Some((relative.to_le_bytes(), class))
			})
			.collect();

		// Each locator, with its class and the offset of its subobject.
		let mut locators = Vec::new();

		for (reference, descriptor) in self.references(
			&descriptors
				.iter()
				.map(|&(relative, _)| relative)
				.collect::<Vec<_>>(),
		) {
			let Some(locator) =
				reference.checked_sub(std::mem::offset_of!(CompleteObjectLocator, type_descriptor))
			else {
				continue;
			};
			let Some(located) = self
				.rtti_read(locator, size_of::<CompleteObjectLocator>())
				.and_then(CompleteObjectLocator::parse)
			else {
				continue;
			};

			// MSVC x64 complete-object locator: signature=1, no construction
			// displacement, self RVA. Its offset is the subobject's.
			if located.signature != CompleteObjectLocator::SIGNATURE
				|| located.constructor_displacement != 0
				|| self.base.checked_add(located.this as usize) != Some(locator)
			{
				continue;
			}

			locators.push((locator, descriptors[descriptor].1, located.offset as usize));
		}

		let mut tables = vec![Vec::new(); classes.len()];

		for (pointer, locator) in self.references(
			&locators
				.iter()
				.map(|&(locator, ..)| locator.to_le_bytes())
				.collect::<Vec<_>>(),
		) {
			let (_, class, offset) = locators[locator];
			let Some(table) = pointer.checked_add(8) else {
				continue;
			};

			if self.valid_table(table, slot) {
				tables[class].push((offset, table));
			}
		}

		for tables in &mut tables {
			tables.sort_unstable();
			tables.dedup();
		}

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
	///
	/// Each search reads the whole snapshot a few times, so find several classes
	/// with [`Self::primary_vtables`].
	pub fn primary_vtable(&self, class: &str, slot: usize) -> Option<usize> {
		self.primary_vtables(&[class], slot).pop().flatten()
	}

	/// Finds one primary vtable for each of several global, unqualified C++
	/// class names, in their order, as [`Self::primary_vtable`] finds one. The
	/// snapshot is read the same few times however many classes there are.
	pub fn primary_vtables(&self, classes: &[&str], slot: usize) -> Vec<Option<usize>> {
		if self.base == 0 {
			return vec![None; classes.len()];
		}

		#[cfg(target_os = "windows")]
		let candidates = self.msvc_all(classes, slot);

		#[cfg(target_os = "linux")]
		let candidates = self.itanium_all(classes, slot);

		candidates
			.into_iter()
			.map(|tables| match tables[..] {
				[table] => Some(table),
				_ => None,
			})
			.collect()
	}

	/// Every aligned word in the data regions equal to one of `words`, as their
	/// little-endian bytes, with the index of the word it equals.
	fn references<const N: usize>(&self, words: &[[u8; N]]) -> Vec<(usize, usize)> {
		self.matches_any(
			&words.iter().map(<[u8; N]>::as_slice).collect::<Vec<_>>(),
			N,
		)
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

	/// Finds every vtable of a global, unqualified C++ class name through MSVC
	/// x64 RTTI, primary and secondary, each with the offset in the complete
	/// object of the subobject whose vtable pointer holds it: 0 for the primary
	/// table, and a base's offset for each table of a base that does not start
	/// the object. Tables lacking an executable entry at `slot` are skipped. An
	/// entry may point to a hook trampoline outside this image.
	///
	/// The result is sorted by offset, then address. Several tables at one
	/// offset are ambiguous, which callers must reject. As for
	/// [`Self::primary_vtable`], the addresses are snapshot metadata only.
	#[cfg(target_os = "windows")]
	#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
	pub fn vtables(&self, class: &str, slot: usize) -> Vec<(usize, usize)> {
		if self.base == 0 || class.is_empty() || class.as_bytes().contains(&0) {
			return Vec::new();
		}

		self.msvc_tables(class, slot)
	}
}

#[cfg(not(target_os = "windows"))]
#[link(name = "stdc++")]
unsafe extern "C" {
	/// libstdc++'s vtable of `__cxxabiv1::__class_type_info`, the type
	/// information of a class without bases, whose `__do_dyncast` a cast
	/// calls.
	#[link_name = "_ZTVN10__cxxabiv117__class_type_infoE"]
	static CLASS_TYPE_INFO_VTABLE: [*const c_void; 0];
}

/// The bytes to search for of each class's name, which `pattern` gives as
/// RTTI stores it, or nothing, which matches nothing, for a name that RTTI
/// cannot hold.
fn class_patterns(classes: &[&str], pattern: impl Fn(&str) -> String) -> Vec<Vec<u8>> {
	classes
		.iter()
		.map(|&class| {
			if class.is_empty() || class.as_bytes().contains(&0) {
				Vec::new()
			} else {
				pattern(class).into_bytes()
			}
		})
		.collect()
}

/// The address point of libstdc++'s vtable for `__cxxabiv1::__class_type_info`,
/// for the [`ClassTypeInfo`] of a class without bases. This crate links
/// libstdc++ on Linux for it.
#[cfg(not(target_os = "windows"))]
pub const fn class_type_info_vtable() -> *const *const c_void {
	// An object's vtable pointer skips the offset to the top and the type
	// information before the first slot.
	(&raw const CLASS_TYPE_INFO_VTABLE)
		.cast::<*const c_void>()
		.wrapping_add(2)
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
	let offset = offset_to_top.checked_neg()?;

	// SAFETY: As above.
	(!name.is_null()).then(|| (offset, unsafe { CStr::from_ptr(name) }))
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
