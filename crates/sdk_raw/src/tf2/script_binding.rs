//! The native VScript bindings TF2's entities expose: the class descriptors
//! `CBaseEntity::GetScriptDesc` returns, their member-function bindings, and
//! the `ScriptVariant_t` values those take and return.
//!
//! [`call`] invokes a selected native method through the game's typed VScript
//! adapter. This does not evaluate script text or invoke a VM. The class
//! descriptors retain the compiler-generated adapters and member-function
//! pointers, which avoids guessing TF2 layouts or the platform's
//! member-pointer ABI.
//!
//! Results come back in a `ScriptVariant_t` that the member adapter assigns
//! (`public/vscript/vscript_templates.h`). With the default allocator's
//! `ALWAYS_COPY` of 0 (`public/vscript/variant.h`), a returned `const char *`
//! is stored as is, without a copy and without [`SV_FREE`], so a string result
//! borrows whatever storage the method returned it from.

use crate::util::cstr::{borrow_cstr, copy_cstr};
use crate::{vcall, vtable_slot};
use std::ffi::{CStr, CString, c_int, c_uint};
use std::mem::zeroed;
use std::ptr::NonNull;

// GetScriptDesc precedes all TF2-specific additions to CBaseEntity, directly
// following GetDataDescMap, so fakes of an entity's vtable can place it there.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(
		vtable_slot!(Vtable, CBaseEntity_GetScriptDesc)
			== vtable_slot!(Vtable, CBaseEntity_GetDataDescMap) + 1
	);
};

// The member adapter stores a returned `const char *` without a copy, and so
// without `SV_FREE`, only while the default allocator does not always copy.
const _: () = assert!(sys::CVariantDefaultAllocator_ALWAYS_COPY == 0);

/// The script type of a `bool` (`FIELD_BOOLEAN`).
pub const BOOL: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_BOOLEAN as _;

/// The script type of an `f32` (`FIELD_FLOAT`).
pub const FLOAT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_FLOAT as _;

/// The script type of a script object handle (`FIELD_HSCRIPT`).
pub const HANDLE: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_HSCRIPT as _;

/// The script type of a 32-bit integer (`FIELD_INTEGER`).
pub const INT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_INTEGER as _;

/// How many descriptors of an entity's base chain [`call`] searches for the
/// declaring class.
const MAX_CLASS_DEPTH: usize = 64;

/// The most bindings [`call`] expects in one class descriptor. A larger count
/// is taken as a signature mismatch.
const MAX_FUNCTION_BINDINGS: c_int = 4096;

/// `SF_MEMBER_FUNC` from `public/vscript/ivscript.h`: the binding wraps a
/// member function, whose object [`call`] passes as the adapter's context.
pub const SF_MEMBER_FUNC: c_uint = 0x01;

/// The script type of a C string (`FIELD_CSTRING`).
pub const STRING: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_CSTRING as _;

/// `SV_FREE` from `public/vscript/variant.h`: the variant owns its pointed-to
/// data, which only the game's allocator may free.
pub const SV_FREE: u16 = 0x01;

/// The script type of no value (`FIELD_VOID`), for methods returning nothing.
pub const VOID: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_VOID as _;

/// Why [`call`] did not return a native method's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
	/// The entity's descriptor chain has no class with the class name, that
	/// class has no binding with the method name, or the binding has no
	/// adapter.
	#[error("the entity does not expose the requested native method")]
	Unavailable,

	/// The class's binding list looks malformed, or the binding's parameter or
	/// return types, its flags, or the returned variant differ from what the
	/// caller expects. When only the returned variant differs, the method has
	/// already run; a variant that owns an allocation ([`SV_FREE`]) is then
	/// leaked, since only the game's allocator may free it.
	#[error("the native method's runtime signature does not match the SDK")]
	SignatureMismatch,

	/// The binding's adapter returned false.
	#[error("the native method rejected its arguments")]
	Rejected,
}

