//! `ICvar`, the registry of console variables and commands.

use crate::NotThreadSafe;
use crate::commands::{CommandBaseKind, CommandFlags};
use sdk_raw::util::cstr::{borrow_cstr, copy_cstr};
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The most entries [`Cvar::command_bases`] lists.
const MAX_LISTED: usize = 1 << 16;

interface! {
	/// The registry of console variables and commands (`ICvar`).
	#[doc(alias = "ICvar")]
	pub struct Cvar(sys::ICvar) = Engine c"VEngineCvar004";
}

/// A console variable or command the registry lists (`ConCommandBase`), from
/// [`Cvar::command_bases`].
///
/// The handle stays usable for all of `'s`, even if the entry is unregistered,
/// since condition 5 of [`Server::new`] keeps it allocated.
///
/// [`Server::new`]: crate::Server::new
#[doc(alias = "ConCommandBase")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandBase<'s> {
	raw: NonNull<sys::ConCommandBase>,
	kind: CommandBaseKind,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> CommandBase<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::ConCommandBase {
		self.raw.as_ptr()
	}

	/// The variable, or `None` for a command.
	pub const fn as_var(self) -> Option<ConVar<'s>> {
		match self.kind {
			CommandBaseKind::Command => None,

			// SAFETY: The registry listed the variable, so condition 5 of
			// `Server::new` keeps it allocated for `'s`. Its `ConCommandBase` is
			// at its start, as `sdk_raw::commands::variable` asserts, so the pointer to
			// one is a pointer to the other.
			CommandBaseKind::Variable => Some(unsafe { ConVar::from_raw(self.raw.cast()) }),
		}
	}

	/// The flags, which for a variable are those of the variable that holds
	/// its value, as [`ConVar::flags`] reads them.
	#[doc(alias = "GetFlags")]
	#[doc(alias = "IsFlagSet")]
	pub fn flags(self) -> CommandFlags {
		if let Some(var) = self.as_var() {
			return var.flags();
		}

		// SAFETY: As for `name`.
		CommandFlags::from_bits_retain(unsafe { (&raw const (*self.as_ptr()).m_nFlags).read() })
	}

	/// Whether this is a command or a variable, as its `IsCommand` reported
	/// when the registry was listed.
	#[doc(alias = "IsCommand")]
	pub const fn kind(self) -> CommandBaseKind {
		self.kind
	}

	/// The name it is registered under.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: The registry listed the entry, so condition 5 of `Server::new`
		// keeps it allocated, with its name unchanged, for `'s`. The field is
		// read without forming a reference.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).m_pszName).read()) }.unwrap_or_default()
	}
}

/// The console variables and commands the registry listed when
/// [`Cvar::command_bases`] was called, in its order.
#[derive(Debug, Clone)]
pub struct CommandBases<'s> {
	listed: std::vec::IntoIter<CommandBase<'s>>,
}

impl DoubleEndedIterator for CommandBases<'_> {
	fn next_back(&mut self) -> Option<Self::Item> {
		self.listed.next_back()
	}
}

impl ExactSizeIterator for CommandBases<'_> {}

impl FusedIterator for CommandBases<'_> {}

impl<'s> Iterator for CommandBases<'s> {
	type Item = CommandBase<'s>;

	fn next(&mut self) -> Option<Self::Item> {
		self.listed.next()
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		self.listed.size_hint()
	}
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
	/// `raw` must point to a variable the registry lists at any point during
	/// `'s`, which condition 5 of [`Server::new`](crate::Server::new) keeps
	/// allocated, with its name unchanged, for `'s`. It is used only on the
	/// server's main thread.
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

