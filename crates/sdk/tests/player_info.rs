//! Tests of reading players' state through the game's player information
//! (`IPlayerInfoManager`).

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::{PlayerInfoManager, ValveEngine};
use source_sdk_2013::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use source_sdk_2013::test_support::interfaces::player_info_manager::{
	global_vars, serve_global_vars,
};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::RefCell;
use std::ptr::NonNull;

thread_local! {
	/// Every edict passed to `GetPlayerInfo`, in order.
	static QUERIED: RefCell<Vec<*mut sys::edict_t>> = const { RefCell::new(Vec::new()) };
}

/// `IPlayerInfoManager::GetPlayerInfo`, which records the edict and returns a
/// dangling player state, never read.
unsafe extern "C" fn player_info(
	_: *mut sys::IPlayerInfoManager,
	edict: *mut sys::edict_t,
) -> *mut sys::IPlayerInfo {
	QUERIED.with_borrow_mut(|queried| queried.push(edict));
	NonNull::dangling().as_ptr()
}

/// The game casts any edict's entity to a player, so only occupied player
/// slots may reach `GetPlayerInfo`.
#[test]
fn player_info_asks_only_for_occupied_player_slots() {
	serve_global_vars(2);

	// SAFETY: The vtables hold only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patches only write slots of the
	// vtable being built.
	let (manager_vtable, engine_vtable) = unsafe {
		(
			mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
					(&raw mut (*vtable).IPlayerInfoManager_GetPlayerInfo).write(player_info);
				},
			),
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
				},
			),
		)
	};

	export(
		Module::GameServer,
		PlayerInfoManager::VERSION,
		leak(sys::IPlayerInfoManager {
			vtable_: Box::leak(manager_vtable),
		}),
	);
	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer {
			vtable_: Box::leak(engine_vtable),
		}),
	);

	// Slot 1 is free. Unlike the engine, the mock serves free slots too, so
	// the test can pass one.
	let mut table = edict_table(4, |slot| slot == 1);

	serve_edicts(table.as_mut_ptr(), table.len());

	let scope = ();
	let server = mock_server(&scope);
	let manager = server.player_info_manager().unwrap();
	let edict = |index| {
		server
			.valve_engine()
			.unwrap()
			.edict_of_index(index)
			.unwrap()
	};

	assert!(manager.player_info(edict(0)).is_none());
	assert!(manager.player_info(edict(1)).is_none());
	assert!(manager.player_info(edict(2)).is_some());
	assert!(manager.player_info(edict(3)).is_none());
	QUERIED.with_borrow(|queried| assert_eq!(*queried, [edict(2).as_ptr()]));
}
