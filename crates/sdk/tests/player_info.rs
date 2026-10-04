//! Tests of reading players' state through the game's player information
//! (`IPlayerInfoManager`).

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::{PlayerInfoManager, ValveEngine};
use source_sdk_2013::math::Vector;
use source_sdk_2013::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use source_sdk_2013::test_support::interfaces::player_info_manager::{
	global_vars, serve_global_vars,
};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ptr::{NonNull, null_mut};

/// A `GetPlayerInfo` mock.
type GetPlayerInfo =
	unsafe extern "C" fn(*mut sys::IPlayerInfoManager, *mut sys::edict_t) -> *mut sys::IPlayerInfo;

thread_local! {
	/// What [`served_player_info`] returns.
	static INFO: Cell<*mut sys::IPlayerInfo> = const { Cell::new(null_mut()) };

	/// Every edict passed to `GetPlayerInfo`, in order.
	static QUERIED: RefCell<Vec<*mut sys::edict_t>> = const { RefCell::new(Vec::new()) };
}

/// `IPlayerInfo::GetAbsOrigin`, which returns (1, 2, 3) through the hidden
/// result pointer of the MSVC ABI.
#[cfg(target_os = "windows")]
unsafe extern "C" fn abs_origin(
	_: *mut sys::IPlayerInfo,
	result: *mut sys::Vector,
) -> *mut sys::Vector {
	// SAFETY: The caller passes storage for the result.
	unsafe {
		result.write(sys::Vector {
			x: 1.0,
			y: 2.0,
			z: 3.0,
		})
	};
	result
}

/// `IPlayerInfo::GetAbsOrigin`, which returns (1, 2, 3) in registers, as the
/// Itanium ABI returns a trivially copyable class.
#[cfg(target_os = "linux")]
unsafe extern "C" fn abs_origin(_: *mut sys::IPlayerInfo) -> sys::Vector {
	sys::Vector {
		x: 1.0,
		y: 2.0,
		z: 3.0,
	}
}

/// Makes the mock server export a player information manager whose
/// `GetPlayerInfo` is `player_info`, serving a client limit of 2, and an
/// engine serving four edicts, of which slot 1 is free. Unlike the engine,
/// the mock serves free slots too, so a test can pass one.
fn export_manager(player_info: GetPlayerInfo) -> Box<[sys::edict_t]> {
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

	let mut table = edict_table(4, |slot| slot == 1);

	serve_edicts(table.as_mut_ptr(), table.len());
	table
}

/// `IPlayerInfo::GetHealth`, which returns 125.
unsafe extern "C" fn health(_: *mut sys::IPlayerInfo) -> c_int {
	125
}

/// `IPlayerInfo::GetMaxHealth`, which returns 175.
unsafe extern "C" fn max_health(_: *mut sys::IPlayerInfo) -> c_int {
	175
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
	let _table = export_manager(player_info);
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

/// Health comes from the game's own methods, and vectors come back by value
/// as each ABI returns them.
#[test]
fn player_state_is_read_through_the_games_methods() {
	// SAFETY: As for `export_manager`'s vtables.
	let info_vtable = unsafe {
		mock_vtable::<sys::IPlayerInfo__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IPlayerInfo_GetHealth).write(health);
			(&raw mut (*vtable).IPlayerInfo_GetMaxHealth).write(max_health);
			(&raw mut (*vtable).IPlayerInfo_GetAbsOrigin).write(abs_origin);
		})
	};

	INFO.set(leak(sys::IPlayerInfo {
		vtable_: Box::leak(info_vtable),
	}));

	let _table = export_manager(served_player_info);
	let scope = ();
	let server = mock_server(&scope);
	let edict = server.valve_engine().unwrap().edict_of_index(2).unwrap();
	let info = server
		.player_info_manager()
		.unwrap()
		.player_info(edict)
		.unwrap();

	assert_eq!((info.health(), info.max_health()), (125, 175));
	assert_eq!(info.abs_origin(), Vector::new(1.0, 2.0, 3.0));
}

/// `IPlayerInfoManager::GetPlayerInfo`, which returns the player state in
/// [`INFO`].
unsafe extern "C" fn served_player_info(
	_: *mut sys::IPlayerInfoManager,
	_: *mut sys::edict_t,
) -> *mut sys::IPlayerInfo {
	INFO.get()
}
