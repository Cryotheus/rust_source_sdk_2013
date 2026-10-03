//! The variable object the engine sees as a `ConVar`.
//!
//! A `ConVar` derives from both `ConCommandBase` and `IConVar`, so the engine
//! reaches one through two vtables: the primary one at its start, and
//! `IConVar`'s in the subobject after `ConCommandBase`, through which callers
//! set values. Both tables are preceded by run-time type information, since
//! the engine `dynamic_cast`s every variable it describes in the console.

use super::CommandFlags;

use super::error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, UnregisterCommandError,
	validate_name,
};

use super::object::{base_is_registered, register_base, unregister_base};
use super::registrar::{CommandRegistrar, UnlinksBeforeUnload};
use super::route::drop_payload;
use crate::abi::CppDestructors;
use crate::ffi::{NotThreadSafe, borrow_cstr};
use crate::interfaces::Cvar;
use crate::server::{Server, ServerBinding};
use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::{PhantomData, PhantomPinned};
use std::mem::{MaybeUninit, offset_of, size_of};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::ptr::{self, NonNull};

const _: () = {
	assert!(offset_of!(ConsoleVariable, raw) == 0);
	assert!(offset_of!(sys::ConVar, _base) == 0);
	assert!(INTERFACE_OFFSET == size_of::<sys::ConCommandBase>());
};

// Every slot must line up with the engine's declaration under each ABI.
const _: () = {
	use sys::ConCommandBase__bindgen_vtable as Base;

	assert!(offset_of!(PrimaryVtable, is_command) == offset_of!(Base, ConCommandBase_IsCommand));
	assert!(offset_of!(PrimaryVtable, is_flag_set) == offset_of!(Base, ConCommandBase_IsFlagSet));
	assert!(offset_of!(PrimaryVtable, add_flags) == offset_of!(Base, ConCommandBase_AddFlags));
	assert!(offset_of!(PrimaryVtable, get_name) == offset_of!(Base, ConCommandBase_GetName));
	assert!(
		offset_of!(PrimaryVtable, get_help_text) == offset_of!(Base, ConCommandBase_GetHelpText)
	);
	assert!(
		offset_of!(PrimaryVtable, is_registered) == offset_of!(Base, ConCommandBase_IsRegistered)
	);
	assert!(
		offset_of!(PrimaryVtable, get_dll_identifier)
			== offset_of!(Base, ConCommandBase_GetDLLIdentifier)
	);
	assert!(offset_of!(PrimaryVtable, create_base) == offset_of!(Base, ConCommandBase_CreateBase));
	assert!(offset_of!(PrimaryVtable, init) == offset_of!(Base, ConCommandBase_Init));

	// `ConVar`'s own slots follow the base's.
	assert!(offset_of!(PrimaryVtable, init) + SLOT == size_of::<Base>());

	// The slot counts of `ConVar`'s vtables in TF2's 64-bit binaries.
	#[cfg(target_os = "windows")]
	assert!(size_of::<PrimaryVtable>() == 17 * SLOT);
	#[cfg(not(target_os = "windows"))]
	assert!(size_of::<PrimaryVtable>() == 21 * SLOT);
	assert!(size_of::<sys::IConVar__bindgen_vtable>() == 5 * SLOT);

	// Each table directly follows its type information.
	#[cfg(target_os = "windows")]
	{
		assert!(offset_of!(Vtables, primary) == offset_of!(Vtables, primary_locator) + SLOT);
		assert!(offset_of!(Vtables, secondary) == offset_of!(Vtables, secondary_locator) + SLOT);
	}

	#[cfg(not(target_os = "windows"))]
	{
		assert!(offset_of!(Vtables, primary) == offset_of!(Vtables, primary_type_info) + SLOT);
		assert!(offset_of!(Vtables, secondary) == offset_of!(Vtables, secondary_type_info) + SLOT);
		assert!(offset_of!(Vtables, primary_type_info) == offset_of!(Vtables, primary_top) + SLOT);
		assert!(
			offset_of!(Vtables, secondary_type_info) == offset_of!(Vtables, secondary_top) + SLOT
		);
	}
};

const _: () = assert!(size_of::<VtablePage>() == 4096);

/// Where the `IConVar` subobject sits in a `ConVar`. Callers holding an
/// `IConVar *` point here.
const INTERFACE_OFFSET: usize = offset_of!(sys::ConVar, _base_1);

/// The size of one vtable slot, a pointer.
const SLOT: usize = size_of::<*const ()>();

