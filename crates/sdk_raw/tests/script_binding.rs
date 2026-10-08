#![cfg(feature = "tf2")]
//! Tests of calling TF2 entities' native VScript bindings.

use source_sdk_2013_raw::test_support::{mock_vtable, unexpected_call, utl_vector};
use source_sdk_2013_raw::tf2::script_binding::{
	BindingError, FLOAT, SF_MEMBER_FUNC, STRING, SV_FREE, VOID, call, call_string, float, int,
	string,
};
use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::mem::zeroed;
use std::ptr::{NonNull, null, null_mut};

/// A fake entity, whose vtable's `GetScriptDesc` returns `description`.
#[repr(C)]
struct Object {
	vtable: *const sys::CBaseEntity__bindgen_vtable,
	description: *mut sys::ScriptClassDesc_t,
	calls: Cell<usize>,
	/// The string [`string_adapter`] returns.
	text: Cell<*const c_char>,
	/// The variant flags [`string_adapter`] returns.
	flags: Cell<u16>,
}

impl Object {
	/// An object with no calls yet, returning a null string.
	fn new(
		vtable: &sys::CBaseEntity__bindgen_vtable,
		description: *mut sys::ScriptClassDesc_t,
	) -> Self {
		Self {
			vtable,
			description,
			calls: Cell::new(0),
			text: Cell::new(null()),
			flags: Cell::new(0),
		}
	}
}

unsafe extern "C" fn adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: i32,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(function.val_0, 0x1234);
	assert_eq!(count, 1);

	// SAFETY: `call` passes the test's live object.
	let object = unsafe { &*object.cast::<Object>() };

	object.calls.set(object.calls.get() + 1);

	if !result.is_null() {
		// SAFETY: The one argument is a float, as the binding declares, and
		// the result is writable.
		unsafe { result.write(float((*arguments).__bindgen_anon_1.m_float * 2.0)) };
	}

	true
}

/// A fake entity vtable, whose only method is [`get_description`].
fn entity_vtable() -> Box<sys::CBaseEntity__bindgen_vtable> {
	// SAFETY: The generated vtable consists of function pointer slots, and
	// `unexpected_call` aborts the test if any other slot is called.
	unsafe {
		mock_vtable(
			unexpected_call as *const (),
			|vtable: *mut sys::CBaseEntity__bindgen_vtable| {
				(&raw mut (*vtable).CBaseEntity_GetScriptDesc).write(get_description);
			},
		)
	}
}

/// A fake `GetScriptDesc`, returning the object's description.
unsafe extern "C" fn get_description(object: *mut sys::CBaseEntity) -> *mut sys::ScriptClassDesc_t {
	// SAFETY: The tests call this only for a live `Object`.
	unsafe { (*object.cast::<Object>()).description }
}

#[test]
fn native_adapter_receives_typed_arguments_and_void_has_no_return_pointer() {
	let mut parameters = [FLOAT];
	// SAFETY: All-zero bindings are null and empty.
	let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
	bindings[0].m_desc.m_pszScriptName = c"SetValue".as_ptr();
	bindings[0].m_desc.m_ReturnType = FLOAT;
	bindings[0].m_desc.m_Parameters = utl_vector(&mut parameters);
	bindings[0].m_flags = SF_MEMBER_FUNC;
	bindings[0].m_pfnBinding = Some(adapter);
	bindings[0].m_pFunction.val_0 = 0x1234;
	// SAFETY: An all-zero class descriptor is null and empty.
	let mut base: sys::ScriptClassDesc_t = unsafe { zeroed() };
	base.m_pszClassname = c"Base".as_ptr();
	base.m_FunctionBindings = utl_vector(&mut bindings);
	// The binding and the object are changed below only through the
	// pointers `call` reads them by, since writing through the locals would
	// invalidate those pointers.
	let binding = base.m_FunctionBindings.m_Memory.m_pMemory;
	// SAFETY: An all-zero class descriptor is null and empty.
	let mut derived: sys::ScriptClassDesc_t = unsafe { zeroed() };
	derived.m_pszClassname = c"Derived".as_ptr();
	derived.m_pBaseDesc = &raw mut base;
	let vtable = entity_vtable();
	let mut object = Object::new(&vtable, &raw mut derived);
	let object = NonNull::from(&mut object);
	let entity = object.cast();
	let calls = || {
		// SAFETY: The object is live, and only read through shared cells.
		unsafe { object.as_ref() }.calls.get()
	};

	// SAFETY: The fake entity, its descriptors and the adapter are live, and
	// the adapter accepts one float.
	unsafe {
		let value = call(entity, c"Base", c"SetValue", &mut [float(3.0)], FLOAT).unwrap();
		assert_eq!(value.__bindgen_anon_1.m_float, 6.0);
		assert_eq!(calls(), 1);
		assert_eq!(
			call(entity, c"Base", c"SetValue", &mut [int(3)], FLOAT).err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(
			call(entity, c"Base", c"SetValue", &mut [], FLOAT).err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(
			call(entity, c"Other", c"SetValue", &mut [float(3.0)], FLOAT).err(),
			Some(BindingError::Unavailable)
		);
		assert_eq!(calls(), 1);
		(*binding).m_desc.m_ReturnType = VOID;
		call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).unwrap();
		assert_eq!(calls(), 2);
		(*binding).m_flags = 0;
		assert_eq!(
			call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).err(),
			Some(BindingError::SignatureMismatch)
		);
		(*object.as_ptr()).description = null_mut();
		assert_eq!(
			call(entity, c"Base", c"SetValue", &mut [float(3.0)], VOID).err(),
			Some(BindingError::Unavailable)
		);
	}
}

