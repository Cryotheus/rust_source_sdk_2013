//! Tests of the loadout reapplier's re-entrancy guard and of the game events
//! it handles.

use super::*;
use crate::test_support::players::user;
use crate::test_support::server::mock_server;
use crate::tf2::attributes::trust_shipped_schema;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::ffi::c_char;
use std::ptr::NonNull;

/// A game event with a name and, optionally, a `userid`.
#[repr(C)]
struct MockEvent {
	raw: sys::IGameEvent,
	name: &'static CStr,
	user_id: Option<c_int>,
}

impl MockEvent {
	fn new(name: &'static CStr, user_id: Option<c_int>) -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = Box::leak(unsafe {
			mock_vtable::<sys::IGameEvent__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IGameEvent_GetName).write(event_name);
				(&raw mut (*vtable).IGameEvent_IsEmpty).write(event_is_empty);
				(&raw mut (*vtable).IGameEvent_GetInt).write(event_get_int);
			})
		});

		Self {
			raw: sys::IGameEvent { vtable_: vtable },
			name,
			user_id,
		}
	}

	fn event(&mut self) -> GameEvent<'_> {
		// SAFETY: The mock outlives the borrow, and is used on this thread.
		// The pointer covers the whole mock, which the callbacks read.
		unsafe { GameEvent::from_raw(NonNull::from(self).cast()) }
	}
}

#[test]
fn applying_refuses_nested_calls_and_records_them() {
	let scope = ();
	let server = mock_server(&scope);
	// SAFETY: No attribute is written: applying fails before any item is
	// reached.
	let token = unsafe { trust_shipped_schema(server) };
	let mut reapplier = LoadoutReapplier::new();
	let mut inventory = MockEvent::new(c"post_inventory_application", Some(2));
	let mut spawn = MockEvent::new(c"player_spawn", Some(2));
	let mut unknown = MockEvent::new(c"post_inventory_application", Some(4));

	reapplier.set(user(2), Loadout::new().with_wearable(def(106)).unwrap());
	assert!(!reapplier.is_applying());

	// The mock server exports no engine interface, so applying fails, and
	// the reapplier is no longer applying afterwards.
	assert!(matches!(
		// SAFETY: Without the engine's interface, no player is found, so
		// nothing is given. The same holds for every call below.
		unsafe { reapplier.apply(server, user(2), token) },
		Err(LoadoutError::Interface(_))
	));
	assert!(!reapplier.is_applying());

	// As seen from a nested listener, while an outer call applies.
	let outer = reapplier.begin(user(2)).unwrap();

	assert!(reapplier.is_applying());
	assert_eq!(
		reapplier.begin(user(5)).unwrap_err(),
		LoadoutError::Reentrant
	);
	assert!(matches!(
		// SAFETY: As above.
		unsafe { reapplier.apply(server, user(2), token) },
		Err(LoadoutError::Reentrant)
	));
	assert!(matches!(
		// SAFETY: As above.
		unsafe { reapplier.apply(server, user(3), token) },
		Err(LoadoutError::Reentrant)
	));
	assert!(matches!(
		// SAFETY: As above.
		unsafe { reapplier.on_game_event(server, inventory.event(), token) },
		Err(LoadoutError::Reentrant)
	));

	// Events the reapplier ignores are not refused, nor recorded.
	assert!(matches!(
		// SAFETY: As above.
		unsafe { reapplier.on_game_event(server, spawn.event(), token) },
		Ok(None)
	));
	assert!(matches!(
		// SAFETY: As above.
		unsafe { reapplier.on_game_event(server, unknown.event(), token) },
		Ok(None)
	));
	assert!(reapplier.is_applying(), "refusals keep the outer mark");

	// Each refused user ID is recorded once, in order, for the outer call.
	assert_eq!(*reapplier.deferred.borrow(), [5, 2, 3].map(user));

	// The guard forgets what no report took.
	drop(outer);
	assert!(!reapplier.is_applying());
	assert!(reapplier.deferred.borrow().is_empty());
}

fn def(index: u16) -> ItemDefinitionIndex {
	ItemDefinitionIndex::new(index).unwrap()
}

unsafe extern "C" fn event_get_int(
	event: *const sys::IGameEvent,
	key: *const c_char,
	default: c_int,
) -> c_int {
	// SAFETY: The wrappers pass NUL-terminated keys.
	assert_eq!(unsafe { CStr::from_ptr(key) }, USER_ID_KEY);

	// SAFETY: Every mock event's vtable belongs to a `MockEvent`.
	unsafe { (&raw const (*event.cast::<MockEvent>()).user_id).read() }.unwrap_or(default)
}

unsafe extern "C" fn event_is_empty(event: *mut sys::IGameEvent, key: *const c_char) -> bool {
	// SAFETY: As for `event_get_int`.
	let user_id = unsafe { (&raw const (*event.cast::<MockEvent>()).user_id).read() };
	// SAFETY: As for `event_get_int`.
	let key = unsafe { CStr::from_ptr(key) };

	key != USER_ID_KEY || user_id.is_none()
}

unsafe extern "C" fn event_name(event: *const sys::IGameEvent) -> *const c_char {
	// SAFETY: As for `event_get_int`.
	unsafe { (&raw const (*event.cast::<MockEvent>()).name).read() }.as_ptr()
}

#[test]
fn only_post_inventory_application_for_known_players_is_handled() {
	let scope = ();
	let server = mock_server(&scope);
	// SAFETY: No attribute is written: handling fails before any item is
	// reached.
	let token = unsafe { trust_shipped_schema(server) };
	let mut reapplier = LoadoutReapplier::default();
	let handle = |reapplier: &LoadoutReapplier, name, user_id| {
		let mut event = MockEvent::new(name, user_id);

		// SAFETY: Without the engine's interface, which the mock server does
		// not export, no player is found, so nothing is given.
		unsafe { reapplier.on_game_event(server, event.event(), token) }
	};

	// No loadout at all: nothing is asked of the engine, which the mock
	// server does not export.
	assert!(matches!(
		handle(&reapplier, c"post_inventory_application", Some(2)),
		Ok(None)
	));

	reapplier.set(user(2), Loadout::new().with_wearable(def(106)).unwrap());
	reapplier.set(user(3), Loadout::new());

	// Other events are ignored, even for players with a loadout.
	for name in [c"player_spawn", c"player_regenerate", c"post_inventory"] {
		assert!(matches!(handle(&reapplier, name, Some(2)), Ok(None)));
	}

	// Players without a loadout, or with an empty one, are ignored.
	assert!(matches!(
		handle(&reapplier, c"post_inventory_application", Some(4)),
		Ok(None)
	));
	assert!(matches!(
		handle(&reapplier, c"post_inventory_application", Some(3)),
		Ok(None)
	));

	// A malformed `userid` is an error.
	for user_id in [None, Some(0), Some(-1), Some(65_536)] {
		assert!(matches!(
			handle(&reapplier, c"post_inventory_application", user_id),
			Err(LoadoutError::InvalidUserId)
		));
	}

	// A player with a loadout goes on to be resolved, which needs the
	// engine's interface.
	assert!(matches!(
		handle(&reapplier, c"post_inventory_application", Some(2)),
		Err(LoadoutError::Interface(_))
	));
	assert!(!reapplier.is_applying());
}