/// A `bool` argument.
pub fn boolean(value: bool) -> sys::ScriptVariant_t {
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
/// [`SV_FREE`] is the only one, fails with [`BindingError::SignatureMismatch`]
/// after the method has run, leaking the allocation. Scalar results (bool,
/// int, float, or HSCRIPT) are values. A [`STRING`] result is the pointer the
/// method returned, without a copy or ownership: it may be null, and it stays
/// valid only as long as the storage the method returned it from. Prefer
/// [`call_string`], which copies it before any other game code runs.
///
/// # Safety
///
/// - `entity` points to a live `CBaseEntity` of the loaded game module, which
///   stays loaded for the call, and the call is made on the server's main
///   thread.
/// - The selected method accepts this entity and these argument values.
///   Pointer arguments, such as [`string`]'s, remain valid for the call and
///   for as long as the method retains them.
/// - The game code the method runs, and everything it reaches, frees entities
///   only through the engine's deferred deletion.
pub unsafe fn call(
	entity: NonNull<sys::CBaseEntity>,
	class: &CStr,
	name: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
	result_type: sys::ScriptDataType_t,
) -> Result<sys::ScriptVariant_t, BindingError> {
	let this = entity.as_ptr();

	// SAFETY: The entity is live, and its primary vtable has the generated
	// layout up to GetScriptDesc under both supported 64-bit ABIs.
	let mut descriptor =
		unsafe { vcall!(this as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetScriptDesc()) };

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

				// SAFETY: As above; the name belongs to the loaded game DLL.
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
						this.cast(),
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
pub unsafe fn call_string(
	entity: NonNull<sys::CBaseEntity>,
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
/// a string that owns its allocation ([`SV_FREE`]), which only the game's
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
pub fn float(value: f32) -> sys::ScriptVariant_t {
	let mut result = variant(FLOAT);
	result.__bindgen_anon_1.m_float = value;
	result
}

/// A script object handle argument, which may be null.
pub fn handle(value: sys::HSCRIPT) -> sys::ScriptVariant_t {
	let mut result = variant(HANDLE);
	result.__bindgen_anon_1.m_hScript = value;
	result
}

/// A 32-bit integer argument.
pub fn int(value: i32) -> sys::ScriptVariant_t {
	let mut result = variant(INT);
	result.__bindgen_anon_1.m_int = value;
	result
}

/// A borrowed C string argument. The caller of [`call`] keeps the string
/// alive until the native call ends, and for as long as the method retains it.
pub fn string(value: &CStr) -> sys::ScriptVariant_t {
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
	use crate::util::mock::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ffi::{c_char, c_void};
	use std::marker::PhantomData;
	use std::ptr::{null, null_mut};

	/// A fake entity, whose vtable's `GetScriptDesc` returns `description`.
	#[repr(C)]
	struct Object {
		vtable: *const sys::CBaseEntity__bindgen_vtable,
		description: *mut sys::ScriptClassDesc_t,
		calls: Cell<usize>,
		/// The string [`string_adapter`] returns.
		text: Cell<*const c_char>,
		/// The variant flags [`string_adapter`] returns.
		flags: Cell<u16>,
	}

	impl Object {
		/// An object with no calls yet, returning a null string.
		fn new(
			vtable: &sys::CBaseEntity__bindgen_vtable,
			description: *mut sys::ScriptClassDesc_t,
		) -> Self {
			Self {
				vtable,
				description,
				calls: Cell::new(0),
				text: Cell::new(null()),
				flags: Cell::new(0),
			}
		}
	}

	unsafe extern "C" fn adapter(
		function: sys::ScriptFunctionBindingStorageType_t,
		object: *mut c_void,
		arguments: *mut sys::ScriptVariant_t,
		count: i32,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		assert_eq!(function.val_0, 0x1234);
		assert_eq!(count, 1);

		// SAFETY: `call` passes the test's live object.
		let object = unsafe { &*object.cast::<Object>() };

		object.calls.set(object.calls.get() + 1);

		if !result.is_null() {
			// SAFETY: The one argument is a float, as the binding declares, and
			// the result is writable.
			unsafe { result.write(float((*arguments).__bindgen_anon_1.m_float * 2.0)) };
		}

		true
	}

	#[test]
	fn copy_string_refuses_other_types_and_owned_strings() {
		// SAFETY: Every string here is a static literal or null.
		unsafe {
			assert_eq!(
				copy_string(string(c"borrowed")),
				Ok(Some(c"borrowed".to_owned()))
			);
			assert_eq!(copy_string(variant(STRING)), Ok(None));
			assert_eq!(copy_string(int(1)), Err(BindingError::SignatureMismatch));
			assert_eq!(
				copy_string(variant(VOID)),
				Err(BindingError::SignatureMismatch)
			);

			let mut owned = string(c"owned");
			owned.m_flags = SV_FREE;
			assert_eq!(copy_string(owned), Err(BindingError::SignatureMismatch));
		}
	}

	/// A fake entity vtable, whose only method is [`get_description`].
	fn entity_vtable() -> Box<sys::CBaseEntity__bindgen_vtable> {
		// SAFETY: The generated vtable consists of function pointer slots, and
		// `unexpected_call` aborts the test if any other slot is called.
		unsafe {
			mock_vtable(
				unexpected_call as *const (),
				|vtable: *mut sys::CBaseEntity__bindgen_vtable| {
					(&raw mut (*vtable).CBaseEntity_GetScriptDesc).write(get_description);
				},
			)
		}
	}

	/// A fake `GetScriptDesc`, returning the object's description.
	unsafe extern "C" fn get_description(
		object: *mut sys::CBaseEntity,
	) -> *mut sys::ScriptClassDesc_t {
		// SAFETY: The tests call this only for a live `Object`.
		unsafe { (*object.cast::<Object>()).description }
	}

	#[test]
	fn native_adapter_receives_typed_arguments_and_void_has_no_return_pointer() {
		let mut parameters = [FLOAT];
		// SAFETY: All-zero bindings are null and empty.
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
		bindings[0].m_desc.m_pszScriptName = c"SetValue".as_ptr();
		bindings[0].m_desc.m_ReturnType = FLOAT;
		bindings[0].m_desc.m_Parameters = vector(&mut parameters);
		bindings[0].m_flags = SF_MEMBER_FUNC;
		bindings[0].m_pfnBinding = Some(adapter);
		bindings[0].m_pFunction.val_0 = 0x1234;
		// SAFETY: As above.
		let mut base: sys::ScriptClassDesc_t = unsafe { zeroed() };
		base.m_pszClassname = c"Base".as_ptr();
		base.m_FunctionBindings = vector(&mut bindings);
		// The binding and the object are changed below only through the
		// pointers `call` reads them by, since writing through the locals would
		// invalidate those pointers.
		let binding = base.m_FunctionBindings.m_Memory.m_pMemory;
		// SAFETY: As above.
		let mut derived: sys::ScriptClassDesc_t = unsafe { zeroed() };
		derived.m_pszClassname = c"Derived".as_ptr();
		derived.m_pBaseDesc = &raw mut base;
		let vtable = entity_vtable();
		let mut object = Object::new(&vtable, &raw mut derived);
		let object = NonNull::from(&mut object);
		let entity = object.cast();
		let calls = || {
			// SAFETY: The object is live, and only read through shared cells.
			unsafe { object.as_ref() }.calls.get()
		};

		// SAFETY: The fake entity, its descriptors and the adapter are live, and
		// the adapter accepts one float.
		unsafe {
			let value = call(entity, c"Base", c"SetValue", &mut [float(3.0)], FLOAT).unwrap();
			assert_eq!(value.__bindgen_anon_1.m_float, 6.0);
			assert_eq!(calls(), 1);
			assert_eq!(
				call(entity, c"Base", c"SetValue", &mut [int(3)], FLOAT).err(),
				Some(BindingError::SignatureMismatch)
			);
			assert_eq!(
				call(entity, c"Base", c"SetValue", &mut [], FLOAT).err(),
				Some(BindingError::SignatureMismatch)
			);
			assert_eq!(
				call(entity, c"Other", c"SetValue", &mut [float(3.0)], FLOAT).err(),
				Some(BindingError::Unavailable)
			);
			assert_eq!(calls(), 1);
			(*binding).m_desc.m_ReturnType = VOID;
			call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).unwrap();
			assert_eq!(calls(), 2);
			(*binding).m_flags = 0;
			assert_eq!(
				call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).err(),
				Some(BindingError::SignatureMismatch)
			);
			(*object.as_ptr()).description = null_mut();
			assert_eq!(
				call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).err(),
				Some(BindingError::Unavailable)
			);
		}
	}

	/// A member adapter for `const char *Object::GetText()`. Like the SDK's
	/// `*pReturn = const char *`, it stores the pointer without a copy.
	unsafe extern "C" fn string_adapter(
		function: sys::ScriptFunctionBindingStorageType_t,
		object: *mut c_void,
		_: *mut sys::ScriptVariant_t,
		count: i32,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		assert_eq!(function.val_0, 0x5678);
		assert_eq!(count, 0);
		assert!(!result.is_null());

		// SAFETY: `call` passes the test's live object.
		let object = unsafe { &*object.cast::<Object>() };

		object.calls.set(object.calls.get() + 1);

		let mut value = variant(STRING);
		value.__bindgen_anon_1.m_pszString = object.text.get();
		value.m_flags = object.flags.get();

		// SAFETY: A string method gets a writable result.
		unsafe { result.write(value) };
		true
	}

	#[test]
	fn string_results_are_copied_before_returning_and_owned_strings_refused() {
		// SAFETY: All-zero bindings are null and empty.
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
		bindings[0].m_desc.m_pszScriptName = c"GetText".as_ptr();
		bindings[0].m_desc.m_ReturnType = STRING;
		bindings[0].m_flags = SF_MEMBER_FUNC;
		bindings[0].m_pfnBinding = Some(string_adapter);
		bindings[0].m_pFunction.val_0 = 0x5678;
		// SAFETY: As above.
		let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };
		description.m_pszClassname = c"Base".as_ptr();
		description.m_FunctionBindings = vector(&mut bindings);
		// Changed below only through the pointer `call` reads it by.
		let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
		let vtable = entity_vtable();
		let mut text = *b"effects/jarate_overlay\0";
		// Written only through this pointer, which the object also returns.
		let storage = text.as_mut_ptr();
		let mut object = Object::new(&vtable, &raw mut description);
		object.text.set(storage.cast_const().cast());
		let object = NonNull::from(&mut object);
		let entity = object.cast();

		// SAFETY: The object is live, and only used through shared cells.
		let object = unsafe { object.as_ref() };

		// SAFETY: The fake entity, its descriptor and the adapter are live, and
		// the adapter returns static or live, terminated strings.
		unsafe {
			let copied = call_string(entity, c"Base", c"GetText", &mut []).unwrap();
			// The game may change its storage once the call returns.
			storage.write(b'X');
			assert_eq!(copied.as_deref(), Some(c"effects/jarate_overlay"));
			assert_eq!(
				call_string(entity, c"Base", c"GetText", &mut [])
					.unwrap()
					.as_deref(),
				Some(c"Xffects/jarate_overlay")
			);
			assert_eq!(object.calls.get(), 2);

			object.text.set(c"".as_ptr());
			assert_eq!(
				call_string(entity, c"Base", c"GetText", &mut []),
				Ok(Some(CString::default()))
			);
			object.text.set(null());
			assert_eq!(call_string(entity, c"Base", c"GetText", &mut []), Ok(None));
			// `call` itself passes the method's pointer through without a copy.
			let overlay = c"effects/imcookin";
			object.text.set(overlay.as_ptr());
			let result = call(entity, c"Base", c"GetText", &mut [], STRING).unwrap();
			assert_eq!(result.__bindgen_anon_1.m_pszString, overlay.as_ptr());
			assert_eq!(object.calls.get(), 5);

			// An owned string has already been returned, but cannot be freed.
			object.flags.set(SV_FREE);
			assert_eq!(
				call_string(entity, c"Base", c"GetText", &mut []),
				Err(BindingError::SignatureMismatch)
			);
			assert_eq!(
				call(entity, c"Base", c"GetText", &mut [], STRING).err(),
				Some(BindingError::SignatureMismatch)
			);
			assert_eq!(object.calls.get(), 7);

			object.flags.set(0);
			(*binding).m_desc.m_ReturnType = FLOAT;
			assert_eq!(
				call_string(entity, c"Base", c"GetText", &mut []),
				Err(BindingError::SignatureMismatch)
			);
			assert_eq!(object.calls.get(), 7);
		}
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
