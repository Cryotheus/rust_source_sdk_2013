//! Tests of `crate::tf2::scoreboard`: player scores against a fake game
//! statistics singleton, fake entities, send tables and datamaps shaped as
//! TF2's, and an engine that records changes as the real one does.

use super::*;
use crate::Module;
use crate::test_support::datatables::{direct_table, int32_proxy, prop, table, table_prop};
use crate::test_support::edicts::{edict_of_index, edict_table, serve_edicts};
use crate::test_support::interfaces::server_game_dll::export_standard_proxies;
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
use sdk_raw::entities::NUM_SERIAL_NUM_SHIFT_BITS;
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::tf2::scoreboard::game_stats;
use sdk_raw::test_support::{mock_vtable, unexpected_call};

use sdk_raw::tf2::scoreboard::{
	PLAYER_STATS_SIZE, STATS_ACCUMULATED, STATS_CURRENT_ROUND, STREAKS_PER_SLOT,
};

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::mem::zeroed;
use std::ptr::null_mut;

/// Where the fake game statistics singleton keeps its blocks, as the Windows
/// build does.
const ARRAY: usize = 0xd8;

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

/// Where players keep their scoring data, `tfsharedlocaldata`, in words of
/// their data.
const LOCAL: usize = SHARED + 8;

/// Where players embed their `CPlayerState`, `pl`, in words of their data.
const PLAYER_STATE: usize = 4;

/// Where the scoring data keeps `m_iPoints`, in words, as the Windows build
/// does.
const POINTS: usize = 0x54 / ELEMENT_SIZE;

const RED: usize = 111;

/// The edict indices of the player resource and the teams.
const RESOURCE: usize = 110;

/// Where players keep the round's scoring data, in words of their data.
const ROUND_SCORE_DATA: usize = LOCAL + SCORE_DATA_WORDS;

/// Where players keep the session's scoring data, in words of their data.
const SCORE_DATA: usize = LOCAL;

/// Words of each scoring data.
const SCORE_DATA_WORDS: usize = 30;

/// Where the scoring data keeps `m_iKills`, in words.
const SCORING_KILLS: usize = 2;

/// Where players keep `m_Shared`, in words of their data.
const SHARED: usize = 32;

/// Where players keep `m_nStreaks`, in words of their data.
const STREAKS: usize = SHARED;

const TEAM_CAPTURES: usize = 3;

/// Where the teams keep `m_iTeamNum`, `m_iScore`, and `m_nFlagCaptures`, in
/// words of their data.
const TEAM_NUMBER: usize = 0;

const TEAM_SCORE: usize = 1;

