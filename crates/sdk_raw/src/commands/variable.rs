//! The object the engine sees as a `ConVar`, with its vtables, run-time type
//! information, and the thunks in them, and the C library parsing and
//! formatting tier1 applies to values.
//!
//! A `ConVar` derives from both `ConCommandBase` and `IConVar`, so the engine
//! reaches one through two vtables: the primary one at its start, and
//! `IConVar`'s in the subobject after `ConCommandBase`, through which callers
//! set values. Both tables are preceded by run-time type information, since
//! the engine `dynamic_cast`s every variable it describes in the console.

use crate::abi::{VTABLE_SLOT_SIZE, VtablePage};
use crate::util::cstr::borrow_cstr;

#[cfg(not(target_os = "windows"))]
use crate::util::rtti::ClassTypeInfo;

#[cfg(target_os = "windows")]
use crate::util::rtti::CompleteObjectLocator;

use crate::vtable_slot;
use std::cell::{Cell, UnsafeCell};
use std::ffi::{CStr, CString, c_char, c_int};

#[cfg(target_os = "windows")]
use std::ffi::{c_uint, c_void};

use std::marker::{PhantomData, PhantomPinned};
use std::mem::{MaybeUninit, offset_of};
use std::ptr::{self, NonNull};

const _: () = {
	use sys::ConCommandBase__bindgen_vtable as Base;
	use sys::ConVar__bindgen_vtable as Primary;

	assert!(offset_of!(ConVarObject, raw) == 0);
	assert!(offset_of!(sys::ConVar, _base) == 0);
	assert!(ICONVAR_OFFSET == size_of::<sys::ConCommandBase>());

	// The engine calls variables through `ConCommandBase`'s slots, which start
	// the primary table.
	assert!(
		vtable_slot!(Primary, ConVar_IsCommand) == vtable_slot!(Base, ConCommandBase_IsCommand)
	);
	assert!(vtable_slot!(Primary, ConVar_Init) == vtable_slot!(Base, ConCommandBase_Init));

	// The slot counts of `ConVar`'s vtables in TF2's 64-bit binaries.
	#[cfg(target_os = "windows")]
	assert!(size_of::<Primary>() == 17 * VTABLE_SLOT_SIZE);
	#[cfg(not(target_os = "windows"))]
	assert!(size_of::<Primary>() == 21 * VTABLE_SLOT_SIZE);
	assert!(size_of::<sys::IConVar__bindgen_vtable>() == 5 * VTABLE_SLOT_SIZE);

	// Each table directly follows its type information.
	#[cfg(target_os = "windows")]
	{
		assert!(
			offset_of!(Vtables, primary) == offset_of!(Vtables, primary_locator) + VTABLE_SLOT_SIZE
		);

		assert!(
			offset_of!(Vtables, secondary)
				== offset_of!(Vtables, secondary_locator) + VTABLE_SLOT_SIZE
		);
	}

	#[cfg(not(target_os = "windows"))]
	{
		assert!(
			offset_of!(Vtables, primary)
				== offset_of!(Vtables, primary_type_info) + VTABLE_SLOT_SIZE
		);

		assert!(
			offset_of!(Vtables, secondary)
				== offset_of!(Vtables, secondary_type_info) + VTABLE_SLOT_SIZE
		);

		assert!(
			offset_of!(Vtables, primary_type_info)
				== offset_of!(Vtables, primary_top) + VTABLE_SLOT_SIZE
		);

		assert!(
			offset_of!(Vtables, secondary_type_info)
				== offset_of!(Vtables, secondary_top) + VTABLE_SLOT_SIZE
		);
	}
};

/// Where the `IConVar` subobject sits in a `ConVar`. Callers holding an
/// `IConVar *` point here.
pub const ICONVAR_OFFSET: usize = offset_of!(sys::ConVar, _base_1);

/// The vtables every prepared [`ConVarObject`] points at, alone on their
/// page, as for a [`ConCommandObject`](super::ConCommandObject)'s: other
/// plugins may patch them in place. Rust never reads the tables; the engine
/// only gets their addresses.
static VTABLES: VtablePage<Vtables> = VtablePage::new(Vtables {
	#[cfg(target_os = "windows")]
	primary_locator: type_information::locator(type_information::PRIMARY),

	#[cfg(not(target_os = "windows"))]
	primary_top: 0,

	#[cfg(not(target_os = "windows"))]
	primary_type_info: &raw const type_information::TYPE_INFO,

	primary: sys::ConVar__bindgen_vtable {
		#[cfg(target_os = "windows")]
		ConVar_destructor: destructor,
		#[cfg(not(target_os = "windows"))]
		ConVar_complete_destructor: destructor,
		#[cfg(not(target_os = "windows"))]
		ConVar_deleting_destructor: destructor,
		ConVar_IsCommand: is_command,
		ConVar_IsFlagSet: is_flag_set,
		ConVar_AddFlags: add_flags,
		ConVar_GetName: get_name,
		ConVar_GetHelpText: get_help_text,
		ConVar_IsRegistered: is_registered,
		ConVar_GetDLLIdentifier: get_dll_identifier,
		ConVar_CreateBase: create_base,
		ConVar_Init: init,
		#[cfg(not(target_os = "windows"))]
		ConVar_SetValue: set_value_string,
		#[cfg(not(target_os = "windows"))]
		ConVar_SetValue1: set_value_float,
		#[cfg(not(target_os = "windows"))]
		ConVar_SetValue2: set_value_int,
		ConVar_InternalSetValue: set_value_string,
		ConVar_InternalSetFloatValue: set_value_float,
		ConVar_InternalSetIntValue: set_value_int,
		ConVar_ClampValue: clamp_value,
		ConVar_ChangeStringValue: change_string_value,
		ConVar_Create_Vtbl: create_vtbl,
		ConVar_InternalSetFloatValue2: internal_set_float_value2,
	},

	#[cfg(target_os = "windows")]
	secondary_locator: type_information::locator(type_information::SECONDARY),

	#[cfg(not(target_os = "windows"))]
	secondary_top: -(ICONVAR_OFFSET as isize),

	#[cfg(not(target_os = "windows"))]
	secondary_type_info: &raw const type_information::TYPE_INFO,

	secondary: sys::IConVar__bindgen_vtable {
		IConVar_SetValue: interface_set_value_string,
		IConVar_SetValue1: interface_set_value_float,
		IConVar_SetValue2: interface_set_value_int,
		IConVar_GetName: interface_get_name,
		IConVar_IsFlagSet: interface_is_flag_set,
	},
});

