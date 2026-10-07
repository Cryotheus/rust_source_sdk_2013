//! Tests of `crate::tf2::bots`: a fake bot's and NextBot actor's native
//! methods, the command lines of `tf_bot_add` requests, and the commands
//! queued with a mock engine.

use super::*;
use crate::interfaces::ValveEngine;
use crate::server::Module;
use crate::test_support::edicts::edict_table;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap, set_networking};
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use sdk_raw::test_support::entities::data_map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::STRING;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::ptr::{NonNull, null_mut};

/// A native method of a fake script class: its name, return type, and
/// parameter types.
type Method = (
	&'static CStr,
	sys::ScriptDataType_t,
	&'static [sys::ScriptDataType_t],
);

/// The methods the fake `CTFBot` script class declares.
const BOT_METHODS: &[Method] = &[
	(c"AddBotAttribute", VOID, &[INT]),
	(c"AddBotTag", VOID, &[STRING]),
	(c"AddWeaponRestriction", VOID, &[INT]),
	(c"ClearAllBotAttributes", VOID, &[]),
	(c"ClearAllWeaponRestrictions", VOID, &[]),
	(c"ClearBehaviorFlag", VOID, &[INT]),
	(c"GetBotId", INT, &[]),
	(c"GetDifficulty", INT, &[]),
	(c"GetMaxVisionRangeOverride", FLOAT, &[]),
	(c"GetMission", INT, &[]),
	(c"GetPrevMission", INT, &[]),
	(c"HasBotAttribute", BOOL, &[INT]),
	(c"HasBotTag", BOOL, &[STRING]),
	(c"HasWeaponRestriction", BOOL, &[INT]),
	(c"IsBehaviorFlagSet", BOOL, &[INT]),
	(c"PressFireButton", VOID, &[FLOAT]),
	(c"RemoveBotAttribute", VOID, &[INT]),
	(c"RemoveWeaponRestriction", VOID, &[INT]),
	(c"SetBehaviorFlag", VOID, &[INT]),
	(c"SetDifficulty", VOID, &[INT]),
	(c"SetMaxVisionRangeOverride", VOID, &[FLOAT]),
	(c"SetMission", VOID, &[INT, BOOL]),
	// Declared with the wrong return type, as by a game that changed it.
	(c"ShouldQuickBuild", INT, &[]),
];

/// The methods the fake `NextBotCombatCharacter` script class declares.
const NEXT_BOT_METHODS: &[Method] = &[(c"GetBotId", INT, &[]), (c"IsImmobile", BOOL, &[])];

/// The methods the fake `CTFPlayer` script class declares.
const PLAYER_METHODS: &[Method] = &[(c"GetBotType", INT, &[])];