thread_local! {
	static ACCESSORS: Cell<*mut sys::IChangeInfoAccessor> = const { Cell::new(null_mut()) };
	static BY_CLASS: RefCell<Vec<(&'static CStr, *mut sys::CBaseEntity)>> = const { RefCell::new(Vec::new()) };

	/// The players the fake `CalcPlayerScore` was given, by address.
	static CALC_PLAYERS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
	static EDICTS: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
	static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// Whether players have the `scoreboard_minigame` attribute.
	static MINIGAME: Cell<bool> = const { Cell::new(false) };
	static SHARED_INFO: Cell<*mut sys::CSharedEdictChangeInfo> = const { Cell::new(null_mut()) };
}

/// How one array of the player resource is broken, for layout tests.
#[derive(Debug, Clone, Copy)]
enum Breakage {
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

/// A fake `CTF_GameStats`, large enough for the blocks at [`ARRAY`].
#[repr(C, align(8))]
struct Singleton([u8; ARRAY + MAX_PLAYERS_ARRAY_SAFE * PLAYER_STATS_SIZE]);

/// A fake game: a player resource, two teams, players, the game statistics,
/// and the interfaces the scoreboard reaches them through.
struct World {
	/// The data word of each array's first element, by name.
	arrays: Vec<(&'static CStr, usize)>,
	blue: *mut FakeEntity,
	game_stats: GameStats,
	player_class: *mut sys::ServerClass,
	player_map: *mut sys::datamap_t,
	players: Vec<(usize, *mut FakeEntity)>,
	red: *mut FakeEntity,
	resource: *mut FakeEntity,
	resource_class: *mut sys::ServerClass,

	/// The fake `CTF_GameStats`, leaked.
	singleton: *mut Singleton,
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
		SHARED_INFO.set(shared);

		// SAFETY: The entity list holds integers, a `bool` and raw pointers, for
		// all of which zero is valid.
		LIST.set(Box::into_raw(unsafe {
			Box::<sys::CGlobalEntityList>::new_zeroed().assume_init()
		}));
		BY_CLASS.take();
		CALC_PLAYERS.take();
		MINIGAME.set(false);

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

		// SAFETY: The singleton is bytes, for which zero is valid.
		let singleton = Box::into_raw(unsafe { Box::<Singleton>::new_zeroed().assume_init() });
		let game_stats = game_stats(
			NonNull::new(singleton).unwrap().cast(),
			size_of::<Singleton>(),
			ARRAY,
			calc_player_score,
			find_player_stats,
		);

		let mut world = Self {
			arrays,
			blue: null_mut(),
			game_stats,
			player_class: player_class(),
			player_map: player_map(
				player_state_map(
					offset_of!(sys::CPlayerState, deadflag),
					offset_of!(sys::CPlayerState, v_angle),
				),
				DATA + FRAGS * ELEMENT_SIZE,
			),
			players: Vec::new(),
			red: null_mut(),
			resource: null_mut(),
			resource_class,
			singleton,
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

		for slot in [1, 2, 3, slots - 1] {
			let player = world.spawn(slot, 1, world.player_class, c"player", world.player_map);

			world.players.push((slot, player));
		}

		world
	}

	/// Reads a player's data word `word`.
	fn data(&self, slot: usize, word: usize) -> i32 {
		// SAFETY: `spawn` leaks the players, and no reference to them is live.
		unsafe { (*self.fake(slot)).data[word] }
	}

	/// The fake entity of the player in `slot`.
	fn fake(&self, slot: usize) -> *mut FakeEntity {
		self.players
			.iter()
			.find(|(found, _)| *found == slot)
			.unwrap()
			.1
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
		// SAFETY: `spawn` leaks the players, which are in the entity list, and
		// their vtable answers what the scoreboard calls.
		unsafe { Entity::from_raw(NonNull::new(self.fake(slot).cast()).unwrap()) }
	}

	/// Writes `name`'s element `element` in the player resource, without
	/// telling the engine.
	fn put(&self, name: &CStr, element: usize, value: i32) {
		// SAFETY: As for `get`.
		unsafe { (*self.resource).data[self.word(name, element)] = value };
	}

	/// Writes a player's data word `word`.
	fn put_data(&self, slot: usize, word: usize, value: i32) {
		// SAFETY: As for `data`.
		unsafe { (*self.fake(slot)).data[word] = value };
	}

	/// Writes statistic `stat` of `slot`'s block at `block`, such as
	/// [`STATS_ACCUMULATED`].
	fn put_stat(&self, slot: usize, block: usize, stat: Stat, value: i32) {
		// SAFETY: The singleton is leaked, the statistic lies within it, and no
		// reference to it is live.
		unsafe {
			self.singleton
				.byte_add(stat_offset(slot, block, stat))
				.cast::<i32>()
				.write_unaligned(value);
		}
	}

	/// The scoring state of the player in `slot`, with the fake statistics.
	fn score<'w>(&'w self, server: Server<'w>, slot: usize) -> PlayerScore<'w> {
		ScoreboardLayout::new()
			.player_with(server, self.player(slot), || Ok(self.game_stats))
			.unwrap()
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

	/// Reads statistic `stat` of `slot`'s block at `block`.
	fn stat(&self, slot: usize, block: usize, stat: Stat) -> i32 {
		// SAFETY: As for `put_stat`.
		unsafe {
			self.singleton
				.byte_add(stat_offset(slot, block, stat))
				.cast::<i32>()
				.read_unaligned()
		}
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
fn a_replaced_resource_is_found_again() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let mut layout = ScoreboardLayout::new();
	let player = world.player(1);
	let fake = || Ok(world.game_stats);

	layout
		.player_with(server, player, fake)
		.unwrap()
		.add_points(2, Adjust::default())
		.unwrap();
	assert_eq!(world.get(c"m_iTotalScore", 1), 2);

	// The resource is replaced, in the same edict, by one with another serial
	// number.
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

	layout
		.player_with(server, player, fake)
		.unwrap()
		.add_points(3, Adjust::default())
		.unwrap();
	assert_eq!(score(), 3);
	assert_eq!(world.get(c"m_iTotalScore", 1), 2);

	// An entity reporting another server class is not trusted with the
	// layout either: it is resolved again from that class.
	// SAFETY: The world leaks its server classes, which are plain data.
	let copy = leak(unsafe { world.resource_class.read() });

	// SAFETY: As for `score`.
	unsafe { (*replacement).class = copy };
	layout
		.player_with(server, player, fake)
		.unwrap()
		.add_points(1, Adjust::default())
		.unwrap();
	assert_eq!(score(), 4);
	assert_eq!(
		layout.resource.as_ref().map(|cache| cache.class),
		Some(copy.addr())
	);
}

#[test]
fn arrays_must_be_contiguous_and_long_enough() {
	for breakage in [Breakage::Stride(8), Breakage::Len(1)] {
		let world = World::new(8, Some((c"m_iTotalScore", breakage)));
		let scope = ();
		let server = mock_server(&scope);

		assert_eq!(
			ScoreboardLayout::new()
				.player_with(server, world.player(1), || Ok(world.game_stats))
				.unwrap_err(),
			ScoreError::UnexpectedLayout {
				name: "m_iTotalScore",
				needed: 2,
			}
		);
	}

	// The array covers slots 1 and 2, but not 3.
	let world = World::new(8, Some((c"m_iTotalScore", Breakage::Len(3))));
	let scope = ();
	let server = mock_server(&scope);

	assert!(
		ScoreboardLayout::new()
			.player_with(server, world.player(2), || Ok(world.game_stats))
			.is_ok()
	);
	assert_eq!(
		ScoreboardLayout::new()
			.player_with(server, world.player(3), || Ok(world.game_stats))
			.unwrap_err(),
		ScoreError::UnexpectedLayout {
			name: "m_iTotalScore",
			needed: 4,
		}
	);

	// A broken column only fails what needs it.
	let world = World::new(8, Some((c"m_iDeaths", Breakage::Stride(8))));
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 1);

	assert_eq!(score.frags_range(), Ok(-2048..=2047));
	assert_eq!(
		score.deaths_range(),
		Err(ScoreError::UnexpectedLayout {
			name: "m_iDeaths",
			needed: 2,
		})
	);
}

/// `CTFGameRules::CalcPlayerScore`, as the SDK's source computes it for the
/// statistics the tests use: kills and points, one each, damage, a point per
/// 600, and with the `scoreboard_minigame` attribute, kills again and -3 per
/// death, clamped at 0. Records the player it is given.
unsafe extern "C" fn calc_player_score(
	stats: *mut RoundStats,
	player: *mut sys::CTFPlayer,
) -> c_int {
	CALC_PLAYERS.with_borrow_mut(|players| players.push(player.addr()));

	// SAFETY: The scoreboard passes a complete `RoundStats`.
	let stats = unsafe { stats.read() }.stat;
	let mut score = stats[stat::KILLS]
		.wrapping_add(stats[stat::KILLS_RUNECARRIER])
		.wrapping_add(stats[stat::DAMAGE] / 600);

	if !player.is_null() && MINIGAME.get() {
		score = score
			.wrapping_add(stats[stat::KILLS])
			.wrapping_sub(3 * stats[stat::DEATHS]);
	}

	score.max(0)
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
		let shared = SHARED_INFO.get();

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
fn changes_past_the_engines_limit_mark_the_whole_entity() {
	let limit = usize::from(MAX_CHANGE_OFFSETS);
	let _world = World::new(8, None);
	let scope = ();
	let engine = mock_server(&scope).valve_engine().unwrap();

	for count in [limit, limit + 1] {
		let changes = Changes {
			offsets: (0..count).map(|index| 8 + index * ELEMENT_SIZE).collect(),
		};

		changes.flush(engine, edict_at(RESOURCE));

		match changed(RESOURCE) {
			Changed::Offsets(offsets) => assert_eq!((offsets.len(), count), (limit, limit)),
			changed => assert_eq!((changed, count), (Changed::Full, limit + 1)),
		}

		end_snapshot();
	}

	// An offset the engine cannot record marks the entity as a whole too.
	let far = Changes {
		offsets: vec![usize::from(u16::MAX) + 1],
	};

	far.flush(engine, edict_at(RESOURCE));
	assert_eq!(changed(RESOURCE), Changed::Full);
	end_snapshot();

	let near = Changes {
		offsets: vec![12, 8],
	};

	near.flush(engine, edict_at(RESOURCE));
	assert_eq!(changed(RESOURCE), Changed::Offsets(vec![8, 12]));
}

#[test]
fn clamped_scores_change_by_what_clients_see() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);

	world.put_stat(2, STATS_ACCUMULATED, Stat::Kills, 2);
	world.put(c"m_iTotalScore", 2, 2);

	let score = world.score(server, 2);

	// Taken below 0, the Score shows 0, and the points stay owed.
	assert_eq!(
		score.add_points(-5, Adjust::default()),
		Ok(Applied {
			round_shown: 0,
			shown: -2,
		})
	);
	assert_eq!(score.total(), 0);
	assert_eq!(score.points(), -5);
	assert_eq!(world.get(c"m_iTotalScore", 2), 0);
	end_snapshot();

	// The total is set exactly, whatever is owed.
	assert_eq!(
		score.set_total(4, Adjust::default()),
		Ok(Applied {
			round_shown: 2,
			shown: 4,
		})
	);
	assert_eq!((score.total(), score.points()), (4, 2));
	assert_eq!(world.get(c"m_iTotalScore", 2), 4);
	assert_eq!(score.set_total(0, Adjust::default()).unwrap().shown, -4);
	assert_eq!((score.total(), score.points()), (0, -2));
	end_snapshot();

	// A total already shown as 0 needs nothing written.
	world.put_stat(2, STATS_ACCUMULATED, Stat::KillsRuneCarrier, -40);
	assert_eq!(
		score.set_total(0, Adjust::default()),
		Ok(Applied::default())
	);
	assert_eq!(score.points(), -40);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(changed(2), Changed::Nothing);
	assert_eq!(score.set_total(1, Adjust::default()).unwrap().shown, 1);
	assert_eq!((score.total(), score.points()), (1, -1));
}

/// `IServerNetworkable::GetClassName`, which returns the fake entity's.
unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
	// SAFETY: Only fake entities' networkables have this vtable, and `spawn`
	// leaks the entities.
	unsafe { (*container(this)).class_name }
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

		let shared = SHARED_INFO.get();

		(*shared).m_iSerialNumber += 1;
		(*shared).m_nChangeInfos = 0;
	}
}

unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	LIST.get()
}

