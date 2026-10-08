//! Tests of the engine's game event manager (`IGameEventManager2`): listeners
//! it keeps and calls by address, and the events created, decoded, fired, and
//! freed across the boundary.

use sdk_raw::bitbuf::BfRead;
use sdk_raw::interfaces::game_event::MAX_EVENT_BITS;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::bitbuf::BitWriter;
use source_sdk_2013::interfaces::GameEventManager;
use source_sdk_2013::interfaces::game_event::{GameEvent, GameEventHandler, GameEventListener};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::{NonNull, null_mut};

thread_local! {
	/// The manager `AddListener` or `RemoveListener` last received.
	static RECEIVED_MANAGER: Cell<*mut sys::IGameEventManager2> = const { Cell::new(null_mut()) };

	/// The listener `AddListener` last received, which the manager delivers
	/// events to.
	static ADDED_LISTENER: Cell<*mut sys::IGameEventListener2> = const { Cell::new(null_mut()) };

	/// Whether `AddListener` last registered a server-side listener.
	static RECEIVED_SERVER_SIDE: Cell<bool> = const { Cell::new(false) };

	/// The listener `RemoveListener` last received.
	static REMOVED_LISTENER: Cell<*mut sys::IGameEventListener2> = const { Cell::new(null_mut()) };

	/// How many events `FreeEvent` freed.
	static FREED: Cell<usize> = const { Cell::new(0) };

	/// How many events `FireEvent` fired.
	static FIRED: Cell<usize> = const { Cell::new(0) };

	/// How many times `UnserializeEvent` was called.
	static UNSERIALIZED: Cell<usize> = const { Cell::new(0) };

	/// The names of the events the handler received, in order.
	static DELIVERED: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// The only event the manager creates, a `player_regenerate`.
	static EVENT: *mut sys::IGameEvent = {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IGameEvent__bindgen_vtable>(unexpected_call as *const (), |vtable| {
				(&raw mut (*vtable).IGameEvent_GetName).write(event_name);
				(&raw mut (*vtable).IGameEvent_IsEmpty).write(event_is_empty);
				(&raw mut (*vtable).IGameEvent_GetInt).write(event_get_int);
			})
		};

		leak(sys::IGameEvent {
			vtable_: Box::leak(vtable),
		})
	};
}

/// A listener built in a constant, as plugins build theirs for statics.
const RECORDING_LISTENER: GameEventListener<RecordingHandler> =
	GameEventListener::new(RecordingHandler);

/// Records the names of the events it receives, after checking their data.
#[derive(Debug)]
struct RecordingHandler;

impl GameEventHandler for RecordingHandler {
	fn fire_game_event(&self, event: GameEvent<'_>) {
		assert_eq!(event.get_int(c"userid"), Some(7));
		assert_eq!(event.get_int(c"attacker"), None);
		DELIVERED.with_borrow_mut(|delivered| delivered.push(event.name().to_owned()));
	}
}

/// `IGameEventManager2::CreateEvent`, which creates only `player_regenerate`.
unsafe extern "C" fn create_event(
	_: *mut sys::IGameEventManager2,
	name: *const c_char,
	force: bool,
) -> *mut sys::IGameEvent {
	assert!(!force);

	// SAFETY: The wrappers pass NUL-terminated names.
	if unsafe { CStr::from_ptr(name) } == c"player_regenerate" {
		EVENT.with(|&event| event)
	} else {
		null_mut()
	}
}

/// `IGameEvent::GetInt`, which reads 7 for every key it holds.
unsafe extern "C" fn event_get_int(_: *const sys::IGameEvent, _: *const c_char, _: c_int) -> c_int {
	7
}

/// `IGameEvent::IsEmpty`, for an event that holds `userid` only.
unsafe extern "C" fn event_is_empty(_: *mut sys::IGameEvent, key: *const c_char) -> bool {
	// SAFETY: The wrappers pass NUL-terminated keys.
	let key = unsafe { CStr::from_ptr(key) };

	key != c"userid"
}

/// `IGameEvent::GetName`.
unsafe extern "C" fn event_name(_: *const sys::IGameEvent) -> *const c_char {
	c"player_regenerate".as_ptr()
}

/// `IGameEventManager2::FireEvent`, which delivers the event to the listener
/// at the address `AddListener` received, then frees it, as the engine does.
unsafe extern "C" fn fire_event(
	_: *mut sys::IGameEventManager2,
	event: *mut sys::IGameEvent,
	dont_broadcast: bool,
) -> bool {
	assert!(dont_broadcast);
	FIRED.set(FIRED.get() + 1);

	let listener = ADDED_LISTENER.get();

	// SAFETY: The listener is registered, and so still alive, and the event
	// is live for the call.
	unsafe { ((*(*listener).vtable_).IGameEventListener2_FireGameEvent)(listener, event) };
	true
}

/// `IGameEventManager2::FreeEvent`, which counts the events freed.
unsafe extern "C" fn free_event(_: *mut sys::IGameEventManager2, _: *mut sys::IGameEvent) {
	FREED.set(FREED.get() + 1);
}