thread_local! {
	/// What the fake methods read and write.
	static STATE: RefCell<FakeState> = RefCell::new(FakeState::default());

	/// The commands the mock engine was asked to queue.
	static COMMANDS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// The fields of the fake actor that its native methods read and write.
#[derive(Debug, Default)]
struct FakeState {
	bot_type: c_int,
	attributes: c_int,
	behavior: c_int,
	restrictions: c_int,
	difficulty: c_int,
	mission: c_int,
	previous_mission: c_int,
	/// Whether the last `SetMission` asked to reset the bot's behaviour.
	reset_behavior: bool,
	vision: f32,
	tags: Vec<CString>,
	presses: Vec<f32>,
	/// Whether every method refuses its call, as an adapter does when its
	/// arguments do not convert, without running the method.
	rejects: bool,
	/// The calls that reached a method, refused or not.
	calls: usize,
}

/// A fake binding's adapter, which runs the fake method named by the
/// binding's function, on the fake state.
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

		if state.rejects {
			return false;
		}

		// SAFETY: Each method reads the union member of its declared parameter
		// types, which `call` checked.
		let value = unsafe {
			match name.to_bytes() {
				b"AddBotAttribute" => {
					state.attributes |= argument(0).m_int;
					None
				}

				b"AddBotTag" => {
					state
						.tags
						.push(CStr::from_ptr(argument(0).m_pszString).to_owned());

					None
				}

				b"AddWeaponRestriction" => {
					state.restrictions |= argument(0).m_int;
					None
				}

				b"ClearAllBotAttributes" => {
					state.attributes = 0;
					None
				}

				b"ClearAllWeaponRestrictions" => {
					state.restrictions = 0;
					None
				}

				b"ClearBehaviorFlag" => {
					state.behavior &= !argument(0).m_int;
					None
				}

				b"GetBotId" => Some(int(7)),
				b"GetBotType" => Some(int(state.bot_type)),
				b"GetDifficulty" => Some(int(state.difficulty)),
				b"GetMaxVisionRangeOverride" => Some(float(state.vision)),
				b"GetMission" => Some(int(state.mission)),
				b"GetPrevMission" => Some(int(state.previous_mission)),
				b"HasBotAttribute" => Some(boolean(state.attributes & argument(0).m_int != 0)),

				b"HasBotTag" => {
					let tag = CStr::from_ptr(argument(0).m_pszString).to_bytes();

					Some(boolean(
						state
							.tags
							.iter()
							.any(|stored| stored.to_bytes().eq_ignore_ascii_case(tag)),
					))
				}

				b"HasWeaponRestriction" => {
					Some(boolean(state.restrictions & argument(0).m_int != 0))
				}

				b"IsBehaviorFlagSet" => Some(boolean(state.behavior & argument(0).m_int != 0)),
				b"IsImmobile" => Some(boolean(true)),

				b"PressFireButton" => {
					state.presses.push(argument(0).m_float);
					None
				}

				b"RemoveBotAttribute" => {
					state.attributes &= !argument(0).m_int;
					None
				}

				b"RemoveWeaponRestriction" => {
					state.restrictions &= !argument(0).m_int;
					None
				}

				b"SetBehaviorFlag" => {
					state.behavior |= argument(0).m_int;
					None
				}

				b"SetDifficulty" => {
					state.difficulty = argument(0).m_int;
					None
				}

				b"SetMaxVisionRangeOverride" => {
					state.vision = argument(0).m_float;
					None
				}

				b"SetMission" => {
					state.previous_mission = state.mission;
					state.mission = argument(0).m_int;
					state.reset_behavior = argument(1).m_bool;
					None
				}

				b"ShouldQuickBuild" => Some(int(1)),
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

/// The descriptor of the fake `CTFBot` script class, deriving from the fake
/// `CTFPlayer`.
fn bot_description() -> *mut sys::ScriptClassDesc_t {
	let player = description(c"CTFPlayer", PLAYER_METHODS, null_mut());

	description(c"CTFBot", BOT_METHODS, player)
}

#[test]
fn bot_flags_round_trip_through_the_native_methods() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the bot calls.
	let bot = TfBot::new(server, unsafe { entity_of(&mut mock) }).unwrap();
	let boss = BotAttributes::MINIBOSS | BotAttributes::USE_BOSS_HEALTH_BAR;

	assert_eq!(bot.attributes(), Ok(BotAttributes::empty()));
	bot.add_attributes(boss | BotAttributes::ALWAYS_CRIT)
		.unwrap();
	assert_eq!(
		state(|state| state.attributes),
		(1 << 15) | (1 << 16) | (1 << 9)
	);
	assert_eq!(bot.attributes(), Ok(boss | BotAttributes::ALWAYS_CRIT));
	assert_eq!(
		bot.has_any_attribute(BotAttributes::AUTO_JUMP | boss),
		Ok(true)
	);
	bot.remove_attributes(boss).unwrap();
	assert_eq!(bot.attributes(), Ok(BotAttributes::ALWAYS_CRIT));
	assert_eq!(bot.has_any_attribute(boss), Ok(false));
	bot.clear_attributes().unwrap();
	assert_eq!(bot.attributes(), Ok(BotAttributes::empty()));

	let ignored = BehaviorFlags::IGNORE_ENEMY_SPIES | BehaviorFlags::IGNORE_SCENARIO_GOALS;

	bot.add_behavior_flags(ignored).unwrap();
	assert_eq!(state(|state| state.behavior), 0x0100 | 0x0400);
	assert_eq!(bot.behavior_flags(), Ok(ignored));
	bot.remove_behavior_flags(BehaviorFlags::all()).unwrap();
	assert_eq!(bot.behavior_flags(), Ok(BehaviorFlags::empty()));

	bot.add_weapon_restrictions(WeaponRestrictions::MELEE_ONLY)
		.unwrap();
	assert_eq!(state(|state| state.restrictions), 1);
	assert_eq!(
		bot.weapon_restrictions(),
		Ok(WeaponRestrictions::MELEE_ONLY)
	);
	assert_eq!(
		bot.has_any_weapon_restriction(WeaponRestrictions::PRIMARY_ONLY),
		Ok(false)
	);
	bot.remove_weapon_restrictions(WeaponRestrictions::all())
		.unwrap();
	assert_eq!(bot.weapon_restrictions(), Ok(WeaponRestrictions::empty()));
	bot.add_weapon_restrictions(WeaponRestrictions::SECONDARY_ONLY)
		.unwrap();
	bot.clear_weapon_restrictions().unwrap();
	assert_eq!(state(|state| state.restrictions), 0);

	// Tags are compared by the game, ignoring ASCII case.
	bot.add_tag(c"Boss").unwrap();
	assert_eq!(bot.has_tag(c"BOSS"), Ok(true));
	assert_eq!(bot.has_tag(c"Escort"), Ok(false));
	assert_eq!(state(|state| state.tags.clone()), [c"Boss".to_owned()]);
}

#[test]
fn bots_are_told_apart_by_their_bot_type() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the bot calls.
	let entity = unsafe { entity_of(&mut mock) };

	assert!(TfBot::new(server, entity).is_ok());

	// Another TF2 game's server.
	let other_game = null_server(Game::SourceSdk2013NextBot, &scope);

	assert_eq!(
		TfBot::new(other_game, entity).err(),
		Some(BotError::UnsupportedGame)
	);

	// A human player, whose datamaps are a player's, not an actor's.
	STATE.with_borrow_mut(|state| state.bot_type = 0);
	assert_eq!(TfBot::new(server, entity).err(), Some(BotError::NotATfBot));
	assert_eq!(
		NextBot::new(server, entity).err(),
		Some(BotError::NotANextBot)
	);
	STATE.with_borrow_mut(|state| state.bot_type = TF_BOT_TYPE);

	// An entity whose script classes lack `CTFPlayer`.
	set_script_description(null_mut());
	assert_eq!(TfBot::new(server, entity).err(), Some(BotError::NotATfBot));
	set_script_description(bot_description());

	mock.set_eflags(1);
	assert_eq!(
		TfBot::new(server, entity).err(),
		Some(BotError::MarkedForDeletion)
	);
	assert_eq!(
		NextBot::new(server, entity).err(),
		Some(BotError::MarkedForDeletion)
	);
}

#[test]
fn commands_are_queued_with_the_engine() {
	let (_vtable, mut engine) = mock_engine();

	export(Module::Engine, ValveEngine::VERSION, &raw mut engine);

	let scope = ();
	let server = mock_server(&scope);
	let pair = TfBotRequest::new(NonZeroU8::new(2).unwrap()).with_team(Some(BotTeam::Red));

	add_tf_bots(server, &pair).unwrap();
	kick_tf_bots(server, None).unwrap();
	kick_tf_bots(server, Some(BotTeam::Blue)).unwrap();
	assert_eq!(
		COMMANDS.take(),
		[
			c"tf_bot_add 2 red noquota\n".to_owned(),
			c"tf_bot_kick all\n".to_owned(),
			c"tf_bot_kick blue\n".to_owned(),
		]
	);

	// A bot is kicked by the user ID of its edict's client.
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the bot calls.
	let bot = TfBot::new(server, unsafe { entity_of(&mut mock) }).unwrap();

	assert_eq!(bot.kick(server), Err(BotError::NotATfBot));

	let mut edicts = edict_table(2, |_| false);

	set_networking(null_mut(), &raw mut edicts[1]);
	bot.kick(server).unwrap();
	assert_eq!(COMMANDS.take(), [c"kickid 42\n".to_owned()]);

	// Outside TF2, nothing is queued.
	let other_game = null_server(Game::SourceSdk2013, &scope);

	assert_eq!(
		add_tf_bots(other_game, &pair),
		Err(BotError::UnsupportedGame)
	);
	assert_eq!(
		kick_tf_bots(other_game, None),
		Err(BotError::UnsupportedGame)
	);
	assert!(COMMANDS.take().is_empty());

	// Without the engine's interface.
	let no_engine = null_server(Game::TeamFortress2, &scope);

	assert!(matches!(
		add_tf_bots(no_engine, &pair),
		Err(BotError::Interface(_))
	));
}

#[test]
fn deleted_bots_and_refused_calls_are_reported() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the bot calls.
	let bot = TfBot::new(server, unsafe { entity_of(&mut mock) }).unwrap();

	STATE.with_borrow_mut(|state| state.rejects = true);
	assert_eq!(
		bot.add_attributes(BotAttributes::ALWAYS_CRIT),
		Err(BotError::Rejected)
	);
	assert_eq!(bot.next_bot().bot_id(), Err(BotError::Rejected));
	STATE.with_borrow_mut(|state| state.rejects = false);
	assert_eq!(state(|state| (state.attributes, state.calls)), (0, 3));

	// Methods the game lacks, or declares otherwise, never reach an adapter.
	assert_eq!(bot.remove_tag(c"Boss"), Err(BotError::UnsupportedMethod));
	assert_eq!(bot.quick_build(), Err(BotError::UnsupportedMethod));
	assert_eq!(state(|state| state.calls), 3);

	mock.set_eflags(1);
	assert_eq!(
		bot.add_attributes(BotAttributes::ALWAYS_CRIT),
		Err(BotError::MarkedForDeletion)
	);
	assert_eq!(bot.next_bot().bot_id(), Err(BotError::MarkedForDeletion));
	assert_eq!(state(|state| state.calls), 3);
}