#[test]
fn every_score_change_is_compensated_and_marked() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let player_offset = |word: usize| u16::try_from(DATA + word * ELEMENT_SIZE).unwrap();
	let points = player_offset(SCORE_DATA + POINTS);
	let round_points = player_offset(ROUND_SCORE_DATA + POINTS);

	// The game's last think saw a Score of 10 and a round score of 4.
	world.put_stat(3, STATS_ACCUMULATED, Stat::Kills, 10);
	world.put_stat(3, STATS_CURRENT_ROUND, Stat::Kills, 4);
	world.put(c"m_iTotalScore", 3, 10);
	world.put_data(3, SCORE_DATA + POINTS, 10);
	world.put_data(3, ROUND_SCORE_DATA + POINTS, 4);

	let score = world.score(server, 3);

	assert_eq!(
		score.add_points(3, Adjust::default()),
		Ok(Applied {
			round_shown: 3,
			shown: 3,
		})
	);
	assert_eq!(world.get(c"m_iTotalScore", 3), 13);
	assert_eq!(world.data(3, SCORE_DATA + POINTS), 13);
	assert_eq!(world.data(3, ROUND_SCORE_DATA + POINTS), 7);
	assert_eq!(
		changed(RESOURCE),
		Changed::Offsets(vec![world.offset(c"m_iTotalScore", 3)])
	);
	assert_eq!(changed(3), Changed::Offsets(vec![points, round_points]));
	end_snapshot();

	// Without the round, only the session's score moves.
	assert_eq!(
		score.add_points(-1, Adjust { round: false }),
		Ok(Applied {
			round_shown: 0,
			shown: -1,
		})
	);
	assert_eq!(
		world.stat(3, STATS_CURRENT_ROUND, Stat::KillsRuneCarrier),
		3
	);
	assert_eq!(world.data(3, ROUND_SCORE_DATA + POINTS), 7);
	assert_eq!(world.data(3, SCORE_DATA + POINTS), 12);
	assert_eq!(changed(3), Changed::Offsets(vec![points]));
	end_snapshot();

	// A difference the game has not sent yet is kept, so it still reports it:
	// the compensation adds what the change shows, without recomputing.
	world.put_stat(3, STATS_ACCUMULATED, Stat::Kills, 15);
	score.add_points(1, Adjust::default()).unwrap();
	assert_eq!(world.get(c"m_iTotalScore", 3), 13);
	assert_eq!(score.total(), 18);
	end_snapshot();

	// A statistic that does not move the scores is written without marks.
	assert_eq!(
		score.add_stat(Stat::Damage, 599, Adjust::default()),
		Ok(Applied::default())
	);
	assert_eq!(world.stat(3, STATS_ACCUMULATED, Stat::Damage), 599);
	assert_eq!(world.stat(3, STATS_CURRENT_ROUND, Stat::Damage), 599);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(changed(3), Changed::Nothing);
	assert_eq!(
		score.add_stat(Stat::Damage, 1, Adjust::default()),
		Ok(Applied {
			round_shown: 1,
			shown: 1,
		})
	);
	assert_eq!(
		score.set_stat(Stat::Damage, 1200, Adjust { round: false }),
		Ok(Applied {
			round_shown: 0,
			shown: 1,
		})
	);
	assert_eq!(score.stat(Stat::Damage, StatScope::Session), 1200);
	assert_eq!(score.stat(Stat::Damage, StatScope::Round), 600);
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

