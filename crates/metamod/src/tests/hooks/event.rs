//! Tests of `crate::hooks::event`: hooks of `FireEvent` on mock game event
//! managers, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::abi::VTABLE_SLOT_SIZE;
use source_sdk_2013::raw::vtable_slot;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

/// The slot of `FreeEvent`, which the hook frees blocked events with.
const FREE_EVENT_SLOT: usize = vtable_slot!(
	sys::IGameEventManager2__bindgen_vtable,
	IGameEventManager2_FreeEvent
);

thread_local! {
	/// What happened during the calls since the last [`fire`], in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
}

/// An event of a C++ class, which knows its name.
#[repr(C)]
struct Event {
	vtable: *mut *mut c_void,
	name: &'static CStr,
}

impl Event {
	fn new(name: &'static CStr) -> Box<Self> {
		let vtable = mock_vtable::<sys::IGameEvent__bindgen_vtable>(&[
			(
				vtable_slot!(sys::IGameEvent__bindgen_vtable, IGameEvent_GetName),
				event_name as *mut c_void,
			),
			(
				vtable_slot!(sys::IGameEvent__bindgen_vtable, IGameEvent_SetInt),
				event_set_int as *mut c_void,
			),
		]);

		Box::new(Self { vtable, name })
	}

	fn ptr(&mut self) -> *mut sys::IGameEvent {
		ptr::from_mut(self).cast()
	}
}

/// A game event manager of a C++ class, as far as hooks know it.
#[repr(C)]
struct Manager {
	vtable: *mut *mut c_void,
}

impl Manager {
	fn new() -> Box<Self> {
		let vtable = mock_vtable::<sys::IGameEventManager2__bindgen_vtable>(&[
			(FIRE_EVENT_SLOT, manager_fire as *mut c_void),
			(FREE_EVENT_SLOT, manager_free as *mut c_void),
		]);

		Box::new(Self { vtable })
	}

	fn ptr(&mut self) -> NonNull<sys::IGameEventManager2> {
		NonNull::from(self).cast()
	}
}

/// Something the manager, an event, or the callback noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
	/// The callback saw the event of this name, broadcast or not.
	Callback(&'static str, bool),

	/// The manager fired the event at this address, with `bDontBroadcast`.
	Fired(usize, bool),

	/// The manager freed the event at this address.
	Freed(usize),

	/// The event of this name had this key set to this value.
	SetInt(&'static str, &'static str, c_int),
}

/// The mock event's `GetName`.
unsafe extern "C" fn event_name(this: *const sys::IGameEvent) -> *const c_char {
	// SAFETY: Only mock events have this vtable.
	unsafe { (*this.cast::<Event>()).name.as_ptr() }
}

/// The mock event's `SetInt`, which notes the key and value.
unsafe extern "C" fn event_set_int(this: *mut sys::IGameEvent, key: *const c_char, value: c_int) {
	// SAFETY: Only mock events have this vtable, and the keys are literals.
	let (name, key) = unsafe { ((*this.cast::<Event>()).name, CStr::from_ptr(key)) };
	let leak = |text: &CStr| &*String::leak(text.to_str().unwrap().to_owned());

	SEEN.with_borrow_mut(|seen| seen.push(Seen::SetInt(leak(name), leak(key), value)));
}

#[test]
fn events_an_earlier_hook_blocked_reach_neither_the_callback_nor_free_event() {
	on_both(|harness| {
		let api = harness.api();
		let mut manager = Manager::new();
		let mut blocked = Event::new(c"blocked");

		fn block(_call: &HookCall<'_, FireEvent>) -> HookAction<bool> {
			HookAction::Supersede(false)
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				FIRE_EVENT,
				HookTarget::instance(manager.ptr()),
				HookTiming::Pre,
				&block,
			)
			.unwrap();
			api.install_fire_event(manager.ptr(), tf2_binding(no_interfaces), on_fire)
				.unwrap();
		}

		assert_eq!(
			fire(harness, &mut manager, Some(&mut blocked), true),
			(false, vec![])
		);
	});
}

