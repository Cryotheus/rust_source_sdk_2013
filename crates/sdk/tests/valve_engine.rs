//! Tests of the engine's services for the game (`IVEngineServer`): the
//! arguments its methods receive, edict and user ID lookups, the lock of the
//! network string tables, and visibility queries.

use sdk_raw::edicts::FL_EDICT_FREE;
use sdk_raw::players::ABSOLUTE_PLAYER_LIMIT;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tier0::MAX_PATH;
use source_sdk_2013::Module;
use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::ValveEngine;
use source_sdk_2013::math::Vector;
use source_sdk_2013::players::UserId;
use source_sdk_2013::test_support::edicts::edict_table;
use source_sdk_2013::test_support::interfaces::valve_engine::{
	lock_network_string_tables, lock_requests, set_tables_locked, tables_locked,
};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_uchar};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, null_mut};

/// The game directory the mock engine reports.
const GAME_DIR: &CStr = c"C:/srcds/tf";

thread_local! {
	/// The engine and command `ServerCommand` last received.
	static RECEIVED: Cell<(*mut sys::IVEngineServer, *const c_char)> =
		const { Cell::new((null_mut(), ptr::null())) };

	/// The length of every set passed to `CheckOriginInPVS` and `CheckBoxInPVS`.
	static PVS_LENGTHS: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };

	/// The edict table and clients the mock engine serves.
	static SERVER: RefCell<MockServer> = const { RefCell::new(MockServer::EMPTY) };
}

/// A stand-in for the engine's edict table and client list.
struct MockServer {
	table: *mut sys::edict_t,
	table_len: usize,
	/// The edict and user ID of each client, as `GetPlayerUserId` sees them.
	clients: Vec<(*const sys::edict_t, c_int)>,
	/// Whether `PEntityOfEntIndex` returns free slots, which the engine does not.
	returns_free_slots: bool,
	/// Every index passed to `PEntityOfEntIndex`.
	requested: Vec<c_int>,
}

impl MockServer {
	const EMPTY: Self = Self {
		table: null_mut(),
		table_len: 0,
		clients: Vec::new(),
		returns_free_slots: false,
		requested: Vec::new(),
	};
}

/// `IVEngineServer::PEntityOfEntIndex`, which records the index and returns
/// the slot of the served table, unless it is free and the mock does not
/// return free slots.
unsafe extern "C" fn edict_of_index(
	_: *mut sys::IVEngineServer,
	index: c_int,
) -> *mut sys::edict_t {
	SERVER.with_borrow_mut(|server| {
		server.requested.push(index);

		let Some(slot) = usize::try_from(index)
			.ok()
			.filter(|&slot| slot < server.table_len)
		else {
			return null_mut();
		};

		// SAFETY: The slot lies within the served table, which the test keeps
		// alive.
		let (edict, flags) = unsafe {
			let edict = server.table.add(slot);

			(edict, (&raw const (*edict)._base.m_fStateFlags).read())
		};

		if flags & FL_EDICT_FREE != 0 && !server.returns_free_slots {
			null_mut()
		} else {
			edict
		}
	})
}

