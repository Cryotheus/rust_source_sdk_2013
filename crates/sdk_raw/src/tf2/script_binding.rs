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
//! borrows whatever storage the method returned it from. Vectors and angles
//! are copied into an allocation the result owns, so [`call`] takes them only
//! as arguments.
//!
//! Native methods take and return entities as their script instances
//! (`HSCRIPT`), which [`ensure_script_instance`] gets for an entity.

use crate::entities::datamap::DataMaps;
use crate::entities::find_base_entity_field;
use crate::util::cstr::{borrow_cstr, copy_cstr};
use crate::{vcall, vtable_slot};
use std::ffi::{CStr, CString, c_int, c_uint, c_void};
use std::mem::{offset_of, zeroed};
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

// `game/server/baseentity.h` declares the instance handle directly before
// the script ID, with nothing between them on either ABI.
const _: () = assert!(SCRIPT_INSTANCE_BEFORE_SCRIPT_ID == size_of::<sys::HSCRIPT>());

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

/// The script type of a `QAngle` (`FIELD_QANGLE`), for [`qangle`] arguments.
///
/// Native methods returning angles copy them into an allocation the result
/// owns, so [`call`] refuses this as a result type.
pub const QANGLE: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_QANGLE as _;

/// How many bytes `CBaseEntity::m_hScriptInstance` lies before
/// `m_iszScriptId`, the member after it, whose offset the `CBaseEntity`
/// datamap declares.
const SCRIPT_INSTANCE_BEFORE_SCRIPT_ID: usize =
	offset_of!(sys::CBaseEntity, m_iszScriptId) - offset_of!(sys::CBaseEntity, m_hScriptInstance);

/// `SF_MEMBER_FUNC` from `public/vscript/ivscript.h`: the binding wraps a
/// member function, whose object [`call`] passes as the adapter's context.
pub const SF_MEMBER_FUNC: c_uint = 0x01;

/// The script type of a C string (`FIELD_CSTRING`).
pub const STRING: sys::ScriptDataType_t = sys::ExtendedFieldType_t_FIELD_CSTRING as _;

/// `SV_FREE` from `public/vscript/variant.h`: the variant owns its pointed-to
/// data, which only the game's allocator may free.
pub const SV_FREE: u16 = 0x01;

/// The script type of a `Vector` (`FIELD_VECTOR`), for [`vector`] arguments.
///
/// Native methods returning vectors copy them into an allocation the result
/// owns, so [`call`] refuses this as a result type.
pub const VECTOR: sys::ScriptDataType_t = sys::_fieldtypes_FIELD_VECTOR as _;

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
/// binding's adapter, as [`call_on`] does with the descriptor the entity's
/// `GetScriptDesc` returns.
///
/// # Safety
///
/// As for [`call_on`], with `entity` pointing to a live `CBaseEntity` of the
/// loaded game module.
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
	let descriptor =
		unsafe { vcall!(this as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetScriptDesc()) };

	// SAFETY: The descriptor is the entity's own, and the caller upholds the
	// rest of `call_on`'s contract.
	unsafe {
		call_on(
			entity.cast(),
			descriptor,
			class,
			name,
			arguments,
			result_type,
		)
	}
}

