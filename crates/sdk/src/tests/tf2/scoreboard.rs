//! Tests of `crate::tf2::scoreboard`: the scoreboard against fake entities,
//! send tables shaped as TF2's, and an engine that records changes as the real
//! one does.

use super::*;
use crate::Module;
use crate::test_support::datatables::{direct_table, int32_proxy, prop, table, table_prop};
use crate::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::players::user;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
use sdk_raw::entities::NUM_SERIAL_NUM_SHIFT_BITS;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char};
use std::mem::{offset_of, zeroed};
use std::ptr::{NonNull, null_mut};

/// The player resource's arrays: name, bits, flags, and elements per slot.
const ARRAYS: [(&CStr, c_int, PropFlags, usize); 17] = {
	const U: PropFlags = PropFlags::UNSIGNED;
	const S: PropFlags = PropFlags::empty();
	const V: PropFlags = PropFlags::UNSIGNED.union(PropFlags::NORMAL);

	[
		// `DT_PlayerResource`, the base class's table.
		(c"m_iPing", 10, U, 1),
		(c"m_iScore", 12, S, 1),
		(c"m_iDeaths", 12, S, 1),
		(c"m_bAlive", 1, U, 1),
		(c"m_iTeam", 4, S, 1),
		// `DT_TFPlayerResource`. `SendPropInt` turns the -1 bits into 32.
		(c"m_iTotalScore", 32, V, 1),
		(c"m_iPlayerClass", 5, U, 1),
		(c"m_iActiveDominations", 6, U, 1),
		(c"m_iDamage", 32, V, 1),
		(c"m_iDamageAssist", 32, V, 1),
		(c"m_iDamageBoss", 32, V, 1),
		(c"m_iHealing", 32, V, 1),
		(c"m_iHealingAssist", 32, V, 1),
		(c"m_iDamageBlocked", 32, V, 1),
		(c"m_iCurrencyCollected", 32, V, 1),
		(c"m_iBonusPoints", 32, V, 1),
		(c"m_iStreaks", 32, V, STREAKS_PER_SLOT),
	]
};

/// How many of [`ARRAYS`] belong to `DT_PlayerResource`.
const BASE_ARRAYS: usize = 5;

const BLUE: usize = 112;

/// Bytes from the start of a fake entity to its data.
const DATA: usize = offset_of!(FakeEntity, data);

/// `int`s of data each fake entity holds, enough for 102 player slots.
const DATA_WORDS: usize = 2100;

const DEATHS: usize = 1;

/// Slots in the fake edict table.
const EDICT_COUNT: usize = 128;

/// Where players keep `m_iFrags` and `m_iDeaths`, in words of their data.
const FRAGS: usize = 0;

const RED: usize = 111;

/// The edict indices of the player resource and the teams.
const RESOURCE: usize = 110;

const TEAM_CAPTURES: usize = 3;

/// Where the teams keep `m_iTeamNum`, `m_iScore`, and `m_nFlagCaptures`, in
/// words of their data.
const TEAM_NUMBER: usize = 0;

const TEAM_SCORE: usize = 1;

thread_local! {
	static ACCESSORS: Cell<*mut sys::IChangeInfoAccessor> = const { Cell::new(null_mut()) };
	static BY_CLASS: RefCell<Vec<(&'static CStr, *mut sys::CBaseEntity)>> = const { RefCell::new(Vec::new()) };
	static EDICTS: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
	static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
	static SHARED: Cell<*mut sys::CSharedEdictChangeInfo> = const { Cell::new(null_mut()) };
	static USER_IDS: RefCell<[c_int; EDICT_COUNT]> = const { RefCell::new([-1; EDICT_COUNT]) };
}

/// How one array of the player resource is broken, for layout tests.
#[derive(Debug, Clone, Copy)]
enum Breakage {
	/// The array is left out.
	Missing,

	/// The array has this many elements instead.
	Len(usize),

	/// Elements are this many bytes apart instead of 4.
	Stride(usize),
}

/// What the engine recorded as changed for an edict since the last snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Changed {
	Nothing,
	Full,
	Offsets(Vec<u16>),
}

/// An entity the game DLL could have created, with its own class, edict and
/// data.
#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	networkable: sys::IServerNetworkable,
	handle: u32,
	class: *mut sys::ServerClass,
	edict: *mut sys::edict_t,
	class_name: *const c_char,
	map: *mut sys::datamap_t,
	resets: usize,
	data: [i32; DATA_WORDS],
}

