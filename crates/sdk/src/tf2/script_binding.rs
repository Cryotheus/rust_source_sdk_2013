//! Calls a selected native method through the game's typed VScript adapter.
//!
//! This does not evaluate script text or invoke a VM. The class descriptors
//! retain the compiler-generated adapters and member-function pointers, which
//! avoids guessing TF2 layouts or the platform's member-pointer ABI.
//!
//! Results come back in a `ScriptVariant_t` that the member adapter assigns
//! (`public/vscript/vscript_templates.h`). With the default allocator's
//! `ALWAYS_COPY` of 0 (`public/vscript/variant.h`), a returned `const char *`
//! is stored as is, without a copy and without `SV_FREE`, so a string result
//! borrows whatever storage the method returned it from.

use crate::entities::Entity;
use sdk_raw::util::cstr::{borrow_cstr, copy_cstr};
use std::ffi::{CStr, CString, c_int, c_uint};
use std::mem::{offset_of, size_of, transmute, zeroed};

// The member adapter stores a returned `const char *` without a copy, and so
// without `SV_FREE`, only while the default allocator does not always copy.
const _: () = assert!(sys::CVariantDefaultAllocator_ALWAYS_COPY == 0);

/// The script type of a `bool` (`FIELD_BOOLEAN`).
pub(crate) const BOOL: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_BOOLEAN as _;

/// The script type of an `f32` (`FIELD_FLOAT`).
pub(crate) const FLOAT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_FLOAT as _;

/// The script type of a script object handle (`FIELD_HSCRIPT`).
pub(crate) const HANDLE: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_HSCRIPT as _;

/// The script type of a 32-bit integer (`FIELD_INTEGER`).
pub(crate) const INT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_INTEGER as _;

/// How many descriptors of an entity's base chain [`call`] searches for the
/// declaring class.
const MAX_CLASS_DEPTH: usize = 64;

/// The most bindings [`call`] expects in one class descriptor. A larger count
/// is taken as a signature mismatch.
const MAX_FUNCTION_BINDINGS: c_int = 4096;

/// `SF_MEMBER_FUNC` from `public/vscript/ivscript.h`: the binding wraps a
/// member function, whose object [`call`] passes as the adapter's context.
const SF_MEMBER_FUNC: c_uint = 0x01;

/// The script type of a C string (`FIELD_CSTRING`).
pub(crate) const STRING: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_CSTRING as _;

/// `SV_FREE` from `public/vscript/variant.h`: the variant owns its pointed-to
/// data, which only the game's allocator may free.
const SV_FREE: u16 = 0x01;

/// The script type of no value (`FIELD_VOID`), for methods returning nothing.
pub(crate) const VOID: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_VOID as _;

/// Why [`call`] did not return a native method's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BindingError {
	/// The entity's descriptor chain has no class with the class name, that
	/// class has no binding with the method name, or the binding has no
	/// adapter.
	#[error("the entity does not expose the requested native method")]
	Unavailable,
	/// The class's binding list looks malformed, or the binding's parameter or
	/// return types, its flags, or the returned variant differ from what the
	/// caller expects. When only the returned variant differs, the method has
	/// already run; a variant that owns an allocation (`SV_FREE`) is then
	/// leaked, since only the game's allocator may free it.
	#[error("the native method's runtime signature does not match the SDK")]
	SignatureMismatch,
	/// The binding's adapter returned false.
	#[error("the native method rejected its arguments")]
	Rejected,
}

/// A `bool` argument.
pub(crate) fn boolean(value: bool) -> sys::ScriptVariant_t {
	let mut result = variant(BOOL);
	result.__bindgen_anon_1.m_bool = value;
	result
}

