//! Tests of `crate::key_values_hooks`: hooks of `ClientCommandKeyValues` on a
//! mock game interface, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::abi::VTABLE_SLOT_SIZE;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;

thread_local! {
	/// What happened during the calls since the last [`send`], in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// The address of the key values the callback blocks.
	static BLOCKED: Cell<usize> = const { Cell::new(0) };
}

/// The game's interface, as far as hooks know it.
#[repr(C)]
struct Clients {
	vtable: *mut *mut c_void,
}

impl Clients {
	fn new() -> Box<Self> {
		let mut slots = vec![
			unexpected_call as *mut c_void;
			size_of::<sys::IServerGameClients__bindgen_vtable>() / VTABLE_SLOT_SIZE
		];

		slots[CLIENT_COMMAND_KEY_VALUES_SLOT] = game_key_values as *mut c_void;

		Box::new(Self {
			vtable: Vec::leak(slots).as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::IServerGameClients> {
		NonNull::from(self).cast()
	}
}

/// Something the game or the callback noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
	/// The callback saw the command at this address, from this edict.
	Callback(usize, usize),

	/// The game handled the command at this address, from this edict.
	Game(usize, usize),
}

#[test]
fn commands_an_earlier_hook_blocked_reach_neither_the_callback_nor_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut clients = Clients::new();

		fn block(_call: &HookCall<'_, ClientCommandKeyValues>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: The mock interface has `ClientCommandKeyValues` at its slot,
		// and is leaked with its vtable.
		unsafe {
			api.add_hook(
				CLIENT_COMMAND_KEY_VALUES,
				HookTarget::instance(clients.ptr()),
				HookTiming::Pre,
				&block,
			)
			.unwrap();
			api.install_client_key_values(clients.ptr(), tf2_binding(no_interfaces), on_key_values)
				.unwrap();
		}

		assert_eq!(send(harness, &mut clients, 0x10, 0x20), vec![]);
	});
}

#[test]
fn commands_reach_the_callback_and_blocked_ones_skip_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut clients = Clients::new();

		// SAFETY: The mock interface has `ClientCommandKeyValues` at its slot,
		// and is leaked with its vtable.
		unsafe {
			api.install_client_key_values(clients.ptr(), tf2_binding(no_interfaces), on_key_values)
		}
		.unwrap();

		// A second hook is refused, so that each command is decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_client_key_values(
					clients.ptr(),
					tf2_binding(no_interfaces),
					on_key_values,
				)
			},
			Err(HookError::AlreadyInstalled)
		));

		BLOCKED.set(0x30);

		assert_eq!(
			send(harness, &mut clients, 0x10, 0x20),
			vec![Seen::Callback(0x10, 0x20), Seen::Game(0x10, 0x20)]
		);

		assert_eq!(
			send(harness, &mut clients, 0x10, 0x30),
			vec![Seen::Callback(0x10, 0x30)]
		);

		// Null arguments reach the game only.
		assert_eq!(
			send(harness, &mut clients, 0, 0x30),
			vec![Seen::Game(0, 0x30)]
		);

		assert_eq!(
			send(harness, &mut clients, 0x10, 0),
			vec![Seen::Game(0x10, 0)]
		);
	});
}

/// The mock game's `ClientCommandKeyValues`, which notes the command.
unsafe extern "C" fn game_key_values(
	_this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	key_values: *mut sys::KeyValues,
) {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Game(edict.addr(), key_values.addr())));
}

/// The callback, which notes what it saw, and blocks the key values at
/// [`BLOCKED`].
fn on_key_values(
	_server: Server<'_>,
	edict: Edict<'_>,
	key_values: KeyValues<'_>,
) -> ClientKeyValuesAction {
	let (edict, key_values) = (edict.as_ptr().addr(), key_values.as_ptr().addr());

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Callback(edict, key_values)));

	match key_values == BLOCKED.get() {
		true => ClientKeyValuesAction::Block,
		false => ClientKeyValuesAction::Continue,
	}
}

/// Sends the command at `key_values` from the client at `edict` through
/// `clients`' hooked vtable, and returns what happened. Neither address is
/// read.
fn send(harness: &Harness, clients: &mut Clients, edict: usize, key_values: usize) -> Vec<Seen> {
	SEEN.take();

	harness.call::<ClientCommandKeyValues>(
		clients.ptr().as_ptr(),
		CLIENT_COMMAND_KEY_VALUES_SLOT,
		(
			ptr::without_provenance_mut(edict),
			ptr::without_provenance_mut(key_values),
		),
	);

	SEEN.take()
}

/// A slot no test expects to be called, which aborts the test process.
extern "C" fn unexpected_call() {
	expect(false, "unexpected virtual call");
}
