//! Checks of a polymorphic object's class through the run-time type
//! information its compiler emitted, before relying on a layout of the
//! engine's that no public header declares.

use std::ffi::{CStr, c_char, c_void};

/// Where a `_TypeDescriptor`'s decorated name starts: after its vtable pointer
/// and the undecorated name the runtime caches.
#[cfg(target_os = "windows")]
const TYPE_DESCRIPTOR_NAME_OFFSET: usize = 2 * size_of::<*const c_void>();

/// MSVC's `_RTTICompleteObjectLocator` for 64-bit images, whose references
/// are relative to the image's base.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CompleteObjectLocator {
	/// 1 for 64-bit images.
	pub(crate) signature: u32,
	/// The subobject's offset in its complete object.
	pub(crate) offset: u32,
	/// `cdOffset`, the constructor displacement offset, which this module
	/// does not read.
	pub(crate) constructor_displacement: u32,
	/// The image offset of the complete object's `_TypeDescriptor`, which
	/// holds its decorated class name.
	pub(crate) type_descriptor: u32,
	/// The image offset of the complete object's
	/// `_RTTIClassHierarchyDescriptor`.
	pub(crate) class_descriptor: u32,
	/// The locator's own offset in the image.
	pub(crate) this: u32,
}

/// As the Windows version, for the Itanium ABI: the vtable stores the offset
/// from the subobject to its complete object two slots before its first, and
/// the `std::type_info` one slot before.
///
/// # Safety
///
/// As for [`subobject_offset`].
#[cfg(not(target_os = "windows"))]
unsafe fn dynamic_type<'a>(object: *const c_void) -> Option<(isize, &'a CStr)> {
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

	// SAFETY: The name is a string in the image, which stays loaded as long as
	// its objects exist.
	(!name.is_null()).then(|| (-offset_to_top, unsafe { CStr::from_ptr(name) }))
}

/// The offset of the subobject in its complete object, and the complete
/// object's decorated class name.
///
/// # Safety
///
/// As for [`subobject_offset`].
#[cfg(target_os = "windows")]
unsafe fn dynamic_type<'a>(object: *const c_void) -> Option<(isize, &'a CStr)> {
	// SAFETY: A polymorphic subobject starts with its vtable pointer, and MSVC
	// stores the locator's address in the slot before the vtable's first.
	let locator = unsafe { object.cast::<*const *const c_void>().read().sub(1).read() }
		.cast::<CompleteObjectLocator>();

	if locator.is_null() || !locator.is_aligned() {
		return None;
	}

	// SAFETY: The compiler emitted the locator with the vtable.
	let located = unsafe { locator.read() };

	if located.signature != 1 {
		return None;
	}

	let image = locator.addr().checked_sub(located.this as usize)?;

	let name = image
		.checked_add(located.type_descriptor as usize)?
		.checked_add(TYPE_DESCRIPTOR_NAME_OFFSET)?;

	// SAFETY: As above, and the name is a string in the image, which stays
	// loaded as long as its objects exist.
	Some((located.offset as isize, unsafe {
		CStr::from_ptr(locator.cast::<c_char>().with_addr(name))
	}))
}

/// MSVC decorates class names as `.?AVName@@`.
#[cfg(target_os = "windows")]
fn match_class_name(name: &[u8], class: &str) -> bool {
	name.strip_prefix(b".?AV")
		.and_then(|name| name.strip_suffix(b"@@"))
		.is_some_and(|name| name == class.as_bytes())
}

/// The Itanium ABI mangles class names as their length, then the name.
#[cfg(not(target_os = "windows"))]
fn match_class_name(name: &[u8], class: &str) -> bool {
	let length = class.len().to_string();

	name.strip_prefix(length.as_bytes())
		.is_some_and(|name| name == class.as_bytes())
}

/// Where the subobject `object` points to sits in its complete object, if
/// that object's class is named `class`, such as `CGameClient`.
///
/// # Safety
///
/// `object` must point to a live polymorphic subobject, whose vtable the
/// compiler emitted with run-time type information for the target's ABI.
pub(crate) unsafe fn subobject_offset(object: *const c_void, class: &str) -> Option<isize> {
	// SAFETY: As the caller promises.
	let (offset, name) = unsafe { dynamic_type(object) }?;

	match_class_name(name.to_bytes(), class).then_some(offset)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_follow_the_abi() {
		#[cfg(target_os = "windows")]
		{
			assert!(match_class_name(b".?AVCGameClient@@", "CGameClient"));
			assert!(!match_class_name(b".?AVCGameClientX@@", "CGameClient"));
			assert!(!match_class_name(b"11CGameClient", "CGameClient"));
		}

		#[cfg(not(target_os = "windows"))]
		{
			assert!(match_class_name(b"11CGameClient", "CGameClient"));
			assert!(!match_class_name(b"12CGameClientX", "CGameClient"));
			assert!(!match_class_name(b".?AVCGameClient@@", "CGameClient"));
		}
	}
}