/// Finds and invokes a native member on a named declaring class.
///
/// Searches the entity's script class descriptor and its bases for the class
/// named `class`, then calls that class's binding named `name` through the
/// binding's adapter. The binding must declare the types of `arguments` and
/// return `result_type`; a [`VOID`] call returns an empty variant.
///
/// The returned variant must not own an allocation: any flag, of which
/// `SV_FREE` is the only one, fails with [`BindingError::SignatureMismatch`]
/// after the method has run, leaking the allocation. Scalar results (bool,
/// int, float, or HSCRIPT) are values. A [`STRING`] result is the pointer the
/// method returned, without a copy or ownership: it may be null, and it stays
/// valid only as long as the storage the method returned it from. Prefer
/// [`call_string`], which copies it before any other game code runs.
///
/// # Safety
///
/// The selected method must accept this live entity and argument values.
/// Pointer arguments must remain valid for the call and any lifetime the
/// method retains them for. The method and any callbacks it reaches must
/// uphold [`Server::new`]'s entity-deletion and callback-scope requirements.
///
/// [`Server::new`]: crate::Server::new
pub(crate) unsafe fn call(
	entity: Entity<'_>,
	class: &CStr,
	name: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
	result_type: sys::ScriptDataType_t,
) -> Result<sys::ScriptVariant_t, BindingError> {
	type GetScriptDesc = unsafe extern "C" fn(*mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t;
	const SLOT: usize = offset_of!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetScriptDesc)
		/ size_of::<usize>();
	// GetScriptDesc precedes all TF2-specific additions to CBaseEntity.
	const _: () = assert!(SLOT == sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 1);
	// SAFETY: Entity is live, and the generated primary-vtable slot has this
	// signature under both supported 64-bit ABIs.
	let get_desc: GetScriptDesc = unsafe {
		transmute(
			entity
				.as_ptr()
				.cast::<*const *const ()>()
				.read()
				.add(SLOT)
				.read(),
		)
	};
	// SAFETY: The game owns and initializes these class descriptors.
	let mut descriptor = unsafe { get_desc(entity.as_ptr()) };
	for _ in 0..MAX_CLASS_DEPTH {
		if descriptor.is_null() {
			break;
		}
		// SAFETY: The descriptor and its names belong to the loaded game DLL.
		if unsafe { borrow_cstr((*descriptor).m_pszClassname) } == Some(class) {
			// SAFETY: Native descriptors contain a valid vector. Copy its header
			// without borrowing storage that the engine may mutate.
			let (bindings, count) = unsafe {
				(
					(*descriptor).m_FunctionBindings.m_Memory.m_pMemory,
					(*descriptor).m_FunctionBindings.m_Size,
				)
			};
			if !(0..=MAX_FUNCTION_BINDINGS).contains(&count) || (count != 0 && bindings.is_null()) {
				return Err(BindingError::SignatureMismatch);
			}
			for index in 0..count as usize {
				// SAFETY: The vector owns `count` initialized bindings.
				let binding = unsafe { bindings.add(index) };
				if unsafe { borrow_cstr((*binding).m_desc.m_pszScriptName) } != Some(name) {
					continue;
				}
				// SAFETY: As above. Copy all metadata before invoking game code.
				let (parameter_count, parameters, returns, flags, adapter, function) = unsafe {
					(
						(*binding).m_desc.m_Parameters.m_Size,
						(*binding).m_desc.m_Parameters.m_Memory.m_pMemory,
						(*binding).m_desc.m_ReturnType,
						(*binding).m_flags,
						(*binding).m_pfnBinding,
						(*binding).m_pFunction,
					)
				};
				if parameter_count < 0
					|| parameter_count as usize != arguments.len()
					|| returns != result_type
					|| flags != SF_MEMBER_FUNC
					|| (!arguments.is_empty() && parameters.is_null())
				{
					return Err(BindingError::SignatureMismatch);
				}
				for (index, argument) in arguments.iter().enumerate() {
					// SAFETY: The native descriptor has `parameter_count` entries.
					if unsafe { parameters.add(index).read() } != i32::from(argument.m_type) {
						return Err(BindingError::SignatureMismatch);
					}
				}
				let adapter = adapter.ok_or(BindingError::Unavailable)?;
				// Without SV_FREE, the `Free` that precedes each assignment to the
				// result frees nothing.
				let mut result = variant(VOID);
				// The void specialization explicitly requires a null return pointer.
				let result_ptr = if result_type == VOID {
					std::ptr::null_mut()
				} else {
					&raw mut result
				};
				// SAFETY: The validated adapter handles the native member pointer
				// representation. The caller vouches for argument values/effects.
				// `parameter_count` was checked to equal `arguments.len()`.
				if !unsafe {
					adapter(
						function,
						entity.as_ptr().cast(),
						arguments.as_mut_ptr(),
						parameter_count,
						result_ptr,
					)
				} {
					return Err(BindingError::Rejected);
				}
				// Any flag, including SV_FREE, marks data only the game may free.
				if i32::from(result.m_type) != result_type || result.m_flags != 0 {
					return Err(BindingError::SignatureMismatch);
				}
				return Ok(result);
			}
			return Err(BindingError::Unavailable);
		}
		// SAFETY: Descriptor inheritance links are owned by the game DLL.
		descriptor = unsafe { (*descriptor).m_pBaseDesc };
	}
	Err(BindingError::Unavailable)
}

