//! `ICvar`, the registry of console variables and commands.

#[cfg(test)]
#[path = "../../tests/interfaces/cvar.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::commands::{CommandBaseKind, CommandFlags, drop_payload};
use crate::server::{Server, ServerBinding};
use sdk_raw::util::cstr::{borrow_cstr, copy_cstr};
use sdk_raw::vcall;
use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int};
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

/// The most entries [`Cvar::command_bases`] lists.
const MAX_LISTED: usize = 1 << 16;

thread_local! {
	/// The watch of this library's, if one runs, kept on the server's main
	/// thread, where it started. Other threads find none, so the engine's
	/// calls on them pass nothing on. It needs no destructor, so no thread,
	/// the engine's included, gets one registered from this library.
	static WATCHING: Cell<Option<Watching>> = const { Cell::new(None) };
}

interface! {
	/// The registry of console variables and commands (`ICvar`).
	#[doc(alias("ICvar"))]
	pub struct Cvar(sys::ICvar) = Engine sdk_raw::interfaces::cvar::VERSION;
}

/// Runs for each change of a console variable's value while a [`ConVarWatch`]
/// exists, inside the change, on the server's main thread. See
/// [`Cvar::watch_changes`] for which changes it sees, and what it may do.
pub type ConVarChangeFn = for<'s> fn(server: Server<'s>, change: ConVarChange<'s>);

