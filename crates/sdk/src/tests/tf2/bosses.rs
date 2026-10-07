//! Tests of `crate::tf2::bosses`: spawning through mock server tools, the
//! inputs the spawned entities are sent, and the boss bar through a fake
//! monster resource.

use super::*;
use crate::interfaces::{ModelInfo, ValveEngine};
use crate::server::Module;
use crate::test_support::entities::{
	MOCK_HEALTH_OFFSET, MOCK_MAX_HEALTH_OFFSET, MockEntity, activations, base_entity_fields,
	health_fields, set_datamap, take_inputs,
};
use crate::test_support::leak;
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::{export, mock_server, null_server};

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, input, int_prop, register_class_name,
};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use crate::test_support::tf2::spawning::{
	SpawnEvent, change_team, export_spawn_tools, patch_slots, refuse_key, remove_on_spawn,
	set_created, take_events,
};

use sdk_raw::test_support::entities::data_map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::BOOL;
use sdk_raw::tf2::teams::CHANGE_TEAM_SLOT;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

/// The precached model of the mock model registry.
const TANK_MODEL: &CStr = c"models/bots/boss_bot/boss_tank.mdl";

thread_local! {
	/// The flags `SetResolvePlayerCollisions` was called with.
	static RESOLVES: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn base_bosses_spawn_with_their_key_values_and_take_inputs() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = spawnable(
		c"CTFBaseBoss",
		vec![
			input(c"Enable", sys::_fieldtypes_FIELD_VOID),
			input(c"Disable", sys::_fieldtypes_FIELD_VOID),
			input(c"SetSpeed", sys::_fieldtypes_FIELD_FLOAT),
			input(c"SetStepHeight", sys::_fieldtypes_FIELD_FLOAT),
			input(c"SetMaxJumpHeight", sys::_fieldtypes_FIELD_FLOAT),
		],
	);

	export_model_info();

	let origin = Vector::new(0.5, 2.0, -64.0);
	let mut spawn = BaseBossSpawn::new(origin, TANK_MODEL, 5000);

	// Refused before anything is created.
	for (changed, error) in [
		(
			BaseBossSpawn {
				model: c"models/unknown.mdl",
				..spawn
			},
			BossError::ModelNotPrecached,
		),
		(
			BaseBossSpawn { health: 0, ..spawn },
			BossError::NonPositiveHealth(0),
		),
		(
			BaseBossSpawn {
				speed: Some(f32::NAN),
				..spawn
			},
			BossError::NonFinite,
		),
		(
			BaseBossSpawn {
				origin: Vector::new(f32::INFINITY, 0.0, 0.0),
				..spawn
			},
			BossError::NonFinite,
		),
	] {
		assert_eq!(BaseBoss::spawn(server, &changed).err(), Some(error));
	}

	assert_eq!(take_events(), []);

	spawn.speed = Some(80.0);
	spawn.team = Team::Blue;
	spawn.start_disabled = true;
	set_created(mock.as_ptr());

	let boss = BaseBoss::spawn(server, &spawn).unwrap();

	assert_eq!(boss.entity().as_ptr(), mock.as_ptr());
	assert_eq!(
		take_events(),
		[
			created(c"base_boss"),
			key_value(c"origin", c"0.5 2 -64"),
			key_value(c"model", TANK_MODEL),
			key_value(c"health", c"5000"),
			key_value(c"teamnumber", c"3"),
			key_value(c"start_disabled", c"1"),
			key_value(c"speed", c"80"),
			SpawnEvent::Spawned,
		]
	);

	// It is activated, as the level's own are.
	assert_eq!(activations(), [mock.as_ptr()]);

	boss.enable().unwrap();
	boss.disable().unwrap();
	boss.set_speed(120.0).unwrap();
	boss.set_step_height(18.0).unwrap();
	boss.set_max_jump_height(64.0).unwrap();

	let inputs = take_inputs();

	assert_eq!(
		inputs
			.iter()
			.map(|input| input.name.as_c_str())
			.collect::<Vec<_>>(),
		[
			c"Enable",
			c"Disable",
			c"SetSpeed",
			c"SetStepHeight",
			c"SetMaxJumpHeight"
		]
	);
	assert_eq!(
		f32::from_ne_bytes(inputs[2].payload[..4].try_into().unwrap()),
		120.0
	);
	assert!(
		inputs
			.iter()
			.all(|input| input.activator == mock.as_ptr() && input.caller == mock.as_ptr())
	);
	assert_eq!(
		boss.set_speed(f32::NAN).err(),
		Some(BossError::Input(InputError::NonFinite))
	);

	// The script method that only `CTFBaseBoss`'s script class declares.
	boss.set_resolve_player_collisions(false).unwrap();
	assert_eq!(RESOLVES.take(), [false]);

	// Wrapping checks the class.
	assert!(BaseBoss::new(server, boss.entity()).is_ok());
	set_datamap(data_map(
		c"CBaseEntity",
		base_entity_fields().to_vec(),
		null_mut(),
	));
	assert_eq!(
		BaseBoss::new(server, boss.entity()).err(),
		Some(BossError::WrongClass {
			class: c"CTFBaseBoss"
		})
	);
}

