//! Tests of finding a class name's entity factory through the game's entity
//! factory dictionary.

use super::*;
use crate::Game;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char};
use std::ptr::null_mut;

thread_local! {
	/// The dictionary `GetEntityFactoryDictionary` returns.
	static DICTIONARY: Cell<*mut sys::IEntityFactoryDictionary> = const { Cell::new(null_mut()) };

	/// The factory `FindFactory` finds for `known_class`.
	static FACTORY: Cell<*mut sys::IEntityFactory> = const { Cell::new(null_mut()) };

	/// Every class name `FindFactory` was asked for, in order.
	static NAMES: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// `IServerTools::GetEntityFactoryDictionary`, which returns [`DICTIONARY`].
unsafe extern "C" fn entity_factory_dictionary(
	_: *mut sys::IServerTools,
) -> *mut sys::IEntityFactoryDictionary {
	DICTIONARY.get()
}

#[test]
fn factories_are_found_by_class_name() {
	// SAFETY: The vtables hold only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and each patch only writes a slot of the
	// vtable being built.
	let (tools_vtable, dictionary_vtable, factory_vtable) = unsafe {
		(
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_GetEntityFactoryDictionary)
						.write(entity_factory_dictionary)
				},
			),
			mock_vtable::<sys::IEntityFactoryDictionary__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IEntityFactoryDictionary_FindFactory).write(find_factory)
				},
			),
			mock_vtable::<sys::IEntityFactory__bindgen_vtable>(
				unexpected_call as *const (),
				|_| {},
			),
		)
	};

	let mut interface = sys::IServerTools {
		vtable_: &raw const *tools_vtable,
	};

	let mut dictionary = sys::IEntityFactoryDictionary {
		vtable_: &raw const *dictionary_vtable,
	};

	let mut factory = sys::IEntityFactory {
		vtable_: &raw const *factory_vtable,
	};

	DICTIONARY.set(&raw mut dictionary);
	FACTORY.set(&raw mut factory);
	NAMES.take();

	// SAFETY: The mocks outlive the handle.
	let tools =
		unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };

	assert_eq!(
		tools
			.entity_factory(c"known_class")
			.map(EntityFactory::as_ptr),
		Some(&raw mut factory)
	);
	assert_eq!(
		tools
			.entity_factory(c"Known_Class")
			.map(EntityFactory::as_ptr),
		Some(&raw mut factory)
	);
	assert_eq!(tools.entity_factory(c"unknown_class"), None);

	assert_eq!(
		NAMES.take(),
		[c"known_class", c"Known_Class", c"unknown_class"].map(CStr::to_owned)
	);

	// A game DLL without a dictionary has no factories.
	DICTIONARY.set(null_mut());
	assert_eq!(tools.entity_factory(c"known_class"), None);
	assert_eq!(NAMES.take(), Vec::<CString>::new());
}

/// `IEntityFactoryDictionary::FindFactory`, which finds [`FACTORY`] for
/// `known_class`, ignoring case as the game does, and nothing for any other
/// name.
unsafe extern "C" fn find_factory(
	_: *mut sys::IEntityFactoryDictionary,
	class_name: *const c_char,
) -> *mut sys::IEntityFactory {
	// SAFETY: The wrapper passes a NUL-terminated name.
	let class_name = unsafe { CStr::from_ptr(class_name) }.to_owned();
	let known = class_name.to_bytes().eq_ignore_ascii_case(b"known_class");

	NAMES.with_borrow_mut(|names| names.push(class_name));

	if known { FACTORY.get() } else { null_mut() }
}