/// The leaked descriptor of the script class `name`, declaring `methods`
/// through [`adapter`] and deriving from `base`.
fn description(
	name: &'static CStr,
	methods: &[Method],
	base: *mut sys::ScriptClassDesc_t,
) -> *mut sys::ScriptClassDesc_t {
	let bindings = methods
		.iter()
		.map(|&(method, returns, parameters)| {
			let mut binding =
				member_binding(method, returns, Vec::from(parameters).leak(), Some(adapter));

			binding.m_pFunction.val_0 = method.as_ptr() as isize;
			binding
		})
		.collect::<Vec<_>>();

	leak(class_description(name, bindings.leak(), base))
}

#[test]
fn difficulty_missions_and_overrides_map_to_the_games_values() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the bot calls.
	let bot = TfBot::new(server, unsafe { entity_of(&mut mock) }).unwrap();

	bot.set_difficulty(Difficulty::Hard).unwrap();
	assert_eq!(state(|state| state.difficulty), 2);
	assert_eq!(bot.difficulty(), Ok(Some(Difficulty::Hard)));
	STATE.with_borrow_mut(|state| state.difficulty = 7);
	assert_eq!(bot.difficulty(), Ok(None));

	bot.set_mission(Mission::DestroySentries, true).unwrap();
	bot.set_mission(Mission::Sniper, false).unwrap();
	assert_eq!(
		state(|state| (state.mission, state.previous_mission, state.reset_behavior)),
		(3, 2, false)
	);
	assert_eq!(bot.mission(), Ok(Some(Mission::Sniper)));
	assert_eq!(bot.previous_mission(), Ok(Some(Mission::DestroySentries)));

	// The game's -1 means no override.
	STATE.with_borrow_mut(|state| state.vision = -1.0);
	assert_eq!(bot.max_vision_range_override(), Ok(None));
	bot.set_max_vision_range_override(Some(1500.0)).unwrap();
	assert_eq!(bot.max_vision_range_override(), Ok(Some(1500.0)));
	bot.set_max_vision_range_override(None).unwrap();
	assert_eq!(state(|state| state.vision), -1.0);

	// Without a duration, the button is pressed until the next update.
	bot.press_fire_button(None).unwrap();
	bot.press_fire_button(Some(2.5)).unwrap();
	assert_eq!(state(|state| state.presses.clone()), [-1.0, 2.5]);

	for difficulty in Difficulty::ALL {
		assert_eq!(Difficulty::from_raw(difficulty.to_raw()), Some(difficulty));
	}

	assert_eq!(Difficulty::from_raw(-1), None);
	assert_eq!(Mission::from_raw(6), Some(Mission::Reprogrammed));
	assert_eq!(Mission::from_raw(7), None);
}

