//! The variable object the engine sees as a `ConVar`.
//!
//! The engine's calls to set a variable's value run [`HOOKS`], which apply
//! tier1's setter semantics here: bounds, change detection,
//! `FCVAR_NEVER_AS_STRING`, and the change callbacks, which need the
//! [`Server`] a variable's binding gives. The variable's C++ layout, vtables,
//! and the parsing and formatting of values, which follow the C library, are
//! [`ConVarObject`]'s.

use super::CommandFlags;

use super::error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, UnregisterCommandError,
	validate_name,
};

use super::object::{register_base, unregister_base};
use super::registrar::{CommandRegistrar, UnlinksBeforeUnload};
use super::route::drop_payload;
use crate::interfaces::Cvar;
use crate::server::{Server, ServerBinding};

use sdk_raw::commands::{
	ConVarHooks, ConVarObject, format_float, format_int, is_registered, parse_float,
};

use std::cell::Cell;
use std::ffi::{CStr, CString, c_int};
use std::mem::offset_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::ptr::NonNull;

const _: () = assert!(offset_of!(ConsoleVariable, object) == 0);

/// What the engine's calls that set the value of every [`ConsoleVariable`]
/// run. Only [`ConsoleVariable::new`] creates objects with these hooks.
static HOOKS: ConVarHooks = ConVarHooks {
	set_string: engine_set_string,
	set_float: engine_set_float,
	set_int: engine_set_int,
	clamp: engine_clamp,
	change_string: engine_change_string,
};

/// A console variable implemented in Rust, laid out so the engine sees a
/// `ConVar`.
///
/// It behaves as one the game declares: the console sets it, clamped to its
/// bounds, and its changes run the engine's change callbacks, which announce
/// a variable marked `FCVAR_NOTIFY` to players. As for a
/// [`ConsoleCommand`](super::ConsoleCommand), the engine keeps a registered
/// variable's address and writes its C++ fields, so a variable is registered
/// pinned and `'static`, and is usually a `static`:
///
/// ```
/// use source_sdk_2013::commands::ConsoleVariable;
///
/// static ROUNDS: ConsoleVariable = ConsoleVariable::new(c"sb_rounds", c"3")
///     .help(c"How many rounds are played.")
///     .min(1.0);
/// ```
///
/// Its value is read and changed on the server's main thread, as a
/// [`Server`] proves. It holds its default until changed, registered or not.
///
/// A variable is `Sync`: everything the engine or Rust writes after
/// construction is only touched on the server's main thread.
#[doc(alias = "ConVar")]
#[repr(C)]
pub struct ConsoleVariable {
	/// The engine-visible `ConVar`, created with [`HOOKS`], which also holds
	/// the value and the default. It makes the variable neither `Send`, `Sync`,
	/// nor `Unpin`.
	object: ConVarObject,

	name: &'static CStr,
	help: &'static CStr,
	flags: CommandFlags,
	min: Option<f32>,
	max: Option<f32>,

	/// The server the variable was last registered with, for calls from the
	/// engine.
	binding: Cell<Option<ServerBinding>>,
}

impl ConsoleVariable {
	/// Creates an unregistered variable holding `default`, which is also what
	/// the console reverts it to.
	///
	/// # Panics
	///
	/// If [`validate_name`] rejects `name`. For a `static`, this is a
	/// compile-time error.
	pub const fn new(name: &'static CStr, default: &'static CStr) -> Self {
		match validate_name(name) {
			Ok(()) => {}
			Err(InvalidCommandName::Empty) => panic!("console variable names cannot be empty"),

			Err(InvalidCommandName::TooLong) => {
				panic!("console variable names cannot be longer than 63 bytes")
			}

			Err(InvalidCommandName::InvalidByte { .. }) => {
				panic!("console variable names may only contain ASCII letters, digits, and `_`")
			}

			Err(InvalidCommandName::Reserved) => {
				panic!("the engine reserves this name for its own client commands")
			}
		}

		Self {
			object: ConVarObject::new(default, &HOOKS),
			name,
			help: c"",
			flags: CommandFlags::NONE,
			min: None,
			max: None,
			binding: Cell::new(None),
		}
	}

	/// A pointer to the whole object, which the engine passes back to every
	/// slot of the primary vtable.
	fn as_base(&self) -> NonNull<sys::ConCommandBase> {
		ConVarObject::as_base(self.object_ptr())
	}

	/// The value as a boolean: whether its integer is not 0.
	#[doc(alias = "GetBool")]
	pub fn bool(&self, server: Server<'_>) -> bool {
		self.int(server) != 0
	}