/// A fake game: a player resource, two teams, players, and the interfaces
/// the scoreboard reaches them through.
struct World {
	/// The data word of each array's first element, by name.
	arrays: Vec<(&'static CStr, usize)>,
	blue: *mut FakeEntity,
	players: Vec<(usize, *mut FakeEntity)>,
	red: *mut FakeEntity,
	resource: *mut FakeEntity,
	resource_class: *mut sys::ServerClass,
	vtable: *const *const (),
	networkable_vtable: *const sys::IServerNetworkable__bindgen_vtable,
}

impl World {
	/// A game whose arrays hold `slots` player slots, with one array broken
	/// as `broken` says.
	fn new(slots: usize, broken: Option<(&CStr, Breakage)>) -> Self {
		// The interfaces.
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let engine_vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
					(&raw mut (*vtable).IVEngineServer_GetPlayerUserId).write(player_user_id);
					(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
					(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo)
						.write(shared_change_info);
				},
			)
		};
		// SAFETY: As for the engine's vtable.
		let tools_vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_FindEntityByClassname)
						.write(find_by_class_name);
					(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
				},
			)
		};

		export(
			Module::Engine,
			ValveEngine::VERSION,
			leak(sys::IVEngineServer {
				vtable_: Box::leak(engine_vtable),
			}),
		);
		export(
			Module::GameServer,
			ServerTools::VERSION,
			leak(sys::IServerTools {
				vtable_: Box::leak(tools_vtable),
			}),
		);
		export_standard_proxies();

		// The engine's edicts and change tracking.
		let edicts = Box::leak(edict_table(EDICT_COUNT, |_| false)).as_mut_ptr();
		EDICTS.set(edicts);
		serve_edicts(edicts, EDICT_COUNT);

		// SAFETY: An accessor is two integers, for which zero is valid.
		let accessors = (0..EDICT_COUNT)
			.map(|_| unsafe { zeroed::<sys::IChangeInfoAccessor>() })
			.collect::<Vec<_>>();
		ACCESSORS.set(Box::leak(accessors.into_boxed_slice()).as_mut_ptr());

		// SAFETY: The shared change info is plain integers, for which zero is
		// valid.
		let shared = leak(unsafe { zeroed::<sys::CSharedEdictChangeInfo>() });
		// SAFETY: The shared change info is leaked, and nothing else refers to it
		// yet.
		unsafe { (*shared).m_iSerialNumber = 1 };
		SHARED.set(shared);

		// SAFETY: The entity list holds integers, a `bool` and raw pointers, for
		// all of which zero is valid.
		LIST.set(Box::into_raw(unsafe {
			Box::<sys::CGlobalEntityList>::new_zeroed().assume_init()
		}));
		BY_CLASS.take();
		USER_IDS.set([-1; EDICT_COUNT]);

		// The player resource's tables, its arrays laid out one after another.
		let mut arrays = Vec::new();
		let mut base_props = Vec::new();
		let mut tf_props = Vec::new();
		let mut word = 0;

		for (index, &(name, bits, flags, per_slot)) in ARRAYS.iter().enumerate() {
			let breakage = broken
				.filter(|(broken, _)| *broken == name)
				.map(|(_, how)| how);
			let (len, stride) = match breakage {
				Some(Breakage::Missing) => continue,
				Some(Breakage::Len(len)) => (len, ELEMENT_SIZE),
				Some(Breakage::Stride(stride)) => (slots * per_slot, stride),
				None => (slots * per_slot, ELEMENT_SIZE),
			};

			let array = int_array(name, DATA + word * ELEMENT_SIZE, len, stride, bits, flags);

			arrays.push((name, word));
			word += len * stride / ELEMENT_SIZE;
			assert!(word <= DATA_WORDS);

			if index < BASE_ARRAYS {
				base_props.push(array);
			} else {
				tf_props.push(array);
			}
		}

		let base_table = leak(table(
			c"DT_PlayerResource",
			Box::leak(base_props.into_boxed_slice()),
		));

		tf_props.insert(
			0,
			table_prop(c"baseclass", 0, base_table, Some(direct_table)),
		);

		let tf_table = leak(table(
			c"DT_TFPlayerResource",
			Box::leak(tf_props.into_boxed_slice()),
		));
		let resource_class = server_class_of(c"CTFPlayerResource", tf_table);

		// The teams' tables.
		let team_props = Box::leak(Box::new([
			int_prop(
				c"m_iTeamNum",
				DATA + TEAM_NUMBER * ELEMENT_SIZE,
				5,
				PropFlags::default(),
			),
			// `SendPropInt` turns these 0 bits into 32, and the scoreboard must too.
			int_prop(
				c"m_iScore",
				DATA + TEAM_SCORE * ELEMENT_SIZE,
				0,
				PropFlags::default(),
			),
			int_prop(
				c"m_iRoundsWon",
				DATA + 2 * ELEMENT_SIZE,
				8,
				PropFlags::default(),
			),
		]));
		let team_table = leak(table(c"DT_Team", team_props));
		let tf_team_props = Box::leak(Box::new([
			table_prop(c"baseclass", 0, team_table, Some(direct_table)),
			int_prop(
				c"m_nFlagCaptures",
				DATA + TEAM_CAPTURES * ELEMENT_SIZE,
				8,
				PropFlags::default(),
			),
		]));
		let tf_team_table = leak(table(c"DT_TFTeam", tf_team_props));
		let team_class = server_class_of(c"CTFTeam", tf_team_table);

		// The entities' vtables.
		let reset_slot =
			sdk_raw::vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_ResetScores);
		let mut vtable = vec![
			unexpected_call as *const ();
			reset_slot.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT) + 1
		];

		vtable[sdk_raw::vtable_slot!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		)] = networkable as *const ();
		vtable[sdk_raw::vtable_slot!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetRefEHandle
		)] = handle as *const ();
		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[reset_slot] = reset_player_scores as *const ();

		// SAFETY: As for the engine's vtable.
		let networkable_vtable = unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
					(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
				},
			)
		};

		let mut world = Self {
			arrays,
			blue: null_mut(),
			players: Vec::new(),
			red: null_mut(),
			resource: null_mut(),
			resource_class,
			vtable: vtable.leak().as_ptr(),
			networkable_vtable: Box::leak(networkable_vtable),
		};

		world.resource = world.spawn(RESOURCE, 7, resource_class, RESOURCE_CLASS_NAME, null_mut());
		world.red = world.spawn(RED, 3, team_class, TEAM_CLASS_NAME, null_mut());
		world.blue = world.spawn(BLUE, 3, team_class, TEAM_CLASS_NAME, null_mut());

		// SAFETY: `spawn` leaks the teams, and no reference to them is live.
		unsafe {
			(*world.red).data[TEAM_NUMBER] = ScoringTeam::Red.to_raw();
			(*world.blue).data[TEAM_NUMBER] = ScoringTeam::Blue.to_raw();
		}

		// Players declare their frags and deaths in `CBasePlayer`'s datamap.
		let declare = |name: &'static CStr, word: usize| {
			let mut field = field(
				name,
				sys::_fieldtypes_FIELD_INTEGER,
				DATA + word * ELEMENT_SIZE,
			);

			field.fieldSizeInBytes = ELEMENT_SIZE as c_int;
			field
		};
		let base_player = data_map(
			c"CBasePlayer",
			vec![declare(c"m_iFrags", FRAGS), declare(c"m_iDeaths", DEATHS)],
			null_mut(),
		);
		let player_map = data_map(c"CTFPlayer", Vec::new(), base_player);

		for slot in [1, 2, 3, slots - 1] {
			let player = world.spawn(slot, 1, null_mut(), c"player", player_map);

			world.players.push((slot, player));
		}

		world
	}

	/// Reads `name`'s element `element` in the player resource.
	fn get(&self, name: &CStr, element: usize) -> i32 {
		// SAFETY: `spawn` leaks the resource, and no reference to it is live.
		unsafe { (*self.resource).data[self.word(name, element)] }
	}

	/// The offset of `name`'s element `element` in the player resource.
	fn offset(&self, name: &CStr, element: usize) -> u16 {
		u16::try_from(DATA + self.word(name, element) * ELEMENT_SIZE).unwrap()
	}

	/// The player in `slot`.
	fn player(&self, slot: usize) -> Entity<'_> {
		let (_, player) = self
			.players
			.iter()
			.find(|(found, _)| *found == slot)
			.unwrap();

		// SAFETY: `spawn` leaks the players, which are in the entity list, and
		// their vtable answers what the scoreboard calls.
		unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) }
	}

	/// Writes `name`'s element `element` in the player resource, without
	/// telling the engine.
	fn put(&self, name: &CStr, element: usize, value: i32) {
		// SAFETY: As for `get`.
		unsafe { (*self.resource).data[self.word(name, element)] = value };
	}

	/// Assigns `name`'s element `element` as the game's `CNetworkArray::Set`
	/// does, marking it changed if the value differs.
	fn set(&self, server: Server<'_>, name: &CStr, element: usize, value: i32) {
		if self.get(name, element) != value {
			self.put(name, element, value);
			edict_at(RESOURCE)
				.state_changed(server.valve_engine().unwrap(), self.offset(name, element));
		}
	}

	/// Creates an entity in edict `index`, with the serial number `serial`.
	fn spawn(
		&self,
		index: usize,
		serial: u32,
		class: *mut sys::ServerClass,
		class_name: &'static CStr,
		map: *mut sys::datamap_t,
	) -> *mut FakeEntity {
		let entity = leak(FakeEntity {
			vtable: self.vtable,
			networkable: sys::IServerNetworkable {
				vtable_: self.networkable_vtable,
			},
			handle: index as u32 | serial << NUM_SERIAL_NUM_SHIFT_BITS,
			class,
			// SAFETY: Entities are only spawned at indices within the table of
			// `EDICT_COUNT` edicts.
			edict: unsafe { EDICTS.get().add(index) },
			class_name: class_name.as_ptr(),
			map,
			resets: 0,
			data: [0; DATA_WORDS],
		});

		// SAFETY: The entity list is leaked, and `index`, below `EDICT_COUNT`,
		// lies within its `m_EntPtrArray`.
		unsafe {
			let info = (&raw mut (*LIST.get())._base.m_EntPtrArray)
				.cast::<sys::CEntInfo>()
				.add(index);

			(&raw mut (*info).m_pEntity).write(entity.cast());
			(&raw mut (*info).m_SerialNumber).write(serial as c_int);
		}

		BY_CLASS.with_borrow_mut(|entities| entities.push((class_name, entity.cast())));
		entity
	}

	/// Reads a team's data word `word`.
	fn team(&self, team: ScoringTeam, word: usize) -> i32 {
		let entity = match team {
			ScoringTeam::Red => self.red,
			ScoringTeam::Blue => self.blue,
		};

		// SAFETY: `spawn` leaks the teams, and no reference to them is live.
		unsafe { (*entity).data[word] }
	}

	/// The data word of `name`'s element `element`.
	fn word(&self, name: &CStr, element: usize) -> usize {
		let (_, first) = self
			.arrays
			.iter()
			.find(|(array, _)| *array == name)
			.unwrap();

		first + element
	}
}

