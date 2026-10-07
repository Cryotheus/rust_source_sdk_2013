//! Calls a selected native method through the game's typed VScript adapter,
//! for entities of the current engine callback.
//!
//! This wraps [`sdk_raw::tf2::script_binding`], which describes the class
//! descriptors, bindings and variants involved.

use crate::entities::Entity;
use sdk_raw::tf2::script_binding as raw;
use std::ffi::{CStr, CString};
use std::ptr::NonNull;

pub(crate) use raw::{BindingError, FLOAT, INT, VOID, float, string};

/// Finds and invokes a native member on a named declaring class, as
/// [`sdk_raw::tf2::script_binding::call`] does.
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
	// SAFETY: An entity's pointer is never null, and it stays live on the
	// main thread for its callback, whose game module stays loaded. The
	// caller vouches for the method, its arguments and its effects.
	unsafe {
		raw::call(
			NonNull::new_unchecked(entity.as_ptr()),
			class,
			name,
			arguments,
			result_type,
		)
	}
}

/// Calls a native member that returns a C string, and copies the string, as
/// [`sdk_raw::tf2::script_binding::call_string`] does.
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
	// SAFETY: As for `call`, and the caller vouches for the returned string.
	unsafe {
		raw::call_string(
			NonNull::new_unchecked(entity.as_ptr()),
			class,
			name,
			arguments,
		)
	}
}