/// What the engine's calls that set a [`ConVarObject`]'s value run, chosen
/// by its owner.
///
/// Each hook is called on the thread the engine calls variables on, with the
/// pointer to the variable the engine was handed, adjusted back from its
/// `IConVar` for calls through that, and arguments that are live for the
/// call. A hook must not unwind: a panic aborts the process.
#[derive(Debug)]
pub struct ConVarHooks {
	/// Sets the value from a string, `None` for a null one, for
	/// `SetValue(const char *)` and `InternalSetValue`.
	pub set_string: unsafe fn(variable: NonNull<ConVarObject>, value: Option<&CStr>),

	/// Sets the value from a float, for `SetValue(float)` and
	/// `InternalSetFloatValue`, which never force it, and
	/// `InternalSetFloatValue2`.
	pub set_float: unsafe fn(variable: NonNull<ConVarObject>, value: f32, force: bool),

	/// Sets the value from an integer, for `SetValue(int)` and
	/// `InternalSetIntValue`.
	pub set_int: unsafe fn(variable: NonNull<ConVarObject>, value: c_int),

	/// Clamps a value to the variable's bounds, returning whether it changed,
	/// for `ClampValue`.
	pub clamp: unsafe fn(variable: NonNull<ConVarObject>, value: &mut f32) -> bool,

	/// Installs a string and runs the change callbacks if it differs from the
	/// old one, whose float is `old_float`, for `ChangeStringValue`. A null
	/// string is given as empty.
	pub change_string: unsafe fn(variable: NonNull<ConVarObject>, value: &CStr, old_float: f32),
}

/// A console variable Rust implements, laid out so the engine sees a `ConVar`
/// once [prepared](Self::prepare).
///
/// It is the first field of its owner's `#[repr(C)]` container, and runs the
/// owner's [`ConVarHooks`] for the engine's calls that set its value (see the
/// [module documentation](super)). The engine, its host, and other plugins
/// keep a registered variable's address and write its C++ fields, such as its
/// list link, registered flag, flags, and change callback, through their own
/// pointers at any time, so Rust only reaches those fields through raw
/// pointers.
///
/// It holds its value as tier1's `ConVar` does: a float, an integer, and a
/// string, which is its default until [replaced](Self::replace_string).
#[doc(alias("ConVar"))]
#[repr(C)]
pub struct ConVarObject {
	/// The engine-visible `ConVar`, which Rust never forms a reference into.
	raw: UnsafeCell<sys::ConVar>,

	/// What the engine's calls that set the value run.
	hooks: &'static ConVarHooks,

	/// The value the variable reverts to, and its string until replaced.
	default: &'static CStr,

	/// The string `raw` holds, unless it holds the default.
	string: Cell<Option<CString>>,

	/// Returned from `GetDLLIdentifier`; given by `prepare`.
	dll_identifier: Cell<sys::CVarDLLIdentifier_t>,

	_pinned: PhantomPinned,
	_not_thread_safe: PhantomData<*mut ()>,
}

impl ConVarObject {
	/// Creates an unprepared variable holding `default`, whose float and
	/// integer are parsed from it as tier1's `ConVar::Create` parses them:
	/// separately, so large integers keep their bits. The hooks run for the
	/// engine's calls once it is prepared.
	pub const fn new(default: &'static CStr, hooks: &'static ConVarHooks) -> Self {
		let text = default.to_bytes();

		// SAFETY: Zero is valid for every field of `ConVar`: null pointers, no
		// callback, `false`, and 0. The engine only sees the object once
		// `prepare` has filled in the rest.
		let mut raw: sys::ConVar = unsafe { MaybeUninit::zeroed().assume_init() };

		// The string is only ever replaced, never written through, so it may
		// point to the static default.
		raw.m_pszDefaultValue = default.as_ptr();
		raw.m_pszString = default.as_ptr().cast_mut();
		raw.m_StringLength = if text.len() < c_int::MAX as usize {
			text.len() as c_int + 1
		} else {
			c_int::MAX
		};
		raw.m_fValue = parse_float(text) as f32;
		raw.m_nValue = parse_int(text);

		Self {
			raw: UnsafeCell::new(raw),
			hooks,
			default,
			string: Cell::new(None),
			dll_identifier: Cell::new(0),
			_pinned: PhantomPinned,
			_not_thread_safe: PhantomData,
		}
	}

	/// The pointer to hand the engine, which it passes back to every slot of
	/// the primary vtable.
	///
	/// `this` should point to the owner's whole container, so the hooks can
	/// reach the rest of it through the pointer.
	pub const fn as_base(this: NonNull<Self>) -> NonNull<sys::ConCommandBase> {
		this.cast()
	}

	/// The variable as a `ConVar`, with the provenance of `this`, for the
	/// engine's functions that take one.
	pub const fn as_var(this: NonNull<Self>) -> NonNull<sys::ConVar> {
		this.cast()
	}