/// `CTFGameStats::FindPlayerStats`, as the game implements it: the block of
/// the player's edict index, without checking it.
unsafe extern "C" fn find_player_stats(
	this: *mut c_void,
	player: *mut sys::CBasePlayer,
) -> *mut PlayerStats {
	// SAFETY: The scoreboard passes fake entities, whose edicts are in the
	// world's table.
	let index = unsafe { (*(*player.cast::<FakeEntity>()).edict)._base.m_EdictIndex };

	this.wrapping_byte_add(ARRAY)
		.wrapping_byte_add(usize::try_from(index).unwrap() * PLAYER_STATS_SIZE)
		.cast()
}

#[test]
fn frags_and_deaths_are_written_with_the_engines_copy() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 2);
	let copy = |field: usize| PLAYER_STATE + field / ELEMENT_SIZE;
	let frags_copy = copy(offset_of!(sys::CPlayerState, frags));
	let deaths_copy = copy(offset_of!(sys::CPlayerState, deaths));

	assert_eq!(score.set_frags(17), Ok(true));
	assert_eq!(score.add_deaths(-2), Ok(true));
	assert_eq!((world.data(2, FRAGS), world.data(2, frags_copy)), (17, 17));
	assert_eq!(
		(world.data(2, DEATHS), world.data(2, deaths_copy)),
		(-2, -2)
	);
	assert_eq!((score.frags(), score.deaths()), (Ok(17), Ok(-2)));

	// A copy that disagreed is not written: the count may be elsewhere.
	world.put_data(2, frags_copy, 5);
	assert_eq!(score.add_frags(1), Ok(false));
	assert_eq!((world.data(2, FRAGS), world.data(2, frags_copy)), (18, 5));
	assert_eq!(score.add_frags(i32::MAX), Err(ScoreError::Overflow));
	assert_eq!(world.data(2, FRAGS), 18);

	// A player state laid out unlike the bindings' is not written at all.
	let wrong = player_state_map(
		offset_of!(sys::CPlayerState, deadflag),
		offset_of!(sys::CPlayerState, v_angle) + ELEMENT_SIZE,
	);

	// SAFETY: `spawn` leaks the players, and no reference to them is live.
	unsafe { (*world.fake(2)).map = player_map(wrong, DATA + FRAGS * ELEMENT_SIZE) };
	assert_eq!(
		score.set_frags(1),
		Err(ScoreError::MissingField {
			class: "CBasePlayer",
			name: "pl",
		})
	);
	assert_eq!(world.data(2, FRAGS), 18);

	// Nor is a misaligned count.
	let right = player_state_map(
		offset_of!(sys::CPlayerState, deadflag),
		offset_of!(sys::CPlayerState, v_angle),
	);

	// SAFETY: As above.
	unsafe { (*world.fake(2)).map = player_map(right, DATA + 2) };
	assert_eq!(
		score.set_frags(1),
		Err(ScoreError::MissingField {
			class: "CBasePlayer",
			name: "m_iFrags",
		})
	);
	assert_eq!(world.data(2, FRAGS), 18);
}