/// A console variable or command the registry lists (`ConCommandBase`), from
/// [`Cvar::command_bases`].
///
/// The handle stays usable for all of `'s`, even if the entry is unregistered,
/// since condition 5 of [`Server::new`] keeps it allocated.
///
/// [`Server::new`]: crate::Server::new
#[doc(alias("ConCommandBase"))]
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
	#[doc(alias("GetFlags", "IsFlagSet"))]
	pub fn flags(self) -> CommandFlags {
		if let Some(var) = self.as_var() {
			return var.flags();
		}

		// SAFETY: As for `name`.
		CommandFlags::from_bits_retain(unsafe { (&raw const (*self.as_ptr()).m_nFlags).read() })
	}

	/// Whether this is a command or a variable, as its `IsCommand` reported
	/// when the registry was listed.
	#[doc(alias("IsCommand"))]
	pub const fn kind(self) -> CommandBaseKind {
		self.kind
	}

	/// The name it is registered under.
	#[doc(alias("GetName"))]
	pub fn name(self) -> &'s CStr {
		// SAFETY: The registry listed the entry, so condition 5 of `Server::new`
		// keeps it allocated, with its name unchanged, for `'s`. The field is
		// read without forming a reference.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).m_pszName).read()) }.unwrap_or_default()
	}

	/// Its help text, as `help` prints it, or empty: for a variable, that of
	/// the variable that holds its value, as [`ConVar::help_text`] reads it.
	#[doc(alias("GetHelpText"))]
	pub fn help_text(self) -> &'s CStr {
		if let Some(var) = self.as_var() {
			return var.help_text();
		}

		// SAFETY: As for `name`. The engine keeps the text with the entry.
		unsafe { borrow_cstr((&raw const (*self.as_ptr()).m_pszHelpString).read()) }
			.unwrap_or_default()
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
	#[doc(alias("GetDefault"))]
	pub fn default_string(self) -> CString {
		// SAFETY: As for `string`.
		unsafe { copy_cstr((&raw const (*self.parent()).m_pszDefaultValue).read()) }
			.unwrap_or_default()
	}

	/// The least and the most value the variable takes, each `None` where it
	/// has no bound. Setting it clamps a value to these.
	#[doc(alias("GetMin", "GetMax"))]
	pub fn bounds(self) -> (Option<f32>, Option<f32>) {
		let parent = self.parent();

		// SAFETY: As for `name`.
		unsafe {
			(
				(&raw const (*parent).m_bHasMin)
					.read()
					.then(|| (&raw const (*parent).m_fMinVal).read()),
				(&raw const (*parent).m_bHasMax)
					.read()
					.then(|| (&raw const (*parent).m_fMaxVal).read()),
			)
		}
	}

	/// The help text of the variable that holds the value, as `help` prints it,
	/// or empty.
	#[doc(alias("GetHelpText"))]
	pub fn help_text(self) -> &'s CStr {
		// SAFETY: As for `name`. The engine keeps the text with the variable.
		unsafe { borrow_cstr((&raw const (*self.parent())._base.m_pszHelpString).read()) }
			.unwrap_or_default()
	}

	/// The flags of the variable that holds the value, which keep every bit
	/// the engine stores.
	#[doc(alias("GetFlags", "IsFlagSet"))]
	pub fn flags(self) -> CommandFlags {
		// SAFETY: As for `name`.
		CommandFlags::from_bits_retain(unsafe {
			(&raw const (*self.parent())._base.m_nFlags).read()
		})
	}

	/// The current value as a float.
	#[doc(alias("GetFloat"))]
	pub fn float(self) -> f32 {
		// SAFETY: As for `name`.
		unsafe { (&raw const (*self.parent()).m_fValue).read() }
	}

	/// The current value as an integer.
	#[doc(alias("GetInt"))]
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
	#[doc(alias("GetName"))]
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

	/// Runs `f` with the variable's `FCVAR_NOTIFY` flag cleared, then sets the
	/// flag back if it was set, even if `f` panics. The engine announces a
	/// change to players and the server log only for a variable with the flag,
	/// so what `f` changes of this variable, through any handle to it, is not
	/// announced, as for [`Self::set_string_quietly`]. Flags `f` adds are kept.
	pub fn quietly<R>(self, f: impl FnOnce() -> R) -> R {
		/// Sets the flag back as it drops.
		struct Announce {
			flags: *mut c_int,
			announced: c_int,
		}

		impl Drop for Announce {
			fn drop(&mut self) {
				// SAFETY: As for `quietly`'s reads.
				unsafe { self.flags.write(self.flags.read() | self.announced) };
			}
		}

		let notify = CommandFlags::NOTIFY.bits();

		// SAFETY: As for `name`. The flags are read and written without forming
		// references, since C++ writes them too.
		let flags = unsafe { &raw mut (*self.parent())._base.m_nFlags };

		// SAFETY: As above.
		let announced = unsafe { flags.read() } & notify;

		// SAFETY: As above.
		unsafe { flags.write(flags.read() & !notify) };

		let _announce = Announce { flags, announced };

		f()
	}

	/// Sets the value from a float, which the string then shows with six
	/// decimals. Nothing happens if the float value is unchanged.
	#[doc(alias("SetValue"))]
	pub fn set_float(self, value: f32) {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.interface() => IConVar_SetValue1(value)) };
	}

	/// Sets the value from an integer. Nothing happens if the integer value is
	/// unchanged.
	#[doc(alias("SetValue"))]
	pub fn set_int(self, value: c_int) {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.interface() => IConVar_SetValue2(value)) };
	}

	/// Sets the value from a string, as the console does.
	#[doc(alias("SetValue"))]
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
	/// the engine's callback checks it then, as [`Self::quietly`] does. The
	/// engine also recalculates the server's tags (`sv_tags`) for an announced
	/// change, inferred to be in that callback too, since the engine is not
	/// public (see [`engine_replay`]). So a variable that tags the server, such
	/// as `sv_gravity`, leaves its tag as it was;
	/// [`EngineReplay::recalculate_tags`] catches up.
	///
	/// [`engine_replay`]: crate::interfaces::engine_replay
	/// [`EngineReplay::recalculate_tags`]: crate::interfaces::EngineReplay::recalculate_tags
	pub fn set_string_quietly(self, value: &CStr) {
		self.quietly(|| self.set_string(value));
	}

	/// The current value as a string.
	#[doc(alias("GetString"))]
	pub fn string(self) -> CString {
		// SAFETY: As for `name`. Changing the value reallocates the string, so
		// it is copied immediately.
		unsafe { copy_cstr((&raw const (*self.parent()).m_pszString).read()) }.unwrap_or_default()
	}
}