/// Calls a native member that returns a C string, and copies the string.
///
/// As [`call`] with a [`STRING`] result, whose borrowed pointer is copied
/// before any other game code runs. Returns `None` when the method returned a
/// null pointer, and an empty string when it returned `""`.
///
/// # Safety
///
/// As for [`call`]. A non-null pointer the method returns must also reference
/// a NUL-terminated string that is still allocated when the method returns,
/// such as a member array, a pooled `string_t` or a static buffer, but not
/// storage the method frees before returning.
pub(crate) unsafe fn call_string(
	entity: Entity<'_>,
	class: &CStr,
	name: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
) -> Result<Option<CString>, BindingError> {
	// SAFETY: The caller upholds `call`'s contract.
	let result = unsafe { call(entity, class, name, arguments, STRING) }?;

	// SAFETY: The caller vouches that the returned pointer outlives the method,
	// and no game code has run since it returned.
	unsafe { copy_string(result) }
}

/// Copies a non-owning [`STRING`] result, or returns `None` for null.
///
/// Fails with [`BindingError::SignatureMismatch`] for any other type, and for
/// a string that owns its allocation (`SV_FREE`), which only the game's
/// allocator may free.
///
/// # Safety
///
/// A non-null string pointer must reference a NUL-terminated string for the
/// duration of the call.
unsafe fn copy_string(result: sys::ScriptVariant_t) -> Result<Option<CString>, BindingError> {
	if i32::from(result.m_type) != STRING || result.m_flags & SV_FREE != 0 {
		return Err(BindingError::SignatureMismatch);
	}

	// SAFETY: The checked type selects the string union member, which the
	// caller vouches for.
	Ok(unsafe { copy_cstr(result.__bindgen_anon_1.m_pszString) })
}

/// An `f32` argument.
pub(crate) fn float(value: f32) -> sys::ScriptVariant_t {
	let mut result = variant(FLOAT);
	result.__bindgen_anon_1.m_float = value;
	result
}

/// A script object handle argument, which may be null.
pub(crate) fn handle(value: sys::HSCRIPT) -> sys::ScriptVariant_t {
	let mut result = variant(HANDLE);
	result.__bindgen_anon_1.m_hScript = value;
	result
}

/// A 32-bit integer argument.
pub(crate) fn int(value: i32) -> sys::ScriptVariant_t {
	let mut result = variant(INT);
	result.__bindgen_anon_1.m_int = value;
	result
}

/// A borrowed C string argument. The caller of [`call`] keeps the string
/// alive until the native call ends, and for as long as the method retains it.
pub(crate) fn string(value: &CStr) -> sys::ScriptVariant_t {
	let mut result = variant(STRING);
	result.__bindgen_anon_1.m_pszString = value.as_ptr();
	result
}