#[test]
fn full_length_arrays_reach_the_last_player_slot() {
	let world = World::new(MAX_PLAYERS_ARRAY_SAFE, None);
	let scope = ();
	let server = mock_server(&scope);
	let last = MAX_PLAYERS_ARRAY_SAFE - 1;

	world
		.score(server, last)
		.add_points(5, Adjust::default())
		.unwrap();
	assert_eq!(world.get(c"m_iTotalScore", last), 5);
	assert_eq!(
		world.stat(last, STATS_ACCUMULATED, Stat::KillsRuneCarrier),
		5
	);
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
fn killstreaks_are_the_players_kill_element() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 1);
	let element = STREAKS + KILL_STREAK;

	score.set_killstreak(7).unwrap();
	assert_eq!(world.data(1, element), 7);
	assert_eq!(score.killstreak(), Ok(7));
	assert_eq!(
		(STREAKS..STREAKS + STREAKS_PER_SLOT)
			.filter(|&word| world.data(1, word) != 0)
			.count(),
		1
	);
	assert_eq!(
		changed(1),
		Changed::Offsets(vec![u16::try_from(DATA + element * ELEMENT_SIZE).unwrap()])
	);
	assert_eq!(
		score.set_killstreak(-1),
		Err(ScoreError::OutOfRange {
			name: "m_nStreaks",
			value: -1,
			min: 0,
			max: i32::MAX,
		})
	);
	assert_eq!(world.data(1, element), 7);
}

#[test]
fn layouts_and_players_are_checked() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let unreachable =
		|| -> Result<GameStats, GameStatsError> { panic!("the game's statistics were resolved") };
	let mut layout = ScoreboardLayout::new();

	// Only TF2 players, with statistics blocks, have scores; the game's
	// statistics are only resolved for them.
	// SAFETY: `spawn` leaks the resource, which is in the entity list, and its
	// vtable answers what the scoreboard calls.
	let resource = unsafe { Entity::from_raw(NonNull::new(world.resource.cast()).unwrap()) };

	assert_eq!(
		layout
			.player_with(server, resource, unreachable)
			.unwrap_err(),
		ScoreError::NotTfPlayer
	);
	assert_eq!(
		layout
			.player_with(
				null_server(Game::SourceSdk2013, &scope),
				world.player(1),
				unreachable
			)
			.unwrap_err(),
		ScoreError::NotTf2
	);

	let beyond = world.spawn(
		MAX_PLAYERS_ARRAY_SAFE,
		1,
		world.player_class,
		c"player",
		world.player_map,
	);
	// SAFETY: As for `resource`.
	let beyond = unsafe { Entity::from_raw(NonNull::new(beyond.cast()).unwrap()) };

	assert_eq!(
		layout.player_with(server, beyond, unreachable).unwrap_err(),
		ScoreError::NoPlayerStats
	);

	// A failed resolution is the game statistics' error.
	assert_eq!(
		layout
			.player_with(server, world.player(1), || {
				Err(GameStatsError::SelfTestFailed)
			})
			.unwrap_err(),
		ScoreError::GameStats(GameStatsError::SelfTestFailed)
	);

	// The game finds the block of the player's edict, which must be the one of
	// its entity index.
	// SAFETY: `spawn` leaks the players, and no reference to them is live.
	unsafe { (*world.fake(2)).edict = EDICTS.get().add(3) };
	assert_eq!(
		layout
			.player_with(server, world.player(2), || Ok(world.game_stats))
			.unwrap_err(),
		ScoreError::NoPlayerStats
	);

	// Without a player resource, no level runs.
	BY_CLASS.with_borrow_mut(|entities| entities.retain(|(name, _)| *name != RESOURCE_CLASS_NAME));
	assert_eq!(
		ScoreboardLayout::new()
			.player_with(server, world.player(1), || Ok(world.game_stats))
			.unwrap_err(),
		ScoreError::NoPlayerResource
	);
}