/// Finds and invokes a native member of `object`, whose script class
/// descriptor is `descriptor`, on a named declaring class.
///
/// Searches `descriptor` and its bases for the class named `class`, then
/// calls that class's binding named `name` through the binding's adapter. The
/// binding must declare the types of `arguments` and return `result_type`; a
/// [`VOID`] call returns an empty variant. This reaches the native methods of
/// script objects that are not entities, such as a NextBot's locomotion
/// interface, as well as an entity's.
///
/// The returned variant must not own an allocation: any flag, of which
/// [`SV_FREE`] is the only one, fails with [`BindingError::SignatureMismatch`]
/// after the method has run, leaking the allocation. Scalar results (bool,
/// int, float, or HSCRIPT) are values. A [`STRING`] result is the pointer the
/// method returned, without a copy or ownership: it may be null, and it stays
/// valid only as long as the storage the method returned it from. Prefer
/// [`call_string`], which copies it before any other game code runs. A
/// [`VECTOR`] or [`QANGLE`] result, which the game's adapter would copy into
/// such an allocation, fails with [`BindingError::SignatureMismatch`] before
/// the method runs.
///
/// # Safety
///
/// - `object` points to a live object of the loaded game module, which stays
///   loaded for the call, `descriptor` is null or the descriptor of the
///   object's script class or one of its bases, and the call is made on the
///   server's main thread.
/// - The selected method accepts this object and these argument values.
///   Pointer arguments, such as [`string`]'s, remain valid for the call and
///   for as long as the method retains them.
/// - The game code the method runs, and everything it reaches, frees entities
///   only through the engine's deferred deletion.
pub unsafe fn call_on(
	object: NonNull<c_void>,
	descriptor: *mut sys::ScriptClassDesc_t,
	class: &CStr,
	name: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
	result_type: sys::ScriptDataType_t,
) -> Result<sys::ScriptVariant_t, BindingError> {
	if result_type == VECTOR || result_type == QANGLE {
		return Err(BindingError::SignatureMismatch);
	}

	let mut descriptor = descriptor;

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
						object.as_ptr(),
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

/// Returns the entity's script instance, the `HSCRIPT` native methods take and
/// return for it, creating it if it has none yet.
///
/// The instance is read from `CBaseEntity::m_hScriptInstance` at `offset`,
/// which [`script_instance_offset`] finds. An entity without one gets one
/// from the `CBaseEntity` binding `ValidateScriptScope`, which creates the
/// instance and the script scope that scripts touching the entity would
/// create. Fails with [`BindingError::Rejected`] if the game has no script
/// VM, such as under `-scripting`, and with [`BindingError::Unavailable`] if
/// the entity still has no instance afterwards.
///
/// # Safety
///
/// As for [`call`], with `offset` from [`script_instance_offset`] of this
/// entity's datamaps. The entity must not be marked for deletion: one whose
/// `UpdateOnRemove` already removed its instance would register a new one
/// that nothing removes.
#[doc(alias("GetScriptInstance", "ValidateScriptScope", "m_hScriptInstance"))]
pub unsafe fn ensure_script_instance(
	entity: NonNull<sys::CBaseEntity>,
	offset: usize,
) -> Result<NonNull<sys::HSCRIPT__>, BindingError> {
	// SAFETY: The caller vouches for the entity and the offset.
	if let Some(instance) = NonNull::new(unsafe { script_instance(entity, offset) }) {
		return Ok(instance);
	}

	// SAFETY: As the caller promises. The binding takes no arguments, and only
	// creates the entity's script instance and scope.
	let result = unsafe {
		call(
			entity,
			c"CBaseEntity",
			c"ValidateScriptScope",
			&mut [],
			BOOL,
		)
	}?;

	// SAFETY: `call` checked that the adapter returned a `FIELD_BOOLEAN`
	// variant, which it assigns through the bool member.
	if !unsafe { result.__bindgen_anon_1.m_bool } {
		return Err(BindingError::Rejected);
	}

	// SAFETY: As above.
	NonNull::new(unsafe { script_instance(entity, offset) }).ok_or(BindingError::Unavailable)
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

/// A borrowed `QAngle` argument. The caller of [`call`] keeps the angles alive
/// until the native call ends. The method only reads them.
pub fn qangle(value: &sys::QAngle) -> sys::ScriptVariant_t {
	let mut result = variant(QANGLE);
	result.__bindgen_anon_1.m_pData = std::ptr::from_ref(value).cast_mut().cast();
	result
}

/// Reads the entity's script instance at `offset`: the `HSCRIPT` its
/// `GetScriptInstance` registered, or null if nothing has made one yet.
///
/// # Safety
///
/// `entity` points to a live `CBaseEntity` of the loaded game module, and
/// `offset` is from [`script_instance_offset`] of its datamaps.
#[doc(alias("m_hScriptInstance"))]
pub unsafe fn script_instance(entity: NonNull<sys::CBaseEntity>, offset: usize) -> sys::HSCRIPT {
	// SAFETY: The offset is the instance handle's in every entity, aligned for
	// it, as the caller vouches.
	unsafe { entity.byte_add(offset).cast::<sys::HSCRIPT>().read() }
}

/// The offset of `CBaseEntity::m_hScriptInstance` in the entity whose
/// datamaps `maps` are: the member before `m_iszScriptId`, which the
/// `CBaseEntity` datamap declares (`baseentity.cpp`).
///
/// Returns `None` if that map does not declare the script ID as a pooled
/// string, or the instance handle would not be aligned for a pointer.
#[doc(alias("m_hScriptInstance", "m_iszScriptId"))]
pub fn script_instance_offset(maps: DataMaps<'_>) -> Option<usize> {
	let script_id = find_base_entity_field(
		maps,
		c"m_iszScriptId",
		sys::_fieldtypes_FIELD_STRING,
		size_of::<sys::string_t>(),
	)?;

	script_id
		.checked_sub(SCRIPT_INSTANCE_BEFORE_SCRIPT_ID)
		.filter(|offset| offset.is_multiple_of(align_of::<sys::HSCRIPT>()))
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

/// A borrowed `Vector` argument. The caller of [`call`] keeps the vector alive
/// until the native call ends. The method only reads it.
pub fn vector(value: &sys::Vector) -> sys::ScriptVariant_t {
	let mut result = variant(VECTOR);
	result.__bindgen_anon_1.m_pVector = value;
	result
}
