//! `ICvar`, the registry of console variables and commands.

use crate::commands::CommandFlags;
use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr, vcall};
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// The registry of console variables and commands (`ICvar`).
	#[doc(alias = "ICvar")]
	pub struct Cvar(sys::ICvar) = Engine c"VEngineCvar004";
}

/// A console variable (`ConVar`).
///
/// Every accessor reads the current value, which commands and code may change
/// at any time.
///
/// The setters change the value as the engine does when the console sets it,
/// through the variable's own `SetValue`, which clamps it to the variable's
/// bounds and runs its change callbacks: those tell clients of a value marked
/// `FCVAR_REPLICATED`, and announce one marked `FCVAR_NOTIFY`. The console's
/// own checks are skipped, so a variable marked `FCVAR_CHEAT` changes whether
/// or not `sv_cheats` is set, as it does for other server plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConVar<'s> {
	raw: NonNull<sys::ConVar>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ConVar<'s> {
	/// # Safety
	///
	/// `raw` must point to a live, registered variable that stays registered
	/// for `'s`, used only on the server's main thread.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::ConVar>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::ConVar {
		self.raw.as_ptr()
	}

	/// The value the variable was declared with.
	#[doc(alias = "GetDefault")]
	pub fn default_string(self) -> CString {
		// SAFETY: As for `string`.
		unsafe { copy_cstr((&raw const (*self.parent()).m_pszDefaultValue).read()) }
			.unwrap_or_default()
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

	/// The value's `IConVar` interface, whose `SetValue` overloads the variable
	/// implements.
	///
	/// MSVC places those overloads only in this interface's vtable, not in the
	/// variable's own, and they expect the pointer to this subobject.
	fn interface(self) -> *mut sys::IConVar {
		// SAFETY: The variable is live, and the subobject lies within it.
		unsafe { &raw mut (*self.as_ptr())._base_1 }
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

	/// Sets the value from a float, which the string then shows with six
	/// decimals. Nothing happens if the float value is unchanged.
	#[doc(alias = "SetValue")]
	pub fn set_float(self, value: f32) {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.interface() => IConVar_SetValue1(value)) };
	}

	/// Sets the value from an integer. Nothing happens if the integer value is
	/// unchanged.
	#[doc(alias = "SetValue")]
	pub fn set_int(self, value: c_int) {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.interface() => IConVar_SetValue2(value)) };
	}

	/// Sets the value from a string, as the console does.
	#[doc(alias = "SetValue")]
	pub fn set_string(self, value: &CStr) {
		// SAFETY: As for `name`. The variable copies the string.
		unsafe { vcall!(self.interface() => IConVar_SetValue(value.as_ptr())) };
	}

	/// Sets the value from a string as [`Self::set_string`] does, without
	/// announcing the change to players and the server log, even if the
	/// variable is marked `FCVAR_NOTIFY`. Clients are still told of a
	/// replicated value.
	///
	/// The flag is cleared while the variable's change callbacks run, since
	/// the engine's checks it then.
	pub fn set_string_quietly(self, value: &CStr) {
		let notify = CommandFlags::NOTIFY.bits();

		// SAFETY: As for `name`. The flags are read and written without forming
		// references, since C++ writes them too.
		let flags = unsafe { &raw mut (*self.parent())._base.m_nFlags };

		// SAFETY: As above.
		let announced = unsafe { flags.read() } & notify;

		// SAFETY: As above.
		unsafe { flags.write(flags.read() & !notify) };
		self.set_string(value);

		// SAFETY: As above. Flags the callbacks added are kept.
		unsafe { flags.write(flags.read() | announced) };
	}

	/// The current value as a string.
	#[doc(alias = "GetString")]
	pub fn string(self) -> CString {
		// SAFETY: As for `name`. Changing the value reallocates the string, so
		// it is copied immediately.
		unsafe { copy_cstr((&raw const (*self.parent()).m_pszString).read()) }.unwrap_or_default()
	}
}

impl<'s> Cvar<'s> {
	/// Reserves an identifier, which `ICvar::UnregisterConCommands` uses to
	/// unlink every command a module registered.
	#[doc(alias = "AllocateDLLIdentifier")]
	pub(crate) fn allocate_dll_identifier(self) -> sys::CVarDLLIdentifier_t {
		// SAFETY: As for `find_var`.
		unsafe { vcall!(self.as_ptr() => ICvar_AllocateDLLIdentifier()) }
	}

	/// Runs the callbacks every variable's change runs, such as the engine's,
	/// which tells clients of replicated values and announces notifying ones.
	///
	/// # Safety
	///
	/// `var` must be a live, registered variable whose value just changed from
	/// `old_value`, as `ConVar::ChangeStringValue` calls it.
	#[doc(alias = "CallGlobalChangeCallbacks")]
	pub(crate) unsafe fn call_global_change_callbacks(
		self,
		var: NonNull<sys::ConVar>,
		old_value: &CStr,
		old_float: f32,
	) {
		// SAFETY: As for `find_var`, and the caller upholds the contract.
		unsafe {
			vcall!(self.as_ptr() => ICvar_CallGlobalChangeCallbacks(
				var.as_ptr(),
				old_value.as_ptr(),
				old_float,
			))
		};
	}

	/// Prints to the console display functions, which a dedicated server does
	/// not install; see [`Server::console_print`](crate::Server::console_print).
	#[doc(alias = "ConsolePrintf")]
	pub(crate) fn console_printf(self, message: &CStr) {
		// SAFETY: As for `find_var`. The message is passed as an argument of a
		// constant format, so it is never interpreted as one.
		unsafe { vcall!(self.as_ptr() => ICvar_ConsolePrintf(c"%s".as_ptr(), message.as_ptr())) };
	}

	/// Finds a console variable or command by name, ignoring case.
	#[doc(alias = "FindCommandBase")]
	pub(crate) fn find_command_base(self, name: &CStr) -> Option<NonNull<sys::ConCommandBase>> {
		// SAFETY: As for `find_var`.
		NonNull::new(unsafe { vcall!(self.as_ptr() => ICvar_FindCommandBase(name.as_ptr())) })
	}

	/// Finds a console variable by name. Commands are not variables.
	#[doc(alias = "FindVar")]
	pub fn find_var(self, name: &CStr) -> Option<ConVar<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let var = NonNull::new(unsafe { vcall!(self.as_ptr() => ICvar_FindVar(name.as_ptr())) })?;

		// SAFETY: The registry holds live variables, which the server keeps
		// registered for `'s`.
		Some(unsafe { ConVar::from_raw(var) })
	}

	/// Links a command into the registry.
	///
	/// # Safety
	///
	/// `command` must be a live `ConCommandBase` that stays at its address,
	/// with its code loaded, until it is unregistered.
	#[doc(alias = "RegisterConCommand")]
	pub(crate) unsafe fn register_con_command(self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `find_var`, and the caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => ICvar_RegisterConCommand(command.as_ptr())) };
	}

	/// Unlinks a command from the registry.
	///
	/// # Safety
	///
	/// `command` must be a live `ConCommandBase`.
	#[doc(alias = "UnregisterConCommand")]
	pub(crate) unsafe fn unregister_con_command(self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `find_var`, and the caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => ICvar_UnregisterConCommand(command.as_ptr())) };
	}
}