	/// Runs the change callback the engine installed in the variable, if any,
	/// as tier1's `ConVar::ChangeStringValue` does: with the variable's
	/// `IConVar`, its old string, and its old float.
	///
	/// # Safety
	///
	/// `this` must point to a prepared object, with the provenance of the
	/// pointer handed to the engine, since the callback may pass it back to
	/// the variable's slots. This must run on the thread the engine calls
	/// variables on, while the module that installed the callback is loaded.
	#[doc(alias("m_fnChangeCallback"))]
	pub unsafe fn call_change_callback(this: NonNull<Self>, old: &CStr, old_float: f32) {
		let raw = Self::as_var(this).as_ptr();

		// SAFETY: The caller guarantees the object is live. The field is read
		// without forming a reference. The engine may have installed a callback
		// when another module registered a variable of the same name.
		let callback = unsafe { (&raw const (*raw).m_fnChangeCallback).read() };

		if let Some(callback) = callback {
			// SAFETY: Callbacks receive the `IConVar` subobject, as C++'s
			// conversion from `ConVar *` gives them, and the caller keeps the
			// module that installed the callback loaded.
			unsafe { callback(Self::interface(this).as_ptr(), old.as_ptr(), old_float) };
		}
	}

	/// The variable's `IConVar` subobject, with the provenance of `this`,
	/// through which callers set values.
	///
	/// # Safety
	///
	/// `this` must point to a live object.
	pub unsafe fn interface(this: NonNull<Self>) -> NonNull<sys::IConVar> {
		// SAFETY: The subobject lies within the live object.
		unsafe { this.byte_add(ICONVAR_OFFSET) }.cast()
	}

	/// Fills in the engine-visible fields before the variable is linked into
	/// the engine's registry: its vtables, a cleared list link, its name, its
	/// help text, its flags, itself as the parent that holds its value, its
	/// bounds, and the DLL identifier the registry allocated.
	///
	/// # Safety
	///
	/// `this` must point to a live object that is not registered, so nothing
	/// else accesses its C++ fields, with the provenance of the whole
	/// container the engine is handed, since the parent pointer stored here is
	/// passed back to the variable's slots. Once prepared, it may be handed to
	/// the engine through [`Self::as_base`], after which, until the engine no
	/// longer lists it, it must stay where it is, the module containing its
	/// hooks' code must stay loaded, and it must only be accessed on the thread
	/// the engine calls variables on.
	pub unsafe fn prepare(
		this: NonNull<Self>,
		name: &'static CStr,
		help: &'static CStr,
		flags: c_int,
		min: Option<f32>,
		max: Option<f32>,
		dll_identifier: sys::CVarDLLIdentifier_t,
	) {
		let raw = Self::as_var(this).as_ptr();

		// SAFETY: The caller guarantees the object is live and not registered, so
		// nothing else accesses these fields, and they are written through the
		// cell without forming references. The variable is its own parent, as
		// every variable the engine lists is.
		unsafe {
			(&raw mut (*raw)._base.vtable_).write(primary_vtable().cast());
			(&raw mut (*raw)._base_1.vtable_).write(secondary_vtable());
			(&raw mut (*raw)._base.m_pNext).write(ptr::null_mut());
			(&raw mut (*raw)._base.m_pszName).write(name.as_ptr());
			(&raw mut (*raw)._base.m_pszHelpString).write(help.as_ptr());
			(&raw mut (*raw)._base.m_nFlags).write(flags);
			(&raw mut (*raw).m_pParent).write(raw);
			(&raw mut (*raw).m_bHasMin).write(min.is_some());
			(&raw mut (*raw).m_fMinVal).write(min.unwrap_or_default());
			(&raw mut (*raw).m_bHasMax).write(max.is_some());
			(&raw mut (*raw).m_fMaxVal).write(max.unwrap_or_default());
		}

		// SAFETY: As above; the identifier lies outside the C++ fields.
		unsafe { (*this.as_ptr()).dll_identifier.set(dll_identifier) };
	}

	/// The value the variable reverts to.
	#[doc(alias("GetDefault"))]
	pub const fn default(&self) -> &'static CStr {
		self.default
	}

	/// The flags the engine currently sees, which other plugins may change.
	#[doc(alias("m_nFlags"))]
	pub fn flags(&self) -> c_int {
		// SAFETY: As for `float`.
		unsafe { (&raw const (*self.raw.get())._base.m_nFlags).read() }
	}

	/// The value as a float.
	#[doc(alias("GetFloat"))]
	pub fn float(&self) -> f32 {
		// SAFETY: The field is read through the cell without forming a
		// reference. `prepare`'s contract keeps C++ from writing it on another
		// thread.
		unsafe { (&raw const (*self.raw.get()).m_fValue).read() }
	}

	/// The value as an integer.
	#[doc(alias("GetInt"))]
	pub fn int(&self) -> c_int {
		// SAFETY: As for `float`.
		unsafe { (&raw const (*self.raw.get()).m_nValue).read() }
	}

	/// Installs a string, as tier1's `ConVar::ChangeStringValue` does before
	/// running the change callbacks, which the returned
	/// [`ReplacedString`] keeps the old string alive for, even if they change
	/// the variable again.
	pub fn replace_string(&self, value: CString) -> ReplacedString {
		let raw = self.raw.get();
		let length = c_int::try_from(value.as_bytes_with_nul().len()).unwrap_or(c_int::MAX);
		let previous = self.string.take();
		let changed = previous.as_deref().unwrap_or(self.default) != value.as_c_str();

		// SAFETY: The fields are written through the cell without forming
		// references. `prepare`'s contract keeps C++ from accessing them on
		// another thread. The string's buffer stays where it is when the string
		// moves into `self.string` below.
		unsafe {
			(&raw mut (*raw).m_pszString).write(value.as_ptr().cast_mut());
			(&raw mut (*raw).m_StringLength).write(length);
		}

		self.string.set(Some(value));

		ReplacedString {
			previous,
			default: self.default,
			changed,
		}
	}

	/// Stores the value's float and integer, as tier1's setters do before
	/// they replace its string.
	pub fn set_numbers(&self, float: f32, int: c_int) {
		let raw = self.raw.get();

		// SAFETY: As for `replace_string`.
		unsafe {
			(&raw mut (*raw).m_fValue).write(float);
			(&raw mut (*raw).m_nValue).write(int);
		}
	}

	/// The value as a string, copied, or empty if another module cleared it.
	#[doc(alias("GetString"))]
	pub fn string(&self) -> CString {
		// SAFETY: As for `float`. The string is the static default or the one
		// `self.string` holds, which is only replaced on this thread, and is
		// copied immediately.
		unsafe { borrow_cstr((&raw const (*self.raw.get()).m_pszString).read()) }
			.unwrap_or_default()
			.to_owned()
	}
}

