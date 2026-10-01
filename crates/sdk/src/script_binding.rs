//! Calls a selected native method through the game's typed VScript adapter.
//!
//! This does not evaluate script text or invoke a VM. The class descriptors
//! retain the compiler-generated adapters and member-function pointers, which
//! avoids guessing TF2 layouts or the platform's member-pointer ABI.

use crate::entities::Entity;
use crate::ffi::borrow_cstr;
use std::ffi::CStr;
use std::mem::{offset_of, size_of, transmute, zeroed};

pub(crate) const BOOL: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_BOOLEAN as _;
pub(crate) const FLOAT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_FLOAT as _;
pub(crate) const HANDLE: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_HSCRIPT as _;
pub(crate) const INT: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_INTEGER as _;
pub(crate) const STRING: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_CSTRING as _;
pub(crate) const VOID: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_VOID as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum BindingError {
	#[error("the entity does not expose the requested native method")]
	Unavailable,
	#[error("the native method's runtime signature does not match the SDK")]
	SignatureMismatch,
	#[error("the native method rejected its arguments")]
	Rejected,
}

pub(crate) fn boolean(value: bool) -> sys::ScriptVariant_t {
	let mut result = variant(BOOL);
	result.__bindgen_anon_1.m_bool = value;
	result
}

/// Finds and invokes a native member on a named declaring class.
///
/// # Safety
///
/// The selected method must accept this live entity and argument values.
/// Pointer arguments must remain valid for the call and any lifetime the
/// method retains them for. The method and any callbacks it reaches must
/// uphold `Server::new`'s entity-deletion and callback-scope requirements.
/// Return values must be non-owning scalar variants (void, bool, int, float,
/// or HSCRIPT); allocated variants need the game's allocator to free them.
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
	const _: () = assert!(SLOT == sys::CBASEENTITY_DATAMAP_VTABLE_SLOT + 1);
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
	for _ in 0..64 {
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
			if !(0..=4096).contains(&count) || (count != 0 && bindings.is_null()) {
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
					|| flags != 1
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
				let mut result = variant(VOID);
				// The void specialization explicitly requires a null return pointer.
				let result_ptr = if result_type == VOID {
					std::ptr::null_mut()
				} else {
					&raw mut result
				};
				// SAFETY: The validated adapter handles the native member pointer
				// representation. The caller vouches for argument values/effects.
				if !unsafe {
					adapter(
						function,
						entity.as_ptr().cast(),
						arguments.as_mut_ptr(),
						arguments.len() as i32,
						result_ptr,
					)
				} {
					return Err(BindingError::Rejected);
				}
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

pub(crate) fn float(value: f32) -> sys::ScriptVariant_t {
	let mut result = variant(FLOAT);
	result.__bindgen_anon_1.m_float = value;
	result
}

pub(crate) fn handle(value: sys::HSCRIPT) -> sys::ScriptVariant_t {
	let mut result = variant(HANDLE);
	result.__bindgen_anon_1.m_hScript = value;
	result
}

pub(crate) fn int(value: i32) -> sys::ScriptVariant_t {
	let mut result = variant(INT);
	result.__bindgen_anon_1.m_int = value;
	result
}

/// The caller of `call` keeps this string alive until the native call ends.
pub(crate) fn string(value: &CStr) -> sys::ScriptVariant_t {
	let mut result = variant(STRING);
	result.__bindgen_anon_1.m_pszString = value.as_ptr();
	result
}

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
	use std::ptr::{NonNull, null_mut};

	#[repr(C)]
	struct Object {
		vtable: *const *const (),
		description: *mut sys::ScriptClassDesc_t,
		calls: Cell<usize>,
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
		bindings[0].m_flags = 1;
		bindings[0].m_pfnBinding = Some(adapter);
		bindings[0].m_pFunction.val_0 = 0x1234;
		let mut base: sys::ScriptClassDesc_t = unsafe { zeroed() };
		base.m_pszClassname = c"Base".as_ptr();
		base.m_FunctionBindings = vector(&mut bindings);
		let mut derived: sys::ScriptClassDesc_t = unsafe { zeroed() };
		derived.m_pszClassname = c"Derived".as_ptr();
		derived.m_pBaseDesc = &raw mut base;
		let mut vtable = [std::ptr::null(); 16];
		vtable[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT + 1] = get_description as *const ();
		let mut object = Object {
			vtable: vtable.as_ptr(),
			description: &raw mut derived,
			calls: Cell::new(0),
		};
		let entity = unsafe { Entity::from_raw(NonNull::from(&mut object).cast()) };
		let value =
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], FLOAT) }.unwrap();
		assert_eq!(unsafe { value.__bindgen_anon_1.m_float }, 6.0);
		assert_eq!(object.calls.get(), 1);
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
		assert_eq!(object.calls.get(), 1);
		bindings[0].m_desc.m_ReturnType = VOID;
		unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.unwrap();
		assert_eq!(object.calls.get(), 2);
		unsafe { (&raw mut bindings[0].m_flags).write(0) };
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.err(),
			Some(BindingError::SignatureMismatch)
		);
		unsafe { (&raw mut object.description).write(null_mut()) };
		assert_eq!(
			unsafe { call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID) }.err(),
			Some(BindingError::Unavailable)
		);
	}

	fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
		sys::CUtlVector {
			_phantom_0: Default::default(),
			_phantom_1: Default::default(),
			m_Memory: sys::CUtlMemory {
				_phantom_0: Default::default(),
				m_pMemory: values.as_mut_ptr(),
				m_nAllocationCount: values.len() as i32,
				m_nGrowSize: 0,
			},
			m_Size: values.len() as i32,
			m_pElements: values.as_mut_ptr(),
		}
	}
}
