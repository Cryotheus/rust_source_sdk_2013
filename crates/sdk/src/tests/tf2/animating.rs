//! Tests of `crate::tf2::animating`: attachment lookups through an animating
//! entity's native member.

use super::*;
use crate::test_support::entities::MockEntity;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use sdk_raw::tf2::script_binding::STRING;
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_void};
use std::ptr::{NonNull, null_mut};

thread_local! {
	/// The index the adapter returns.
	static INDEX: Cell<c_int> = const { Cell::new(0) };

	/// The entities and names the adapter was called with.
	static LOOKED_UP: RefCell<Vec<(*mut c_void, CString)>> = const { RefCell::new(Vec::new()) };

	/// Whether the adapter reports failure.
	static REJECTS: Cell<bool> = const { Cell::new(false) };
}

/// A mock entity whose `GetScriptDesc` returns the descriptor
/// [`set_script_description`] sets, for as long as the mocks live, with the
/// vtable it then uses, which must outlive it.
fn described(mock: &mut MockEntity) -> (Entity<'static>, Vec<*const ()>) {
	let pointer = mock.as_ptr();
	let mut vtable = vec![std::ptr::null::<()>(); SCRIPT_DESCRIPTION_SLOT + 1];

	// The mock's own vtable answers every slot before the descriptor's, which
	// include the datamap's.
	// SAFETY: A mock entity starts with the pointer to its vtable, which has
	// slots up to TF2's `Teleport`, past `GetScriptDesc`.
	unsafe {
		let original = pointer.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(SCRIPT_DESCRIPTION_SLOT) {
			*entry = original.add(slot).read();
		}
	}

	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();
	// SAFETY: A mock entity starts with the pointer to its vtable, and the
	// caller keeps the new vtable alive while the entity is used.
	unsafe { pointer.cast::<*const *const ()>().write(vtable.as_ptr()) };

	// SAFETY: Mock entities are leaked, and their vtable answers what the
	// lookup calls of an entity.
	(
		unsafe { Entity::from_raw(NonNull::new(pointer).unwrap()) },
		vtable,
	)
}

/// The adapter of `CBaseAnimating::LookupAttachment`, which records the
/// entity and the name, and returns [`INDEX`], unless [`REJECTS`] is set.
unsafe extern "C" fn lookup_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 1);

	if REJECTS.get() {
		return false;
	}

	// SAFETY: The binding declares one string parameter, so the caller passes
	// one string variant, NUL-terminated.
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

	LOOKED_UP.with_borrow_mut(|looked_up| looked_up.push((object, name.to_owned())));
	// SAFETY: The binding returns an integer, so the caller passes a writable
	// result.
	unsafe { result.write(sdk_raw::tf2::script_binding::int(INDEX.get())) };
	true
}

#[test]
fn attachments_are_looked_up_through_the_animating_member() {
	let scope = ();
	let server = mock_server(&scope);
	let mut parameters = [STRING];
	let mut bindings = [member_binding(
		c"LookupAttachment",
		binding::INT,
		&mut parameters,
		Some(lookup_adapter),
	)];
	let mut animating = class_description(c"CBaseAnimating", &mut bindings, null_mut());

	// Changed below only through the pointer `call` reads it by.
	let binding = animating.m_FunctionBindings.m_Memory.m_pMemory;
	let mut prop = class_description(c"CDynamicProp", &mut [], &raw mut animating);
	let mut mock = MockEntity::new(1);
	let (entity, _vtable) = described(&mut mock);

	set_script_description(&raw mut prop);
	LOOKED_UP.take();

	INDEX.set(3);
	assert_eq!(lookup_attachment(server, entity, c"head"), Ok(Some(3)));
	assert_eq!(
		LOOKED_UP.take(),
		[(entity.as_ptr().cast(), c"head".to_owned())]
	);

	// The game's 0 is no attachment of the name, or no model.
	INDEX.set(0);
	assert_eq!(lookup_attachment(server, entity, c"head"), Ok(None));

	INDEX.set(256);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::OutOfRange(256))
	);
	INDEX.set(-1);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::OutOfRange(-1))
	);

	REJECTS.set(true);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::Rejected)
	);
	REJECTS.set(false);

	// An entity marked for deletion is not used.
	mock.set_eflags(1);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::MarkedForDeletion)
	);
	mock.set_eflags(0);
	LOOKED_UP.take();

	// Another signature is refused before the call.
	// SAFETY: `binding` points to the binding in `bindings`, which is alive,
	// and which the lookup only reads through the same pointer.
	unsafe { (*binding).m_desc.m_ReturnType = binding::FLOAT };
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::UnsupportedMethod)
	);
	assert!(LOOKED_UP.take().is_empty());
	// SAFETY: As above.
	unsafe { (*binding).m_desc.m_ReturnType = binding::INT };

	// An entity that is not animating has no such member.
	let mut entity_description = class_description(c"CBaseEntity", &mut [], null_mut());

	set_script_description(&raw mut entity_description);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::UnsupportedMethod)
	);
	assert!(LOOKED_UP.take().is_empty());
	set_script_description(null_mut());
}