#[test]
fn a_missed_after_frame_marks_the_resource_fully_changed() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	// SAFETY: The world leaks its teams, and no reference to them is live.
	unsafe { (*world.blue).data[TEAM_SCORE] = 1 };
	board
		.set_team_stat(
			server,
			ScoringTeam::Blue,
			TeamStat::Score,
			Override::Fixed(5),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// The values written back without a mark are sent as they are, since the
	// overrides never return.
	board.before_frame(server);
	end_snapshot();

	board.before_frame(server);
	assert_eq!(changed(RESOURCE), Changed::Full);
	assert_eq!(changed(BLUE), Changed::Full);
	assert_eq!(changed(RED), Changed::Nothing);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_SCORE), 1);

	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_SCORE), 5);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
}

#[test]
fn a_missed_before_frame_keeps_the_real_value() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Offset(1000))
		.unwrap();

	// Until a `before_frame` has run, `after_frame` only records the game's
	// value, so a hook that starts first cannot leave an override in place.
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
	assert_eq!(board.phase, Phase::Idle);

	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 1120);
	end_snapshot();

	// Memory still holding what was applied means the game left it alone.
	board.after_frame(server);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
	assert_eq!(world.get(c"m_iTotalScore", 1), 1120);
	assert_eq!(changed(RESOURCE), Changed::Nothing);

	// Anything else is the game's new value.
	world.set(server, c"m_iTotalScore", 1, 130);
	board.after_frame(server);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(130)
	);
	assert_eq!(world.get(c"m_iTotalScore", 1), 1130);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
}

#[test]
fn a_replaced_resource_is_never_written_back_to() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// The resource is replaced, in the same edict, by one with another serial
	// number, which holds the game's values.
	BY_CLASS.with_borrow_mut(|entities| entities.retain(|(name, _)| *name != RESOURCE_CLASS_NAME));
	let replacement = world.spawn(
		RESOURCE,
		8,
		world.resource_class,
		RESOURCE_CLASS_NAME,
		null_mut(),
	);
	let element = world.word(c"m_iTotalScore", 1);
	// SAFETY: `spawn` leaks the replacement, and no reference to it is live.
	let score = move || unsafe { (*replacement).data[element] };

	// SAFETY: As for `score`.
	unsafe { (*replacement).data[element] = 50 };
	board.before_frame(server);
	assert_eq!(score(), 50);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);

	board.after_frame(server);
	assert_eq!(score(), 9999);
	assert_eq!(board.real_player_stat(user(5), PlayerStat::Score), Some(50));
	end_snapshot();

	// An entity reporting another server class is not trusted with the
	// layout either.
	// SAFETY: The world leaks its server classes, which are plain data.
	let copy = leak(unsafe { world.resource_class.read() });

	// SAFETY: As for `score`.
	unsafe { (*replacement).class = copy };
	board.before_frame(server);
	assert_eq!(score(), 9999);
	board.after_frame(server);
	assert_eq!(score(), 9999);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(9999)
	);
}

#[test]
fn a_user_id_found_in_another_slot_releases_the_first() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// The client is in another slot before any frame callback noticed.
	disconnect(1);
	connect(2, 5);
	board
		.set_player_stat(server, user(5), PlayerStat::Kills, Override::Fixed(3))
		.unwrap();
	assert_eq!(board.slot_of(user(5)), Some(2));
	assert_eq!(board.player_override(user(5), PlayerStat::Score), None);
	assert_eq!(
		board.player_override(user(5), PlayerStat::Kills),
		Some(Override::Fixed(3))
	);
	assert_eq!(board.overridden_players().collect::<Vec<_>>(), [user(5)]);

	// The first slot's value is written back and sent.
	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(world.get(c"m_iScore", 2), 3);
	assert_eq!(board.players.keys().collect::<Vec<_>>(), [&2]);
}

#[test]
fn always_flag_marks_every_write() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board.set_always_flag(true);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	end_snapshot();

	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
}

#[test]
fn an_after_frame_without_interfaces_leaves_the_repair_to_before_frame() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// The game's value is written back without a mark, and the `after_frame`
	// that would apply the override again cannot reach the interfaces.
	board.before_frame(server);
	board.after_frame(null_server(Game::TeamFortress2, &scope));
	assert_eq!(board.phase, Phase::Restored { unflagged: true });
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	end_snapshot();

	board.before_frame(server);
	assert_eq!(changed(RESOURCE), Changed::Full);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
}