static VTABLES: VtablePage = VtablePage(UnsafeCell::new(Vtables {
	#[cfg(target_os = "windows")]
	primary_locator: type_information::locator(type_information::PRIMARY),

	#[cfg(not(target_os = "windows"))]
	primary_top: 0,

	#[cfg(not(target_os = "windows"))]
	primary_type_info: type_information::type_info(),

	primary: PrimaryVtable {
		destructors: CppDestructors::new_noop(),
		is_command,
		is_flag_set,
		add_flags,
		get_name,
		get_help_text,
		is_registered,
		get_dll_identifier,
		create_base,
		init,
		#[cfg(not(target_os = "windows"))]
		set_value_string,
		#[cfg(not(target_os = "windows"))]
		set_value_float,
		#[cfg(not(target_os = "windows"))]
		set_value_int,
		internal_set_value: set_value_string,
		internal_set_float_value: set_value_float,
		internal_set_int_value: set_value_int,
		clamp_value,
		change_string_value,
		create_vtbl,
		internal_set_float_value2,
	},

	#[cfg(target_os = "windows")]
	secondary_locator: type_information::locator(type_information::SECONDARY),

	#[cfg(not(target_os = "windows"))]
	secondary_top: -(INTERFACE_OFFSET as isize),

	#[cfg(not(target_os = "windows"))]
	secondary_type_info: type_information::type_info(),

	secondary: sys::IConVar__bindgen_vtable {
		IConVar_SetValue: interface_set_value_string,
		IConVar_SetValue1: interface_set_value_float,
		IConVar_SetValue2: interface_set_value_int,
		IConVar_GetName: interface_get_name,
		IConVar_IsFlagSet: interface_is_flag_set,
	},
}));

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
	/// The engine-visible `ConVar`. C++ writes its list link, registered flag,
	/// flags, and change callback at any time, including while Rust holds a
	/// reference to the variable, so Rust never forms a reference into it.
	raw: UnsafeCell<sys::ConVar>,

	name: &'static CStr,
	help: &'static CStr,
	default: &'static CStr,
	flags: CommandFlags,
	min: Option<f32>,
	max: Option<f32>,

	/// The string `raw` holds, unless it holds the default, which is static.
	string: Cell<Option<CString>>,

	/// The server the variable was last registered with, for calls from the
	/// engine.
	binding: Cell<Option<ServerBinding>>,

	/// Returned from `GetDLLIdentifier`; allocated at registration.
	dll_identifier: Cell<sys::CVarDLLIdentifier_t>,

	_pinned: PhantomPinned,
	_not_thread_safe: NotThreadSafe,
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

		let text = default.to_bytes();

		// SAFETY: Zero is valid for every field of `ConVar`: null pointers, no
		// callback, `false`, and 0. The engine only sees the object once
		// registration has filled in the rest.
		let mut raw: sys::ConVar = unsafe { MaybeUninit::zeroed().assume_init() };

		// As tier1's `ConVar::Create`, which parses the integer separately so
		// large integers keep their bits. The string is only ever replaced,
		// never written through, so it may point to the static default.
		raw.m_pszDefaultValue = default.as_ptr();
		raw.m_pszString = default.as_ptr().cast_mut();
		raw.m_StringLength = text.len() as c_int + 1;
		raw.m_fValue = parse_float(text) as f32;
		raw.m_nValue = parse_int(text);

		Self {
			raw: UnsafeCell::new(raw),
			name,
			help: c"",
			default,
			flags: CommandFlags::NONE,
			min: None,
			max: None,
			string: Cell::new(None),
			binding: Cell::new(None),
			dll_identifier: Cell::new(0),
			_pinned: PhantomPinned,
			_not_thread_safe: PhantomData,
		}
	}

	/// A pointer to the whole object, which the engine passes back to every
	/// slot of the primary vtable.
	fn as_base(&self) -> NonNull<sys::ConCommandBase> {
		NonNull::from(self).cast()
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
		if !unsafe { base_is_registered(self.as_base()) } {
			return None;
		}

		server?.cvar().ok()
	}

	/// Installs a string, then runs the change callbacks if it differs from
	/// the previous one, as tier1's `ConVar::ChangeStringValue` does.
	fn change_string(&self, callbacks: Option<Cvar<'_>>, value: CString, old_float: f32) {
		let raw = self.raw.get();
		let length = c_int::try_from(value.as_bytes_with_nul().len()).unwrap_or(c_int::MAX);

		// SAFETY: Fields are accessed through the cell without forming
		// references, on the main thread.
		let old = unsafe { (&raw const (*raw).m_pszString).read() };

		// SAFETY: The string is never null. It is the default, which is static,
		// or the buffer of the string `self.string` holds, which is kept below
		// until the callbacks have seen it, even if they change the variable
		// again.
		let old = unsafe { CStr::from_ptr(old) };
		let changed = old != value.as_c_str();

		// SAFETY: As above.
		unsafe {
			(&raw mut (*raw).m_pszString).write(value.as_ptr().cast_mut());
			(&raw mut (*raw).m_StringLength).write(length);
		}

		let previous = self.string.replace(Some(value));

		if let (true, Some(cvar)) = (changed, callbacks) {
			// SAFETY: As above. The engine may have installed a callback when
			// another module registered a variable of the same name.
			let callback = unsafe { (&raw const (*raw).m_fnChangeCallback).read() };

			if let Some(callback) = callback {
				// SAFETY: Callbacks receive the `IConVar` subobject, as C++'s
				// conversion from `ConVar *` gives them.
				unsafe {
					callback(
						raw.byte_add(INTERFACE_OFFSET).cast(),
						old.as_ptr(),
						old_float,
					)
				};
			}

			// SAFETY: The variable is registered, and its string just changed.
			unsafe {
				cvar.call_global_change_callbacks(NonNull::from(self).cast(), old, old_float)
			};
		}

		drop(previous);
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

	fn current_flags(&self) -> c_int {
		// SAFETY: As for `current_float`.
		unsafe { (&raw const (*self.raw.get())._base.m_nFlags).read() }
	}

	/// The float the engine sees, on the main thread.
	fn current_float(&self) -> f32 {
		// SAFETY: The field is read through the cell without forming a reference.
		unsafe { (&raw const (*self.raw.get()).m_fValue).read() }
	}

	fn current_int(&self) -> c_int {
		// SAFETY: As for `current_float`.
		unsafe { (&raw const (*self.raw.get()).m_nValue).read() }
	}

	/// The value the variable reverts to.
	#[doc(alias = "GetDefault")]
	pub const fn default_value(&self) -> &'static CStr {
		self.default
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
		self.current_float()
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
		self.current_int()
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

	/// Fills in the engine-visible fields before linking.
	fn prepare(&self, binding: ServerBinding, dll_identifier: sys::CVarDLLIdentifier_t) {
		let raw = self.raw.get();

		// SAFETY: The variable is not registered, so nothing else accesses these
		// fields, and they are written through the cell without forming
		// references. The variable is its own parent, as every variable the
		// engine lists is.
		unsafe {
			(&raw mut (*raw)._base.vtable_).write(primary_vtable().cast());
			(&raw mut (*raw)._base_1.vtable_).write(secondary_vtable());
			(&raw mut (*raw)._base.m_pNext).write(ptr::null_mut());
			(&raw mut (*raw)._base.m_pszName).write(self.name.as_ptr());
			(&raw mut (*raw)._base.m_pszHelpString).write(self.help.as_ptr());
			(&raw mut (*raw)._base.m_nFlags).write(self.flags.bits());
			(&raw mut (*raw).m_pParent).write(raw);
			(&raw mut (*raw).m_bHasMin).write(self.min.is_some());
			(&raw mut (*raw).m_fMinVal).write(self.min.unwrap_or_default());
			(&raw mut (*raw).m_bHasMax).write(self.max.is_some());
			(&raw mut (*raw).m_fMaxVal).write(self.max.unwrap_or_default());
		}

		self.binding.set(Some(binding));
		self.dll_identifier.set(dll_identifier);
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

		// SAFETY: The variable is pinned and `'static`, `prepare` fills in every
		// field the engine reads, and the caller unregisters it in time.
		unsafe {
			register_base(
				variable.as_base(),
				variable.name,
				CommandBaseKind::Variable,
				server,
				registrar,
				|dll_identifier| variable.prepare(binding, dll_identifier),
			)
		}
	}

	/// Sets the value back to the default.
	#[doc(alias = "Revert")]
	pub fn revert(&self, server: Server<'_>) {
		self.set_string(server, self.default);
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
		if value == self.current_float() && !force {
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
		if value == self.current_int() {
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
		let raw = self.raw.get();
		let old_float = self.current_float();

		// SAFETY: As for `change_string`.
		unsafe {
			(&raw mut (*raw).m_fValue).write(float);
			(&raw mut (*raw).m_nValue).write(int);
		}

		if self.current_flags() & CommandFlags::NEVER_AS_STRING.bits() == 0 {
			self.change_string(callbacks, text(), old_float);
		}
	}

	/// The value as a string.
	#[doc(alias = "GetString")]
	pub fn string(&self, _server: Server<'_>) -> CString {
		// SAFETY: The string is never null, and is only replaced on the main
		// thread. It is copied immediately.
		unsafe { CStr::from_ptr((&raw const (*self.raw.get()).m_pszString).read()) }.to_owned()
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
			.field("default", &self.default)
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

/// The primary `ConVar` vtable, in the order of `public/tier1/convar.h`: the
/// `ConCommandBase` slots, then `ConVar`'s own.
///
/// The Itanium ABI also gives the `IConVar` methods `ConVar` overrides slots
/// here, which are called without adjusting `this`. MSVC only places them in
/// the `IConVar` vtable.
#[repr(C)]
struct PrimaryVtable {
	destructors: CppDestructors,
	is_command: unsafe extern "C" fn(this: *const sys::ConVar) -> bool,
	is_flag_set: unsafe extern "C" fn(this: *const sys::ConVar, flag: c_int) -> bool,
	add_flags: unsafe extern "C" fn(this: *mut sys::ConVar, flags: c_int),
	get_name: unsafe extern "C" fn(this: *const sys::ConVar) -> *const c_char,
	get_help_text: unsafe extern "C" fn(this: *const sys::ConVar) -> *const c_char,
	is_registered: unsafe extern "C" fn(this: *const sys::ConVar) -> bool,
	get_dll_identifier: unsafe extern "C" fn(this: *const sys::ConVar) -> sys::CVarDLLIdentifier_t,
	create_base: unsafe extern "C" fn(
		this: *mut sys::ConVar,
		name: *const c_char,
		help: *const c_char,
		flags: c_int,
	),
	init: unsafe extern "C" fn(this: *mut sys::ConVar),
	#[cfg(not(target_os = "windows"))]
	set_value_string: unsafe extern "C" fn(this: *mut sys::ConVar, value: *const c_char),
	#[cfg(not(target_os = "windows"))]
	set_value_float: unsafe extern "C" fn(this: *mut sys::ConVar, value: f32),
	#[cfg(not(target_os = "windows"))]
	set_value_int: unsafe extern "C" fn(this: *mut sys::ConVar, value: c_int),
	internal_set_value: unsafe extern "C" fn(this: *mut sys::ConVar, value: *const c_char),
	internal_set_float_value: unsafe extern "C" fn(this: *mut sys::ConVar, value: f32),
	internal_set_int_value: unsafe extern "C" fn(this: *mut sys::ConVar, value: c_int),
	clamp_value: unsafe extern "C" fn(this: *mut sys::ConVar, value: *mut f32) -> bool,
	change_string_value:
		unsafe extern "C" fn(this: *mut sys::ConVar, value: *const c_char, old_float: f32),
	create_vtbl: unsafe extern "C" fn(
		this: *mut sys::ConVar,
		name: *const c_char,
		default: *const c_char,
		flags: c_int,
		help: *const c_char,
		has_min: bool,
		min: f32,
		has_max: bool,
		max: f32,
		callback: sys::FnChangeCallback_t,
	),
	internal_set_float_value2:
		unsafe extern "C" fn(this: *mut sys::ConVar, value: f32, force: bool),
}

/// The vtables every [`ConsoleVariable`] points at, alone on their page, as
/// for [`ConsoleCommand`](super::ConsoleCommand)'s: other plugins may patch
/// them in place, and KHook sets a page it patched back to read-and-execute.
#[repr(C, align(4096))]
struct VtablePage(UnsafeCell<Vtables>);

// SAFETY: Rust never accesses the tables after initialization, except to take
// their addresses. Only the engine and hooking libraries read or write them, on
// the server's main thread.
unsafe impl Sync for VtablePage {}

/// Both vtables, each after the type information its ABI reads before it.
#[repr(C)]
struct Vtables {
	/// The `_RTTICompleteObjectLocator` MSVC reads before the table.
	#[cfg(target_os = "windows")]
	primary_locator: *const c_void,

	/// The offset from the subobject to its complete object, negated, which the
	/// Itanium ABI reads two slots before the table.
	#[cfg(not(target_os = "windows"))]
	primary_top: isize,

	/// The `std::type_info` the Itanium ABI reads before the table.
	#[cfg(not(target_os = "windows"))]
	primary_type_info: *const c_void,

	primary: PrimaryVtable,

	#[cfg(target_os = "windows")]
	secondary_locator: *const c_void,

	#[cfg(not(target_os = "windows"))]
	secondary_top: isize,

	#[cfg(not(target_os = "windows"))]
	secondary_type_info: *const c_void,

	secondary: sys::IConVar__bindgen_vtable,
}

unsafe extern "C" fn add_flags(this: *mut sys::ConVar, flags: c_int) {
	// SAFETY: See below.
	unsafe {
		let field = &raw mut (*this)._base.m_nFlags;

		field.write(field.read() | flags);
	}
}

// The slots below are only called by the engine, other plugins, and this
// module's other slots, on the server's main thread, with `this` pointing to a
// prepared variable. They read the C++ fields without forming references,
// since C++ writes them too.

unsafe extern "C" fn change_string_value(
	this: *mut sys::ConVar,
	value: *const c_char,
	old_float: f32,
) {
	// SAFETY: See above. The engine passes a string for the call, or null.
	unsafe {
		from_engine(this, |variable, callbacks| {
			let value = borrow_cstr(value).unwrap_or_default().to_owned();

			variable.change_string(callbacks, value, old_float)
		})
	};
}

unsafe extern "C" fn clamp_value(this: *mut sys::ConVar, value: *mut f32) -> bool {
	// SAFETY: See above. The engine passes a float by reference.
	unsafe {
		from_engine(this, |variable, _| {
			value.as_mut().is_some_and(|value| variable.clamp(value))
		})
	}
	.unwrap_or(false)
}

/// Only tier1's constructors of the module owning a variable call this.
unsafe extern "C" fn create_base(
	_this: *mut sys::ConVar,
	_name: *const c_char,
	_help: *const c_char,
	_flags: c_int,
) {
}

/// Only tier1's constructors of the module owning a variable call this.
#[allow(clippy::too_many_arguments, reason = "the engine's signature")]
unsafe extern "C" fn create_vtbl(
	_this: *mut sys::ConVar,
	_name: *const c_char,
	_default: *const c_char,
	_flags: c_int,
	_help: *const c_char,
	_has_min: bool,
	_min: f32,
	_has_max: bool,
	_max: f32,
	_callback: sys::FnChangeCallback_t,
) {
}

/// Formats a float as `%f` does in tier1's 32-byte buffers.
fn format_float(value: f32) -> CString {
	/// The size of tier1's buffers, whose last byte holds the terminator.
	const BUFFER_LENGTH: usize = 32;

	let mut text = match value {
		value if value.is_nan() && value.is_sign_negative() => "-nan".to_owned(),
		value if value.is_nan() => "nan".to_owned(),
		value => format!("{:.6}", f64::from(value)),
	};

	text.truncate(BUFFER_LENGTH - 1);

	// SAFETY: A formatted number contains no NUL.
	unsafe { CString::from_vec_unchecked(text.into_bytes()) }
}

/// Formats an integer as `%d` does.
fn format_int(value: c_int) -> CString {
	// SAFETY: A formatted number contains no NUL.
	unsafe { CString::from_vec_unchecked(value.to_string().into_bytes()) }
}

/// Runs `f` for the variable whose slot the engine called, with the registry
/// if its change callbacks should run. A panic is caught, since it must not
/// unwind into the engine, and `None` returned.
///
/// # Safety
///
/// `this` must be a pointer the engine passes to a slot of the primary vtable.
unsafe fn from_engine<R>(
	this: *const sys::ConVar,
	f: impl FnOnce(&ConsoleVariable, Option<Cvar<'_>>) -> R,
) -> Option<R> {
	let outcome = catch_unwind(AssertUnwindSafe(|| {
		// SAFETY: Only prepared variables use the vtables, and the engine passes
		// back the pointer it was given, to the whole variable, which is
		// `'static` and never mutably borrowed.
		let variable = unsafe { &*this.cast::<ConsoleVariable>() };
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

/// The whole variable an `IConVar *` the engine passes points into.
///
/// # Safety
///
/// `this` must be a pointer the engine passes to a slot of the `IConVar`
/// vtable.
unsafe fn from_interface(this: *const sys::IConVar) -> *mut sys::ConVar {
	// SAFETY: The engine derived the pointer from the whole variable's, as
	// C++'s conversion to a base does.
	unsafe { this.byte_sub(INTERFACE_OFFSET) }
		.cast::<sys::ConVar>()
		.cast_mut()
}

unsafe extern "C" fn get_dll_identifier(this: *const sys::ConVar) -> sys::CVarDLLIdentifier_t {
	// SAFETY: See above.
	unsafe { &*this.cast::<ConsoleVariable>() }
		.dll_identifier
		.get()
}

unsafe extern "C" fn get_help_text(this: *const sys::ConVar) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszHelpString).read() }
}

unsafe extern "C" fn get_name(this: *const sys::ConVar) -> *const c_char {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_pszName).read() }
}

/// Only tier1's registration of the module owning a variable calls this.
unsafe extern "C" fn init(_this: *mut sys::ConVar) {}

unsafe extern "C" fn interface_get_name(this: *const sys::IConVar) -> *const c_char {
	// SAFETY: See above.
	unsafe { get_name(from_interface(this)) }
}

unsafe extern "C" fn interface_is_flag_set(this: *const sys::IConVar, flag: c_int) -> bool {
	// SAFETY: See above.
	unsafe { is_flag_set(from_interface(this), flag) }
}

unsafe extern "C" fn interface_set_value_float(this: *mut sys::IConVar, value: f32) {
	// SAFETY: See above.
	unsafe { set_value_float(from_interface(this), value) };
}

unsafe extern "C" fn interface_set_value_int(this: *mut sys::IConVar, value: c_int) {
	// SAFETY: See above.
	unsafe { set_value_int(from_interface(this), value) };
}

unsafe extern "C" fn interface_set_value_string(this: *mut sys::IConVar, value: *const c_char) {
	// SAFETY: See above.
	unsafe { set_value_string(from_interface(this), value) };
}

unsafe extern "C" fn internal_set_float_value2(this: *mut sys::ConVar, value: f32, force: bool) {
	// SAFETY: See above.
	unsafe {
		from_engine(this, |variable, callbacks| {
			variable.set_float_value(callbacks, value, force)
		})
	};
}

unsafe extern "C" fn is_command(_this: *const sys::ConVar) -> bool {
	false
}

unsafe extern "C" fn is_flag_set(this: *const sys::ConVar, flag: c_int) -> bool {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_nFlags).read() & flag != 0 }
}

unsafe extern "C" fn is_registered(this: *const sys::ConVar) -> bool {
	// SAFETY: See above.
	unsafe { (&raw const (*this)._base.m_bRegistered).read() }
}

/// Parses the leading decimal number of a string as C's `atof` does: an
/// optional sign, digits with an optional point, and an optional exponent,
/// after whitespace. Anything else reads as 0.
pub(super) const fn parse_float(text: &[u8]) -> f64 {
	/// More digits than this are only counted, since they cannot change the
	/// nearest `f32`.
	const MAX_DIGITS: u64 = 1_000_000_000_000_000_000;

	/// Past this power of ten in either direction, every non-zero mantissa
	/// overflows to infinity or underflows to 0.
	const MAX_EXPONENT: i64 = 400;

	let mut index = skip_space(text);
	let negative = index < text.len() && text[index] == b'-';

	if index < text.len() && (text[index] == b'-' || text[index] == b'+') {
		index += 1;
	}

	let mut mantissa: u64 = 0;
	let mut exponent: i64 = 0;
	let mut digits = false;

	while index < text.len() && text[index].is_ascii_digit() {
		if mantissa < MAX_DIGITS {
			mantissa = mantissa * 10 + (text[index] - b'0') as u64;
		} else {
			exponent += 1;
		}

		digits = true;
		index += 1;
	}

	if index < text.len() && text[index] == b'.' {
		index += 1;

		while index < text.len() && text[index].is_ascii_digit() {
			if mantissa < MAX_DIGITS {
				mantissa = mantissa * 10 + (text[index] - b'0') as u64;
				exponent -= 1;
			}

			digits = true;
			index += 1;
		}
	}

	if !digits {
		return 0.0;
	}

	// Every digit is 0, so the value is too, whatever the exponent. Scaling it
	// would give NaN once the scale overflows to infinity.
	if mantissa == 0 {
		return if negative { -0.0 } else { 0.0 };
	}

	// An exponent counts only if a digit follows its marker and sign.
	if index < text.len() && (text[index] == b'e' || text[index] == b'E') {
		let mut cursor = index + 1;
		let exponent_negative = cursor < text.len() && text[cursor] == b'-';

		if cursor < text.len() && (text[cursor] == b'-' || text[cursor] == b'+') {
			cursor += 1;
		}

		let mut written: i64 = 0;
		let mut exponent_digits = false;

		while cursor < text.len() && text[cursor].is_ascii_digit() {
			written = written
				.saturating_mul(10)
				.saturating_add((text[cursor] - b'0') as i64);
			exponent_digits = true;
			cursor += 1;
		}

		if exponent_digits {
			exponent = exponent.saturating_add(if exponent_negative { -written } else { written });
		}
	}

	let exponent = if exponent > MAX_EXPONENT {
		MAX_EXPONENT
	} else if exponent < -MAX_EXPONENT {
		-MAX_EXPONENT
	} else {
		exponent
	};

	let mut value = mantissa as f64;
	let mut scale = 1.0;
	let mut steps = exponent.unsigned_abs();

	while steps > 0 {
		scale *= 10.0;
		steps -= 1;
	}

	if exponent < 0 {
		value /= scale;
	} else {
		value *= scale;
	}

	if negative { -value } else { value }
}

/// Parses the leading integer of a string as C's `atoi` does, saturating.
pub(super) const fn parse_int(text: &[u8]) -> c_int {
	let mut index = skip_space(text);
	let negative = index < text.len() && text[index] == b'-';

	if index < text.len() && (text[index] == b'-' || text[index] == b'+') {
		index += 1;
	}

	let mut value: i64 = 0;

	while index < text.len() && text[index].is_ascii_digit() {
		let digit = (text[index] - b'0') as i64;

		value = value.saturating_mul(10).saturating_add(digit);
		index += 1;
	}

	if negative {
		value = -value;
	}

	if value > c_int::MAX as i64 {
		c_int::MAX
	} else if value < c_int::MIN as i64 {
		c_int::MIN
	} else {
		value as c_int
	}
}

/// The address of the primary table in [`VTABLES`].
fn primary_vtable() -> *const PrimaryVtable {
	// SAFETY: Only the address is taken; nothing is read.
	unsafe { &raw const (*VTABLES.0.get()).primary }
}

/// The address of the `IConVar` table in [`VTABLES`].
fn secondary_vtable() -> *const sys::IConVar__bindgen_vtable {
	// SAFETY: Only the address is taken; nothing is read.
	unsafe { &raw const (*VTABLES.0.get()).secondary }
}

unsafe extern "C" fn set_value_float(this: *mut sys::ConVar, value: f32) {
	// SAFETY: See above.
	unsafe {
		from_engine(this, |variable, callbacks| {
			variable.set_float_value(callbacks, value, false)
		})
	};
}

unsafe extern "C" fn set_value_int(this: *mut sys::ConVar, value: c_int) {
	// SAFETY: See above.
	unsafe {
		from_engine(this, |variable, callbacks| {
			variable.set_int_value(callbacks, value)
		})
	};
}

unsafe extern "C" fn set_value_string(this: *mut sys::ConVar, value: *const c_char) {
	// SAFETY: See above. The engine passes a string for the call, or null.
	unsafe {
		from_engine(this, |variable, callbacks| {
			variable.set_string_value(callbacks, borrow_cstr(value))
		})
	};
}

/// The index of the first byte of `text` that is not whitespace, as C's
/// `isspace` judges it.
const fn skip_space(text: &[u8]) -> usize {
	let mut index = 0;

	while index < text.len() && matches!(text[index], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
		index += 1;
	}

	index
}

/// The run-time type information MSVC's `dynamic_cast` reads, describing a
/// class of its own without bases, so no cast to one of the engine's classes
/// succeeds.
///
/// Every reference in it is an offset from an image's base, which the
/// runtime finds as the locator's address minus the locator's own offset.
/// All of it lies in one static, which the offsets treat as the image.
#[cfg(target_os = "windows")]
mod type_information {
	use super::INTERFACE_OFFSET;
	use crate::rtti::CompleteObjectLocator;
	use std::cell::UnsafeCell;
	use std::ffi::c_void;
	use std::mem::offset_of;
	use std::ptr;

	/// `BCD_HASPCHD`: the descriptor refers to its class's hierarchy.
	const HAS_HIERARCHY: u32 = 0x40;

	pub(super) const PRIMARY: usize = offset_of!(TypeInformation, primary);
	pub(super) const SECONDARY: usize = offset_of!(TypeInformation, secondary);

	static TYPE_INFORMATION: TypeInformationCell =
		TypeInformationCell(UnsafeCell::new(TypeInformation {
			_image_start: 0,
			type_descriptor: TypeDescriptor {
				vtable: ptr::null(),
				undecorated_name: ptr::null_mut(),
				name: *b".?AVRustConsoleVariable@@\0",
			},
			primary: CompleteObjectLocator {
				signature: 1,
				offset: 0,
				constructor_displacement: 0,
				type_descriptor: offset_of!(TypeInformation, type_descriptor) as u32,
				class_descriptor: offset_of!(TypeInformation, hierarchy) as u32,
				this: PRIMARY as u32,
			},
			secondary: CompleteObjectLocator {
				signature: 1,
				offset: INTERFACE_OFFSET as u32,
				constructor_displacement: 0,
				type_descriptor: offset_of!(TypeInformation, type_descriptor) as u32,
				class_descriptor: offset_of!(TypeInformation, hierarchy) as u32,
				this: SECONDARY as u32,
			},
			hierarchy: ClassHierarchyDescriptor {
				signature: 0,
				attributes: 0,
				base_classes: 1,
				base_class_array: offset_of!(TypeInformation, base_class_array) as u32,
			},
			base_class_array: [offset_of!(TypeInformation, base_class) as u32],
			base_class: BaseClassDescriptor {
				type_descriptor: offset_of!(TypeInformation, type_descriptor) as u32,
				contained_bases: 0,
				member_displacement: 0,
				vbtable_displacement: -1,
				vbtable_offset: 0,
				attributes: HAS_HIERARCHY,
				class_descriptor: offset_of!(TypeInformation, hierarchy) as u32,
			},
		}));

	/// `_RTTIBaseClassDescriptor` for 64-bit images.
	#[repr(C)]
	struct BaseClassDescriptor {
		type_descriptor: u32,
		contained_bases: u32,
		/// `PMD::mdisp`, the base's offset in the class.
		member_displacement: i32,
		/// `PMD::pdisp`, -1 for a base that is not virtual.
		vbtable_displacement: i32,
		/// `PMD::vdisp`.
		vbtable_offset: i32,
		attributes: u32,
		class_descriptor: u32,
	}

	/// `_RTTIClassHierarchyDescriptor` for 64-bit images.
	#[repr(C)]
	struct ClassHierarchyDescriptor {
		signature: u32,
		/// Neither `CHD_MULTINH` nor `CHD_VIRTINH`.
		attributes: u32,
		/// The class itself, and no bases.
		base_classes: u32,
		base_class_array: u32,
	}

	/// `std::type_info`'s layout: its vtable, the undecorated name the runtime
	/// caches when asked for it, and the decorated name, which casts compare.
	#[repr(C)]
	struct TypeDescriptor {
		vtable: *const c_void,
		undecorated_name: *mut c_void,
		name: [u8; 26],
	}

	#[repr(C)]
	struct TypeInformation {
		/// Keeps every offset above 0, which could read as absent.
		_image_start: u64,
		type_descriptor: TypeDescriptor,
		primary: CompleteObjectLocator,
		secondary: CompleteObjectLocator,
		hierarchy: ClassHierarchyDescriptor,
		base_class_array: [u32; 1],
		base_class: BaseClassDescriptor,
	}

	/// Writable, since the runtime caches the undecorated name in the type
	/// descriptor.
	#[repr(transparent)]
	struct TypeInformationCell(UnsafeCell<TypeInformation>);

	// SAFETY: Rust never accesses the information after initialization, except
	// to take its address. Only the C++ runtime reads it, and writes the cached
	// name, on the server's main thread.
	unsafe impl Sync for TypeInformationCell {}

	/// The address of a locator, `PRIMARY` or `SECONDARY`.
	pub(super) const fn locator(offset: usize) -> *const c_void {
		(&raw const TYPE_INFORMATION)
			.cast::<u8>()
			.wrapping_add(offset)
			.cast()
	}
}

/// The run-time type information libstdc++'s `dynamic_cast` reads, describing
/// a class of its own without bases, so no cast to one of the engine's classes
/// succeeds.
#[cfg(not(target_os = "windows"))]
mod type_information {
	use std::ffi::{CStr, c_char, c_void};

	/// The Itanium ABI mangles a class's name as its length, then the name.
	const MANGLED_NAME: &CStr = c"19RustConsoleVariable";

	static TYPE_INFO: TypeInfo = TypeInfo {
		// An object's vtable pointer skips the two slots before the first.
		vtable: (&raw const CLASS_TYPE_INFO_VTABLE)
			.cast::<*const c_void>()
			.wrapping_add(2),
		name: MANGLED_NAME.as_ptr(),
	};

	/// `__cxxabiv1::__class_type_info`'s layout: `std::type_info`'s vtable and
	/// mangled name.
	#[repr(C)]
	struct TypeInfo {
		vtable: *const *const c_void,
		name: *const c_char,
	}

	// SAFETY: The information is immutable.
	unsafe impl Sync for TypeInfo {}

	#[link(name = "stdc++")]
	unsafe extern "C" {
		/// libstdc++'s vtable of `__cxxabiv1::__class_type_info`, the type
		/// information of a class without bases, whose `__do_dyncast` a cast
		/// calls.
		#[link_name = "_ZTVN10__cxxabiv117__class_type_infoE"]
		static CLASS_TYPE_INFO_VTABLE: [*const c_void; 0];
	}

	#[cfg(test)]
	pub(super) const fn class_type_info_vtable() -> *const *const c_void {
		(&raw const CLASS_TYPE_INFO_VTABLE)
			.cast::<*const c_void>()
			.wrapping_add(2)
	}

	pub(super) const fn type_info() -> *const c_void {
		(&raw const TYPE_INFO).cast()
	}
}

#[cfg(test)]
pub(super) mod test_support {
	use super::*;

	/// libstdc++'s vtable for type information of classes without bases.
	#[cfg(not(target_os = "windows"))]
	pub(crate) fn class_type_info_vtable() -> *const *const c_void {
		type_information::class_type_info_vtable()
	}

	/// Where the engine sees a variable's `IConVar`.
	pub(crate) fn interface_of(variable: &ConsoleVariable) -> *mut sys::IConVar {
		// SAFETY: The subobject lies within the variable.
		unsafe { variable.raw.get().byte_add(INTERFACE_OFFSET) }.cast()
	}

	/// The run-time type information a variable's primary vtable describes.
	#[cfg(not(target_os = "windows"))]
	pub(crate) fn type_info() -> *const c_void {
		type_information::type_info()
	}
}
