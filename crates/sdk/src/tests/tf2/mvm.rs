//! Tests of `crate::tf2::mvm`: a fake player's native methods, a fake
//! objective resource's networked wave, the populator interface's inputs, and
//! the command `load_popfile` queues.

use super::*;
use crate::interfaces::ValveEngine;
use crate::server::Module;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap, take_inputs};
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, bool_prop, float_prop, input, int_prop,
	register_class_name,
};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use crate::test_support::tf2::spawning::{
	SpawnEvent, export_spawn_tools, patch_slots, remove_on_spawn, set_created, take_events,
};

use sdk_raw::test_support::entities::data_map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

/// A native method of the fake `CTFPlayer` script class: its name, return
/// type, and parameter types.
type Method = (
	&'static CStr,
	sys::ScriptDataType_t,
	&'static [sys::ScriptDataType_t],
);

/// The methods the fake `CTFPlayer` script class declares.
const PLAYER_METHODS: &[Method] = &[
	(c"AddCurrency", VOID, &[INT]),
	(c"GetCurrency", INT, &[]),
	(c"GrantOrRemoveAllUpgrades", VOID, &[BOOL, BOOL]),
	(c"IsMiniBoss", BOOL, &[]),
	(c"RemoveCurrency", VOID, &[INT]),
	(c"SetCurrency", VOID, &[INT]),
	(c"SetIsMiniBoss", VOID, &[BOOL]),
	(c"SetUseBossHealthBar", VOID, &[BOOL]),
];

thread_local! {
	/// What the fake methods read and write.
	static STATE: RefCell<FakePlayer> = RefCell::new(FakePlayer::default());

	/// The commands the mock engine was asked to queue.
	static COMMANDS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// The fields of the fake player that its native methods read and write.
#[derive(Debug, Default)]
struct FakePlayer {
	currency: c_int,
	mini_boss: bool,
	boss_bar: bool,
	/// The arguments of each `GrantOrRemoveAllUpgrades`.
	upgrades: Vec<(bool, bool)>,
	/// The calls that reached a method.
	calls: usize,
}

/// A fake binding's adapter, which runs the fake method named by the
/// binding's function as the game's does, on the fake state.
unsafe extern "C" fn adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	_: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	// SAFETY: Every fake binding stores the address of its method's name, a
	// static C string, as its function.
	let name = unsafe { CStr::from_ptr(function.val_0 as *const c_char) };

	let argument = |index: usize| {
		assert!(index < count as usize);

		// SAFETY: `call` checked the arguments' types against the binding, and
		// passes `count` of them.
		unsafe { &(*arguments.add(index)).__bindgen_anon_1 }
	};

	STATE.with_borrow_mut(|state| {
		state.calls += 1;

		// SAFETY: Each method reads the union member of its declared parameter
		// types, which `call` checked.
		let value = unsafe {
			match name.to_bytes() {
				b"AddCurrency" => {
					state.currency = (state.currency + argument(0).m_int).clamp(0, 30_000);
					None
				}

				b"GetCurrency" => Some(int(state.currency)),

				b"GrantOrRemoveAllUpgrades" => {
					state
						.upgrades
						.push((argument(0).m_bool, argument(1).m_bool));

					None
				}

				b"IsMiniBoss" => Some(boolean(state.mini_boss)),

				b"RemoveCurrency" => {
					state.currency = (state.currency - argument(0).m_int).max(0);
					None
				}

				b"SetCurrency" => {
					state.currency = argument(0).m_int;
					None
				}

				b"SetIsMiniBoss" => {
					state.mini_boss = argument(0).m_bool;
					None
				}

				b"SetUseBossHealthBar" => {
					state.boss_bar = argument(0).m_bool;
					None
				}

				_ => panic!("unexpected method {name:?}"),
			}
		};

		if let Some(value) = value {
			assert!(!result.is_null());

			// SAFETY: The method returns a value, so the caller passes a
			// writable result.
			unsafe { result.write(value) };
		} else {
			assert!(result.is_null());
		}

		true
	})
}