/// A handle to the mock, which it outlives.
///
/// # Safety
///
/// The mock's vtable must answer what the wrappers call.
unsafe fn entity_of<'s>(mock: &mut MockEntity) -> Entity<'s> {
	// SAFETY: Mock entities are leaked, and the caller promises the rest.
	unsafe { Entity::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
}

/// A mock entity whose `GetScriptDesc` returns the fake `CTFBot` descriptor,
/// as [`set_script_description`] sets it on this thread, with the fake state
/// reset to a bot's.
fn mock_bot() -> MockEntity {
	let mut mock = MockEntity::new(1);
	let pointer = mock.as_ptr();
	let mut vtable = vec![unexpected_call as *const (); SCRIPT_DESCRIPTION_SLOT + 1];

	// The mock's own vtable answers every slot before the descriptor's, which
	// include the datamap's and the networkable's.
	// SAFETY: A mock entity starts with the pointer to its vtable, which has
	// slots up to TF2's `Teleport`, past `GetScriptDesc`.
	unsafe {
		let original = pointer.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(SCRIPT_DESCRIPTION_SLOT) {
			*entry = original.add(slot).read();
		}
	}

	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();

	// SAFETY: A mock entity starts with the pointer to its vtable, and the new
	// vtable is leaked.
	unsafe {
		pointer
			.cast::<*const *const ()>()
			.write(vtable.leak().as_ptr())
	};
	set_script_description(bot_description());
	STATE.set(FakeState {
		bot_type: TF_BOT_TYPE,
		..FakeState::default()
	});

	mock
}

/// An engine that records the commands it is asked to queue, and gives every
/// edict the user ID 42.
fn mock_engine() -> (
	Box<sys::IVEngineServer__bindgen_vtable>,
	sys::IVEngineServer,
) {
	/// `IVEngineServer::GetPlayerUserId`.
	unsafe extern "C" fn user_id(_: *mut sys::IVEngineServer, edict: *const sys::edict_t) -> c_int {
		assert!(!edict.is_null());
		42
	}

	/// `IVEngineServer::ServerCommand`, which records the command.
	unsafe extern "C" fn server_command(_: *mut sys::IVEngineServer, command: *const c_char) {
		// SAFETY: The wrapper passes a NUL-terminated command.
		let command = unsafe { CStr::from_ptr(command) }.to_owned();

		COMMANDS.with_borrow_mut(|commands| commands.push(command));
	}

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_GetPlayerUserId).write(user_id);
			(&raw mut (*vtable).IVEngineServer_ServerCommand).write(server_command);
		})
	};
	let engine = sys::IVEngineServer {
		vtable_: &raw const *vtable,
	};

	COMMANDS.take();
	(vtable, engine)
}