/// A member adapter for `const char *Object::GetText()`. Like the SDK's
/// `*pReturn = const char *`, it stores the pointer without a copy.
unsafe extern "C" fn string_adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	_: *mut sys::ScriptVariant_t,
	count: i32,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(function.val_0, 0x5678);
	assert_eq!(count, 0);
	assert!(!result.is_null());

	// SAFETY: `call` passes the test's live object.
	let object = unsafe { &*object.cast::<Object>() };

	object.calls.set(object.calls.get() + 1);

	// A borrowed string, pointing at the object's text.
	let mut value = string(c"");
	value.__bindgen_anon_1.m_pszString = object.text.get();
	value.m_flags = object.flags.get();

	// SAFETY: A string method gets a writable result.
	unsafe { result.write(value) };
	true
}

#[test]
fn string_results_are_copied_before_returning_and_owned_strings_refused() {
	// SAFETY: All-zero bindings are null and empty.
	let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };
	bindings[0].m_desc.m_pszScriptName = c"GetText".as_ptr();
	bindings[0].m_desc.m_ReturnType = STRING;
	bindings[0].m_flags = SF_MEMBER_FUNC;
	bindings[0].m_pfnBinding = Some(string_adapter);
	bindings[0].m_pFunction.val_0 = 0x5678;
	// SAFETY: An all-zero class descriptor is null and empty.
	let mut description: sys::ScriptClassDesc_t = unsafe { zeroed() };
	description.m_pszClassname = c"Base".as_ptr();
	description.m_FunctionBindings = utl_vector(&mut bindings);
	// Changed below only through the pointer `call` reads it by.
	let binding = description.m_FunctionBindings.m_Memory.m_pMemory;
	let vtable = entity_vtable();
	let mut text = *b"effects/jarate_overlay\0";
	// Written only through this pointer, which the object also returns.
	let storage = text.as_mut_ptr();
	let mut object = Object::new(&vtable, &raw mut description);
	object.text.set(storage.cast_const().cast());
	let object = NonNull::from(&mut object);
	let entity = object.cast();

	// SAFETY: The object is live, and only used through shared cells.
	let object = unsafe { object.as_ref() };

	// SAFETY: The fake entity, its descriptor and the adapter are live, and
	// the adapter returns static or live, terminated strings.
	unsafe {
		let copied = call_string(entity, c"Base", c"GetText", &mut []).unwrap();
		// The game may change its storage once the call returns.
		storage.write(b'X');
		assert_eq!(copied.as_deref(), Some(c"effects/jarate_overlay"));
		assert_eq!(
			call_string(entity, c"Base", c"GetText", &mut [])
				.unwrap()
				.as_deref(),
			Some(c"Xffects/jarate_overlay")
		);
		assert_eq!(object.calls.get(), 2);

		object.text.set(c"".as_ptr());
		assert_eq!(
			call_string(entity, c"Base", c"GetText", &mut []),
			Ok(Some(CString::default()))
		);
		object.text.set(null());
		assert_eq!(call_string(entity, c"Base", c"GetText", &mut []), Ok(None));
		// `call` itself passes the method's pointer through without a copy.
		let overlay = c"effects/imcookin";
		object.text.set(overlay.as_ptr());
		let result = call(entity, c"Base", c"GetText", &mut [], STRING).unwrap();
		assert_eq!(result.__bindgen_anon_1.m_pszString, overlay.as_ptr());
		assert_eq!(object.calls.get(), 5);

		// An owned string has already been returned, but cannot be freed.
		object.flags.set(SV_FREE);
		assert_eq!(
			call_string(entity, c"Base", c"GetText", &mut []),
			Err(BindingError::SignatureMismatch)
		);
		assert_eq!(
			call(entity, c"Base", c"GetText", &mut [], STRING).err(),
			Some(BindingError::SignatureMismatch)
		);
		assert_eq!(object.calls.get(), 7);

		object.flags.set(0);
		(*binding).m_desc.m_ReturnType = FLOAT;
		assert_eq!(
			call_string(entity, c"Base", c"GetText", &mut []),
			Err(BindingError::SignatureMismatch)
		);
		assert_eq!(object.calls.get(), 7);
	}
}
