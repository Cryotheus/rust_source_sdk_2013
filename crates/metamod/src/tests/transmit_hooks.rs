//! Tests of `crate::transmit_hooks`: hooks of `ShouldTransmit` and
//! `SetTransmit` on mock entity classes, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::test_support::edicts::mock_edict;
use std::cell::RefCell;
use std::mem::zeroed;

thread_local! {
	/// What ran during the calls since the last [`take_calls`], in order.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

	/// What [`decide`] decides.
	static DECISION: Cell<TransmitDecision> = const { Cell::new(TransmitDecision::Game) };

	/// What [`mark`] does.
	static ACTION: Cell<TransmitAction> = const { Cell::new(TransmitAction::Allow) };
}

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds [`game_should_transmit`]
	/// and [`game_set_transmit`] at their slots.
	fn of_new_class() -> Box<Self> {
		let slots = Vec::leak(vec![
			game_should_transmit as *mut c_void;
			SET_TRANSMIT_SLOT.max(SHOULD_TRANSMIT_SLOT) + 1
		]);

		slots[SET_TRANSMIT_SLOT] = game_set_transmit as *mut c_void;

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn entity(&mut self) -> Entity<'static> {
		let scope = Box::leak(Box::new(()));
		// SAFETY: The binding's server reaches no interface.
		let server = unsafe { tf2_binding(no_interfaces).server(scope) };

		// SAFETY: The mock outlives the test's use of the handle, which only
		// passes its address.
		unsafe { Entity::from_live(server, NonNull::new(self.ptr()).unwrap()) }
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		NonNull::from(self).cast().as_ptr()
	}
}

/// A client's record, whose player's edict is `client`.
fn check_info(client: &mut sys::edict_t) -> Box<sys::CCheckTransmitInfo> {
	// SAFETY: The record is plain data, for which zero is valid.
	let mut info: Box<sys::CCheckTransmitInfo> = Box::new(unsafe { zeroed() });

	info.m_pClientEnt = client;
	info
}

/// The callback of `ShouldTransmit`, which notes that it ran, and returns
/// [`DECISION`].
fn decide(_: Server<'_>, _: Entity<'_>, check: TransmitCheck<'_>) -> TransmitDecision {
	assert_eq!(check.client().map(|client| client.index()), Some(1));
	note("decide");
	DECISION.get()
}

/// The game's `SetTransmit`, which notes that it ran.
unsafe extern "C" fn game_set_transmit(
	_: *mut sys::CBaseEntity,
	_: *mut sys::CCheckTransmitInfo,
	always: bool,
) {
	note(if always {
		"game marks always"
	} else {
		"game marks"
	});
}

/// The game's `ShouldTransmit`, which notes that it ran, and checks the
/// potentially visible set.
unsafe extern "C" fn game_should_transmit(
	_: *mut sys::CBaseEntity,
	_: *const sys::CCheckTransmitInfo,
) -> c_int {
	note("game decides");
	FL_EDICT_PVSCHECK
}

/// The callback of `SetTransmit`, which notes that it ran, and does
/// [`ACTION`].
fn mark(_: Server<'_>, _: Entity<'_>, _: TransmitCheck<'_>, always: bool) -> TransmitAction {
	note(if always { "mark always" } else { "mark" });
	ACTION.get()
}

fn note(name: &'static str) {
	CALLS.with_borrow_mut(|calls| calls.push(name));
}

/// Calls the mock's hooked `SetTransmit`, and returns what ran.
fn set_transmit(
	harness: &Harness,
	mock: &mut Mock,
	info: &mut sys::CCheckTransmitInfo,
	always: bool,
) -> Vec<&'static str> {
	CALLS.take();
	harness.call::<SetTransmit>(mock.ptr(), SET_TRANSMIT_SLOT, (ptr::from_mut(info), always));
	CALLS.take()
}

#[test]
fn set_transmit_hooks_block_marking_but_never_the_clients_own_player() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Mock::of_new_class();
		// Leaked, as the record keeps pointing to it.
		let client = Box::leak(Box::new(mock_edict(1, false)));
		let mut info = check_info(client);
		let client = info.m_pClientEnt;

		let hook = api
			.hook_set_transmit(player.entity(), tf2_binding(no_interfaces), mark)
			.unwrap();

		ACTION.set(TransmitAction::Allow);
		assert_eq!(
			set_transmit(harness, &mut player, &mut info, false),
			["mark", "game marks"]
		);

		ACTION.set(TransmitAction::Block);
		assert_eq!(
			set_transmit(harness, &mut player, &mut info, true),
			["mark always"]
		);

		// The client's own player is marked as the game does, without the
		// callback.
		// SAFETY: The edict is leaked, and only read through the record.
		unsafe { (*client)._base.m_pUnk = player.ptr().cast() };
		assert_eq!(
			set_transmit(harness, &mut player, &mut info, false),
			["game marks"]
		);

		// SAFETY: As above.
		unsafe { (*client)._base.m_pUnk = ptr::null_mut() };
		assert!(api.remove_hook(hook));
		assert_eq!(
			set_transmit(harness, &mut player, &mut info, false),
			["game marks"]
		);

		// A removed hook's class can be hooked again.
		api.hook_set_transmit(player.entity(), tf2_binding(no_interfaces), mark)
			.unwrap();
		assert_eq!(
			set_transmit(harness, &mut player, &mut info, false),
			["mark"]
		);
	});
}

/// Calls the mock's hooked `ShouldTransmit`, and returns its flag and what
/// ran.
fn should_transmit(
	harness: &Harness,
	mock: &mut Mock,
	info: &sys::CCheckTransmitInfo,
) -> (c_int, Vec<&'static str>) {
	CALLS.take();
	let flag =
		harness.call::<ShouldTransmit>(mock.ptr(), SHOULD_TRANSMIT_SLOT, (ptr::from_ref(info),));
	(flag, CALLS.take())
}

#[test]
fn should_transmit_hooks_decide_in_place_of_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut building = Mock::of_new_class();
		let mut client = mock_edict(1, false);
		let info = check_info(&mut client);

		api.hook_should_transmit(building.entity(), tf2_binding(no_interfaces), decide)
			.unwrap();

		assert!(matches!(
			api.hook_should_transmit(building.entity(), tf2_binding(no_interfaces), decide),
			Err(HookError::AlreadyInstalled)
		));

		DECISION.set(TransmitDecision::Game);
		assert_eq!(
			should_transmit(harness, &mut building, &info),
			(FL_EDICT_PVSCHECK, vec!["decide", "game decides"])
		);

		for (decision, flag) in [
			(TransmitDecision::Always, FL_EDICT_ALWAYS),
			(TransmitDecision::DontSend, FL_EDICT_DONTSEND),
			(TransmitDecision::PvsCheck, FL_EDICT_PVSCHECK),
		] {
			DECISION.set(decision);
			assert_eq!(
				should_transmit(harness, &mut building, &info),
				(flag, vec!["decide"])
			);
		}
	});
}