#[test]
fn names_tf_bot_add_would_misread_are_refused() {
	let named = |name: &CStr| TfBotRequest::named(name).map(|request| request.command());

	assert_eq!(named(c""), Err(BotNameError::Empty));
	assert_eq!(
		named(&CString::new([b'a'; 32]).unwrap()),
		Err(BotNameError::TooLong { len: 32 })
	);
	assert!(named(&CString::new([b'a'; 31]).unwrap()).is_ok());

	for (name, index, byte) in [
		(c"Say \"hi\"", 4, b'"'),
		(c"a;quit", 1, b';'),
		(c"tab\there", 3, b'\t'),
		(c"del\x7f", 3, 0x7f),
	] {
		assert_eq!(named(name), Err(BotNameError::InvalidByte { index, byte }));
	}

	// Arguments `tf_bot_add` takes as something else, compared ignoring case,
	// and anything `atoi` reads as a positive count.
	for name in [
		c"Scout",
		c"HEAVYWEAPONS",
		c"red",
		c"Blue",
		c"noquota",
		c"Expert",
		c"3rd",
		c" +12",
		c"007",
		c"99999999999999999999",
	] {
		assert_eq!(named(name), Err(BotNameError::Reserved), "{name:?}");
	}

	// What `atoi` reads as zero or less is a name.
	for name in [c"0", c"-5", c"00", c"+", c"Bot 9", c"heavy"] {
		assert!(named(name).is_ok(), "{name:?}");
	}

	// Bytes beyond ASCII pass through unchanged.
	assert_eq!(
		named(c"\xffbot"),
		Ok(CString::new(b"tf_bot_add \"\xffbot\" noquota\n".to_vec()).unwrap())
	);
}

