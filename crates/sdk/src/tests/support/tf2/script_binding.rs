//! The VScript descriptors through which the game exposes native members,
//! and an entity's `GetScriptDesc` returning them.

use sdk_raw::test_support::utl_vector;
use sdk_raw::tf2::script_binding::SF_MEMBER_FUNC;
use std::cell::Cell;
use std::ffi::CStr;
use std::mem::zeroed;
use std::ptr::null_mut;

/// The vtable slot of `CBaseEntity::GetScriptDesc`, which
/// [`script_description`] takes.
pub const SCRIPT_DESCRIPTION_SLOT: usize =
	sdk_raw::vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetScriptDesc);

thread_local! {
	/// What [`script_description`] returns.
	static DESCRIPTION: Cell<*mut sys::ScriptClassDesc_t> = const { Cell::new(null_mut()) };
}

/// The descriptor of the script class `name`, declaring `bindings` and
/// deriving from `base`, which may be null.
///
/// For tests only. The descriptor views `bindings`, which must stay in place
/// while it is used. Without bindings, it lists none, unallocated, as tier1's
/// empty vectors are.
pub fn class_description(
	name: &'static CStr,
	bindings: &mut [sys::ScriptFunctionBinding_t],
	base: *mut sys::ScriptClassDesc_t,
) -> sys::ScriptClassDesc_t {
	// SAFETY: Zero is valid for every field of `ScriptClassDesc_t`.
	let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };

	description.m_pszClassname = name.as_ptr();
	description.m_pBaseDesc = base;

	if !bindings.is_empty() {
		description.m_FunctionBindings = utl_vector(bindings);
	}

	description
}

/// The binding of the native member function `name`, returning `returns`,
/// taking `parameters`, and called through `adapter`, as `DEFINE_SCRIPTFUNC`
/// declares one.
///
/// For tests only. The binding views `parameters`, which must stay in place
/// while it is used. Without parameters, it lists none, unallocated, as
/// tier1's empty vectors are.
pub fn member_binding(
	name: &'static CStr,
	returns: sys::ScriptDataType_t,
	parameters: &mut [sys::ScriptDataType_t],
	adapter: sys::ScriptBindingFunc_t,
) -> sys::ScriptFunctionBinding_t {
	// SAFETY: Zero is valid for every field of `ScriptFunctionBinding_t`.
	let mut binding: sys::ScriptFunctionBinding_t = unsafe { zeroed() };

	binding.m_desc.m_pszScriptName = name.as_ptr();
	binding.m_desc.m_ReturnType = returns;
	binding.m_flags = SF_MEMBER_FUNC;
	binding.m_pfnBinding = adapter;

	if !parameters.is_empty() {
		binding.m_desc.m_Parameters = utl_vector(parameters);
	}

	binding
}

/// `CBaseEntity::GetScriptDesc`, which returns the descriptor
/// [`set_script_description`] set on this thread, or null.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn script_description(
	_: *mut sys::CBaseEntity,
) -> *mut sys::ScriptClassDesc_t {
	DESCRIPTION.get()
}

/// Sets the descriptor [`script_description`] returns on this thread, or
/// null for none.
///
/// For tests only. The descriptor must stay alive while it is returned.
pub fn set_script_description(description: *mut sys::ScriptClassDesc_t) {
	DESCRIPTION.set(description);
}