#[test]
fn listeners_register_by_address_and_receive_fired_events() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IGameEventManager2__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IGameEventManager2_AddListener).write(record_add_listener);
				(&raw mut (*vtable).IGameEventManager2_RemoveListener)
					.write(record_remove_listener);
				(&raw mut (*vtable).IGameEventManager2_CreateEvent).write(create_event);
				(&raw mut (*vtable).IGameEventManager2_FreeEvent).write(free_event);
				(&raw mut (*vtable).IGameEventManager2_FireEvent).write(fire_event);
			},
		)
	};
	let interface = leak(sys::IGameEventManager2 {
		vtable_: Box::leak(vtable),
	});

	export(Module::Engine, GameEventManager::VERSION, interface);

	let scope = ();
	let manager = mock_server(&scope).game_events().unwrap();
	let listener = std::pin::pin!(RECORDING_LISTENER);
	let listener = listener.into_ref();

	// SAFETY: The listener stays pinned on the stack, and is removed before
	// the test returns.
	assert!(unsafe { manager.add_listener(listener, c"player_regenerate", true) }.is_ok());
	assert_eq!(RECEIVED_MANAGER.get(), interface);
	assert!(RECEIVED_SERVER_SIDE.get());

	// The manager keeps the listener's address.
	let added = ADDED_LISTENER.get();
	let start = (&raw const *listener).addr();

	assert!((start..start + size_of_val(&*listener)).contains(&added.addr()));

	assert_eq!(
		// SAFETY: As above.
		unsafe { manager.add_listener(listener, c"not_an_event", true) }
			.unwrap_err()
			.to_string(),
		"could not listen for `not_an_event`; no such game event is registered"
	);

	// Dropping an unfired event frees it; firing hands it to the engine.
	drop(manager.create_event(c"player_regenerate").unwrap());
	assert_eq!(FREED.get(), 1);
	assert!(manager.create_event(c"not_an_event").is_err());

	let event = manager.create_event(c"player_regenerate").unwrap();

	assert_eq!(event.as_event().name(), c"player_regenerate");
	event.fire(false).unwrap();

	assert_eq!((FIRED.get(), FREED.get()), (1, 1));
	assert_eq!(DELIVERED.take(), [c"player_regenerate".to_owned()]);

	manager.remove_listener(listener);
	assert_eq!(REMOVED_LISTENER.get(), added);
}

#[test]
fn serialized_events_are_decoded_and_freed_by_their_owner() {
	// SAFETY: As above.
	let vtable = unsafe {
		mock_vtable::<sys::IGameEventManager2__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IGameEventManager2_UnserializeEvent).write(unserialize_event);
				(&raw mut (*vtable).IGameEventManager2_FreeEvent).write(free_event);
			},
		)
	};
	let interface = leak(sys::IGameEventManager2 {
		vtable_: Box::leak(vtable),
	});

	export(Module::Engine, GameEventManager::VERSION, interface);

	let scope = ();
	let manager = mock_server(&scope).game_events().unwrap();
	let mut data = BitWriter::new();

	data.write_ubits(7, MAX_EVENT_BITS);
	data.write_u16(5);

	let event = manager.unserialize_event(&data).unwrap();

	assert_eq!(event.as_event().name(), c"player_regenerate");
	drop(event);
	assert_eq!((UNSERIALIZED.get(), FREED.get()), (1, 1));

	// An ID the manager has no description of.
	let mut unknown = BitWriter::new();

	unknown.write_ubits(8, MAX_EVENT_BITS);
	unknown.write_u16(5);
	assert!(manager.unserialize_event(&unknown).is_none());
	assert_eq!((UNSERIALIZED.get(), FREED.get()), (2, 1));

	// Bits that end before the event's field: the event is freed.
	let mut short = BitWriter::new();

	short.write_ubits(7, MAX_EVENT_BITS);
	short.write_u8(5);
	assert!(manager.unserialize_event(&short).is_none());
	assert_eq!((UNSERIALIZED.get(), FREED.get()), (3, 2));

	// Fewer bits than an ID: the manager is not asked.
	let mut tiny = BitWriter::new();

	tiny.write_ubits(7, MAX_EVENT_BITS - 1);
	assert!(manager.unserialize_event(&tiny).is_none());
	assert_eq!((UNSERIALIZED.get(), FREED.get()), (3, 2));
}

/// `IGameEventManager2::AddListener`, which records its arguments and knows
/// only `player_regenerate`.
unsafe extern "C" fn record_add_listener(
	manager: *mut sys::IGameEventManager2,
	listener: *mut sys::IGameEventListener2,
	name: *const c_char,
	server_side: bool,
) -> bool {
	RECEIVED_MANAGER.set(manager);
	ADDED_LISTENER.set(listener);
	RECEIVED_SERVER_SIDE.set(server_side);

	// SAFETY: The wrappers pass NUL-terminated names.
	unsafe { CStr::from_ptr(name) == c"player_regenerate" }
}

/// `IGameEventManager2::RemoveListener`, which records its arguments.
unsafe extern "C" fn record_remove_listener(
	manager: *mut sys::IGameEventManager2,
	listener: *mut sys::IGameEventListener2,
) {
	RECEIVED_MANAGER.set(manager);
	REMOVED_LISTENER.set(listener);
}

/// `IGameEventManager2::UnserializeEvent`, which knows only the ID 7, a
/// `player_regenerate` of one 16-bit field. As the engine does, it marks the
/// buffer overflowed when the field does not fit, and returns the event
/// anyway.
unsafe extern "C" fn unserialize_event(
	_: *mut sys::IGameEventManager2,
	buffer: *mut sys::bf_read,
) -> *mut sys::IGameEvent {
	UNSERIALIZED.set(UNSERIALIZED.get() + 1);

	// SAFETY: The wrapper passes a live buffer of its own, which describes the
	// bits it holds for the call.
	let buffer = unsafe { BfRead::from_sys(NonNull::new(buffer).unwrap()).as_mut() };

	// SAFETY: As above.
	let bits = unsafe { buffer.unread_bits(buffer.data_bits - buffer.cur_bit) }.unwrap();
	let bits = BitWriter::from(bits);
	let mut reader = bits.reader();

	if reader.read_ubits(MAX_EVENT_BITS).unwrap() != 7 {
		return null_mut();
	}

	if reader.read_u16().is_err() {
		buffer.overflow = 1;
	}

	EVENT.with(|&event| event)
}