#[test]
fn next_bot_actors_answer_their_queries() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = mock_bot();
	// SAFETY: The mock is leaked, and its vtable answers what the actor calls.
	let entity = unsafe { entity_of(&mut mock) };

	// A bot's queries go through `CTFBot`'s script class.
	assert_eq!(
		NextBot::new(server, entity).and_then(NextBot::bot_id),
		Ok(7)
	);

	// A `base_boss`, whose datamaps include `NextBotCombatCharacter`'s.
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let character = data_map(c"CBaseCombatCharacter", vec![], base);
	let boss = data_map(
		c"CTFBaseBoss",
		vec![],
		data_map(c"NextBotCombatCharacter", vec![], character),
	);

	set_datamap(boss);
	set_script_description(description(
		c"NextBotCombatCharacter",
		NEXT_BOT_METHODS,
		null_mut(),
	));
	assert_eq!(TfBot::new(server, entity).err(), Some(BotError::NotATfBot));

	let actor = NextBot::new(server, entity).unwrap();

	assert_eq!(actor.bot_id(), Ok(7));
	assert_eq!(actor.is_immobile(), Ok(true));
	assert_eq!(actor.entity(), entity);
	assert_eq!(actor.tick_last_update(), Err(BotError::UnsupportedMethod));

	// Any other entity.
	set_datamap(character);
	assert_eq!(
		NextBot::new(server, entity).err(),
		Some(BotError::NotANextBot)
	);
}

#[test]
fn requests_write_tf_bot_add_command_lines() {
	let three = NonZeroU8::new(3).unwrap();
	let request = TfBotRequest::new(three);

	assert_eq!(request.command().as_c_str(), c"tf_bot_add 3 noquota\n");
	assert_eq!(
		(request.count(), request.name(), request.quota_managed()),
		(three, None, false)
	);

	let request = request
		.with_team(Some(BotTeam::Blue))
		.with_class(Some(PlayerClass::Heavy))
		.with_difficulty(Some(Difficulty::Expert))
		.with_quota_managed(true);

	assert_eq!(
		request.command().as_c_str(),
		c"tf_bot_add 3 blue heavyweapons expert\n"
	);
	assert_eq!(
		(request.team(), request.class(), request.difficulty()),
		(
			Some(BotTeam::Blue),
			Some(PlayerClass::Heavy),
			Some(Difficulty::Expert)
		)
	);

	let named = TfBotRequest::named(c"Bot Name")
		.unwrap()
		.with_team(Some(BotTeam::Red))
		.with_difficulty(Some(Difficulty::Easy));

	assert_eq!(
		named.command().as_c_str(),
		c"tf_bot_add \"Bot Name\" red easy noquota\n"
	);
	assert_eq!(
		(named.count(), named.name()),
		(NonZeroU8::MIN, Some(c"Bot Name"))
	);

	// Each class goes by its name in the class data.
	for class in PlayerClass::ALL {
		let command = TfBotRequest::new(NonZeroU8::MIN)
			.with_class(Some(class))
			.command();

		assert_eq!(
			command.to_bytes(),
			[
				b"tf_bot_add 1 ",
				class_argument(class).to_bytes(),
				b" noquota\n"
			]
			.concat()
		);
	}
}

/// Reads the fake state.
fn state<T>(read: impl FnOnce(&FakeState) -> T) -> T {
	STATE.with_borrow(read)
}