/// `IServerUnknown::GetNetworkable`, which returns the fake entity's.
unsafe extern "C" fn networkable(this: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	// SAFETY: As for `datamap`.
	unsafe { &raw mut (*this.cast::<FakeEntity>()).networkable }
}

#[test]
fn overflows_write_nothing() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);

	world.put_stat(1, STATS_CURRENT_ROUND, Stat::KillsRuneCarrier, i32::MAX - 1);

	let score = world.score(server, 1);

	assert_eq!(
		score.add_points(2, Adjust::default()),
		Err(ScoreError::Overflow)
	);
	assert_eq!(world.stat(1, STATS_ACCUMULATED, Stat::KillsRuneCarrier), 0);

	// Nor may the scores the game compares against overflow.
	world.put(c"m_iTotalScore", 1, i32::MAX);
	assert_eq!(
		score.add_points(1, Adjust { round: false }),
		Err(ScoreError::Overflow)
	);
	assert_eq!(world.stat(1, STATS_ACCUMULATED, Stat::KillsRuneCarrier), 0);
	assert_eq!(world.get(c"m_iTotalScore", 1), i32::MAX);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(changed(1), Changed::Nothing);
	assert_eq!(
		score.set_total(u32::MAX, Adjust::default()),
		Err(ScoreError::Overflow)
	);
}

/// The server class of TF2's players, with the variables the scoreboard uses
/// nested as `DT_TFPlayer` nests them, under tables whose proxies pass their
/// data through.
fn player_class() -> *mut sys::ServerClass {
	let field_offset = |word: usize, base: usize| ((word - base) * ELEMENT_SIZE) as c_int;
	let scoring = Box::leak(Box::new([
		int_prop(
			c"m_iKills",
			SCORING_KILLS * ELEMENT_SIZE,
			10,
			PropFlags::UNSIGNED,
		),
		int_prop(c"m_iPoints", POINTS * ELEMENT_SIZE, 10, PropFlags::UNSIGNED),
	]));
	let scoring = leak(table(c"DT_TFPlayerScoringDataExclusive", scoring));
	let local = Box::leak(Box::new([
		table_prop(
			c"m_ScoreData",
			field_offset(SCORE_DATA, LOCAL),
			scoring,
			Some(direct_table),
		),
		table_prop(
			c"m_RoundScoreData",
			field_offset(ROUND_SCORE_DATA, LOCAL),
			scoring,
			Some(direct_table),
		),
	]));
	let local = leak(table(c"DT_TFPlayerSharedLocal", local));
	let streaks = int_array(
		c"m_nStreaks",
		0,
		STREAKS_PER_SLOT,
		ELEMENT_SIZE,
		32,
		PropFlags::default(),
	);
	let shared = Box::leak(Box::new([
		streaks,
		table_prop(
			c"tfsharedlocaldata",
			field_offset(LOCAL, SHARED),
			local,
			Some(direct_table),
		),
	]));
	let shared = leak(table(c"DT_TFPlayerShared", shared));
	let player = Box::leak(Box::new([table_prop(
		c"m_Shared",
		(DATA + SHARED * ELEMENT_SIZE) as c_int,
		shared,
		Some(direct_table),
	)]));

	server_class_of(c"CTFPlayer", leak(table(c"DT_TFPlayer", player)))
}

/// A `CBasePlayer` datamap declaring the frag count at `frags`, the death
/// count, and `pl` described by `state`, under a `CTFPlayer` one.
fn player_map(state: *mut sys::datamap_t, frags: usize) -> *mut sys::datamap_t {
	let int = |name: &'static CStr, offset: usize| {
		let mut field = field(name, sys::_fieldtypes_FIELD_INTEGER, offset);

		field.fieldSizeInBytes = ELEMENT_SIZE as c_int;
		field
	};
	let mut pl = field(
		c"pl",
		sys::_fieldtypes_FIELD_EMBEDDED,
		DATA + PLAYER_STATE * ELEMENT_SIZE,
	);

	pl.td = state;

	let base_player = data_map(
		c"CBasePlayer",
		vec![
			int(c"m_iFrags", frags),
			int(c"m_iDeaths", DATA + DEATHS * ELEMENT_SIZE),
			pl,
		],
		null_mut(),
	);

	data_map(c"CTFPlayer", Vec::new(), base_player)
}