#[test]
fn an_idle_store_makes_no_engine_calls() {
	// Every interface the store could reach aborts if called.
	for (module, version) in [
		(Module::Engine, ValveEngine::VERSION),
		(Module::GameServer, ServerTools::VERSION),
		(Module::GameServer, ServerGameDll::VERSION),
	] {
		let vtable = Box::leak(vec![unexpected_call as *const (); 256].into_boxed_slice());

		export(module, version, leak(vtable.as_ptr()));
	}

	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	board.before_frame(server);
	board.after_frame(server);
	board.restore_all(server);
	board.clear_player_stat(user(5), PlayerStat::Score);
	board.clear_team_stat(ScoringTeam::Red, TeamStat::Score);
	board.forget_player(user(5));
	board.clear_all_overrides();
	board.level_shutdown(LevelPolicy::ClearOverrides);
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(board.phase, Phase::Applied);
}

#[test]
fn arrays_must_be_contiguous_and_long_enough() {
	for (name, breakage, stat, needed) in [
		(c"m_iDamage", Breakage::Stride(8), PlayerStat::Damage, 2),
		(c"m_iDeaths", Breakage::Len(1), PlayerStat::Deaths, 2),
		(
			c"m_iStreaks",
			Breakage::Len(STREAKS_PER_SLOT),
			PlayerStat::Killstreak,
			5,
		),
	] {
		let _world = World::new(8, Some((name, breakage)));
		let scope = ();
		let server = mock_server(&scope);
		let mut board = Scoreboard::new();

		connect(1, 5);

		let error = board
			.set_player_stat(server, user(5), stat, Override::Fixed(1))
			.unwrap_err();

		assert_eq!(
			error,
			ScoreboardError::UnexpectedLayout {
				name: name.to_str().unwrap(),
				needed,
			}
		);
		assert!(board.player_stat_range(server, stat).is_err());
		assert!(board.player_stat_range(server, PlayerStat::Score).is_ok());
	}

	let _world = World::new(8, Some((c"m_iPing", Breakage::Len(3))));
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	// The array covers slots 1 and 2, but not 3.
	connect(3, 7);
	assert_eq!(
		board.set_player_stat(server, user(7), PlayerStat::Ping, Override::Fixed(1)),
		Err(ScoreboardError::UnexpectedLayout {
			name: "m_iPing",
			needed: 4,
		})
	);
}

/// `IVEngineServer::GetChangeAccessor`, which returns the edict's own accessor.
unsafe extern "C" fn change_accessor(
	_: *mut sys::IVEngineServer,
	edict: *const sys::edict_t,
) -> *mut sys::IChangeInfoAccessor {
	// SAFETY: The engine is only asked about edicts of the world's table.
	let index = usize::try_from(unsafe { (*edict)._base.m_EdictIndex }).unwrap();

	// SAFETY: The world leaks one accessor for each edict of its table, which
	// `index` lies within.
	unsafe { ACCESSORS.get().add(index) }
}

/// What the engine recorded as changed for edict `index` since the last
/// snapshot.
fn changed(index: usize) -> Changed {
	// SAFETY: The world leaks its edicts, their accessors and the shared change
	// info, and the tests only ask about edicts within its table.
	unsafe {
		let flags = (*EDICTS.get().add(index))._base.m_fStateFlags;

		if flags & FL_FULL_EDICT_CHANGED != 0 {
			return Changed::Full;
		}

		if flags & FL_EDICT_CHANGED == 0 {
			return Changed::Nothing;
		}

		let accessor = ACCESSORS.get().add(index);
		let shared = SHARED.get();

		assert_eq!(
			(*accessor).m_iChangeInfoSerialNumber,
			(*shared).m_iSerialNumber
		);

		let info = (*shared).m_ChangeInfos[usize::from((*accessor).m_iChangeInfo)];
		let mut offsets = info.m_ChangeOffsets[..usize::from(info.m_nChangeOffsets)].to_vec();

		offsets.sort_unstable();
		Changed::Offsets(offsets)
	}
}

#[test]
fn changes_past_the_engines_limit_mark_the_whole_resource() {
	let limit = usize::from(MAX_CHANGE_OFFSETS);

	for count in [limit, limit + 1] {
		let _world = World::new(8, None);
		let scope = ();
		let server = mock_server(&scope);
		let mut board = Scoreboard::new();
		let fields = [(1, 5), (2, 6)]
			.into_iter()
			.flat_map(|(slot, id)| PlayerField::ALL.map(|field| (slot, id, field)));

		for (slot, id, field) in fields.take(count) {
			connect(slot, id);
			board
				.set_player_field(server, user(id), field, Override::Fixed(1))
				.unwrap();
		}

		board.before_frame(server);
		board.after_frame(server);

		match changed(RESOURCE) {
			Changed::Offsets(offsets) => assert_eq!((offsets.len(), count), (limit, limit)),
			changed => assert_eq!((changed, count), (Changed::Full, limit + 1)),
		}
	}

	// An offset the engine cannot record marks the entity as a whole too.
	let _world = World::new(8, None);
	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();
	let far = Changes {
		full: false,
		offsets: vec![usize::from(u16::MAX) + 1],
	};

	far.flush(engine, edict_at(RESOURCE));
	assert_eq!(changed(RESOURCE), Changed::Full);
	end_snapshot();

	let near = Changes {
		full: false,
		offsets: vec![12, 8],
	};

	near.flush(engine, edict_at(RESOURCE));
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![8, 12]));
}

/// `IServerNetworkable::GetClassName`, which returns the fake entity's.
unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Only fake entities' networkables have this vtable, and `spawn`
	// leaks the entities.
	unsafe { (*container(this)).class_name }
}

#[test]
fn classes_and_teams_are_bounded() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(2, 6);
	world.put(c"m_iPlayerClass", 2, 3);
	world.put(c"m_bAlive", 2, 1);
	board
		.set_player_class(server, user(6), PlayerClass::Spy)
		.unwrap();
	board.set_player_alive(server, user(6), false).unwrap();
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iPlayerClass", 2), 8);
	assert_eq!(world.get(c"m_bAlive", 2), 0);
	assert_eq!(board.player_class_override(user(6)), Some(PlayerClass::Spy));
	assert_eq!(board.player_alive_override(user(6)), Some(false));
	assert_eq!(board.player_override(user(6), PlayerStat::Score), None);
	assert_eq!(board.overridden_players().collect::<Vec<_>>(), [user(6)]);
	end_snapshot();

	board.clear_player_class(user(6));
	board.clear_player_alive(user(6));
	assert_eq!(board.player_class_override(user(6)), None);
	assert_eq!(board.player_alive_override(user(6)), None);
	assert_eq!(board.overridden_players().count(), 0);
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iPlayerClass", 2), 3);
	assert_eq!(world.get(c"m_bAlive", 2), 1);
	assert_eq!(
		changed(RESOURCE),
		Changed::Offsets(vec![
			world.offset(c"m_bAlive", 2),
			world.offset(c"m_iPlayerClass", 2)
		])
	);
}