/// An empty, non-owning variant of type `kind`, whose union member the caller
/// sets.
fn variant(kind: sys::ScriptDataType_t) -> sys::ScriptVariant_t {
	// SAFETY: Null union storage, FIELD_VOID, and zero flags form an empty
	// non-owning variant. All helpers initialize the selected union member.
	let mut value: sys::ScriptVariant_t = unsafe { zeroed() };
	value.m_type = kind as _;
	value
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;
	use std::ffi::c_char;
	use std::marker::PhantomData;
	use std::ptr::{NonNull, null, null_mut};

	#[repr(C)]
	struct Object {
		vtable: *const *const (),
		description: *mut sys::ScriptClassDesc_t,
		calls: Cell<usize>,
		/// The string [`string_adapter`] returns.
		text: Cell<*const c_char>,
		/// The variant flags [`string_adapter`] returns.
		flags: Cell<u16>,
	}

	unsafe extern "C" fn adapter(
		function: sys::ScriptFunctionBindingStorageType_t,
		object: *mut std::ffi::c_void,
		arguments: *mut sys::ScriptVariant_t,
		count: i32,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		assert_eq!(function.val_0, 0x1234);
		assert_eq!(count, 1);
		let object = unsafe { &*object.cast::<Object>() };
		object.calls.set(object.calls.get() + 1);
		if !result.is_null() {
			unsafe { result.write(float((*arguments).__bindgen_anon_1.m_float * 2.0)) };
		}
		true
	}

	#[test]
	fn copy_string_refuses_other_types_and_owned_strings() {
		assert_eq!(
			unsafe { copy_string(string(c"borrowed")) },
			Ok(Some(c"borrowed".to_owned()))
		);
		assert_eq!(unsafe { copy_string(variant(STRING)) }, Ok(None));
		assert_eq!(
			unsafe { copy_string(int(1)) },
			Err(BindingError::SignatureMismatch)
		);
		assert_eq!(
			unsafe { copy_string(variant(VOID)) },
			Err(BindingError::SignatureMismatch)
		);
		let mut owned = string(c"owned");
		owned.m_flags = SV_FREE;
		assert_eq!(
			unsafe { copy_string(owned) },
			Err(BindingError::SignatureMismatch)
		);
	}

	unsafe extern "C" fn get_description(
		object: *mut sys::CBaseEntity,
	) -> *mut sys::ScriptClassDesc_t {
		unsafe { (*object.cast::<Object>()).description }
	}

	#[test]
	fn native_adapter_receives_typed_arguments_and_void_has_no_return_pointer() {
		let mut parameters = [FLOAT];
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
		bindings[0].m_desc.m_pszScriptName = c"SetValue".as_ptr();
		bindings[0].m_desc.m_ReturnType = FLOAT;
		bindings[0].m_desc.m_Parameters = vector(&mut parameters);
		bindings[0].m_flags = SF_MEMBER_FUNC;
		bindings[0].m_pfnBinding = Some(adapter);
		bindings[0].m_pFunction.val_0 = 0x1234;
		let mut base: sys::ScriptClassDesc_t = unsafe { zeroed() };
		base.m_pszClassname = c"Base".as_ptr();
		base.m_FunctionBindings = vector(&mut bindings);
		// The binding and the object are changed below only through the
		// pointers `call` reads them by, since writing through the locals would
		// invalidate those pointers.
		let binding = base.m_FunctionBindings.m_Memory.m_pMemory;
		let mut derived: sys::ScriptClassDesc_t = unsafe { zeroed() };
		derived.m_pszClassname = c"Derived".as_ptr();
		derived.m_pBaseDesc = &raw mut base;
		let mut vtable = [std::ptr::null(); 16];
		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 1] = get_description as *const ();
		let mut object = Object {
			vtable: vtable.as_ptr(),
			description: &raw mut derived,
			calls: Cell::new(0),
			text: Cell::new(null()),
			flags: Cell::new(0),
		};
		let object = NonNull::from(&mut object);
		let entity = unsafe { Entity::from_raw(object.cast()) };
		let value =
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], FLOAT) }.unwrap();
		assert_eq!(unsafe { value.__bindgen_anon_1.m_float }, 6.0);
		assert_eq!(unsafe { object.as_ref() }.calls.get(), 1);
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [int(3)], FLOAT) }.err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [], FLOAT) }.err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(
			unsafe { call(entity, c"Other", c"SetValue", &mut [float(3.0)], FLOAT) }.err(),
			Some(BindingError::Unavailable)
		);
		assert_eq!(unsafe { object.as_ref() }.calls.get(), 1);
		unsafe { (*binding).m_desc.m_ReturnType = VOID };
		unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.unwrap();
		assert_eq!(unsafe { object.as_ref() }.calls.get(), 2);
		unsafe { (*binding).m_flags = 0 };
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.err(),
			Some(BindingError::SignatureMismatch)
		);
		unsafe { (*object.as_ptr()).description = null_mut() };
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.err(),
			Some(BindingError::Unavailable)
		);
	}

	/// A member adapter for `const char *Object::GetText()`. Like the SDK's
	/// `*pReturn = const char *`, it stores the pointer without a copy.
	unsafe extern "C" fn string_adapter(
		function: sys::ScriptFunctionBindingStorageType_t,
		object: *mut std::ffi::c_void,
		_: *mut sys::ScriptVariant_t,
		count: i32,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		assert_eq!(function.val_0, 0x5678);
		assert_eq!(count, 0);
		assert!(!result.is_null());
		let object = unsafe { &*object.cast::<Object>() };
		object.calls.set(object.calls.get() + 1);
		let mut value = variant(STRING);
		value.__bindgen_anon_1.m_pszString = object.text.get();
		value.m_flags = object.flags.get();
		unsafe { result.write(value) };
		true
	}

	#[test]
	fn string_results_are_copied_before_returning_and_owned_strings_refused() {
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
		bindings[0].m_desc.m_pszScriptName = c"GetText".as_ptr();
		bindings[0].m_desc.m_ReturnType = STRING;
		bindings[0].m_flags = SF_MEMBER_FUNC;
		bindings[0].m_pfnBinding = Some(string_adapter);
		bindings[0].m_pFunction.val_0 = 0x5678;
		let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };
		description.m_pszClassname = c"Base".as_ptr();
		description.m_FunctionBindings = vector(&mut bindings);
		// Changed below only through the pointer `call` reads it by.
		let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
		let mut vtable = [null(); 16];
		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT + 1] = get_description as *const ();
		let mut text = *b"effects/jarate_overlay\0";
		// Written only through this pointer, which the object also returns.
		let storage = text.as_mut_ptr();
		let mut object = Object {
			vtable: vtable.as_ptr(),
			description: &raw mut description,
			calls: Cell::new(0),
			text: Cell::new(storage.cast_const().cast()),
			flags: Cell::new(0),
		};
		let object = NonNull::from(&mut object);
		let entity = unsafe { Entity::from_raw(object.cast()) };
		let calls = || unsafe { object.as_ref() }.calls.get();
		let set_text = |value: *const c_char| unsafe { object.as_ref() }.text.set(value);

		let copied = unsafe { call_string(entity, c"Base", c"GetText", &mut []) }.unwrap();
		// The game may change its storage once the call returns.
		unsafe { storage.write(b'X') };
		assert_eq!(copied.as_deref(), Some(c"effects/jarate_overlay"));
		assert_eq!(
			unsafe { call_string(entity, c"Base", c"GetText", &mut []) }
				.unwrap()
				.as_deref(),
			Some(c"Xffects/jarate_overlay")
		);
		assert_eq!(calls(), 2);

		set_text(c"".as_ptr());
		assert_eq!(
			unsafe { call_string(entity, c"Base", c"GetText", &mut []) },
			Ok(Some(CString::default()))
		);
		set_text(null());
		assert_eq!(
			unsafe { call_string(entity, c"Base", c"GetText", &mut []) },
			Ok(None)
		);
		// `call` itself passes the method's pointer through without a copy.
		let overlay = c"effects/imcookin";
		set_text(overlay.as_ptr());
		let result = unsafe { call(entity, c"Base", c"GetText", &mut [], STRING) }.unwrap();
		assert_eq!(
			unsafe { result.__bindgen_anon_1.m_pszString },
			overlay.as_ptr()
		);
		assert_eq!(calls(), 5);

		// An owned string has already been returned, but cannot be freed.
		unsafe { object.as_ref() }.flags.set(SV_FREE);
		assert_eq!(
			unsafe { call_string(entity, c"Base", c"GetText", &mut []) },
			Err(BindingError::SignatureMismatch)
		);
		assert_eq!(
			unsafe { call(entity, c"Base", c"GetText", &mut [], STRING) }.err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(calls(), 7);

		unsafe { object.as_ref() }.flags.set(0);
		unsafe { (*binding).m_desc.m_ReturnType = FLOAT };
		assert_eq!(
			unsafe { call_string(entity, c"Base", c"GetText", &mut []) },
			Err(BindingError::SignatureMismatch)
		);
		assert_eq!(calls(), 7);
	}

	/// A vector over `values`. Both element pointers come from one
	/// `as_mut_ptr` call, since a second call would invalidate the first.
	fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
		let len = c_int::try_from(values.len()).unwrap();
		let elements = values.as_mut_ptr();

		sys::CUtlVector {
			_phantom_0: PhantomData,
			_phantom_1: PhantomData,
			m_Memory: sys::CUtlMemory {
				_phantom_0: PhantomData,
				m_pMemory: elements,
				m_nAllocationCount: len,
				m_nGrowSize: 0,
			},
			m_Size: len,
			m_pElements: elements,
		}
	}
}