/// A `CPlayerState` datamap declaring `deadflag` and `v_angle` at the given
/// offsets, as the game declares them.
fn player_state_map(deadflag: usize, v_angle: usize) -> *mut sys::datamap_t {
	data_map(
		c"CPlayerState",
		vec![
			field(c"v_angle", sys::_fieldtypes_FIELD_VECTOR, v_angle),
			field(c"deadflag", sys::_fieldtypes_FIELD_BOOLEAN, deadflag),
		],
		null_mut(),
	)
}

#[test]
fn points_are_kept_in_the_players_statistics_block() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 2);
	let instance = world.game_stats.instance().addr().get();

	// The block is the one of the player's entity index.
	assert_eq!(
		score.stats.addr().get() - instance,
		ARRAY + 2 * PLAYER_STATS_SIZE
	);
	assert_eq!(
		score.add_points(5, Adjust::default()),
		Ok(Applied {
			round_shown: 5,
			shown: 5,
		})
	);

	// Statistic 43 of the session's block, at +0x168, and of the round's, at
	// +0xb4, and nothing else.
	// SAFETY: The singleton is leaked, and nothing writes it while the bytes
	// are borrowed.
	let bytes = unsafe { &(*world.singleton).0 };

	for block in [0x168, 0xb4] {
		let at = ARRAY + 2 * PLAYER_STATS_SIZE + block + 43 * ELEMENT_SIZE;

		assert_eq!(
			i32::from_ne_bytes(bytes[at..at + ELEMENT_SIZE].try_into().unwrap()),
			5
		);
	}

	let written = bytes
		.as_chunks::<ELEMENT_SIZE>()
		.0
		.iter()
		.filter(|word| word.iter().any(|&byte| byte != 0))
		.count();

	assert_eq!(written, 2);

	// The game scored the statistics with the player, so its attributes count.
	let player = world.fake(2).addr();

	assert!(CALC_PLAYERS.with_borrow(|players| {
		!players.is_empty() && players.iter().all(|&found| found == player)
	}));
	MINIGAME.set(true);
	assert_eq!(
		score.add_stat(Stat::Kills, 1, Adjust::default()),
		Ok(Applied {
			round_shown: 2,
			shown: 2,
		})
	);
	assert_eq!((score.total(), score.round_total()), (7, 7));
}

#[test]
fn ranges_come_from_the_live_bits_and_flags() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 1);
	let mut layout = ScoreboardLayout::new();

	assert_eq!(score.frags_range(), Ok(-2048..=2047));
	assert_eq!(score.deaths_range(), Ok(-2048..=2047));
	assert_eq!(
		layout.team_stat_range(server, ScoringTeam::Red, TeamStat::Score),
		Ok(i32::MIN..=i32::MAX)
	);
	assert_eq!(
		layout.team_stat_range(server, ScoringTeam::Blue, TeamStat::FlagCaptures),
		Ok(-128..=127)
	);
}

#[test]
fn rebasing_writes_the_computed_scores_and_marks_only_changes() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let player_offset = |word: usize| u16::try_from(DATA + word * ELEMENT_SIZE).unwrap();
	let points = player_offset(SCORE_DATA + POINTS);
	let round_points = player_offset(ROUND_SCORE_DATA + POINTS);

	// The slot still holds an earlier player's Score of 7, as the game leaves
	// it, while the player's scoring data agrees with their statistics.
	world.put_stat(3, STATS_ACCUMULATED, Stat::Kills, 10);
	world.put_stat(3, STATS_CURRENT_ROUND, Stat::Kills, 4);
	world.put(c"m_iTotalScore", 3, 7);
	world.put_data(3, SCORE_DATA + POINTS, 10);
	world.put_data(3, ROUND_SCORE_DATA + POINTS, 4);

	let score = world.score(server, 3);

	assert_eq!(
		score.rebase(),
		Ok(Rebased {
			discarded: 3,
			points_discarded: 0,
		})
	);
	assert_eq!(world.get(c"m_iTotalScore", 3), 10);
	assert_eq!(
		changed(RESOURCE),
		Changed::Offsets(vec![world.offset(c"m_iTotalScore", 3)])
	);
	assert_eq!(changed(3), Changed::Nothing);
	end_snapshot();

	// Agreeing values are left alone.
	assert_eq!(score.rebase(), Ok(Rebased::default()));
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(changed(3), Changed::Nothing);

	// Points the game awarded since its last think are discarded from every
	// value it compares against.
	world.put_stat(3, STATS_ACCUMULATED, Stat::Kills, 12);
	world.put_stat(3, STATS_CURRENT_ROUND, Stat::Kills, 6);
	assert_eq!(
		score.rebase(),
		Ok(Rebased {
			discarded: 2,
			points_discarded: 2,
		})
	);
	assert_eq!(world.get(c"m_iTotalScore", 3), 12);
	assert_eq!(world.data(3, SCORE_DATA + POINTS), 12);
	assert_eq!(world.data(3, ROUND_SCORE_DATA + POINTS), 6);
	assert_eq!(
		changed(RESOURCE),
		Changed::Offsets(vec![world.offset(c"m_iTotalScore", 3)])
	);
	assert_eq!(changed(3), Changed::Offsets(vec![points, round_points]));
	end_snapshot();

	// After a plugin's change that kept a difference the game had not sent,
	// the values agree with the scores again.
	world.put_stat(3, STATS_ACCUMULATED, Stat::Kills, 15);
	score.add_points(1, Adjust::default()).unwrap();
	assert_eq!(world.get(c"m_iTotalScore", 3), 13);
	assert_eq!(
		score.rebase(),
		Ok(Rebased {
			discarded: 3,
			points_discarded: 3,
		})
	);
	assert_eq!(world.get(c"m_iTotalScore", 3), score.total());
	assert_eq!(world.data(3, SCORE_DATA + POINTS), 16);
	assert_eq!(
		world.data(3, ROUND_SCORE_DATA + POINTS),
		score.round_total()
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

	assert_eq!(reset_scores(server, resource), Err(ScoreError::NotTfPlayer));
	assert_eq!(resets(), 1);
}

