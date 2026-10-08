//! Tests of `ConVarObject`: the casts the engine makes on it, its values, and
//! the slots of its vtables.

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
		// SAFETY: `from` is a subobject of the live variable, whose vtable
		// carries its type information. The runtime returns the class's type
		// descriptor, whose name is terminated.
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

	// SAFETY: The slots are called as the engine calls them, on a prepared
	// variable.
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