#[test]
fn halloween_bosses_spawn_on_their_team_with_their_health() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = spawnable(c"CHeadlessHatman", Vec::new());
	let origin = Vector::new(1.0, -2.5, 3.0);

	set_created(mock.as_ptr());

	let boss = spawn_halloween_boss(
		server,
		HalloweenBoss::Horsemann,
		origin,
		BossTeam::Halloween,
		Some(4000),
	)
	.unwrap();

	assert_eq!(boss.as_ptr(), mock.as_ptr());
	assert_eq!(
		take_events(),
		[
			created(c"headless_hatman"),
			key_value(c"origin", c"1 -2.5 3"),
			SpawnEvent::Team(5),
			SpawnEvent::Spawned,
		]
	);

	// It is not activated, as the game's own are not.
	assert_eq!(activations(), []);
	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), 4000);
	assert_eq!(mock.int(MOCK_MAX_HEALTH_OFFSET), 4000);

	// The boss's own health is kept without one given.
	mock.set_int(MOCK_HEALTH_OFFSET, 3000);
	set_created(mock.as_ptr());
	spawn_halloween_boss(
		server,
		HalloweenBoss::Monoculus,
		origin,
		BossTeam::Red,
		None,
	)
	.unwrap();
	assert_eq!(
		take_events(),
		[
			created(c"eyeball_boss"),
			key_value(c"origin", c"1 -2.5 3"),
			SpawnEvent::Team(2),
			SpawnEvent::Spawned,
		]
	);
	assert_eq!(mock.int(MOCK_HEALTH_OFFSET), 3000);

	// Refused before anything is created.
	assert_eq!(
		spawn_halloween_boss(
			server,
			HalloweenBoss::Merasmus,
			Vector::new(f32::NAN, 0.0, 0.0),
			BossTeam::Blue,
			None,
		)
		.err(),
		Some(BossError::NonFinite)
	);
	assert_eq!(
		spawn_halloween_boss(
			server,
			HalloweenBoss::Merasmus,
			origin,
			BossTeam::Blue,
			Some(-1),
		)
		.err(),
		Some(BossError::NonPositiveHealth(-1))
	);
	assert_eq!(take_events(), []);

	// The game creates nothing.
	assert_eq!(
		spawn_halloween_boss(
			server,
			HalloweenBoss::Merasmus,
			origin,
			BossTeam::Blue,
			None
		)
		.err(),
		Some(BossError::Spawn(SpawnError::UnknownClass {
			class: c"merasmus".to_owned()
		}))
	);
	assert_eq!(take_events(), [created(c"merasmus")]);

	// A refused key value removes the entity unspawned.
	refuse_key(c"origin");
	set_created(mock.as_ptr());
	assert_eq!(
		spawn_halloween_boss(
			server,
			HalloweenBoss::Merasmus,
			origin,
			BossTeam::Blue,
			None
		)
		.err(),
		Some(BossError::Spawn(SpawnError::KeyRejected {
			key: c"origin".to_owned()
		}))
	);
	assert_eq!(take_events(), [created(c"merasmus"), SpawnEvent::Removed]);

	// Another game's server.
	let other_game = null_server(Game::SourceSdk2013, &scope);

	assert_eq!(
		spawn_halloween_boss(
			other_game,
			HalloweenBoss::Horsemann,
			origin,
			BossTeam::Halloween,
			None
		)
		.err(),
		Some(BossError::UnsupportedGame)
	);
}

