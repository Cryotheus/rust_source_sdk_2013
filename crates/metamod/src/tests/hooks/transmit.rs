//! Tests of `crate::hooks::transmit`: hooks of `ShouldTransmit` and
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

fn filter_snapshot(_: Server<'_>, check: TransmitCheck<'_>, candidates: &[u16]) {
	note("filter snapshot");
	assert_eq!(candidates, &[1, 9]);
	assert_eq!(check.sent_indices().collect::<Vec<_>>(), vec![0, 1, 9, 12]);
	// The parent's mark is absent from the original candidate list.
	for index in check.sent_indices().filter(|&index| index >= 9) {
		assert!(check.withhold(index));
	}
	assert!(!check.withhold(1));
	assert!(!check.withhold(0));
}

/// Simulates game marks, including ones that skipped entity transmit hooks.
unsafe extern "C" fn game_check_transmit(
	_: *mut sys::IServerGameEnts,
	info: *mut sys::CCheckTransmitInfo,
	_: *const u16,
	_: c_int,
) {
	note("game snapshot");
	// SAFETY: The test passes live writable records and bit sets.
	unsafe {
		(*info)
			.m_pTransmitEdict
			.cast::<u32>()
			.write(1 | (1 << 1) | (1 << 9) | (1 << 12));
		(*info)
			.m_pTransmitAlways
			.cast::<u32>()
			.write(1 | (1 << 1) | (1 << 9) | (1 << 12));
	}
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

fn panicking_snapshot(_: Server<'_>, _: TransmitCheck<'_>, _: &[u16]) {
	panic!("snapshot callback panic");
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

#[test]
fn snapshot_post_hooks_filter_final_marks_and_follow_pause_removal_and_panics() {
	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_check_transmit as *mut c_void;
			CHECK_TRANSMIT_SLOT + 1
		]);
		let mut interface = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let game = NonNull::from(&mut interface).cast::<sys::IServerGameEnts>();
		let mut client = mock_edict(1, false);
		let mut info = check_info(&mut client);
		let mut sent = [0_u32; source_sdk_2013::edicts::MAX_EDICTS as usize / 32];
		let mut always = sent;
		info.m_pTransmitEdict = sent.as_mut_ptr().cast();
		info.m_pTransmitAlways = always.as_mut_ptr().cast();
		let candidates = [1_u16, 9];
		let run = || {
			CALLS.take();
			harness.call::<CheckTransmit>(
				game.as_ptr(),
				CHECK_TRANSMIT_SLOT,
				(ptr::from_mut(&mut *info), candidates.as_ptr(), 2),
			);
			CALLS.take()
		};
		// SAFETY: The mock interface and its generated-slot signature stay
		// live throughout this harness's calls and hook removal.
		let hook = unsafe {
			api.install_check_transmit(game, tf2_binding(no_interfaces), filter_snapshot)
		}
		.unwrap();
		assert!(matches!(
			unsafe {
				api.install_check_transmit(game, tf2_binding(no_interfaces), filter_snapshot)
			},
			Err(HookError::AlreadyInstalled)
		));
		let mut run = run;
		assert_eq!(run(), ["game snapshot", "filter snapshot"]);
		assert_eq!(sent[0], 1 | (1 << 1));
		assert_eq!(always[0], sent[0]);
		harness.set_status(true, true, harness.generation);
		assert_eq!(run(), ["game snapshot"]);
		assert_eq!(sent[0], 1 | (1 << 1) | (1 << 9) | (1 << 12));
		harness.set_status(true, false, harness.generation);
		assert_eq!(run(), ["game snapshot", "filter snapshot"]);
		assert!(api.remove_hook(hook));
		assert_eq!(run(), ["game snapshot"]);
		// SAFETY: The same live mock, now without the earlier hook.
		let hook = unsafe {
			api.install_check_transmit(game, tf2_binding(no_interfaces), panicking_snapshot)
		}
		.unwrap();
		assert_eq!(run(), ["game snapshot"]);
		assert_eq!(sent[0], 1 | (1 << 1) | (1 << 9) | (1 << 12));
		assert!(api.remove_hook(hook));
	});
}