	/// The engine's registry, for the change callbacks every change of a
	/// registered variable runs.
	fn change_callbacks<'s>(&self, server: Option<Server<'s>>) -> Option<Cvar<'s>> {
		// SAFETY: The variable is live.
		if !unsafe { is_registered(self.as_base()) } {
			return None;
		}

		server?.cvar().ok()
	}

	/// Installs a string, then runs the change callbacks if it differs from
	/// the previous one, as tier1's `ConVar::ChangeStringValue` does.
	fn change_string(&self, callbacks: Option<Cvar<'_>>, value: CString, old_float: f32) {
		// The old string stays alive until the callbacks have seen it, even if
		// they change the variable again.
		let replaced = self.object.replace_string(value);

		if let (true, Some(cvar)) = (replaced.changed(), callbacks) {
			let object = self.object_ptr();

			// SAFETY: The variable is registered, so prepared, and `object` covers
			// it whole. Callbacks only run with a server, which confines this to
			// the main thread, where the engine keeps the module that installed
			// one loaded.
			unsafe { ConVarObject::call_change_callback(object, replaced.old(), old_float) };

			// SAFETY: The variable is registered, and its string just changed.
			unsafe {
				cvar.call_global_change_callbacks(
					ConVarObject::as_var(object),
					replaced.old(),
					old_float,
				)
			};
		}
	}

	/// Clamps a value to the bounds, as tier1's `ConVar::ClampValue` does,
	/// returning whether it changed. Competitive bounds apply only to clients.
	fn clamp(&self, value: &mut f32) -> bool {
		if let Some(min) = self.min
			&& *value < min
		{
			*value = min;
			return true;
		}

		if let Some(max) = self.max
			&& *value > max
		{
			*value = max;
			return true;
		}

		false
	}

	/// The value the variable reverts to.
	#[doc(alias = "GetDefault")]
	pub const fn default_value(&self) -> &'static CStr {
		self.object.default()
	}

	/// Sets the flags the engine sees once the variable is registered, none by
	/// default. Unlike a command's, they keep [`CommandFlags::GAME_DLL`].
	pub const fn flags(mut self, flags: CommandFlags) -> Self {
		self.flags = flags;
		self
	}

	/// The value as a float.
	#[doc(alias = "GetFloat")]
	pub fn float(&self, _server: Server<'_>) -> f32 {
		self.object.float()
	}

	/// Sets the text `help <name>` shows.
	pub const fn help(mut self, help: &'static CStr) -> Self {
		self.help = help;
		self
	}

	/// The value as an integer, which follows the float unless it was set from
	/// an integer.
	#[doc(alias = "GetInt")]
	pub fn int(&self, _server: Server<'_>) -> c_int {
		self.object.int()
	}

	/// Clamps values above `max` to it.
	pub const fn max(mut self, max: f32) -> Self {
		self.max = Some(max);
		self
	}

	/// Clamps values below `min` to it.
	pub const fn min(mut self, min: f32) -> Self {
		self.min = Some(min);
		self
	}

	/// The name the variable is registered under.
	#[doc(alias = "GetName")]
	pub const fn name(&self) -> &'static CStr {
		self.name
	}

	/// The object, through a pointer that covers the whole variable, which the
	/// engine is handed and the hooks cast back.
	fn object_ptr(&self) -> NonNull<ConVarObject> {
		NonNull::from(self).cast()
	}

	/// Fills in the engine-visible fields before linking.
	///
	/// # Safety
	///
	/// The variable must be pinned, `'static`, and not registered.
	unsafe fn prepare(&self, binding: ServerBinding, dll_identifier: sys::CVarDLLIdentifier_t) {
		// SAFETY: The caller guarantees the variable is not registered and stays
		// where it is, `object_ptr` covers the whole variable, and it is
		// only linked through a registrar, on the main thread, and unlinked
		// before its code is unloaded.
		unsafe {
			ConVarObject::prepare(
				self.object_ptr(),
				self.name,
				self.help,
				self.flags.bits(),
				self.min,
				self.max,
				dll_identifier,
			)
		};

		self.binding.set(Some(binding));
	}

	/// Registers the variable with the engine through a registrar whose host
	/// unlinks it before the plugin is unloaded, such as Metamod:Source's.
	///
	/// Changes the engine makes later run its change callbacks through the
	/// server `binding` gives.
	pub fn register(
		self: Pin<&'static Self>,
		server: Server<'_>,
		binding: ServerBinding,
		registrar: &impl UnlinksBeforeUnload,
	) -> Result<(), RegisterCommandError> {
		// SAFETY: The registrar's host unlinks the variable before the plugin's
		// code is unloaded.
		unsafe { self.register_unmanaged(server, binding, registrar) }
	}

	/// Registers the variable through any registrar.
	///
	/// Before registering, this checks that the variable is not registered
	/// already and that no other command or variable uses its name, since the
	/// engine would make a variable of a taken name defer to the existing one.
	/// Afterwards, it checks that the engine lists the variable under its name.
	///
	/// # Safety
	///
	/// The variable must be unregistered before the module containing this
	/// crate's code is unloaded.
	pub unsafe fn register_unmanaged(
		self: Pin<&'static Self>,
		server: Server<'_>,
		binding: ServerBinding,
		registrar: &impl CommandRegistrar,
	) -> Result<(), RegisterCommandError> {
		let variable = self.get_ref();

		// SAFETY: `register_base` only prepares the variable once it found it
		// unregistered, and the variable is pinned and `'static`.
		let prepare = |dll_identifier| unsafe { variable.prepare(binding, dll_identifier) };

		// SAFETY: The variable is pinned and `'static`, `prepare` fills in every
		// field the engine reads, and the caller unregisters it in time.
		unsafe {
			register_base(
				variable.as_base(),
				variable.name,
				CommandBaseKind::Variable,
				server,
				registrar,
				prepare,
			)
		}
	}

	/// Sets the value back to the default.
	#[doc(alias = "Revert")]
	pub fn revert(&self, server: Server<'_>) {
		self.set_string(server, self.default_value());
	}

	/// Sets the value from a float, which the string then shows with six
	/// decimals, as the console does for `SetValue(float)`. Nothing happens if
	/// the float value is unchanged.
	#[doc(alias = "SetValue")]
	pub fn set_float(&self, server: Server<'_>, value: f32) {
		self.set_float_value(self.change_callbacks(Some(server)), value, false);
	}

	/// As tier1's `ConVar::InternalSetFloatValue2`.
	fn set_float_value(&self, callbacks: Option<Cvar<'_>>, value: f32, force: bool) {
		if value == self.object.float() && !force {
			return;
		}

		let mut value = value;

		self.clamp(&mut value);
		self.set_values(callbacks, value, value as c_int, || format_float(value));
	}

	/// Sets the value from an integer, as the console does for
	/// `SetValue(int)`. Nothing happens if the integer value is unchanged.
	#[doc(alias = "SetValue")]
	pub fn set_int(&self, server: Server<'_>, value: c_int) {
		self.set_int_value(self.change_callbacks(Some(server)), value);
	}

	/// As tier1's `ConVar::InternalSetIntValue`.
	fn set_int_value(&self, callbacks: Option<Cvar<'_>>, value: c_int) {
		if value == self.object.int() {
			return;
		}

		let mut float = value as f32;
		let int = if self.clamp(&mut float) {
			float as c_int
		} else {
			value
		};

		self.set_values(callbacks, float, int, || format_int(int));
	}

	/// Sets the value from a string, as the console does. The float is parsed
	/// as C's `atof` parses decimal numbers, and the integer follows it.
	#[doc(alias = "SetValue")]
	pub fn set_string(&self, server: Server<'_>, value: &CStr) {
		self.set_string_value(self.change_callbacks(Some(server)), Some(value));
	}

	/// As tier1's `ConVar::InternalSetValue`. `None` clears the string.
	fn set_string_value(&self, callbacks: Option<Cvar<'_>>, value: Option<&CStr>) {
		let mut float = value.map_or(0.0, |value| parse_float(value.to_bytes()) as f32);
		let clamped = self.clamp(&mut float);

		// A clamped value is shown as the float it became.
		self.set_values(callbacks, float, float as c_int, || match value {
			_ if clamped => format_float(float),
			Some(value) => value.to_owned(),
			None => CString::default(),
		});
	}

	/// Stores the numbers, then the string `text` gives, unless the variable
	/// never keeps one, and runs the change callbacks if it changed.
	fn set_values(
		&self,
		callbacks: Option<Cvar<'_>>,
		float: f32,
		int: c_int,
		text: impl FnOnce() -> CString,
	) {
		let old_float = self.object.float();

		self.object.set_numbers(float, int);

		if !CommandFlags::from_bits_retain(self.object.flags())
			.contains(CommandFlags::NEVER_AS_STRING)
		{
			self.change_string(callbacks, text(), old_float);
		}
	}

	/// The value as a string.
	#[doc(alias = "GetString")]
	pub fn string(&self, _server: Server<'_>) -> CString {
		self.object.string()
	}

	/// Unregisters the variable, which the console then no longer finds.
	pub fn unregister(
		&self,
		server: Server<'_>,
		registrar: &impl CommandRegistrar,
	) -> Result<(), UnregisterCommandError> {
		// SAFETY: Only `register` makes the engine list a variable, and it takes
		// the variable pinned and `'static`.
		unsafe {
			unregister_base(
				self.as_base(),
				self.name,
				CommandBaseKind::Variable,
				server,
				registrar,
			)
		}
	}
}