#[test]
fn skeletons_spawn_alone_or_from_spawners() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = spawnable(
		c"CZombieSpawner",
		vec![
			input(c"Enable", sys::_fieldtypes_FIELD_VOID),
			input(c"Disable", sys::_fieldtypes_FIELD_VOID),
			input(c"SetMaxActiveZombies", sys::_fieldtypes_FIELD_INTEGER),
		],
	);
	let origin = Vector::new(-8.0, 16.25, 0.0);

	set_created(mock.as_ptr());

	let skeleton = spawn_skeleton(server, origin, BossTeam::Blue).unwrap();

	assert_eq!(skeleton.as_ptr(), mock.as_ptr());
	assert_eq!(
		take_events(),
		[
			created(c"tf_zombie"),
			key_value(c"origin", c"-8 16.25 0"),
			SpawnEvent::Team(3),
			SpawnEvent::Spawned,
		]
	);
	assert_eq!(activations(), []);

	let mut spawn = SkeletonSpawn::new(SkeletonType::King);

	spawn.count = 3;
	spawn.infinite = true;
	spawn.lifetime = Some(12.5);
	set_created(mock.as_ptr());

	let spawner = SkeletonSpawner::spawn(server, origin, &spawn).unwrap();

	// The spawner's skeletons take the Halloween team for an unassigned
	// spawner.
	assert_eq!(
		take_events(),
		[
			created(c"tf_zombie_spawner"),
			key_value(c"origin", c"-8 16.25 0"),
			key_value(c"teamnumber", c"0"),
			key_value(c"zombie_type", c"1"),
			key_value(c"max_zombies", c"3"),
			key_value(c"infinite_zombies", c"1"),
			key_value(c"zombie_lifetime", c"12.5"),
			SpawnEvent::Spawned,
		]
	);
	assert_eq!(activations(), [mock.as_ptr()]);

	spawner.enable().unwrap();
	spawner.set_count(5).unwrap();
	spawner.disable().unwrap();

	let inputs = take_inputs();

	assert_eq!(
		inputs
			.iter()
			.map(|input| input.name.as_c_str())
			.collect::<Vec<_>>(),
		[c"Enable", c"SetMaxActiveZombies", c"Disable"]
	);
	assert_eq!(
		c_int::from_ne_bytes(inputs[1].payload[..4].try_into().unwrap()),
		5
	);

	// A spawner for RED, without a lifetime.
	spawn.team = BossTeam::Red;
	spawn.lifetime = None;
	set_created(mock.as_ptr());
	SkeletonSpawner::spawn(server, origin, &spawn).unwrap();

	let events = take_events();

	assert!(events.contains(&key_value(c"teamnumber", c"2")));
	assert!(events.contains(&key_value(c"zombie_lifetime", c"0")));

	assert_eq!(
		SkeletonSpawner::spawn(
			server,
			origin,
			&SkeletonSpawn {
				lifetime: Some(f32::INFINITY),
				..spawn
			}
		)
		.err(),
		Some(BossError::NonFinite)
	);

	// A skeleton that removes itself as it spawns.
	remove_on_spawn(true);
	set_created(mock.as_ptr());
	assert_eq!(
		spawn_skeleton(server, origin, BossTeam::Halloween).err(),
		Some(BossError::Spawn(SpawnError::RemovedItself))
	);
	assert_eq!(
		take_events(),
		[
			created(c"tf_zombie"),
			key_value(c"origin", c"-8 16.25 0"),
			SpawnEvent::Team(5),
			SpawnEvent::Spawned,
		]
	);

	// Wrapping checks the class.
	assert!(SkeletonSpawner::new(server, spawner.entity()).is_ok());
	set_datamap(data_map(
		c"CBaseEntity",
		base_entity_fields().to_vec(),
		null_mut(),
	));
	assert_eq!(
		SkeletonSpawner::new(server, spawner.entity()).err(),
		Some(BossError::WrongClass {
			class: c"CZombieSpawner"
		})
	);
}

#[test]
fn skeleton_types_and_boss_teams_map_to_the_games_numbers() {
	for (kind, raw) in [
		(SkeletonType::Normal, 0),
		(SkeletonType::King, 1),
		(SkeletonType::Mini, 2),
	] {
		assert_eq!(kind.to_raw(), raw);
		assert_eq!(SkeletonType::from_raw(raw), Some(kind));
	}

	assert_eq!(SkeletonType::from_raw(3), None);

	for (team, raw) in [
		(BossTeam::Red, 2),
		(BossTeam::Blue, 3),
		(BossTeam::Halloween, 5),
	] {
		assert_eq!(team.to_raw(), raw);
		assert_eq!(BossTeam::from_raw(raw), Some(team));
	}

	assert_eq!(BossTeam::from_raw(4), None);
	assert_eq!(
		HalloweenBoss::ALL.map(HalloweenBoss::class_name),
		[c"headless_hatman", c"eyeball_boss", c"merasmus"]
	);
}