/// The string a variable held before [`ConVarObject::replace_string`], which
/// stays alive until this is dropped.
#[derive(Debug)]
#[must_use]
pub struct ReplacedString {
	/// The old string, unless it was the default.
	previous: Option<CString>,
	default: &'static CStr,
	changed: bool,
}

impl ReplacedString {
	/// Whether the new string differs from the old one.
	pub const fn changed(&self) -> bool {
		self.changed
	}

	/// The old string.
	pub fn old(&self) -> &CStr {
		self.previous.as_deref().unwrap_or(self.default)
	}
}

/// Both vtables, each after the type information its ABI reads before it.
#[repr(C)]
struct Vtables {
	/// The `_RTTICompleteObjectLocator` MSVC reads before the table.
	#[cfg(target_os = "windows")]
	primary_locator: *const CompleteObjectLocator,

	/// The offset from the subobject to its complete object, negated, which the
	/// Itanium ABI reads two slots before the table.
	#[cfg(not(target_os = "windows"))]
	primary_top: isize,

	/// The `std::type_info` the Itanium ABI reads before the table.
	#[cfg(not(target_os = "windows"))]
	primary_type_info: *const ClassTypeInfo,

	primary: sys::ConVar__bindgen_vtable,

	#[cfg(target_os = "windows")]
	secondary_locator: *const CompleteObjectLocator,

	#[cfg(not(target_os = "windows"))]
	secondary_top: isize,

	#[cfg(not(target_os = "windows"))]
	secondary_type_info: *const ClassTypeInfo,

	secondary: sys::IConVar__bindgen_vtable,
}

// The slots below are only called by the engine, other plugins, and this
// module's other slots, on the thread the engine calls variables on, with
// `this` pointing to a prepared variable. They read the C++ fields without
// forming references, since C++ writes them too.

unsafe extern "C" fn add_flags(this: *mut sys::ConVar, flags: c_int) {
	// SAFETY: See above.
	unsafe {
		let field = &raw mut (*this)._base.m_nFlags;

		field.write(field.read() | flags);
	}
}

/// The variable whose primary vtable's slot the engine called, with the
/// provenance of the pointer the engine was handed, and the hooks it runs.
///
/// # Safety
///
/// `this` must be a pointer the engine passes to a slot of the primary
/// vtable.
unsafe fn called(this: *const sys::ConVar) -> (NonNull<ConVarObject>, &'static ConVarHooks) {
	// SAFETY: C++ calls a slot through the object, which is never null.
	let variable = unsafe { NonNull::new_unchecked(this.cast_mut()) }.cast::<ConVarObject>();

	// SAFETY: Only prepared objects use the vtables. The hooks are read
	// without forming a reference.
	let hooks = unsafe { (&raw const (*variable.as_ptr()).hooks).read() };

	(variable, hooks)
}

unsafe extern "C" fn change_string_value(
	this: *mut sys::ConVar,
	value: *const c_char,
	old_float: f32,
) {
	// SAFETY: See above. The engine passes a string for the call, or null.
	unsafe {
		let (variable, hooks) = called(this);

		(hooks.change_string)(variable, borrow_cstr(value).unwrap_or_default(), old_float);
	}
}