#[test]
fn clearing_writes_the_game_value_back_and_marks_it() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let deaths = world.offset(c"m_iDeaths", 1);
	let ping = world.offset(c"m_iPing", 1);

	connect(1, 5);
	world.put(c"m_iDeaths", 1, 3);
	world.put(c"m_iPing", 1, 60);
	board
		.set_player_stat(server, user(5), PlayerStat::Deaths, Override::Fixed(0))
		.unwrap();
	board
		.set_player_stat(server, user(5), PlayerStat::Ping, Override::Fixed(500))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// Cleared between frames: written back and marked before the frame.
	board.clear_player_stat(user(5), PlayerStat::Deaths);
	board.before_frame(server);
	assert_eq!(world.get(c"m_iDeaths", 1), 3);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![deaths]));

	// Cleared during the frame: marked after it.
	board.clear_player_stat(user(5), PlayerStat::Ping);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iPing", 1), 60);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![ping, deaths]));
	assert_eq!(board.real_player_stat(user(5), PlayerStat::Ping), None);
	end_snapshot();

	// Nothing is left to do.
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert!(board.players.is_empty());
}

/// Makes `user_id` the client owning player slot `slot`.
fn connect(slot: usize, user_id: u16) {
	USER_IDS.with_borrow_mut(|ids| ids[slot] = c_int::from(user_id));
}

/// The entity owning `networkable`.
fn container(networkable: *const sys::IServerNetworkable) -> *mut FakeEntity {
	networkable
		.wrapping_byte_sub(offset_of!(FakeEntity, networkable))
		.cast::<FakeEntity>()
		.cast_mut()
}

/// `CBaseEntity::GetDataDescMap`, which returns the fake entity's datamap.
unsafe extern "C" fn datamap(this: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this vtable, and `spawn` leaks them.
	unsafe { (*this.cast::<FakeEntity>()).map }
}

/// Leaves player slot `slot` without a client.
fn disconnect(slot: usize) {
	USER_IDS.with_borrow_mut(|ids| ids[slot] = -1);
}

/// `IServerNetworkable::GetEdict`, which returns the fake entity's.
unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	// SAFETY: As for `class_name`.
	unsafe { (*container(this)).edict }
}

/// The edict at `index`.
fn edict_at(index: usize) -> Edict<'static> {
	// SAFETY: The world leaks its edict table, which the tests only index
	// within.
	unsafe { Edict::from_raw(NonNull::new(EDICTS.get().add(index)).unwrap()) }
}

/// A leaked name for element `index` of an array, as `SendPropArray3` names
/// them.
fn element_name(index: usize) -> &'static CStr {
	Box::leak(
		CString::new(format!("{index:03}"))
			.unwrap()
			.into_boxed_c_str(),
	)
}

#[test]
fn encodable_ranges_follow_send_prop_int() {
	let range = |bits: c_int, flags: PropFlags| {
		let prop = int_prop(c"m_iValue", 0, bits, flags);

		// SAFETY: The property is an integer one, as a game DLL's send table
		// holds, and outlives the handle, which is used only in the call.
		encodable_range(unsafe { SendProp::from_raw(NonNull::from(&prop)) }).inclusive()
	};
	let varint = PropFlags::NORMAL;
	let unsigned_varint = PropFlags::NORMAL | PropFlags::UNSIGNED;

	assert_eq!(range(1, PropFlags::UNSIGNED), 0..=1);
	assert_eq!(range(5, PropFlags::UNSIGNED), 0..=31);
	assert_eq!(range(8, PropFlags::default()), -128..=127);
	assert_eq!(range(12, PropFlags::default()), -2048..=2047);
	assert_eq!(range(32, PropFlags::default()), i32::MIN..=i32::MAX);
	assert_eq!(range(32, PropFlags::UNSIGNED), 0..=i32::MAX);
	assert_eq!(range(0, PropFlags::default()), i32::MIN..=i32::MAX);
	assert_eq!(range(-1, PropFlags::UNSIGNED), 0..=i32::MAX);
	assert_eq!(range(40, PropFlags::default()), i32::MIN..=i32::MAX);
	assert_eq!(range(7, varint), i32::MIN..=i32::MAX);
	assert_eq!(range(-1, unsigned_varint), 0..=i32::MAX);
}

/// Sends the frame to clients, as the engine does after the frame: change
/// flags are cleared, and change records start over.
fn end_snapshot() {
	// SAFETY: The world leaks its edict table of `EDICT_COUNT` edicts and the
	// shared change info, and no reference to them is live.
	unsafe {
		for index in 0..EDICT_COUNT {
			(*EDICTS.get().add(index))._base.m_fStateFlags &=
				!(FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED);
		}

		let shared = SHARED.get();

		(*shared).m_iSerialNumber += 1;
		(*shared).m_nChangeInfos = 0;
	}
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	LIST.get()
}

unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	name: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The tools are passed a NUL-terminated class name.
	let name = unsafe { CStr::from_ptr(name) };

	BY_CLASS
		.with_borrow(|entities| {
			let mut matching = entities
				.iter()
				.filter(|(class, _)| *class == name)
				.map(|&(_, entity)| entity);

			if after.is_null() {
				matching.next()
			} else {
				matching.skip_while(|&entity| entity != after).nth(1)
			}
		})
		.unwrap_or(null_mut())
}

#[test]
fn fixed_overrides_are_applied_once_and_restored_around_each_frame() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();

	// Nothing was applied yet, so the first restore has nothing to do.
	board.before_frame(server);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	board.after_frame(server);

	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
	end_snapshot();

	// The game's logic sees its own value, and clients the override, without
	// either being marked changed again.
	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(changed(RESOURCE), Changed::Nothing);

	// Neighbouring slots and columns are untouched.
	assert_eq!(world.get(c"m_iTotalScore", 2), 0);
	assert_eq!(world.get(c"m_iScore", 1), 0);
}

#[test]
fn forgotten_players_are_restored_at_the_next_frame() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	board.forget_player(user(5));
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);

	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert!(board.players.is_empty());
}