/// A change of a console variable's value, which [`Cvar::watch_changes`]
/// passes to its callback.
#[derive(Debug, Clone, Copy)]
pub struct ConVarChange<'s> {
	name: &'s CStr,
	var: Option<ConVar<'s>>,
	old_string: &'s CStr,
	old_float: f32,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ConVarChange<'s> {
	/// The name of the variable that changed.
	#[doc(alias("GetName"))]
	pub const fn name(self) -> &'s CStr {
		self.name
	}

	/// The value as a float before the change.
	pub const fn old_float(self) -> f32 {
		self.old_float
	}

	/// The value as a string before the change, or empty if the engine passed
	/// none.
	pub const fn old_string(self) -> &'s CStr {
		self.old_string
	}

	/// The variable that changed, which already holds its new value, or `None`
	/// if the registry does not list it under its name, as for a variable that
	/// a module declared but has not registered yet, or has unregistered.
	pub const fn var(self) -> Option<ConVar<'s>> {
		self.var
	}
}

/// A watch of every console variable's changes, which [`Cvar::watch_changes`]
/// starts, until it is dropped.
///
/// Drop it, or [stop](Self::stop) it, before the plugin's library unloads:
/// until then the engine keeps calling the function the watch installed, and
/// would call into the unloaded library at the next change. Nothing stops it
/// for the plugin, neither this crate nor Metamod:Source, which only removes
/// the hooks it installed itself. A watch that is never dropped, such as one
/// leaked or kept in a `static`, stays installed for as long as the process
/// runs.
///
/// Under Metamod:Source, stop it in the plugin's `Unload` whatever `Unload`
/// returns. A forced unload, such as `meta force_unload`, unloads the library
/// even when `Unload` refuses, without calling the plugin again, so a plugin
/// that refuses runs on without the watch, and may start another. A `Load`
/// that started a watch and then refuses must stop it before returning, since
/// Metamod unloads the library after a refused `Load` without calling
/// `Unload`.
///
/// Do not unload the plugin from inside a variable's change either, such as
/// from another module's change callback: a change that is already calling
/// its callbacks may still call the function once after the watch stops, as
/// [`Self::stop`] describes, and would find it gone.
///
/// The watch stays on the server's main thread, where it started.
#[must_use = "dropping the watch stops it"]
#[derive(Debug)]
pub struct ConVarWatch {
	cvar: NonNull<sys::ICvar>,

	/// The function installed, passed back unchanged to remove it.
	installed: sys::FnChangeCallback_t,
	_not_thread_safe: NotThreadSafe,
}

impl ConVarWatch {
	/// Stops the watch, as dropping it does: the engine no longer calls the
	/// function it installed, except perhaps once more during a change whose
	/// callbacks it is calling already. `CCvar` counts them before calling the
	/// first, and removing the function leaves it in the list's old last slot if
	/// it was installed last, so the change still calls it there, though that
	/// call no longer reaches this watch's callback.
	#[doc(alias("RemoveGlobalChangeCallback"))]
	pub fn stop(self) {
		drop(self);
	}
}

impl Drop for ConVarWatch {
	fn drop(&mut self) {
		// SAFETY: The registry is the engine's, which outlives every plugin, and
		// the watch is dropped on the main thread it started on, since it is
		// neither `Send` nor `Sync`. The engine removes the function it was
		// given.
		unsafe { vcall!(self.cvar.as_ptr() => ICvar_RemoveGlobalChangeCallback(self.installed)) };
		WATCHING.set(None);
	}
}

/// Why console variables' changes could not be watched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConVarWatchError {
	/// A watch of this library's already exists.
	#[error("console variable changes are already being watched")]
	AlreadyWatching,
}

impl<'s> Cvar<'s> {
	/// Reserves an identifier, which `ICvar::UnregisterConCommands` uses to
	/// unlink every command a module registered.
	#[doc(alias("AllocateDLLIdentifier"))]
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
	#[doc(alias("CallGlobalChangeCallbacks"))]
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
	#[doc(alias("GetCommands", "GetNext"))]
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
	#[doc(alias("ConsolePrintf"))]
	pub(crate) fn console_printf(self, message: &CStr) {
		// SAFETY: As for `find_var`. The message is passed as an argument of a
		// constant format, so it is never interpreted as one.
		unsafe { vcall!(self.as_ptr() => ICvar_ConsolePrintf(c"%s".as_ptr(), message.as_ptr())) };
	}