/// Exports a mock engine that serves the edicts and clients [`serve`] set.
fn export_edict_lookup() {
	// SAFETY: The patch only writes slots of the vtable.
	unsafe {
		export_engine(|vtable| {
			(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
			(&raw mut (*vtable).IVEngineServer_GetPlayerUserId).write(player_user_id);
		})
	};
}

/// Exports a mock engine whose vtable `patch` fills, every other slot failing
/// the test if called, and returns it.
///
/// # Safety
///
/// `patch` must only write functions of the slots' types to the vtable's
/// slots.
unsafe fn export_engine(
	patch: impl FnOnce(*mut sys::IVEngineServer__bindgen_vtable),
) -> *mut sys::IVEngineServer {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the caller's patch only writes slots of
	// the vtable being built.
	let vtable = unsafe { mock_vtable(unexpected_call as *const (), patch) };
	let engine = leak(sys::IVEngineServer {
		vtable_: Box::leak(vtable),
	});

	export(Module::Engine, ValveEngine::VERSION, engine);
	engine
}

/// Exports a mock engine that locks the string tables, and returns it.
fn export_lock() -> *mut sys::IVEngineServer {
	// SAFETY: The patch only writes a slot of the vtable.
	unsafe {
		export_engine(|vtable| {
			(&raw mut (*vtable).IVEngineServer_LockNetworkStringTables)
				.write(lock_network_string_tables);
		})
	}
}

#[test]
fn lookup_ignores_free_slots_even_if_the_engine_returns_them() {
	let mut table = edict_table(8, |slot| slot == 2);

	serve(&mut table, &[(1, 2), (2, 4)], true);
	export_edict_lookup();

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	assert_eq!(engine.edict_of_user_id(UserId::new(4).unwrap()), None);
	assert_eq!(
		engine
			.edict_of_user_id(UserId::new(2).unwrap())
			.map(Edict::index),
		Some(1)
	);
}

#[test]
fn methods_forward_the_interface_and_arguments() {
	// SAFETY: The patch only writes slots of the vtable.
	let interface = unsafe {
		export_engine(|vtable| {
			(&raw mut (*vtable).IVEngineServer_ServerCommand).write(record_server_command);
			(&raw mut (*vtable).IVEngineServer_GetGameDir).write(write_game_dir);
		})
	};

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();
	let command = c"bot\n";

	engine.server_command(command);

	assert_eq!(RECEIVED.get(), (interface, command.as_ptr()));
	assert_eq!(engine.game_dir().as_c_str(), GAME_DIR);
}

/// `IVEngineServer::GetPlayerUserId`, which returns the user ID of a served
/// client's edict, or -1 for any other edict, as the engine does.
unsafe extern "C" fn player_user_id(
	_: *mut sys::IVEngineServer,
	edict: *const sys::edict_t,
) -> c_int {
	SERVER.with_borrow(|server| {
		server
			.clients
			.iter()
			.find(|&&(client_edict, _)| client_edict == edict)
			.map_or(-1, |&(_, user_id)| user_id)
	})
}

/// `IVEngineServer::ServerCommand`, which records its arguments.
unsafe extern "C" fn record_server_command(this: *mut sys::IVEngineServer, command: *const c_char) {
	RECEIVED.set((this, command));
}

/// Every index passed to `PEntityOfEntIndex` so far.
fn requested_indices() -> Vec<c_int> {
	SERVER.with_borrow(|server| server.requested.clone())
}

/// Serves `table` and `clients`, given as `(slot, user ID)`, to the mock
/// engine.
fn serve(table: &mut [sys::edict_t], clients: &[(usize, c_int)], returns_free_slots: bool) {
	let base = table.as_mut_ptr();

	SERVER.set(MockServer {
		table: base,
		table_len: table.len(),
		clients: clients
			.iter()
			.map(|&(slot, user_id)| (base.wrapping_add(slot).cast_const(), user_id))
			.collect(),
		returns_free_slots,
		requested: Vec::new(),
	});
}

#[test]
fn unlocked_scopes_leave_unlocked_tables_unlocked() {
	let interface = export_lock();
	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	set_tables_locked(false);
	engine.with_unlocked_string_tables(|| assert!(!tables_locked()));

	assert!(!tables_locked());
	assert_eq!(lock_requests(), [(interface, false), (interface, false)]);
}

#[test]
fn unlocked_scopes_restore_the_lock_when_they_panic() {
	let interface = export_lock();
	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	let outcome = catch_unwind(AssertUnwindSafe(|| {
		engine.with_unlocked_string_tables(|| {
			if !tables_locked() {
				panic!("the scope panicked");
			}
		});
	}));

	assert!(outcome.is_err());
	assert!(tables_locked());
	assert_eq!(lock_requests(), [(interface, false), (interface, true)]);
}

#[test]
fn user_ids_and_player_edicts_round_trip() {
	// Slot 2 is an empty player slot, slots 4 onwards hold other entities,
	// and slot 256 would exceed the engine's player limit.
	let clients = [(1, 2), (3, 7), (255, 9), (256, 11)];
	let mut table = edict_table(300, |slot| slot == 2);

	serve(&mut table, &clients, false);
	export_edict_lookup();

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	for (slot, user_id) in [(1, 2), (3, 7), (255, 9)] {
		let user_id = UserId::new(user_id).unwrap();
		let edict = engine.edict_of_user_id(user_id).unwrap();

		assert_eq!(edict.index(), slot);
		assert_eq!(engine.edict_of_index(slot), Some(edict));
		assert_eq!(engine.user_id_of_edict(edict), Some(user_id));
	}

	assert_eq!(engine.edict_of_user_id(UserId::new(11).unwrap()), None);
	assert_eq!(engine.edict_of_user_id(UserId::new(5).unwrap()), None);
	assert_eq!(
		engine.user_id_of_edict(engine.edict_of_index(0).unwrap()),
		None
	);
	assert_eq!(
		engine.user_id_of_edict(engine.edict_of_index(4).unwrap()),
		None
	);
	assert_eq!(engine.edict_of_index(2), None);
	assert!(
		requested_indices()
			.iter()
			.all(|index| (0..=ABSOLUTE_PLAYER_LIMIT).contains(index))
	);
}

/// `IVEngineServer::GetGameDir`, which writes [`GAME_DIR`].
unsafe extern "C" fn write_game_dir(
	_: *mut sys::IVEngineServer,
	buffer: *mut c_char,
	length: c_int,
) {
	let length_with_nul = GAME_DIR.to_bytes_with_nul().len();

	assert_eq!(length, MAX_PATH as c_int);

	// SAFETY: The wrapper passes a buffer of `length` bytes, which the
	// directory fits.
	unsafe { ptr::copy_nonoverlapping(GAME_DIR.as_ptr(), buffer, length_with_nul) };
}

/// The clusters of the mock engine's map, three bytes of a set.
const CLUSTERS: c_int = 20;

/// `IVEngineServer::GetClusterForOrigin`, whose map lays its clusters out
/// along the x axis, one unit wide, from 0 to [`CLUSTERS`].
unsafe extern "C" fn cluster_for_origin(
	_: *mut sys::IVEngineServer,
	origin: *const sys::Vector,
) -> c_int {
	// SAFETY: The wrapper passes a vector it holds.
	mock_cluster(unsafe { (*origin).x })
}

/// `IVEngineServer::CheckBoxInPVS`, which records the set's length and checks
/// each cluster the box spans, as the engine does.
unsafe extern "C" fn check_box_in_pvs(
	_: *mut sys::IVEngineServer,
	mins: *const sys::Vector,
	maxs: *const sys::Vector,
	pvs: *const c_uchar,
	length: c_int,
) -> bool {
	PVS_LENGTHS.with_borrow_mut(|lengths| lengths.push(length));

	// SAFETY: The wrapper passes vectors it holds.
	let (mins, maxs) = unsafe { ((*mins).x, (*maxs).x) };

	(mins.floor() as c_int..=maxs.floor() as c_int)
		.filter(|&cluster| (0..CLUSTERS).contains(&cluster))
		// SAFETY: The wrapper passes a set of `length` bytes, which holds a bit for
		// every cluster of the map.
		.any(|cluster| unsafe { mock_pvs_contains(pvs, length, cluster) })
}

/// `IVEngineServer::CheckOriginInPVS`, which records the set's length and
/// checks the point's cluster, as the engine does.
unsafe extern "C" fn check_origin_in_pvs(
	_: *mut sys::IVEngineServer,
	origin: *const sys::Vector,
	pvs: *const c_uchar,
	length: c_int,
) -> bool {
	PVS_LENGTHS.with_borrow_mut(|lengths| lengths.push(length));

	// SAFETY: The wrapper passes a vector it holds.
	let cluster = mock_cluster(unsafe { (*origin).x });

	// SAFETY: As for `check_box_in_pvs`.
	cluster >= 0 && unsafe { mock_pvs_contains(pvs, length, cluster) }
}

/// Exports a mock engine that answers visibility queries for a map of
/// [`CLUSTERS`] clusters.
fn export_visibility() {
	PVS_LENGTHS.take();

	// SAFETY: The patch only writes slots of the vtable.
	unsafe {
		export_engine(|vtable| {
			(&raw mut (*vtable).IVEngineServer_GetClusterForOrigin).write(cluster_for_origin);
			(&raw mut (*vtable).IVEngineServer_GetPVSForCluster).write(pvs_for_cluster);
			(&raw mut (*vtable).IVEngineServer_CheckOriginInPVS).write(check_origin_in_pvs);
			(&raw mut (*vtable).IVEngineServer_CheckBoxInPVS).write(check_box_in_pvs);
		})
	};
}

/// The cluster of the mock map at `x`, or -1 outside it.
fn mock_cluster(x: f32) -> c_int {
	let cluster = x.floor() as c_int;

	if (0..CLUSTERS).contains(&cluster) {
		cluster
	} else {
		-1
	}
}

/// Whether the cluster's bit is set in a set of `length` bytes.
///
/// # Safety
///
/// `pvs` must point to `length` readable bytes, which hold `cluster`'s bit.
unsafe fn mock_pvs_contains(pvs: *const c_uchar, length: c_int, cluster: c_int) -> bool {
	assert!(cluster / 8 < length, "the engine would read past the set");

	// SAFETY: As the caller promises.
	unsafe { *pvs.add(cluster as usize / 8) & (1 << (cluster % 8)) != 0 }
}

/// `IVEngineServer::GetPVSForCluster`, which sees each cluster and the next
/// one from it, and nothing from outside the map, and returns the length of
/// the map's sets, as the engine does.
unsafe extern "C" fn pvs_for_cluster(
	_: *mut sys::IVEngineServer,
	cluster: c_int,
	capacity: c_int,
	pvs: *mut c_uchar,
) -> c_int {
	let length = (CLUSTERS + 7) / 8;

	assert!(
		capacity >= length,
		"the engine stops the server for a short buffer"
	);

	// SAFETY: The wrapper passes a buffer of `capacity` bytes.
	let pvs = unsafe { std::slice::from_raw_parts_mut(pvs, length as usize) };

	pvs.fill(0);

	for visible in [cluster, cluster + 1] {
		if cluster >= 0 && (0..CLUSTERS).contains(&visible) {
			pvs[visible as usize / 8] |= 1 << (visible % 8);
		}
	}

	length
}

#[test]
fn pvs_checks_pass_the_set_and_its_length() {
	export_visibility();

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();
	let at = |x| Vector::new(x, 0.0, 0.0);
	let mut pvs = engine.pvs_for_origin(at(2.5));

	pvs.union_with(&engine.pvs_for_origin(at(10.0)));

	assert!(engine.check_origin_in_pvs(at(3.0), &pvs));
	assert!(engine.check_origin_in_pvs(at(11.9), &pvs));
	assert!(!engine.check_origin_in_pvs(at(4.0), &pvs));
	assert!(!engine.check_origin_in_pvs(at(-5.0), &pvs));
	assert!(!engine.check_box_in_pvs(at(4.0), at(9.5), &pvs));
	assert!(engine.check_box_in_pvs(at(4.0), at(10.0), &pvs));
	assert_eq!(PVS_LENGTHS.take(), [3; 6]);
}

#[test]
fn pvs_sets_have_the_length_the_engine_reports() {
	export_visibility();

	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();
	let at = |x| Vector::new(x, 0.0, 0.0);
	let cluster = engine.cluster_for_origin(at(7.5)).unwrap();
	let pvs = engine.pvs_for_cluster(cluster);

	assert_eq!(cluster.index(), 7);
	assert_eq!(pvs.as_bytes(), [0b1000_0000, 0b0000_0001, 0]);
	assert!(pvs.contains(cluster));
	assert!(!pvs.is_empty());
	assert_eq!(pvs, engine.pvs_for_origin(at(7.0)));

	// Outside the map: no cluster, and an empty set as long as any other.
	assert_eq!(engine.cluster_for_origin(at(-1.0)), None);
	assert_eq!(engine.cluster_for_origin(at(20.0)), None);

	let outside = engine.pvs_for_origin(at(-1.0));

	assert!(outside.is_empty());
	assert_eq!(outside.as_bytes().len(), 3);
	assert!(!engine.check_origin_in_pvs(at(7.0), &outside));
}