#[test]
fn events_reach_the_callback_and_blocked_ones_are_freed_after_the_call() {
	on_both(|harness| {
		let api = harness.api();
		let mut manager = Manager::new();
		let mut plain = Event::new(c"plain");
		let mut edited = Event::new(c"edited");
		let mut blocked = Event::new(c"blocked");
		let blocked_address = blocked.ptr().addr();

		// SAFETY: The mock manager has `FireEvent` and `FreeEvent` at their
		// slots, and is leaked with its vtable.
		unsafe { api.install_fire_event(manager.ptr(), tf2_binding(no_interfaces), on_fire) }
			.unwrap();

		// A second hook is refused, so that each event is decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_fire_event(manager.ptr(), tf2_binding(no_interfaces), on_fire) },
			Err(HookError::AlreadyInstalled)
		));

		for broadcast in [true, false] {
			assert_eq!(
				fire(harness, &mut manager, Some(&mut plain), broadcast),
				(
					true,
					vec![
						Seen::Callback("plain", broadcast),
						Seen::Fired(plain.ptr().addr(), !broadcast)
					]
				)
			);
		}

		// The edit reaches the event before the manager fires it.
		assert_eq!(
			fire(harness, &mut manager, Some(&mut edited), true),
			(
				true,
				vec![
					Seen::Callback("edited", true),
					Seen::SetInt("edited", "team", 254),
					Seen::Fired(edited.ptr().addr(), false)
				]
			)
		);

		// A blocked event is not fired, and is freed once, after the call.
		assert_eq!(
			fire(harness, &mut manager, Some(&mut blocked), true),
			(
				false,
				vec![
					Seen::Callback("blocked", true),
					Seen::Freed(blocked_address)
				]
			)
		);

		// A null event reaches the manager only.
		assert_eq!(
			fire(harness, &mut manager, None, true),
			(false, vec![Seen::Fired(0, false)])
		);
	});
}

/// Fires `event`, or a null event, through `manager`'s hooked vtable, and
/// returns the result and what happened.
fn fire(
	harness: &Harness,
	manager: &mut Manager,
	event: Option<&mut Event>,
	broadcast: bool,
) -> (bool, Vec<Seen>) {
	let event = event.map_or(ptr::null_mut(), Event::ptr);

	SEEN.take();

	let fired =
		harness.call::<FireEvent>(manager.ptr().as_ptr(), FIRE_EVENT_SLOT, (event, !broadcast));

	(fired, SEEN.take())
}

/// The mock manager's `FireEvent`, which notes the event and fires all but
/// null ones.
unsafe extern "C" fn manager_fire(
	_this: *mut sys::IGameEventManager2,
	event: *mut sys::IGameEvent,
	dont_broadcast: bool,
) -> bool {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Fired(event.addr(), dont_broadcast)));
	!event.is_null()
}

/// The mock manager's `FreeEvent`, which notes the event.
unsafe extern "C" fn manager_free(
	_this: *mut sys::IGameEventManager2,
	event: *mut sys::IGameEvent,
) {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Freed(event.addr())));
}

/// A leaked vtable of `V`'s size, whose slots hold `functions` at their
/// indices, and [`unexpected_call`] elsewhere.
fn mock_vtable<V>(functions: &[(usize, *mut c_void)]) -> *mut *mut c_void {
	let mut slots = vec![unexpected_call as *mut c_void; size_of::<V>() / VTABLE_SLOT_SIZE];

	for &(slot, function) in functions {
		slots[slot] = function;
	}

	Vec::leak(slots).as_mut_ptr()
}

/// The callback, which notes what it saw, edits `edited`, and blocks
/// `blocked`.
fn on_fire(_server: Server<'_>, fired: FiredEvent<'_>) -> FireEventAction {
	let name = match fired.event().name().to_bytes() {
		b"plain" => "plain",
		b"edited" => "edited",
		b"blocked" => "blocked",

		_ => {
			expect(false, "the callback saw another event");
			return FireEventAction::Continue;
		}
	};

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Callback(name, fired.broadcast())));

	match name {
		"edited" => {
			fired.event_mut().set_int(c"team", 254);
			FireEventAction::Continue
		}

		"blocked" => FireEventAction::Block,
		_ => FireEventAction::Continue,
	}
}

/// A slot no test expects to be called, which aborts the test process.
extern "C" fn unexpected_call() {
	panic!("unexpected virtual call");
}