	/// Finds a console variable or command by name, ignoring case.
	#[doc(alias("FindCommandBase"))]
	pub(crate) fn find_command_base(self, name: &CStr) -> Option<NonNull<sys::ConCommandBase>> {
		// SAFETY: As for `find_var`.
		NonNull::new(unsafe { vcall!(self.as_ptr() => ICvar_FindCommandBase(name.as_ptr())) })
	}

	/// Finds a console variable by name. Commands are not variables.
	///
	/// Returns `None` if no variable is registered under the name.
	#[doc(alias("FindVar"))]
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
	#[doc(alias("RegisterConCommand"))]
	pub(crate) unsafe fn register_con_command(self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `find_var`, and the caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => ICvar_RegisterConCommand(command.as_ptr())) };
	}

	/// Unlinks a command from the registry.
	///
	/// # Safety
	///
	/// `command` must be a live `ConCommandBase`.
	#[doc(alias("UnregisterConCommand"))]
	pub(crate) unsafe fn unregister_con_command(self, command: NonNull<sys::ConCommandBase>) {
		// SAFETY: As for `find_var`, and the caller upholds the contract.
		unsafe { vcall!(self.as_ptr() => ICvar_UnregisterConCommand(command.as_ptr())) };
	}

	/// Every console variable the registry lists, in its order, leaving out
	/// commands. The registry is listed in full first, as
	/// [`Self::command_bases`] describes.
	#[doc(alias("GetCommands"))]
	pub fn vars(self) -> impl DoubleEndedIterator<Item = ConVar<'s>> + FusedIterator {
		self.command_bases().filter_map(CommandBase::as_var)
	}

	/// Calls `callback` for each change of a console variable's value from now
	/// until the returned watch stops: of the engine's, the game's and other
	/// plugins' variables alike, and of this crate's [`ConsoleVariable`]s. Each
	/// call gets a server scoped to it, from `binding`, and the change: the
	/// variable, and its value before.
	///
	/// This installs one function of this library with
	/// `ICvar::InstallGlobalChangeCallback`, which the engine calls after each
	/// change of a variable's string, after the variable's own change callback.
	/// Setting a variable to the string it holds changes nothing, and a variable
	/// marked [`CommandFlags::NEVER_AS_STRING`] never changes its string, so
	/// neither is seen.
	///
	/// One watch per library may exist at a time: another fails with
	/// [`ConVarWatchError::AlreadyWatching`] until it stops.
	///
	/// # Unloading
	///
	/// The engine calls the function until the watch stops, even while the
	/// plugin is paused, so stop it before the plugin's library unloads, as
	/// [`ConVarWatch`] describes. Under Metamod:Source, that means in the
	/// plugin's `Unload` whatever `Unload` returns, since a forced unload goes
	/// on after a refusal without calling the plugin again, and before a `Load`
	/// that refuses returns, since no `Unload` follows it. Do not unload the
	/// plugin from inside a variable's change either.
	///
	/// # Threads
	///
	/// The engine calls the function on the thread that changed the variable,
	/// inside that change. A server changes its variables on its main thread,
	/// where the console, rcon, configs, the game and plugins all set them. A
	/// change made on another thread, such as one a plugin started, is skipped,
	/// and not passed on later either, since a [`Server`] only exists on the main
	/// thread, where the watch started.
	///
	/// # Inside the callback
	///
	/// The callback runs inside the engine's change of the variable, which
	/// already holds its new value. It may read and set variables, but each
	/// change it makes runs it again, nested inside this call, before that change
	/// returns. Setting the variable that just changed starts another change of
	/// it, nested in this one, so do so only when it does not already hold the
	/// value wanted, or the changes repeat until the stack overflows.
	///
	/// Do not stop the watch from inside its callback: the engine counts its
	/// callbacks before calling the first, so removing one while it calls them
	/// can make it skip the one after it and call the last one twice. A panic is
	/// caught, and the change goes on as though the callback returned.
	///
	/// [`ConsoleVariable`]: crate::commands::ConsoleVariable
	#[doc(alias("InstallGlobalChangeCallback"))]
	pub fn watch_changes(
		self,
		binding: ServerBinding,
		callback: ConVarChangeFn,
	) -> Result<ConVarWatch, ConVarWatchError> {
		if WATCHING.get().is_some() {
			return Err(ConVarWatchError::AlreadyWatching);
		}

		let installed: sys::FnChangeCallback_t = Some(changed);

		WATCHING.set(Some(Watching {
			binding,
			callback,
			cvar: self.raw,
		}));

		// SAFETY: As for `find_var`. The engine may call the function until the
		// watch removes it, which `ConVarWatch` requires of the plugin before this
		// library unloads.
		unsafe { vcall!(self.as_ptr() => ICvar_InstallGlobalChangeCallback(installed)) };

		Ok(ConVarWatch {
			cvar: self.raw,
			installed,
			_not_thread_safe: PhantomData,
		})
	}
}