/// Exports an engine that records the commands it is asked to queue.
fn export_engine() {
	/// `IVEngineServer::ServerCommand`, which records the command.
	unsafe extern "C" fn server_command(_: *mut sys::IVEngineServer, command: *const c_char) {
		// SAFETY: The wrapper passes a NUL-terminated command.
		let command = unsafe { CStr::from_ptr(command) }.to_owned();

		COMMANDS.with_borrow_mut(|commands| commands.push(command));
	}

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_ServerCommand).write(server_command);
		})
	});

	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer { vtable_: vtable }),
	);
}

#[test]
fn missions_are_queued_by_name() {
	let scope = ();
	let server = mock_server(&scope);

	export_engine();
	load_popfile(server, c"mvm_decoy_advanced").unwrap();
	assert_eq!(
		COMMANDS.take(),
		[c"tf_mvm_popfile \"mvm_decoy_advanced\"\n".to_owned()]
	);

	for name in [c"", c"say \"hi\"", c"a\nquit", c"tab\there"] {
		assert_eq!(
			load_popfile(server, name),
			Err(MvmError::InvalidPopfileName)
		);
	}

	assert_eq!(COMMANDS.take(), Vec::<CString>::new());
	assert_eq!(
		load_popfile(null_server(Game::SourceSdk2013, &scope), c"mvm_decoy"),
		Err(MvmError::UnsupportedGame)
	);
}

#[test]
fn players_money_and_flags_go_through_the_native_methods() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = MockEntity::new(1);

	set_datamap(data_map(
		c"CTFPlayer",
		Vec::new(),
		data_map(c"CBaseEntity", base_entity_fields().to_vec(), null_mut()),
	));
	patch_slots(
		&mut mock,
		&[(SCRIPT_DESCRIPTION_SLOT, script_description as *const ())],
	);

	let bindings = PLAYER_METHODS
		.iter()
		.map(|&(method, returns, parameters)| {
			let mut binding =
				member_binding(method, returns, Vec::from(parameters).leak(), Some(adapter));

			binding.m_pFunction.val_0 = method.as_ptr() as isize;
			binding
		})
		.collect::<Vec<_>>();

	set_script_description(leak(class_description(
		c"CTFPlayer",
		bindings.leak(),
		null_mut(),
	)));

	// SAFETY: The mock is leaked, and its vtable answers what the player calls.
	let entity = unsafe { Entity::from_raw(std::ptr::NonNull::new(mock.as_ptr()).unwrap()) };
	let player = MvmPlayer::new(server, entity).unwrap();

	player.set_currency(400).unwrap();
	assert_eq!(player.currency(), Ok(400));
	player.add_currency(50).unwrap();
	assert_eq!(player.currency(), Ok(450));

	// The game caps and floors the money.
	player.add_currency(40_000).unwrap();
	assert_eq!(player.currency(), Ok(MAX_ADDED_CURRENCY));
	player.remove_currency(30_500).unwrap();
	assert_eq!(player.currency(), Ok(0));

	// Sums that overflow are refused before the game is called.
	player.set_currency(c_int::MAX).unwrap();

	let calls = STATE.with_borrow(|state| state.calls);

	assert_eq!(
		player.add_currency(1),
		Err(MvmError::CurrencyOverflow {
			currency: c_int::MAX,
			amount: 1
		})
	);
	assert_eq!(
		player.remove_currency(-1),
		Err(MvmError::CurrencyOverflow {
			currency: c_int::MAX,
			amount: -1
		})
	);

	// Only the reads of the money reached the game.
	assert_eq!(STATE.with_borrow(|state| state.calls), calls + 2);
	assert_eq!(player.currency(), Ok(c_int::MAX));

	assert_eq!(player.is_mini_boss(), Ok(false));
	player.set_mini_boss(true).unwrap();
	assert_eq!(player.is_mini_boss(), Ok(true));

	player.set_use_boss_health_bar(true).unwrap();
	assert!(STATE.with_borrow(|state| state.boss_bar));

	player.remove_upgrades(true).unwrap();
	player.remove_upgrades(false).unwrap();
	assert_eq!(
		STATE.with_borrow(|state| state.upgrades.clone()),
		[(true, true), (true, false)]
	);

	// Wrapping checks the game and the class.
	assert_eq!(
		MvmPlayer::new(null_server(Game::SourceSdk2013NextBot, &scope), entity).err(),
		Some(MvmError::UnsupportedGame)
	);
	set_datamap(data_map(
		c"CBaseEntity",
		base_entity_fields().to_vec(),
		null_mut(),
	));
	assert_eq!(
		MvmPlayer::new(server, entity).err(),
		Some(MvmError::WrongClass {
			class: c"CTFPlayer"
		})
	);
}