/// The engine's float is copied, rather than borrowed, for the hook.
unsafe extern "C" fn clamp_value(this: *mut sys::ConVar, value: *mut f32) -> bool {
	let Some(value) = NonNull::new(value) else {
		return false;
	};

	// SAFETY: See above. The engine passes a float by reference, which is live
	// for the call.
	unsafe {
		let (variable, hooks) = called(this);
		let mut clamped = value.read();
		let changed = (hooks.clamp)(variable, &mut clamped);

		if changed {
			value.write(clamped);
		}

		changed
	}
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

/// A variable belongs to its Rust owner, so C++ destroying or deleting it
/// leaves it intact.
#[cfg(target_os = "windows")]
unsafe extern "C" fn destructor(this: *mut sys::ConVar, _flags: c_uint) -> *mut c_void {
	this.cast()
}

/// As the Windows version, for the Itanium ABI's complete and deleting
/// destructors.
#[cfg(not(target_os = "windows"))]
unsafe extern "C" fn destructor(_this: *mut sys::ConVar) {}

/// Formats a float as `%f` does in tier1's 32-byte buffers, as its
/// `ConVar::InternalSetFloatValue` does for the string of a value set from a
/// float.
pub fn format_float(value: f32) -> CString {
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

/// Formats an integer as `%d` does, as tier1's
/// `ConVar::InternalSetIntValue` does for the string of a value set from an
/// integer.
pub fn format_int(value: c_int) -> CString {
	// SAFETY: A formatted number contains no NUL.
	unsafe { CString::from_vec_unchecked(value.to_string().into_bytes()) }
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
	unsafe { this.byte_sub(ICONVAR_OFFSET) }
		.cast::<sys::ConVar>()
		.cast_mut()
}

unsafe extern "C" fn get_dll_identifier(this: *const sys::ConVar) -> sys::CVarDLLIdentifier_t {
	// SAFETY: See above. The identifier lies outside the C++ fields.
	unsafe { (*this.cast::<ConVarObject>()).dll_identifier.get() }
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
		let (variable, hooks) = called(this);

		(hooks.set_float)(variable, value, force);
	}
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
#[doc(alias("atof"))]
pub const fn parse_float(text: &[u8]) -> f64 {
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
#[doc(alias("atoi"))]
pub const fn parse_int(text: &[u8]) -> c_int {
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
fn primary_vtable() -> *const sys::ConVar__bindgen_vtable {
	// SAFETY: Only the address is taken; nothing is read.
	unsafe { &raw const (*VTABLES.get()).primary }
}

/// The address of the `IConVar` table in [`VTABLES`].
fn secondary_vtable() -> *const sys::IConVar__bindgen_vtable {
	// SAFETY: Only the address is taken; nothing is read.
	unsafe { &raw const (*VTABLES.get()).secondary }
}

unsafe extern "C" fn set_value_float(this: *mut sys::ConVar, value: f32) {
	// SAFETY: See above.
	unsafe {
		let (variable, hooks) = called(this);

		(hooks.set_float)(variable, value, false);
	}
}

unsafe extern "C" fn set_value_int(this: *mut sys::ConVar, value: c_int) {
	// SAFETY: See above.
	unsafe {
		let (variable, hooks) = called(this);

		(hooks.set_int)(variable, value);
	}
}

unsafe extern "C" fn set_value_string(this: *mut sys::ConVar, value: *const c_char) {
	// SAFETY: See above. The engine passes a string for the call, or null.
	unsafe {
		let (variable, hooks) = called(this);

		(hooks.set_string)(variable, borrow_cstr(value));
	}
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
/// class of its own without bases, `RustConsoleVariable`, so no cast to one
/// of the engine's classes succeeds.
///
/// Every reference in it is an offset from an image's base, which the
/// runtime finds as the locator's address minus the locator's own offset.
/// All of it lies in one static, which the offsets treat as the image.
#[cfg(target_os = "windows")]
mod type_information {
	use super::ICONVAR_OFFSET;

	use crate::util::rtti::{
		BaseClassDescriptor, ClassHierarchyDescriptor, CompleteObjectLocator, TypeDescriptor,
	};

	use std::cell::UnsafeCell;
	use std::mem::offset_of;
	use std::ptr;

	/// The class's decorated name, which casts compare.
	const NAME: [u8; 26] = *b".?AVRustConsoleVariable@@\0";

	/// Where the primary table's locator lies in the information.
	pub(super) const PRIMARY: usize = offset_of!(TypeInformation, primary);

	/// Where the `IConVar` table's locator lies in the information.
	pub(super) const SECONDARY: usize = offset_of!(TypeInformation, secondary);

	static TYPE_INFORMATION: TypeInformationCell =
		TypeInformationCell(UnsafeCell::new(TypeInformation {
			_image_start: 0,
			type_descriptor: TypeDescriptor {
				vtable: ptr::null(),
				undecorated_name: ptr::null_mut(),
				name: NAME,
			},
			primary: CompleteObjectLocator {
				signature: CompleteObjectLocator::SIGNATURE,
				offset: 0,
				constructor_displacement: 0,
				type_descriptor: offset_of!(TypeInformation, type_descriptor) as u32,
				class_descriptor: offset_of!(TypeInformation, hierarchy) as u32,
				this: PRIMARY as u32,
			},
			secondary: CompleteObjectLocator {
				signature: CompleteObjectLocator::SIGNATURE,
				offset: ICONVAR_OFFSET as u32,
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
				attributes: BaseClassDescriptor::HAS_HIERARCHY,
				class_descriptor: offset_of!(TypeInformation, hierarchy) as u32,
			},
		}));

	/// Every record a cast reads.
	#[repr(C)]
	struct TypeInformation {
		/// Keeps every offset above 0, which could read as absent.
		_image_start: u64,
		type_descriptor: TypeDescriptor<{ NAME.len() }>,
		primary: CompleteObjectLocator,
		secondary: CompleteObjectLocator,

		/// The class itself, and no bases.
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
	// name, on the thread the engine calls variables on.
	unsafe impl Sync for TypeInformationCell {}

	/// The address of a locator, `PRIMARY` or `SECONDARY`.
	pub(super) const fn locator(offset: usize) -> *const CompleteObjectLocator {
		(&raw const TYPE_INFORMATION)
			.cast::<u8>()
			.wrapping_add(offset)
			.cast()
	}
}

/// The run-time type information libstdc++'s `dynamic_cast` reads, describing
/// a class of its own without bases, `RustConsoleVariable`, so no cast to one
/// of the engine's classes succeeds.
#[cfg(not(target_os = "windows"))]
mod type_information {
	use crate::util::rtti::{ClassTypeInfo, class_type_info_vtable};
	use std::ffi::CStr;

	/// The Itanium ABI mangles a class's name as its length, then the name.
	const MANGLED_NAME: &CStr = c"19RustConsoleVariable";

	/// The class's type information, which both tables' prefixes point to.
	pub(super) static TYPE_INFO: ClassTypeInfo = ClassTypeInfo {
		vtable: class_type_info_vtable(),
		name: MANGLED_NAME.as_ptr(),
	};
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::commands::{FCVAR_GAMEDLL, FCVAR_NOTIFY};
	use crate::util::rtti::subobject_offset;
	use std::cell::RefCell;

	static HOOKS: ConVarHooks = ConVarHooks {
		set_string: record_string,
		set_float: record_float,
		set_int: record_int,
		clamp: record_clamp,
		change_string: record_change,
	};

	thread_local! {
		/// The variables the hooks were called with, and how.
		static CALLS: RefCell<Vec<(*mut ConVarObject, Call)>> = const { RefCell::new(Vec::new()) };
	}

	/// A call of one of the hooks.
	#[derive(Debug, PartialEq)]
	enum Call {
		String(Option<CString>),
		Float(f32, bool),
		Int(c_int),
		Clamp(f32),
		Change(CString, f32),
	}

	fn calls() -> Vec<(*mut ConVarObject, Call)> {
		CALLS.take()
	}

	/// As the Windows version, for libstdc++.
	#[cfg(not(target_os = "windows"))]
	#[test]
	fn casts_to_the_engines_classes_fail() {
		use crate::util::rtti::class_type_info_vtable;
		use std::ffi::c_void;

		unsafe extern "C" {
			/// libstdc++'s runtime for `dynamic_cast` to a pointer.
			fn __dynamic_cast(
				object: *const c_void,
				source: *const c_void,
				target: *const c_void,
				source_to_target: isize,
			) -> *mut c_void;
		}

		let class = |name: &'static CStr| ClassTypeInfo {
			vtable: class_type_info_vtable(),
			name: name.as_ptr(),
		};
		let convar = class(c"6ConVar");
		let iconvar = class(c"7IConVar");
		let bounded = class(c"20ConVar_ServerBounded");

		let variable = prepared(c"sb_cast");
		let object = NonNull::from(&*variable);
		let base = object.as_ptr().cast_const().cast::<c_void>();

		// SAFETY: The variable is live.
		let interface = unsafe { ConVarObject::interface(object) }
			.as_ptr()
			.cast_const()
			.cast::<c_void>();

		let cast = |from: *const c_void, source: *const c_void, target: *const c_void, hint| {
			// SAFETY: `from` is a subobject of the live variable whose type
			// information `source` describes, as the engine's casts pass them.
			unsafe { __dynamic_cast(from, source, target, hint) }
		};

		// As `ConVar_PrintDescription`'s cast, whose target derives from `ConVar`
		// at offset 0.
		assert!(
			cast(
				base,
				(&raw const convar).cast(),
				(&raw const bounded).cast(),
				0
			)
			.is_null()
		);
		assert!(
			cast(
				interface,
				(&raw const iconvar).cast(),
				(&raw const bounded).cast(),
				-1
			)
			.is_null()
		);

		// Every cast fails, since the class claims no bases, but the type
		// information describes the whole variable from either table.
		assert!(
			cast(
				base,
				(&raw const convar).cast(),
				(&raw const type_information::TYPE_INFO).cast(),
				0
			)
			.is_null()
		);

		// SAFETY: Both are subobjects of the live variable, whose vtables carry
		// the type information.
		unsafe {
			assert_eq!(subobject_offset(base, "RustConsoleVariable"), Some(0));
			assert_eq!(
				subobject_offset(interface, "RustConsoleVariable"),
				Some(ICONVAR_OFFSET as isize)
			);
		}
	}

	/// The engine `dynamic_cast`s every variable it describes, as `help` does, to
	/// a class of its own, which must fail rather than crash.
	#[cfg(target_os = "windows")]
	#[test]
	fn casts_to_the_engines_classes_fail() {
		use crate::util::rtti::TypeDescriptor;

		unsafe extern "C" {
			/// MSVC's runtime for `dynamic_cast` to a pointer.
			fn __RTDynamicCast(
				object: *mut c_void,
				vfptr_offset: c_int,
				source: *const c_void,
				target: *const c_void,
				is_reference: c_int,
			) -> *mut c_void;

			/// MSVC's runtime for `typeid`, returning the complete object's
			/// `std::type_info`.
			fn __RTtypeid(object: *mut c_void) -> *mut c_void;
		}

		fn descriptor<const N: usize>(name: [u8; N]) -> TypeDescriptor<N> {
			TypeDescriptor {
				vtable: ptr::null(),
				undecorated_name: ptr::null_mut(),
				name,
			}
		}

		let convar = descriptor(*b".?AVConVar@@\0");
		let iconvar = descriptor(*b".?AVIConVar@@\0");
		let bounded = descriptor(*b".?AVConVar_ServerBounded@@\0");
		let own = descriptor(*b".?AVRustConsoleVariable@@\0");

		let variable = prepared(c"sb_cast");
		let object = NonNull::from(&*variable);
		let base = object.as_ptr().cast::<c_void>();

		// SAFETY: The variable is live.
		let interface = unsafe { ConVarObject::interface(object) }
			.as_ptr()
			.cast::<c_void>();

		let cast = |from, source: *const c_void, target: *const c_void| {
			// SAFETY: `from` is a subobject of the live variable whose type
			// information `source` describes, as the engine's casts pass them.
			unsafe { __RTDynamicCast(from, 0, source, target, 0) }
		};

		assert!(
			cast(
				base,
				(&raw const convar).cast(),
				(&raw const bounded).cast()
			)
			.is_null()
		);
		assert!(
			cast(
				interface,
				(&raw const iconvar).cast(),
				(&raw const bounded).cast()
			)
			.is_null()
		);

		// Every cast fails, since the class claims no bases, but the type
		// information describes the whole variable from either table.
		assert!(cast(base, (&raw const convar).cast(), (&raw const own).cast()).is_null());

		for from in [base, interface] {
			// SAFETY: As above. The runtime returns the class's type descriptor,
			// whose name is terminated.
			let name = unsafe {
				CStr::from_ptr(
					__RTtypeid(from)
						.cast::<c_char>()
						.add(offset_of!(TypeDescriptor<1>, name)),
				)
			};

			assert_eq!(name, c".?AVRustConsoleVariable@@");
		}

		// SAFETY: Both are subobjects of the live variable, whose vtables carry
		// the type information.
		unsafe {
			assert_eq!(subobject_offset(base, "RustConsoleVariable"), Some(0));
			assert_eq!(
				subobject_offset(interface, "RustConsoleVariable"),
				Some(ICONVAR_OFFSET as isize)
			);
		}
	}

	#[test]
	fn defaults_are_parsed_and_replaced_strings_outlive_later_ones() {
		let variable = ConVarObject::new(c"  12.5 rounds", &HOOKS);

		assert_eq!(variable.float(), 12.5);
		assert_eq!(variable.int(), 12);
		assert_eq!(variable.string().as_c_str(), c"  12.5 rounds");
		assert_eq!(variable.default(), c"  12.5 rounds");

		let first = variable.replace_string(c"3".to_owned());

		assert!(first.changed());
		assert_eq!(first.old(), c"  12.5 rounds");

		// A change callback changing the variable again leaves the old string to
		// the first replacement.
		let second = variable.replace_string(c"3".to_owned());

		assert!(!second.changed());
		assert_eq!(second.old(), c"3");
		assert_eq!(first.old(), c"  12.5 rounds");
		assert_eq!(variable.string().as_c_str(), c"3");

		variable.set_numbers(3.0, 3);
		assert_eq!((variable.float(), variable.int()), (3.0, 3));
	}

	/// A prepared variable, as the engine would get it.
	fn prepared(name: &'static CStr) -> Box<ConVarObject> {
		let variable = Box::new(ConVarObject::new(c"1.5", &HOOKS));

		// SAFETY: The variable is live and not registered, and the tests only
		// access it on this thread.
		unsafe {
			ConVarObject::prepare(
				NonNull::from(&*variable),
				name,
				c"Help.",
				FCVAR_GAMEDLL | FCVAR_NOTIFY,
				Some(1.0),
				None,
				3,
			)
		};

		variable
	}

	/// The engine's view of a variable's primary vtable.
	fn primary_of(this: *mut sys::ConVar) -> *const sys::ConVar__bindgen_vtable {
		// SAFETY: The tests pass prepared variables.
		unsafe { (&raw const (*this)._base.vtable_).read() }.cast()
	}

	fn push(variable: NonNull<ConVarObject>, call: Call) {
		CALLS.with_borrow_mut(|calls| calls.push((variable.as_ptr(), call)));
	}

	unsafe fn record_change(variable: NonNull<ConVarObject>, value: &CStr, old_float: f32) {
		push(variable, Call::Change(value.to_owned(), old_float));
	}

	/// Records the call, and clamps values above 10.
	unsafe fn record_clamp(variable: NonNull<ConVarObject>, value: &mut f32) -> bool {
		push(variable, Call::Clamp(*value));

		let clamped = *value > 10.0;

		if clamped {
			*value = 10.0;
		}

		clamped
	}

	unsafe fn record_float(variable: NonNull<ConVarObject>, value: f32, force: bool) {
		push(variable, Call::Float(value, force));
	}

	unsafe fn record_int(variable: NonNull<ConVarObject>, value: c_int) {
		push(variable, Call::Int(value));
	}

	unsafe fn record_string(variable: NonNull<ConVarObject>, value: Option<&CStr>) {
		push(variable, Call::String(value.map(CStr::to_owned)));
	}

	#[test]
	fn the_interface_sets_values_through_the_whole_variable() {
		let variable = prepared(c"sb_interface");
		let object = NonNull::from(&*variable);

		// SAFETY: The variable is live.
		let interface = unsafe { ConVarObject::interface(object) }.as_ptr();

		// SAFETY: The slots are called as the engine calls them, on the
		// `IConVar` of a prepared variable.
		unsafe {
			let vtable = (&raw const (*interface).vtable_).read();

			((*vtable).IConVar_SetValue)(interface, c"4".as_ptr());
			((*vtable).IConVar_SetValue)(interface, ptr::null());
			((*vtable).IConVar_SetValue1)(interface, 2.5);
			((*vtable).IConVar_SetValue2)(interface, 9);

			assert_eq!(
				CStr::from_ptr(((*vtable).IConVar_GetName)(interface)),
				c"sb_interface"
			);

			// Variables keep `FCVAR_GAMEDLL`.
			assert!(((*vtable).IConVar_IsFlagSet)(interface, FCVAR_GAMEDLL));
			assert!(!((*vtable).IConVar_IsFlagSet)(interface, 1));
		}

		let object = object.as_ptr();

		assert_eq!(
			calls(),
			[
				(object, Call::String(Some(c"4".to_owned()))),
				(object, Call::String(None)),
				(object, Call::Float(2.5, false)),
				(object, Call::Int(9)),
			]
		);
	}

	#[test]
	fn the_primary_vtable_runs_the_hooks() {
		let variable = prepared(c"sb_primary");
		let this = ConVarObject::as_var(NonNull::from(&*variable)).as_ptr();
		let vtable = primary_of(this);
		let mut value = 12.0;
		let mut kept = 5.0;

		// SAFETY: The slots are called as the engine calls them, on a prepared
		// variable.
		unsafe {
			#[cfg(target_os = "windows")]
			{
				assert_eq!(((*vtable).ConVar_destructor)(this, 0), this.cast());
				assert_eq!(((*vtable).ConVar_destructor)(this, 1), this.cast());
			}

			#[cfg(not(target_os = "windows"))]
			{
				((*vtable).ConVar_complete_destructor)(this);
				((*vtable).ConVar_deleting_destructor)(this);
			}

			((*vtable).ConVar_InternalSetValue)(this, c"4".as_ptr());
			((*vtable).ConVar_InternalSetFloatValue)(this, 2.5);
			((*vtable).ConVar_InternalSetFloatValue2)(this, 2.5, true);
			((*vtable).ConVar_InternalSetIntValue)(this, 9);
			assert!(((*vtable).ConVar_ClampValue)(this, &raw mut value));
			assert!(!((*vtable).ConVar_ClampValue)(this, &raw mut kept));
			assert!(!((*vtable).ConVar_ClampValue)(this, ptr::null_mut()));
			((*vtable).ConVar_ChangeStringValue)(this, c"7".as_ptr(), 2.5);
			((*vtable).ConVar_ChangeStringValue)(this, ptr::null(), 7.0);

			assert!(!((*vtable).ConVar_IsCommand)(this));
			assert!(!((*vtable).ConVar_IsRegistered)(this));
			assert_eq!(((*vtable).ConVar_GetDLLIdentifier)(this), 3);
			assert_eq!(
				CStr::from_ptr(((*vtable).ConVar_GetHelpText)(this)),
				c"Help."
			);
			assert_eq!((&raw const (*this).m_pParent).read(), this);
			assert!((&raw const (*this).m_bHasMin).read());
			assert!(!(&raw const (*this).m_bHasMax).read());
		}

		let object = this.cast::<ConVarObject>();

		assert_eq!((value, kept), (10.0, 5.0));
		assert_eq!(
			calls(),
			[
				(object, Call::String(Some(c"4".to_owned()))),
				(object, Call::Float(2.5, false)),
				(object, Call::Float(2.5, true)),
				(object, Call::Int(9)),
				(object, Call::Clamp(12.0)),
				(object, Call::Clamp(5.0)),
				(object, Call::Change(c"7".to_owned(), 2.5)),
				(object, Call::Change(c"".to_owned(), 7.0)),
			]
		);
	}

	/// The Itanium ABI also calls `SetValue` through the primary vtable, for
	/// callers holding a `ConVar *`.
	#[cfg(not(target_os = "windows"))]
	#[test]
	fn the_primary_vtable_sets_values_without_adjusting_this() {
		let variable = prepared(c"sb_primary");
		let this = ConVarObject::as_var(NonNull::from(&*variable)).as_ptr();
		let vtable = primary_of(this);

		// SAFETY: As for the primary vtable's other slots.
		unsafe {
			((*vtable).ConVar_SetValue)(this, c"4".as_ptr());
			((*vtable).ConVar_SetValue1)(this, 2.5);
			((*vtable).ConVar_SetValue2)(this, 9);
		}

		let object = this.cast::<ConVarObject>();

		assert_eq!(
			calls(),
			[
				(object, Call::String(Some(c"4".to_owned()))),
				(object, Call::Float(2.5, false)),
				(object, Call::Int(9)),
			]
		);
	}

	#[test]
	fn values_format_as_tier1_does() {
		let long = format_float(1.0e30);

		assert_eq!(long.as_bytes().len(), 31);
		assert!(
			long.to_str()
				.unwrap()
				.starts_with("1000000015047466219876688855040")
		);
		assert_eq!(format_float(2.5).as_c_str(), c"2.500000");
		assert_eq!(format_float(f32::NEG_INFINITY).as_c_str(), c"-inf");
		assert_eq!(format_float(-f32::NAN).as_c_str(), c"-nan");
		assert_eq!(format_int(-7).as_c_str(), c"-7");
	}

	#[test]
	fn values_parse_as_c_does() {
		assert_eq!(parse_float(b"3.0"), 3.0);
		assert_eq!(parse_float(b" \t-1.5e1x"), -15.0);
		assert_eq!(parse_float(b"+2E-1"), 0.2);
		assert_eq!(parse_float(b".5"), 0.5);
		assert_eq!(parse_float(b"5."), 5.0);
		assert_eq!(parse_float(b"1e"), 1.0);
		assert_eq!(parse_float(b"1e+"), 1.0);
		assert_eq!(parse_float(b"e5"), 0.0);
		assert_eq!(parse_float(b"."), 0.0);
		assert_eq!(parse_float(b""), 0.0);
		assert_eq!(parse_float(b"1e999"), f64::INFINITY);
		assert_eq!(parse_float(b"0.01e-99999999999999999999"), 0.0);
		assert_eq!(
			parse_float(b"100000000000000000000e99999999999999999999"),
			f64::INFINITY
		);
		assert_eq!(parse_float(b"0e309"), 0.0);
		assert!(parse_float(b"-0e400").is_sign_negative());
		assert_eq!(
			parse_float(b"123456789012345678901234") as f32,
			1.234_567_9e23
		);

		assert_eq!(parse_int(b" 42 bots"), 42);
		assert_eq!(parse_int(b"-7"), -7);
		assert_eq!(parse_int(b"3.9"), 3);
		assert_eq!(parse_int(b"x"), 0);
		assert_eq!(parse_int(b"99999999999"), c_int::MAX);
		assert_eq!(parse_int(b"-99999999999"), c_int::MIN);
	}

	#[test]
	fn variable_vtables_share_their_page() {
		let variable = prepared(c"sb_page");
		let this = ConVarObject::as_var(NonNull::from(&*variable)).as_ptr();

		// SAFETY: The variable is prepared, and its fields are read without
		// forming references.
		let (primary, secondary) = unsafe {
			(
				(&raw const (*this)._base.vtable_).read().addr(),
				(&raw const (*this)._base_1.vtable_).read().addr(),
			)
		};

		// Both tables, and the type information before each, fill one page.
		assert_eq!(primary / 4096, secondary / 4096);
		assert_eq!(primary % 4096, offset_of!(Vtables, primary));
		assert_eq!(secondary % 4096, offset_of!(Vtables, secondary));
	}
}
