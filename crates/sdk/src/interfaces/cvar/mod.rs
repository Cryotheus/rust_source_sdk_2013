//! `ICvar`, the registry of console variables and commands.

use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr, vcall};
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// The registry of console variables and commands (`ICvar`).
	#[doc(alias = "ICvar")]
	pub struct Cvar(sys::ICvar) = Engine c"VEngineCvar004";
}

impl<'s> Cvar<'s> {
	/// Finds a console variable by name. Commands are not variables.
	#[doc(alias = "FindVar")]
	pub fn find_var(self, name: &CStr) -> Option<ConVar<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let var = NonNull::new(unsafe { vcall!(self.as_ptr() => ICvar_FindVar(name.as_ptr())) })?;

		Some(ConVar {
			raw: var,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}
}

/// A console variable (`ConVar`).
///
/// Every accessor reads the current value, which commands and code may change
/// at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConVar<'s> {
	raw: NonNull<sys::ConVar>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ConVar<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::ConVar {
		self.raw.as_ptr()
	}

	/// The variable's name.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Registered variables stay registered for `'s`, and their names
		// are string literals of the module that registered them. Fields are
		// read without forming references.
		unsafe { borrow_cstr((&raw const (*self.as_ptr())._base.m_pszName).read()) }
			.unwrap_or_default()
	}

	/// The current value as a string.
	#[doc(alias = "GetString")]
	pub fn string(self) -> CString {
		// SAFETY: As for `name`. Changing the value reallocates the string, so
		// it is copied immediately.
		unsafe { copy_cstr((&raw const (*self.parent()).m_pszString).read()) }.unwrap_or_default()
	}

	/// The current value as a float.
	#[doc(alias = "GetFloat")]
	pub fn float(self) -> f32 {
		// SAFETY: As for `name`.
		unsafe { (&raw const (*self.parent()).m_fValue).read() }
	}

	/// The current value as an integer.
	#[doc(alias = "GetInt")]
	pub fn int(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { (&raw const (*self.parent()).m_nValue).read() }
	}

	/// The variable that holds the value, which differs from `self` when
	/// several modules register the same name.
	fn parent(self) -> *mut sys::ConVar {
		// SAFETY: As for `name`.
		let parent = unsafe { (&raw const (*self.as_ptr()).m_pParent).read() };

		if parent.is_null() {
			self.as_ptr()
		} else {
			parent
		}
	}
}