#[test]
fn populators_are_found_and_sent_their_inputs() {
	let fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CPointPopulatorInterface",
			vec![
				input(c"PauseBotSpawning", sys::_fieldtypes_FIELD_VOID),
				input(c"UnpauseBotSpawning", sys::_fieldtypes_FIELD_VOID),
				input(c"ChangeBotAttributes", sys::_fieldtypes_FIELD_STRING),
				input(
					c"ChangeDefaultEventAttributes",
					sys::_fieldtypes_FIELD_STRING,
				),
			],
		)],
		base_fields: Vec::new(),
		table: None,
	});

	register_class_name(fake.as_ptr(), c"point_populator_interface");

	let scope = ();
	let populator = Populator::find_or_spawn(mock_server(&scope)).unwrap();

	assert_eq!(populator.entity().as_ptr(), fake.as_ptr());

	populator.pause_spawning().unwrap();
	populator.unpause_spawning().unwrap();
	populator
		.change_bot_attributes(c"RevertGateBotsBehavior")
		.unwrap();
	populator
		.change_default_event_attributes(c"FirstGateCaptured")
		.unwrap();

	let inputs = take_inputs();

	assert_eq!(
		inputs
			.iter()
			.map(|input| input.name.as_c_str())
			.collect::<Vec<_>>(),
		[
			c"PauseBotSpawning",
			c"UnpauseBotSpawning",
			c"ChangeBotAttributes",
			c"ChangeDefaultEventAttributes"
		]
	);

	let events = inputs[2..]
		.iter()
		// SAFETY: String inputs carry a pooled, NUL-terminated string.
		.map(|input| unsafe { CStr::from_ptr(input.string) }.to_owned())
		.collect::<Vec<_>>();

	assert_eq!(
		events,
		[
			c"RevertGateBotsBehavior".to_owned(),
			c"FirstGateCaptured".to_owned()
		]
	);
	assert!(
		inputs
			.iter()
			.all(|input| input.activator == fake.as_ptr() && input.caller == fake.as_ptr())
	);
}

#[test]
fn populators_are_spawned_where_the_level_has_none() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = MockEntity::new(1);

	set_datamap(data_map(
		c"CPointPopulatorInterface",
		Vec::new(),
		data_map(c"CBaseEntity", base_entity_fields().to_vec(), null_mut()),
	));
	export_spawn_tools();
	set_created(mock.as_ptr());

	let populator = Populator::find_or_spawn(server).unwrap();

	assert_eq!(populator.entity().as_ptr(), mock.as_ptr());
	assert_eq!(
		take_events(),
		[
			SpawnEvent::Created(c"point_populator_interface".to_owned()),
			SpawnEvent::Spawned
		]
	);

	assert_eq!(
		Populator::find_or_spawn(server).err(),
		Some(MvmError::NotCreated)
	);

	remove_on_spawn(true);
	set_created(mock.as_ptr());
	assert_eq!(
		Populator::find_or_spawn(server).err(),
		Some(MvmError::SpawnFailed)
	);
}