#[test]
fn frags_and_deaths_are_written_through_the_player_datamap() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let player = world.player(2);
	// SAFETY: The player is a fake entity, which `spawn` leaks, and no
	// reference to it is live.
	let data = |word: usize| unsafe { (*player.as_ptr().cast::<FakeEntity>()).data[word] };

	set_frags(server, player, 17).unwrap();
	set_deaths(server, player, -2).unwrap();
	assert_eq!((data(FRAGS), data(DEATHS)), (17, -2));

	// SAFETY: `spawn` leaks the resource, which is in the entity list, and its
	// vtable answers what the scoreboard calls.
	let resource = unsafe { Entity::from_raw(NonNull::new(world.resource.cast()).unwrap()) };

	assert_eq!(
		set_frags(server, resource, 1),
		Err(ScoreboardError::NotTfPlayer)
	);

	// A misaligned field is not trusted.
	let misaligned = field(c"m_iFrags", sys::_fieldtypes_FIELD_INTEGER, DATA + 2);

	let base = data_map(c"CBasePlayer", vec![misaligned], null_mut());

	// SAFETY: As for `data`.
	unsafe {
		(*player.as_ptr().cast::<FakeEntity>()).map = data_map(c"CTFPlayer", Vec::new(), base)
	};
	assert_eq!(
		set_frags(server, player, 1),
		Err(ScoreboardError::MissingField {
			class: "CBasePlayer",
			name: "m_iFrags",
		})
	);
	assert_eq!(data(FRAGS), 17);
}

#[test]
fn full_length_arrays_reach_the_last_player_slot() {
	let world = World::new(102, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(101, 9);
	board
		.set_player_stat(server, user(9), PlayerStat::Killstreak, Override::Fixed(3))
		.unwrap();
	board
		.set_player_stat(server, user(9), PlayerStat::Score, Override::Fixed(5))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);

	assert_eq!(
		world.get(c"m_iStreaks", 101 * STREAKS_PER_SLOT + KILL_STREAK),
		3
	);
	assert_eq!(world.get(c"m_iTotalScore", 101), 5);
}

/// `IHandleEntity::GetRefEHandle`, which returns the fake entity's handle.
unsafe extern "C" fn handle(this: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
	// SAFETY: As for `datamap`.
	unsafe { (&raw const (*this.cast::<FakeEntity>()).handle).cast() }
}

/// A `SendPropArray3`-shaped array of `len` integers at `offset`.
fn int_array(
	name: &'static CStr,
	offset: usize,
	len: usize,
	stride: usize,
	bits: c_int,
	flags: PropFlags,
) -> sys::SendProp {
	let elements = (0..len)
		.map(|index| int_prop(element_name(index), index * stride, bits, flags))
		.collect::<Vec<_>>();
	let nested = leak(table(name, Box::leak(elements.into_boxed_slice())));

	table_prop(name, offset as c_int, nested, Some(direct_table))
}

/// An integer property at `offset`.
fn int_prop(name: &'static CStr, offset: usize, bits: c_int, flags: PropFlags) -> sys::SendProp {
	let mut prop = prop(
		name,
		sys::SendPropType_DPT_Int,
		offset as c_int,
		flags,
		Some(int32_proxy),
	);

	prop.m_nBits = bits;
	prop
}

#[test]
fn killstreaks_use_the_kill_element_of_the_slots_group() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(2, 6);
	board
		.set_player_stat(server, user(6), PlayerStat::Killstreak, Override::Fixed(7))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);

	let first = 2 * STREAKS_PER_SLOT;

	assert_eq!(world.get(c"m_iStreaks", first + KILL_STREAK), 7);
	assert_eq!(
		(first..first + STREAKS_PER_SLOT)
			.map(|element| world.get(c"m_iStreaks", element))
			.filter(|&value| value == 7)
			.count(),
		1
	);
	assert_eq!(world.get(c"m_iStreaks", 2), 0);
}

#[test]
fn layouts_and_players_are_checked() {
	let _world = World::new(8, Some((c"m_iCurrencyCollected", Breakage::Missing)));
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);

	// One missing column leaves the others usable.
	assert_eq!(
		board.set_player_stat(
			server,
			user(5),
			PlayerStat::CurrencyCollected,
			Override::Fixed(1)
		),
		Err(ScoreboardError::NetProp(NetPropError::NotFound {
			table: "DT_TFPlayerResource".to_owned(),
			name: "m_iCurrencyCollected".to_owned(),
		}))
	);
	assert!(
		board
			.set_player_stat(server, user(5), PlayerStat::Healing, Override::Fixed(1))
			.is_ok()
	);

	// Players must be connected clients, in a player slot.
	assert_eq!(
		board.set_player_stat(server, user(7), PlayerStat::Score, Override::Fixed(1)),
		Err(ScoreboardError::NotConnected)
	);

	// Without a player resource, no level runs.
	BY_CLASS.with_borrow_mut(|entities| entities.retain(|(name, _)| *name != RESOURCE_CLASS_NAME));
	let mut fresh = Scoreboard::new();

	assert_eq!(
		fresh.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(1)),
		Err(ScoreboardError::NoPlayerResource)
	);
}

#[test]
fn level_shutdown_forgets_entities_by_policy() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board
		.set_team_stat(
			server,
			ScoringTeam::Blue,
			TeamStat::Score,
			Override::Offset(1),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// Shutting down takes no server, so it cannot touch an entity.
	board.level_shutdown(LevelPolicy::KeepOverrides);
	assert_eq!(board.real_player_stat(user(5), PlayerStat::Score), None);
	assert_eq!(
		board.player_override(user(5), PlayerStat::Score),
		Some(Override::Fixed(9999))
	);
	assert_eq!(
		board.team_override(ScoringTeam::Blue, TeamStat::Score),
		Some(Override::Offset(1))
	);
	assert!(board.resource.is_none());
	assert!(!board.has_applied());

	// The next level's entities start from the game's values.
	world.put(c"m_iTotalScore", 1, 0);
	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 0);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_SCORE), 2);

	board.level_shutdown(LevelPolicy::ClearOverrides);
	assert!(!board.has_active());
	assert_eq!(board.player_override(user(5), PlayerStat::Score), None);
	assert_eq!(
		board.team_override(ScoringTeam::Blue, TeamStat::Score),
		None
	);
}

/// `IServerUnknown::GetNetworkable`, which returns the fake entity's.
unsafe extern "C" fn networkable(this: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*this.cast::<FakeEntity>()).networkable }
}

#[test]
fn overrides_end_when_another_client_takes_the_slot() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// The slot is taken by another client without `forget_player`.
	connect(1, 9);
	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	world.set(server, c"m_iTotalScore", 1, 0);
	board.after_frame(server);

	assert_eq!(world.get(c"m_iTotalScore", 1), 0);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	assert_eq!(board.real_player_stat(user(5), PlayerStat::Score), None);
	assert_eq!(board.real_player_stat(user(9), PlayerStat::Score), None);
	assert!(board.players.is_empty());

	// Setting an override for the new client starts afresh, and the previous
	// client's user ID is no longer connected.
	board
		.set_player_stat(server, user(9), PlayerStat::Kills, Override::Fixed(1))
		.unwrap();
	assert_eq!(board.slot_of(user(9)), Some(1));
	assert_eq!(board.player_override(user(9), PlayerStat::Score), None);
	assert_eq!(
		board.set_player_stat(server, user(5), PlayerStat::Kills, Override::Fixed(1)),
		Err(ScoreboardError::NotConnected)
	);
}