impl std::fmt::Debug for ConsoleVariable {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ConsoleVariable")
			.field("name", &self.name)
			.field("default", &self.default_value())
			.field("help", &self.help)
			.field("flags", &self.flags)
			.field("min", &self.min)
			.field("max", &self.max)
			.finish_non_exhaustive()
	}
}

// SAFETY: The interior-mutable parts (the C++ fields and the `Cell`s) are only
// accessed by the engine, which calls variables on its main thread, and by
// methods taking a `Server`, whose contract confines them to that thread.
// Other threads can only reach the immutable fields.
unsafe impl Sync for ConsoleVariable {}

/// The engine changed the string, as tier1's `ConVar::ChangeStringValue`.
///
/// # Safety
///
/// As for [`from_engine`].
unsafe fn engine_change_string(variable: NonNull<ConVarObject>, value: &CStr, old_float: f32) {
	// SAFETY: The caller upholds the contract.
	unsafe {
		from_engine(variable, |variable, callbacks| {
			variable.change_string(callbacks, value.to_owned(), old_float)
		})
	};
}

/// The engine clamps a value to the bounds, as tier1's `ConVar::ClampValue`.
///
/// # Safety
///
/// As for [`from_engine`].
unsafe fn engine_clamp(variable: NonNull<ConVarObject>, value: &mut f32) -> bool {
	// SAFETY: The caller upholds the contract.
	unsafe { from_engine(variable, |variable, _| variable.clamp(value)) }.unwrap_or(false)
}