#[test]
fn the_boss_bar_holds_its_fractions_as_bytes() {
	const HEALTH: usize = FIELDS_OFFSET;
	const STUN: usize = FIELDS_OFFSET + 4;
	const STATE: usize = FIELDS_OFFSET + 8;

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(c"CMonsterResource", Vec::new())],
		base_fields: Vec::new(),
		table: Some((
			c"DT_MonsterResource",
			vec![
				int_prop(c"m_iBossHealthPercentageByte", HEALTH),
				int_prop(c"m_iBossStunPercentageByte", STUN),
				int_prop(c"m_iBossState", STATE),
			],
		)),
	});

	export(
		Module::Engine,
		ValveEngine::VERSION,
		change_tracking_engine().as_ptr(),
	);
	register_class_name(fake.as_ptr(), c"monster_resource");

	let scope = ();
	let server = mock_server(&scope);
	let bar = BossBar::find(server).unwrap().unwrap();

	assert_eq!(bar.entity().as_ptr(), fake.as_ptr());
	assert_eq!(bar.is_shown(), Ok(false));

	bar.set_health(0.5).unwrap();
	assert_eq!(fake.int(HEALTH), 127);
	assert_eq!(bar.health(), Ok(127.0 / 255.0));
	assert_eq!(bar.is_shown(), Ok(true));

	// Fractions are clamped.
	bar.set_stun(2.0).unwrap();
	assert_eq!(fake.int(STUN), 255);
	assert_eq!(bar.stun(), Ok(1.0));
	bar.set_health(-1.0).unwrap();
	assert_eq!(bar.health(), Ok(0.0));
	assert_eq!(bar.set_health(f32::NAN), Err(BossError::NonFinite));

	bar.set_inactive(true).unwrap();
	assert_eq!(fake.int(STATE), 1);
	assert_eq!(bar.is_inactive(), Ok(true));
	bar.set_inactive(false).unwrap();
	assert_eq!(bar.is_inactive(), Ok(false));

	bar.set_health(1.0).unwrap();
	bar.hide().unwrap();
	assert_eq!((fake.int(HEALTH), fake.int(STUN)), (0, 0));
	assert_eq!(bar.is_shown(), Ok(false));
}

/// A `SpawnEvent::Created` of `class`.
fn created(class: &CStr) -> SpawnEvent {
	SpawnEvent::Created(class.to_owned())
}

/// Exports a model registry that has precached [`TANK_MODEL`] alone.
fn export_model_info() {
	/// `IVModelInfo::GetModelIndex`.
	unsafe extern "C" fn model_index(_: *const sys::IVModelInfo, name: *const c_char) -> c_int {
		// SAFETY: The wrapper passes a NUL-terminated name.
		if unsafe { CStr::from_ptr(name) } == TANK_MODEL {
			7
		} else {
			-1
		}
	}

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IVModelInfo__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVModelInfo_GetModelIndex).write(model_index);
		})
	});

	export(
		Module::Engine,
		ModelInfo::VERSION,
		leak(sys::IVModelInfo { vtable_: vtable }),
	);
}

/// A `SpawnEvent::KeyValue` of `key` and `value`.
fn key_value(key: &CStr, value: &CStr) -> SpawnEvent {
	SpawnEvent::KeyValue(key.to_owned(), value.to_owned())
}

/// The adapter of the fake `SetResolvePlayerCollisions`, which records its
/// flag.
unsafe extern "C" fn resolve_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	_: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 1);
	assert!(result.is_null());

	// SAFETY: `call` checked the argument's type against the binding.
	let resolve = unsafe { (*arguments).__bindgen_anon_1.m_bool };

	RESOLVES.with_borrow_mut(|resolves| resolves.push(resolve));
	true
}

/// A mock entity of the class `class`, declaring `inputs`, that the mock
/// spawning tools, exported with an engine, create next. Its `ChangeTeam`
/// records the team, and its `GetScriptDesc` returns a fake `CTFBaseBoss`
/// class declaring `SetResolvePlayerCollisions`.
fn spawnable(class: &'static CStr, inputs: Vec<sys::typedescription_t>) -> MockEntity {
	let mut mock = MockEntity::new(1);
	let mut fields = base_entity_fields().to_vec();

	fields.extend(health_fields());
	set_datamap(data_map(
		class,
		inputs,
		data_map(c"CBaseEntity", fields, null_mut()),
	));
	patch_slots(
		&mut mock,
		&[
			(CHANGE_TEAM_SLOT, change_team as *const ()),
			(SCRIPT_DESCRIPTION_SLOT, script_description as *const ()),
		],
	);

	let parameters = Vec::leak(vec![BOOL]);
	let bindings = Vec::leak(vec![member_binding(
		c"SetResolvePlayerCollisions",
		VOID,
		parameters,
		Some(resolve_adapter),
	)]);

	set_script_description(leak(class_description(
		c"CTFBaseBoss",
		bindings,
		null_mut(),
	)));
	export_spawn_tools();
	export(
		Module::Engine,
		ValveEngine::VERSION,
		change_tracking_engine().as_ptr(),
	);

	mock
}