#[test]
fn round_points_are_found_in_their_own_table() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = world.score(server, 1);

	// The first `m_iPoints` by name is the session's.
	assert_eq!(
		server
			.server_game_dll()
			.unwrap()
			.entity_net_prop(world.player(1), c"m_iPoints")
			.unwrap()
			.offset(),
		score.layout.points
	);
	assert_eq!(
		(score.layout.points, score.layout.round_points),
		(
			DATA + (SCORE_DATA + POINTS) * ELEMENT_SIZE,
			DATA + (ROUND_SCORE_DATA + POINTS) * ELEMENT_SIZE
		)
	);
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
	SHARED_INFO.get()
}

/// The byte offset of statistic `stat` of `slot`'s block at `block` in the
/// fake singleton.
fn stat_offset(slot: usize, block: usize, stat: Stat) -> usize {
	ARRAY + slot * PLAYER_STATS_SIZE + block + stat.index() * ELEMENT_SIZE
}

#[test]
fn teams_are_found_by_number_and_marked() {
	let world = World::new(8, None);
	let scope = ();
	let server = mock_server(&scope);
	let score = (DATA + TEAM_SCORE * ELEMENT_SIZE) as u16;
	let captures = (DATA + TEAM_CAPTURES * ELEMENT_SIZE) as u16;

	set_team_score(server, ScoringTeam::Red, 2).unwrap();
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 2);
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_SCORE), 0);
	assert_eq!(changed(RED), Changed::Offsets(vec![score]));
	assert_eq!(changed(BLUE), Changed::Nothing);
	assert_eq!(add_team_score(server, ScoringTeam::Red, 3), Ok(5));
	assert_eq!(world.team(ScoringTeam::Red, TEAM_SCORE), 5);
	assert_eq!(
		add_team_score(server, ScoringTeam::Red, i32::MAX),
		Err(ScoreError::Overflow)
	);
	end_snapshot();

	set_team_flag_captures(server, ScoringTeam::Blue, -3).unwrap();
	assert_eq!(world.team(ScoringTeam::Blue, TEAM_CAPTURES), -3);
	assert_eq!(changed(BLUE), Changed::Offsets(vec![captures]));
	assert_eq!(changed(RED), Changed::Nothing);
	assert_eq!(changed(RESOURCE), Changed::Nothing);
	assert_eq!(
		set_team_flag_captures(server, ScoringTeam::Blue, 300),
		Err(ScoreError::OutOfRange {
			name: "m_nFlagCaptures",
			value: 300,
			min: -128,
			max: 127,
		})
	);
	assert_eq!(
		ScoreboardLayout::new().add_team_stat(
			server,
			ScoringTeam::Blue,
			TeamStat::FlagCaptures,
			200
		),
		Err(ScoreError::OutOfRange {
			name: "m_nFlagCaptures",
			value: 197,
			min: -128,
			max: 127,
		})
	);
	assert_eq!(
		ScoreboardLayout::new().team_stat(server, ScoringTeam::Blue, TeamStat::FlagCaptures),
		Ok(-3)
	);

	// Without a team entity, nothing can be set.
	BY_CLASS.with_borrow_mut(|entities| entities.retain(|(name, _)| *name != TEAM_CLASS_NAME));
	assert_eq!(
		set_team_score(server, ScoringTeam::Red, 1),
		Err(ScoreError::NoTeam(ScoringTeam::Red))
	);
}
