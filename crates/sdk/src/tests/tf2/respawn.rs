//! Tests of `crate::tf2::respawn`: respawning a fake TF2 player.

use super::*;

use crate::test_support::entities::{
	MOCK_EFLAGS_OFFSET, base_entity_fields, get_datamap, set_datamap,
};

use crate::test_support::server::{mock_server, null_server};
use sdk_raw::test_support::entities::data_map;
use sdk_raw::tf2::respawn::FORCE_RESPAWN_SLOT;
use std::cell::RefCell;
use std::ffi::{CStr, c_int, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null, null_mut};

thread_local! {
	static RESPAWNED: RefCell<Vec<*mut c_void>> = const { RefCell::new(Vec::new()) };
}

/// A player whose fields lie where its datamaps say.
#[repr(C)]
struct FakePlayer {
	vtable: *const *const (),
	padding: [usize; 3],
	eflags: c_int,
}

/// `CTFPlayer::ForceRespawn`, which records the player.
unsafe extern "C" fn force_respawn_entry(player: *mut sys::CTFPlayer) {
	RESPAWNED.with_borrow_mut(|respawned| respawned.push(player.cast()));
}

/// The datamaps of a `class` deriving from `CBaseEntity`.
fn maps(class: &'static CStr) -> *mut sys::datamap_t {
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());

	data_map(class, vec![], base)
}

#[test]
fn tf_players_respawn_through_the_generated_vtable() {
	assert_eq!(offset_of!(FakePlayer, eflags), MOCK_EFLAGS_OFFSET);

	let mut vtable = vec![null(); FORCE_RESPAWN_SLOT + 1];

	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();
	vtable[FORCE_RESPAWN_SLOT] = force_respawn_entry as *const ();

	let player = Box::into_raw(Box::new(FakePlayer {
		vtable: vtable.as_ptr(),
		padding: [0; 3],
		eflags: 0,
	}));

	// SAFETY: The fake player stays allocated until it is dropped at the end of
	// the test, after the last use of the handle, and its vtable answers what
	// the respawn calls.
	let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
	let scope = ();
	let server = mock_server(&scope);

	set_datamap(maps(c"CTFPlayer"));
	RESPAWNED.take();

	assert_eq!(force_respawn(server, entity), Ok(()));
	assert_eq!(RESPAWNED.take(), [player.cast()]);

	// TF2's bots derive from `CTFPlayer`, and respawn as players do.
	set_datamap(data_map(c"CTFBot", vec![], maps(c"CTFPlayer")));
	assert_eq!(force_respawn(server, entity), Ok(()));
	assert_eq!(RESPAWNED.take(), [player.cast()]);

	// Anything else is refused before the game is called.
	set_datamap(maps(c"CBaseCombatCharacter"));
	assert_eq!(
		force_respawn(server, entity),
		Err(RespawnError::NotTfPlayer)
	);

	set_datamap(maps(c"CTFPlayer"));
	assert_eq!(
		force_respawn(null_server(Game::SourceSdk2013, &scope), entity),
		Err(RespawnError::NotTfPlayer)
	);

	// SAFETY: The fake player is allocated, and no reference to it is live.
	unsafe { (&raw mut (*player).eflags).write(1) };
	assert_eq!(
		force_respawn(server, entity),
		Err(RespawnError::MarkedForDeletion)
	);
	assert!(RESPAWNED.take().is_empty());
	// SAFETY: The player came from `Box::into_raw`, and is not used again.
	unsafe { drop(Box::from_raw(player)) };
}

thread_local! {
	static READY_FLAGS: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
	static READY_CALLS: RefCell<Vec<(&'static str, *mut c_void)>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn native_readiness_is_queried_without_forcing_a_respawn() {
	let play_slot = sdk_raw::vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_IsReadyToPlay);
	let spawn_slot =
		sdk_raw::vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_IsReadyToSpawn);
	let mut vtable = vec![null(); play_slot.max(spawn_slot).max(FORCE_RESPAWN_SLOT) + 1];
	vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();
	vtable[play_slot] = ready_to_play_entry as *const ();
	vtable[spawn_slot] = ready_to_spawn_entry as *const ();
	vtable[FORCE_RESPAWN_SLOT] = force_respawn_entry as *const ();
	let player = Box::into_raw(Box::new(FakePlayer {
		vtable: vtable.as_ptr(),
		padding: [0; 3],
		eflags: 0,
	}));
	// SAFETY: The mock stays allocated through the last call below. Its
	// vtable implements datamap lookup and both native readiness methods.
	let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
	let scope = ();
	let server = mock_server(&scope);
	set_datamap(maps(c"CTFPlayer"));
	RESPAWNED.take();
	for (play, spawn) in [(false, false), (false, true), (true, false), (true, true)] {
		READY_FLAGS.set((play, spawn));
		READY_CALLS.take();
		assert_eq!(is_ready_to_spawn(server, entity), Ok(play && spawn));
		let expected = if play {
			vec![("play", player.cast()), ("spawn", player.cast())]
		} else {
			vec![("play", player.cast())]
		};
		assert_eq!(READY_CALLS.take(), expected);
		assert!(RESPAWNED.take().is_empty());
	}
	set_datamap(maps(c"CBaseCombatCharacter"));
	assert_eq!(
		is_ready_to_spawn(server, entity),
		Err(RespawnError::NotTfPlayer)
	);
	assert!(READY_CALLS.take().is_empty());
	set_datamap(maps(c"CTFPlayer"));
	assert_eq!(
		is_ready_to_spawn(null_server(Game::SourceSdk2013, &scope), entity),
		Err(RespawnError::NotTfPlayer)
	);
	assert!(READY_CALLS.take().is_empty());
	// SAFETY: The mock is allocated, and no reference to its fields is live.
	unsafe { (&raw mut (*player).eflags).write(1) };
	assert_eq!(
		is_ready_to_spawn(server, entity),
		Err(RespawnError::MarkedForDeletion)
	);
	assert!(READY_CALLS.take().is_empty());
	// SAFETY: This pointer came from Box::into_raw and is not used again.
	unsafe { drop(Box::from_raw(player)) };
}

unsafe extern "C" fn ready_to_play_entry(player: *mut sys::CTFPlayer) -> bool {
	READY_CALLS.with_borrow_mut(|calls| calls.push(("play", player.cast())));
	READY_FLAGS.get().0
}

unsafe extern "C" fn ready_to_spawn_entry(player: *mut sys::CTFPlayer) -> bool {
	READY_CALLS.with_borrow_mut(|calls| calls.push(("spawn", player.cast())));
	READY_FLAGS.get().1
}