/// What the function a watch installed passes changes to.
#[derive(Clone, Copy)]
struct Watching {
	binding: ServerBinding,
	callback: ConVarChangeFn,
	cvar: NonNull<sys::ICvar>,
}

/// The function [`Cvar::watch_changes`] installs, which the engine calls
/// after each change of a variable's string, with the variable's `IConVar`,
/// and the string and float it held.
unsafe extern "C" fn changed(var: *mut sys::IConVar, old_string: *const c_char, old_float: f32) {
	// Only the main thread, where the watch started, finds it.
	let (Ok(Some(watching)), Some(var)) = (WATCHING.try_with(Cell::get), NonNull::new(var)) else {
		return;
	};

	catch_unwind(AssertUnwindSafe(|| {
		let scope = ();

		// SAFETY: The engine calls this inside its change of a variable, on the
		// main thread, as finding the watch shows, and `scope` ends with the
		// call.
		let server = unsafe { watching.binding.server(&scope) };

		// SAFETY: The registry is the engine's, which stays alive for the call,
		// and the server confines it to the main thread.
		let cvar = unsafe { Cvar::from_raw(watching.cvar) };

		// SAFETY: The engine passes the variable that changed, which stays alive
		// for the call, and the string it held, or null.
		unsafe { notify(server, cvar, var, old_string, old_float, watching.callback) };
	}))
	.map_err(drop_payload)
	.ok();
}

/// Passes a change to `callback`, with the variable that changed if the
/// registry lists it.
///
/// # Safety
///
/// `var` must be the `IConVar` of a live variable whose value just changed
/// from `old_string`, a string or null, and both must stay alive for `'s`.
unsafe fn notify<'s>(
	server: Server<'s>,
	cvar: Cvar<'s>,
	var: NonNull<sys::IConVar>,
	old_string: *const c_char,
	old_float: f32,
	callback: ConVarChangeFn,
) {
	// SAFETY: As the caller promises. `GetName` is called through the
	// interface's own vtable, as C++ calls it, so whatever class implements it
	// finds itself from the interface, on either ABI.
	let name =
		unsafe { borrow_cstr(vcall!(var.as_ptr() => IConVar_GetName())) }.unwrap_or_default();

	// The variable the registry lists under the name is the one that changed
	// only if its `IConVar` is the one the engine passed, which C++'s
	// conversion of the `ConVar *` it changed gave it. The registry's pointer
	// is only converted, never the engine's.
	let listed = cvar
		.find_var(name)
		.filter(|listed| listed.interface() == var.as_ptr());

	// An unlisted variable's name is copied, since nothing keeps it alive.
	let unlisted;
	let name = match listed {
		Some(listed) => listed.name(),

		None => {
			unlisted = name.to_owned();
			unlisted.as_c_str()
		}
	};

	callback(
		server,
		ConVarChange {
			name,
			var: listed,

			// SAFETY: As the caller promises.
			old_string: unsafe { borrow_cstr(old_string) }.unwrap_or_default(),
			old_float,
			_not_thread_safe: PhantomData,
		},
	);
}