#[test]
fn waves_read_what_the_objective_resource_networks() {
	const BETWEEN_WAVES: usize = FIELDS_OFFSET + 24;
	const CARRIER_LEVEL: usize = FIELDS_OFFSET + 28;
	const ENEMIES: usize = FIELDS_OFFSET + 8;
	const HAS_TANKS: usize = FIELDS_OFFSET + 12;
	const MAX_WAVES: usize = FIELDS_OFFSET;
	const MONEY: usize = FIELDS_OFFSET + 16;
	const NEXT_WAVE: usize = FIELDS_OFFSET + 20;
	const POPFILE: usize = FIELDS_OFFSET + 32;
	const WAVE: usize = FIELDS_OFFSET + 4;

	/// `SendProxy_StringT_To_String`, which sends a `string_t`'s string.
	unsafe extern "C" fn string_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		// SAFETY: The property's variable is a `string_t`, and `out` a writable
		// `DVariant`.
		unsafe {
			(*out).__bindgen_anon_1.m_pString = data.cast::<sys::string_t>().read().pszValue;
		}
	}

	let mut popfile = crate::test_support::datatables::prop(
		c"m_iszMvMPopfileName",
		sys::SendPropType_DPT_String,
		c_int::try_from(POPFILE).unwrap(),
		crate::datatables::PropFlags::default(),
		Some(string_proxy),
	);

	popfile.m_nElements = 1;

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(c"CTFObjectiveResource", Vec::new())],
		base_fields: Vec::new(),
		table: Some((
			c"DT_TFObjectiveResource",
			vec![
				int_prop(c"m_nMannVsMachineMaxWaveCount", MAX_WAVES),
				int_prop(c"m_nMannVsMachineWaveCount", WAVE),
				int_prop(c"m_nMannVsMachineWaveEnemyCount", ENEMIES),
				bool_prop(c"m_nMannVsMachineWaveHasTanks", HAS_TANKS),
				int_prop(c"m_nMvMWorldMoney", MONEY),
				float_prop(c"m_flMannVsMachineNextWaveTime", NEXT_WAVE),
				bool_prop(c"m_bMannVsMachineBetweenWaves", BETWEEN_WAVES),
				int_prop(c"m_nFlagCarrierUpgradeLevel", CARRIER_LEVEL),
				popfile,
			],
		)),
	});

	register_class_name(fake.as_ptr(), c"tf_objective_resource");
	fake.set_int(MAX_WAVES, 7);
	fake.set_int(WAVE, 3);
	fake.set_int(ENEMIES, 46);
	fake.set_bool(HAS_TANKS, true);
	fake.set_int(MONEY, 125);
	fake.set_float(NEXT_WAVE, 0.0);
	fake.set_bool(BETWEEN_WAVES, false);
	fake.set_int(CARRIER_LEVEL, 2);
	fake.set_string(POPFILE, c"scripts/population/mvm_decoy_advanced.pop");

	let scope = ();
	let wave = MvmWave::find(mock_server(&scope)).unwrap().unwrap();

	assert_eq!(wave.entity().as_ptr(), fake.as_ptr());
	assert_eq!(wave.count(), Ok(7));
	assert_eq!(wave.number(), Ok(3));
	assert_eq!(wave.enemy_count(), Ok(46));
	assert_eq!(wave.has_tanks(), Ok(true));
	assert_eq!(wave.world_money(), Ok(125));
	assert_eq!(wave.next_wave_time(), Ok(None));
	assert_eq!(wave.is_between_waves(), Ok(false));
	assert_eq!(wave.bomb_carrier_level(), Ok(2));
	assert_eq!(
		wave.popfile(),
		Ok(c"scripts/population/mvm_decoy_advanced.pop".to_owned())
	);

	fake.set_float(NEXT_WAVE, 95.5);
	fake.set_bool(BETWEEN_WAVES, true);
	assert_eq!(wave.next_wave_time(), Ok(Some(95.5)));
	assert_eq!(wave.is_between_waves(), Ok(true));
}