/// `IVEngineServer::GetPlayerUserId`, which returns the user ID [`connect`]
/// gave the edict's slot, or -1.
unsafe extern "C" fn player_user_id(
	_: *mut sys::IVEngineServer,
	edict: *const sys::edict_t,
) -> c_int {
	// SAFETY: The engine is only asked about edicts of the world's table.
	let index = usize::try_from(unsafe { (*edict)._base.m_EdictIndex }).unwrap();

	USER_IDS.with_borrow(|ids| ids[index])
}

#[test]
fn ranges_come_from_the_live_bits_and_flags() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let range = |board: &mut Scoreboard, stat| board.player_stat_range(server, stat).unwrap();

	assert_eq!(range(&mut board, PlayerStat::Kills), -2048..=2047);
	assert_eq!(range(&mut board, PlayerStat::Ping), 0..=1023);
	assert_eq!(range(&mut board, PlayerStat::Dominations), 0..=63);
	assert_eq!(range(&mut board, PlayerStat::Score), 0..=i32::MAX);
	assert_eq!(range(&mut board, PlayerStat::Killstreak), 0..=i32::MAX);
	assert_eq!(
		board.team_stat_range(server, ScoringTeam::Red, TeamStat::Score),
		Ok(i32::MIN..=i32::MAX)
	);
	assert_eq!(
		board.team_stat_range(server, ScoringTeam::Blue, TeamStat::FlagCaptures),
		Ok(-128..=127)
	);

	// Fixed values must fit.
	connect(1, 5);
	let player = user(5);

	assert_eq!(
		board.set_player_stat(server, player, PlayerStat::Kills, Override::Fixed(2048)),
		Err(ScoreboardError::OutOfRange {
			name: "m_iScore",
			value: 2048,
			min: -2048,
			max: 2047,
		})
	);
	assert!(
		board
			.set_player_stat(server, player, PlayerStat::Kills, Override::Fixed(-2049))
			.is_err()
	);
	assert!(
		board
			.set_player_stat(server, player, PlayerStat::Score, Override::Fixed(-1))
			.is_err()
	);
	assert!(
		board
			.set_player_stat(server, player, PlayerStat::Score, Override::Fixed(i32::MAX))
			.is_ok()
	);

	// Offsets saturate.
	world.put(c"m_iPing", 1, 1000);
	world.put(c"m_iScore", 1, -2000);
	world.put(c"m_iDamage", 1, i32::MAX - 5);
	board
		.set_player_stat(server, player, PlayerStat::Ping, Override::Offset(100))
		.unwrap();
	board
		.set_player_stat(server, player, PlayerStat::Kills, Override::Offset(-100))
		.unwrap();
	board
		.set_player_stat(
			server,
			player,
			PlayerStat::Damage,
			Override::Offset(i32::MAX),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);

	assert_eq!(world.get(c"m_iPing", 1), 1023);
	assert_eq!(world.get(c"m_iScore", 1), -2048);
	assert_eq!(world.get(c"m_iDamage", 1), i32::MAX);
}

#[test]
fn real_team_numbers_become_the_games_own() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = (DATA + TEAM_SCORE * ELEMENT_SIZE) as u16;

	set_team_score(server, ScoringTeam::Red, 2).unwrap();
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 2);
	assert_eq!(changed(RED), Changed::Offsets(vec![score]));
	set_team_flag_captures(server, ScoringTeam::Blue, -3).unwrap();
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_CAPTURES), -3);
	assert_eq!(
		set_team_flag_captures(server, ScoringTeam::Blue, 300),
		Err(ScoreboardError::OutOfRange {
			name: "m_nFlagCaptures",
			value: 300,
			min: -128,
			max: 127,
		})
	);
	end_snapshot();

	// Under a display override, a real write between frames is the game's.
	board
		.set_team_stat(
			server,
			ScoringTeam::Red,
			TeamStat::Score,
			Override::Fixed(9),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	set_team_score(server, ScoringTeam::Red, 4).unwrap();
	board.before_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 4);
	board.after_frame(server);
	assert_eq!(
		board.real_team_stat(ScoringTeam::Red, TeamStat::Score),
		Some(4)
	);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 9);
	end_snapshot();

	// Writing the override's own value through the store makes it the game's.
	// Written past the store, it cannot be told from the override, which is
	// why the store's method exists.
	set_team_score(server, ScoringTeam::Red, 9).unwrap();
	board.before_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 4);
	board.after_frame(server);
	end_snapshot();

	board
		.set_real_team_stat(server, ScoringTeam::Red, TeamStat::Score, 9)
		.unwrap();
	assert_eq!(changed(RED), Changed::Offsets(vec![score]));
	assert_eq!(
		board.real_team_stat(ScoringTeam::Red, TeamStat::Score),
		Some(9)
	);
	end_snapshot();

	board.before_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 9);
	board.after_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 9);
	assert_eq!(
		board.real_team_stat(ScoringTeam::Red, TeamStat::Score),
		Some(9)
	);
	assert_eq!(
		board.team_override(ScoringTeam::Red, TeamStat::Score),
		Some(Override::Fixed(9))
	);

	// During the frame, after the game's value was written back, it is the
	// same.
	board.before_frame(server);
	board
		.set_real_team_stat(server, ScoringTeam::Red, TeamStat::Score, 2)
		.unwrap();
	board.after_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 9);
	assert_eq!(
		board.real_team_stat(ScoringTeam::Red, TeamStat::Score),
		Some(2)
	);
	board.before_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 2);

	// Values must fit, as for the free functions.
	assert_eq!(
		board.set_real_team_stat(server, ScoringTeam::Blue, TeamStat::FlagCaptures, 128),
		Err(ScoreboardError::OutOfRange {
			name: "m_nFlagCaptures",
			value: 128,
			min: -128,
			max: 127,
		})
	);
}

/// `CTFPlayer::ResetScores`, which counts the call in the fake entity.
unsafe extern "C" fn reset_player_scores(this: *mut sys::CTFPlayer) {
	// SAFETY: As for `datamap`.
	unsafe { (*this.cast::<FakeEntity>()).resets += 1 };
}

#[test]
fn reset_scores_calls_the_game_once() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let player = world.player(2);
	// SAFETY: The player is a fake entity, which `spawn` leaks, and no
	// reference to it is live.
	let resets = || unsafe { (*player.as_ptr().cast::<FakeEntity>()).resets };

	reset_scores(server, player).unwrap();
	assert_eq!(resets(), 1);

	// SAFETY: `spawn` leaks the resource, which is in the entity list, and its
	// vtable answers what the scoreboard calls.
	let resource = unsafe { Entity::from_raw(NonNull::new(world.resource.cast()).unwrap()) };

	assert_eq!(
		reset_scores(server, resource),
		Err(ScoreboardError::NotTfPlayer)
	);
	assert_eq!(resets(), 1);
}