/// The engine sets the value from a float, as tier1's
/// `ConVar::InternalSetFloatValue2`.
///
/// # Safety
///
/// As for [`from_engine`].
unsafe fn engine_set_float(variable: NonNull<ConVarObject>, value: f32, force: bool) {
	// SAFETY: The caller upholds the contract.
	unsafe {
		from_engine(variable, |variable, callbacks| {
			variable.set_float_value(callbacks, value, force)
		})
	};
}

/// The engine sets the value from an integer, as tier1's
/// `ConVar::InternalSetIntValue`.
///
/// # Safety
///
/// As for [`from_engine`].
unsafe fn engine_set_int(variable: NonNull<ConVarObject>, value: c_int) {
	// SAFETY: The caller upholds the contract.
	unsafe {
		from_engine(variable, |variable, callbacks| {
			variable.set_int_value(callbacks, value)
		})
	};
}

/// The engine sets the value from a string, as tier1's
/// `ConVar::InternalSetValue`.
///
/// # Safety
///
/// As for [`from_engine`].
unsafe fn engine_set_string(variable: NonNull<ConVarObject>, value: Option<&CStr>) {
	// SAFETY: The caller upholds the contract.
	unsafe {
		from_engine(variable, |variable, callbacks| {
			variable.set_string_value(callbacks, value)
		})
	};
}

/// Runs `f` for the variable whose slot the engine called, with the registry
/// if its change callbacks should run. A panic is caught, since it must not
/// unwind into the engine, and `None` returned.
///
/// # Safety
///
/// As [`ConVarHooks`] promises: `variable` is the pointer the engine was
/// given to a variable created with [`HOOKS`], and this runs inside the
/// engine's call to it, on the main thread.
unsafe fn from_engine<R>(
	variable: NonNull<ConVarObject>,
	f: impl FnOnce(&ConsoleVariable, Option<Cvar<'_>>) -> R,
) -> Option<R> {
	let outcome = catch_unwind(AssertUnwindSafe(|| {
		// SAFETY: Only `ConsoleVariable::new` creates objects with `HOOKS`, each
		// the start of a variable, and the engine passes back the pointer it was
		// given, to the whole variable, which is `'static` and never mutably
		// borrowed.
		let variable = unsafe { variable.cast::<ConsoleVariable>().as_ref() };
		let scope = ();

		// SAFETY: This runs inside the engine's call to the variable, on the
		// main thread, which `scope` does not outlive.
		let server = variable
			.binding
			.get()
			.map(|binding| unsafe { binding.server(&scope) });

		f(variable, variable.change_callbacks(server))
	}));

	outcome.map_err(drop_payload).ok()
}