	/// The flags of the variable that holds the value, which keep every bit
	/// the engine stores.
	#[doc(alias = "GetFlags")]
	#[doc(alias = "IsFlagSet")]
	pub fn flags(self) -> CommandFlags {
		// SAFETY: As for `name`.
		CommandFlags::from_bits_retain(unsafe {
			(&raw const (*self.parent())._base.m_nFlags).read()
		})
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

	/// Whether the current value is the string the variable was declared with,
	/// byte for byte, so `1.0` is not the default `1`.
	///
	/// A variable marked [`CommandFlags::NEVER_AS_STRING`] keeps that string
	/// whatever its value, so it always counts as default.
	pub fn is_default(self) -> bool {
		let parent = self.parent();

		// SAFETY: As for `name`. Nothing can change the strings while they are
		// compared, so neither is copied.
		unsafe {
			let current = borrow_cstr((&raw const (*parent).m_pszString).read());
			let default = borrow_cstr((&raw const (*parent).m_pszDefaultValue).read());

			current.unwrap_or_default() == default.unwrap_or_default()
		}
	}

	/// The variable's name.
	#[doc(alias = "GetName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: `from_raw`'s caller guaranteed that the variable stays
		// allocated, with its name unchanged, for `'s`. Fields are read without
		// forming references.
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
	/// the engine's callback checks it then.
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

	/// Every console variable and command the registry lists, in its order.
	///
	/// The registry is read in full before this returns: `ICvar::GetCommands`
	/// gives the first entry, each entry's `m_pNext` the next, and each
	/// entry's `IsCommand` its kind. Only those `IsCommand` calls run during
	/// the walk, so commands and variables registered or unregistered later,
	/// as running a command may, cannot disturb the walk.
	///
	/// The result is a snapshot that does not reflect those changes: an entry
	/// unregistered after the walk is still listed. Its handle stays usable
	/// only because condition 5 of [`Server::new`] keeps every listed entry
	/// allocated for `'s`.
	///
	/// The walk stops after 65,536 entries, far more than any game registers,
	/// so a corrupted registry that links an entry back to an earlier one
	/// cannot hang the server.
	///
	/// [`Server::new`]: crate::Server::new
	#[doc(alias = "GetCommands")]
	#[doc(alias = "GetNext")]
	pub fn command_bases(self) -> CommandBases<'s> {
		let mut listed = Vec::new();

		// SAFETY: As for `find_var`.
		let mut next = unsafe { vcall!(self.as_ptr() => ICvar_GetCommands()) };

		while let Some(base) = NonNull::new(next)
			&& listed.len() < MAX_LISTED
		{
			// SAFETY: The registry links live commands and variables, which
			// condition 5 of `Server::new` keeps allocated, with their code
			// loaded, for `'s`.
			let is_command = unsafe { vcall!(base.as_ptr() => ConCommandBase_IsCommand()) };

			// SAFETY: As above. The link is read without forming a reference,
			// since the registry rewrites it.
			next = unsafe { (&raw const (*base.as_ptr()).m_pNext).read() };

			listed.push(CommandBase {
				raw: base,
				kind: if is_command {
					CommandBaseKind::Command
				} else {
					CommandBaseKind::Variable
				},
				_scope: PhantomData,
				_not_thread_safe: PhantomData,
			});
		}

		CommandBases {
			listed: listed.into_iter(),
		}
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
	///
	/// Returns `None` if no variable is registered under the name.
	#[doc(alias = "FindVar")]
	pub fn find_var(self, name: &CStr) -> Option<ConVar<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let var = NonNull::new(unsafe { vcall!(self.as_ptr() => ICvar_FindVar(name.as_ptr())) })?;

		// SAFETY: The registry lists the variable, so condition 5 of
		// `Server::new` keeps it allocated, with its name unchanged, for `'s`.
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

	/// Every console variable the registry lists, in its order, leaving out
	/// commands. The registry is listed in full first, as
	/// [`Self::command_bases`] describes.
	#[doc(alias = "GetCommands")]
	pub fn vars(self) -> impl DoubleEndedIterator<Item = ConVar<'s>> + FusedIterator {
		self.command_bases().filter_map(CommandBase::as_var)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::ptr::null_mut;

	/// A registry whose `GetCommands` returns `head`.
	#[repr(C)]
	struct MockCvar {
		interface: sys::ICvar,
		head: *mut sys::ConCommandBase,
	}

	unsafe extern "C" fn get_commands(this: *mut sys::ICvar) -> *mut sys::ConCommandBase {
		unsafe { (*this.cast::<MockCvar>()).head }
	}

	unsafe extern "C" fn is_command(_: *const sys::ConCommandBase) -> bool {
		true
	}

	unsafe extern "C" fn is_variable(_: *const sys::ConCommandBase) -> bool {
		false
	}

	#[test]
	fn listing_ends_at_the_last_entry_or_the_limit() {
		assert_eq!(mock_cvar(null_mut()).command_bases().len(), 0);
		assert_eq!(mock_cvar(null_mut()).vars().count(), 0);

		// A corrupted registry that links an entry to itself.
		let looped = mock_command(c"sb_loop", null_mut());

		unsafe { (*looped).m_pNext = looped };

		let mut listed = mock_cvar(looped).command_bases();

		assert_eq!(listed.len(), MAX_LISTED);
		assert!(listed.all(|base| base.as_ptr() == looped));
	}

	/// A `ConCommandBase` whose `IsCommand` reports `kind`, linked to `next`.
	fn mock_base(
		name: &'static CStr,
		kind: CommandBaseKind,
		flags: CommandFlags,
		next: *mut sys::ConCommandBase,
	) -> sys::ConCommandBase {
		let vtable = unsafe {
			mock_vtable::<sys::ConCommandBase__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).ConCommandBase_IsCommand).write(match kind {
						CommandBaseKind::Command => is_command,
						CommandBaseKind::Variable => is_variable,
					});
				},
			)
		};

		sys::ConCommandBase {
			vtable_: Box::leak(vtable),
			m_pNext: next,
			m_bRegistered: true,
			m_pszName: name.as_ptr(),
			m_pszHelpString: c"".as_ptr(),
			m_nFlags: flags.bits(),
		}
	}

	fn mock_command(
		name: &'static CStr,
		next: *mut sys::ConCommandBase,
	) -> *mut sys::ConCommandBase {
		let base = mock_base(name, CommandBaseKind::Command, CommandFlags::NONE, next);

		Box::into_raw(Box::new(base))
	}

	fn mock_cvar(head: *mut sys::ConCommandBase) -> Cvar<'static> {
		let vtable = unsafe {
			mock_vtable::<sys::ICvar__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).ICvar_GetCommands).write(get_commands);
			})
		};
		let cvar = Box::leak(Box::new(MockCvar {
			interface: sys::ICvar {
				vtable_: Box::leak(vtable),
			},
			head,
		}));

		unsafe { Cvar::from_raw(NonNull::from(cvar).cast()) }
	}

	/// A variable that is its own parent, linked to `next`.
	fn mock_var(
		name: &'static CStr,
		default: &'static CStr,
		value: &'static CStr,
		flags: CommandFlags,
		next: *mut sys::ConCommandBase,
	) -> *mut sys::ConVar {
		// SAFETY: Zero is valid for every field of `ConVar`.
		let var = Box::into_raw(Box::new(unsafe { std::mem::zeroed::<sys::ConVar>() }));

		unsafe {
			(*var)._base = mock_base(name, CommandBaseKind::Variable, flags, next);
			(*var).m_pParent = var;
			(*var).m_pszDefaultValue = default.as_ptr();
			(*var).m_pszString = value.as_ptr().cast_mut();
		}

		var
	}

	#[test]
	fn the_registry_lists_variables_and_commands_in_order() {
		let gravity = mock_var(
			c"sv_gravity",
			c"800",
			c"800",
			CommandFlags::REPLICATED | CommandFlags::NOTIFY,
			null_mut(),
		);
		let status = mock_command(c"status", gravity.cast());
		let timelimit = mock_var(c"mp_timelimit", c"0", c"30", CommandFlags::NOTIFY, status);
		let cvar = mock_cvar(timelimit.cast());
		let mut listed = cvar.command_bases();

		assert_eq!(listed.len(), 3);
		assert_eq!(
			listed
				.clone()
				.map(|base| (base.name(), base.kind(), base.flags()))
				.collect::<Vec<_>>(),
			[
				(
					c"mp_timelimit",
					CommandBaseKind::Variable,
					CommandFlags::NOTIFY
				),
				(c"status", CommandBaseKind::Command, CommandFlags::NONE),
				(
					c"sv_gravity",
					CommandBaseKind::Variable,
					CommandFlags::REPLICATED | CommandFlags::NOTIFY
				),
			]
		);
		assert_eq!(listed.next_back().unwrap().as_ptr(), gravity.cast());
		assert_eq!(
			cvar.vars()
				.map(|var| (var.as_ptr(), var.is_default()))
				.collect::<Vec<_>>(),
			[(timelimit, false), (gravity, true)]
		);
	}

	#[test]
	fn variables_report_the_flags_and_default_of_their_parent() {
		// A bit iconvar.h leaves unassigned.
		let unknown = 1 << 27;
		let flags = CommandFlags::from_bits_retain(unknown)
			| CommandFlags::REPLICATED
			| CommandFlags::CHEAT;
		let parent = mock_var(c"sv_cheats", c"0", c"1", flags, null_mut());

		// Another module's variable of the same name, which the engine pointed
		// at the first.
		let child = mock_var(c"sv_cheats", c"1", c"1", CommandFlags::NONE, null_mut());

		unsafe { (*child).m_pParent = parent };

		let parent = unsafe { ConVar::from_raw(NonNull::new(parent).unwrap()) };
		let child = unsafe { ConVar::from_raw(NonNull::new(child).unwrap()) };

		for var in [parent, child] {
			assert_eq!(var.flags(), flags);
			assert_eq!(var.flags().bits() & unknown, unknown);
			assert!(
				var.flags()
					.contains(CommandFlags::REPLICATED | CommandFlags::CHEAT)
			);
			assert!(!var.flags().contains(CommandFlags::SERVER_CANNOT_QUERY));
			assert_eq!(var.default_string().as_c_str(), c"0");
			assert!(!var.is_default());
		}

		let string = unsafe { &raw mut (*parent.as_ptr()).m_pszString };

		unsafe { string.write(c"0".as_ptr().cast_mut()) };
		assert!(child.is_default());

		// Compared byte for byte, not by number.
		unsafe { string.write(c"0.0".as_ptr().cast_mut()) };
		assert!(!child.is_default());

		// A missing string reads as empty, as `GetString` returns it.
		unsafe { string.write(null_mut()) };
		assert_eq!(child.string().as_c_str(), c"");
		assert!(!child.is_default());
		unsafe { (&raw mut (*parent.as_ptr()).m_pszDefaultValue).write(c"".as_ptr()) };
		assert!(child.is_default());
	}
}