#[test]
fn restore_all_writes_everything_back_and_keeps_the_overrides() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board
		.set_team_stat(
			server,
			ScoringTeam::Red,
			TeamStat::Score,
			Override::Fixed(5),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 5);
	end_snapshot();

	board.restore_all(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 0);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
	assert_eq!(
		changed(RED),
		Changed::Offsets(vec![(DATA + TEAM_SCORE * ELEMENT_SIZE) as u16])
	);
	assert_eq!(changed(BLUE), Changed::Nothing);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(120)
	);
	end_snapshot();

	// While paused, the hooks do not run. Once they do again, the overrides
	// return.
	board.before_frame(server);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 5);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score]));
}

/// `IServerNetworkable::GetServerClass`, which returns the fake entity's.
unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	// SAFETY: As for `class_name`.
	unsafe { (*container(this)).class }
}

/// A leaked server class named `name` describing `table`.
fn server_class_of(name: &'static CStr, table: *mut sys::SendTable) -> *mut sys::ServerClass {
	leak(sys::ServerClass {
		m_pNetworkName: name.as_ptr(),
		m_pTable: table,
		m_pNext: null_mut(),
		m_ClassID: 1,
		m_InstanceBaselineIndex: 0,
	})
}

unsafe extern "C" fn shared_change_info(
	_: *mut sys::IVEngineServer,
) -> *mut sys::CSharedEdictChangeInfo {
	SHARED.get()
}

#[test]
fn streaks_must_be_grouped_by_player_slot() {
	// Long enough for slot 1, but not 4 streaks for each of the 8 slots.
	for len in [33, 36, 28] {
		let _world = World::new(8, Some((c"m_iStreaks", Breakage::Len(len))));
		let scope = ();
		let server = mock_server(&scope);
		let mut board = Scoreboard::new();

		connect(1, 5);
		assert_eq!(
			board.set_player_stat(server, user(5), PlayerStat::Killstreak, Override::Fixed(1)),
			Err(ScoreboardError::UnexpectedStreaks { len, slots: 8 })
		);
		assert!(board.player_stat_range(server, PlayerStat::Score).is_ok());
	}

	// Without the score's array, the slots cannot be counted.
	let _world = World::new(8, Some((c"m_iTotalScore", Breakage::Missing)));
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();

	connect(1, 5);
	assert!(matches!(
		board.set_player_stat(server, user(5), PlayerStat::Killstreak, Override::Fixed(1)),
		Err(ScoreboardError::NetProp(NetPropError::NotFound { .. }))
	));
	assert!(
		board
			.set_player_stat(server, user(5), PlayerStat::Kills, Override::Fixed(1))
			.is_ok()
	);
}

#[test]
fn teams_are_found_by_number() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let captures = (DATA + TEAM_CAPTURES * ELEMENT_SIZE) as u16;

	// SAFETY: The world leaks its teams, and no reference to them is live.
	unsafe { (*world.blue).data[TEAM_CAPTURES] = 1 };
	board
		.set_team_stat(
			server,
			ScoringTeam::Blue,
			TeamStat::FlagCaptures,
			Override::Offset(2),
		)
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);

	assert_eq!(world.team(ScoringTeam::Blue, TEAM_CAPTURES), 3);
	assert_eq!(world.team(ScoringTeam::Red, TEAM_CAPTURES), 0);
	assert_eq!(changed(BLUE), Changed::Offsets(vec![captures]));
	assert_eq!(changed(RED), Changed::Nothing);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(
		board.real_team_stat(ScoringTeam::Blue, TeamStat::FlagCaptures),
		Some(1)
	);
	end_snapshot();

	board.clear_team_stat(ScoringTeam::Blue, TeamStat::FlagCaptures);
	board.before_frame(server);
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_CAPTURES), 1);
	assert_eq!(changed(BLUE), Changed::Offsets(vec![captures]));
	board.after_frame(server);
	assert_eq!(
		board.real_team_stat(ScoringTeam::Blue, TeamStat::FlagCaptures),
		None
	);

	// Without a team entity, nothing can be set.
	BY_CLASS.with_borrow_mut(|entities| entities.retain(|(name, _)| *name != TEAM_CLASS_NAME));
	assert_eq!(
		set_team_score(server, ScoringTeam::Red, 1),
		Err(ScoreboardError::NoTeam(ScoringTeam::Red))
	);
}

#[test]
fn the_game_changes_values_between_the_callbacks() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let score = world.offset(c"m_iTotalScore", 1);
	let damage = world.offset(c"m_iDamage", 1);

	connect(1, 5);
	world.put(c"m_iTotalScore", 1, 120);
	world.put(c"m_iDamage", 1, 300);
	board
		.set_player_stat(server, user(5), PlayerStat::Score, Override::Fixed(9999))
		.unwrap();
	board
		.set_player_stat(server, user(5), PlayerStat::Damage, Override::Offset(50))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	assert_eq!(world.get(c"m_iDamage", 1), 350);
	end_snapshot();

	// The game's think recomputes both, from what it sees as its own values.
	board.before_frame(server);
	assert_eq!(world.get(c"m_iTotalScore", 1), 120);
	assert_eq!(world.get(c"m_iDamage", 1), 300);
	world.set(server, c"m_iTotalScore", 1, 122);
	world.set(server, c"m_iDamage", 1, 340);
	board.after_frame(server);

	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Score),
		Some(122)
	);
	assert_eq!(
		board.real_player_stat(user(5), PlayerStat::Damage),
		Some(340)
	);
	assert_eq!(world.get(c"m_iTotalScore", 1), 9999);
	assert_eq!(world.get(c"m_iDamage", 1), 390);

	// The game marked both; an offset follows the game's value, so it changed
	// for clients anyway.
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![score, damage]));
}

#[test]
fn values_written_between_frames_become_the_games_own() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut board = Scoreboard::new();
	let kills = world.offset(c"m_iScore", 1);

	connect(1, 5);
	world.put(c"m_iScore", 1, 4);
	board
		.set_player_stat(server, user(5), PlayerStat::Kills, Override::Fixed(42))
		.unwrap();
	board.before_frame(server);
	board.after_frame(server);
	end_snapshot();

	// Another plugin writes the variable between frames.
	world.put(c"m_iScore", 1, 7);
	board.before_frame(server);
	assert_eq!(world.get(c"m_iScore", 1), 7);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	board.after_frame(server);

	assert_eq!(board.real_player_stat(user(5), PlayerStat::Kills), Some(7));
	assert_eq!(world.get(c"m_iScore", 1), 42);
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![kills]));
}
