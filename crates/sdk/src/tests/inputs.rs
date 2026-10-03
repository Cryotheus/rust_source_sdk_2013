//! Tests of sending inputs to entities: finding them in the game's data
//! description maps, pooling their strings, and the variants `AcceptInput`
//! receives.

use super::*;
use crate::interfaces::ServerTools;
use crate::server::Game;

use crate::test_support::entities::{
	MOCK_NAME_OFFSET, MockEntity, ReceivedInput, base_entity_fields, set_accepts, set_datamap,
	take_inputs,
};

use crate::test_support::leak;
use sdk_raw::entities::datamap::FTYPEDESC_KEY;
use sdk_raw::test_support::entities::data_map as map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char};
use std::ptr::{NonNull, null_mut};

/// How the mock `SetKeyValue` behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pool {
	/// Pools the value into the entity's name, as the game does.
	Works,
	/// Sets another name and reports failure.
	FailsAfterWriting,
	/// Sets another name and reports success.
	WritesOtherString,
}

thread_local! {
	static WORLD: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
	static POOL: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
	static POOL_MODE: Cell<Pool> = const { Cell::new(Pool::Works) };
	static KEYS_SET: Cell<usize> = const { Cell::new(0) };
}

struct Mocks {
	tools: *mut sys::IServerTools,
	world: MockEntity,
	target: MockEntity,
	other: MockEntity,
	maps: Vec<*mut sys::datamap_t>,
}

impl Mocks {
	/// The mock tools, for as long as the mocks live.
	fn tools(&self) -> ServerTools<'static> {
		// SAFETY: The tools are leaked, and their vtable answers what inputs
		// call of them.
		unsafe { ServerTools::from_raw(NonNull::new(self.tools).unwrap(), Game::TeamFortress2) }
	}

	/// Makes mock entities report the chain whose most-derived class is
	/// `class`.
	fn use_chain(&mut self, class: &CStr) {
		let map = self
			.maps
			.iter()
			.copied()
			// SAFETY: The maps are leaked, and named by string literals.
			.find(|&map| unsafe { CStr::from_ptr((*map).dataClassName) } == class)
			.unwrap();

		set_datamap(map);
	}
}

/// A mock entity, for as long as the mocks live.
fn entity(mock: &mut MockEntity) -> Entity<'static> {
	// SAFETY: Mock entities are leaked, and their vtables answer what inputs
	// call of a `CBaseEntity`.
	unsafe { Entity::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
}

/// `IServerTools::GetBaseEntityByEntIndex`, which finds only the world, at
/// index 0.
unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	if index == 0 { WORLD.get() } else { null_mut() }
}

#[test]
fn failed_pooling_sends_nothing_and_restores_the_worlds_name() {
	let mut mocks = mocks();
	let original = c"original";

	mocks.world.set_name(sys::string_t {
		pszValue: original.as_ptr(),
	});

	let target = entity(&mut mocks.target);
	let tools = mocks.tools();
	let send = || {
		tools.accept_input(
			target,
			c"SetDamageFilter",
			InputValue::String(c"filter"),
			target,
			target,
		)
	};

	for mode in [Pool::FailsAfterWriting, Pool::WritesOtherString] {
		POOL_MODE.set(mode);
		assert_eq!(send(), Err(InputError::NotPooled));
		assert_eq!(mocks.world.name().pszValue, original.as_ptr());
	}

	POOL_MODE.set(Pool::Works);
	mocks.world.set_eflags(1);
	assert_eq!(send(), Err(InputError::NotPooled));

	WORLD.set(null_mut());
	assert_eq!(send(), Err(InputError::NotPooled));
	assert!(take_inputs().is_empty());
}

fn input(name: &'static CStr, field_type: sys::fieldtype_t) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut input: sys::typedescription_t = unsafe { std::mem::zeroed() };

	input.fieldType = field_type;
	input.externalName = name.as_ptr();
	input.flags = FTYPEDESC_INPUT;
	input
}

#[test]
fn inputs_are_found_and_checked_as_accept_input_would() {
	let mut mocks = mocks();
	let target = entity(&mut mocks.target);
	let other = entity(&mut mocks.other);
	let tools = mocks.tools();
	let send = |input: &CStr, value| tools.accept_input(target, input, value, target, target);
	let mismatch = |expected| Err(InputError::TypeMismatch { expected });

	assert_eq!(
		send(c"Missing", InputValue::Void),
		Err(InputError::UnknownInput)
	);

	// Keys are not inputs.
	assert_eq!(
		send(c"targetname", InputValue::String(c"x")),
		Err(InputError::UnknownInput)
	);

	// Names are compared ignoring ASCII case, and the most-derived class's
	// input shadows its base's.
	send(c"color", InputValue::String(c"255 0 0")).unwrap();
	assert_eq!(
		send(c"SetTeam", InputValue::Int(2)),
		mismatch(InputType::String)
	);
	send(c"SetTeam", InputValue::String(c"2")).unwrap();

	assert_eq!(
		send(c"SetEnabled", InputValue::Void),
		mismatch(InputType::Bool)
	);
	send(c"SetEnabled", InputValue::Float(0.5)).unwrap();
	assert_eq!(
		send(c"SetSpeed", InputValue::Bool(true)),
		mismatch(InputType::Float)
	);
	send(c"SetSpeed", InputValue::Int(3)).unwrap();
	assert_eq!(
		send(c"Color", InputValue::Vector(Vector::new(1.0, 0.0, 0.0))),
		mismatch(InputType::Color)
	);
	assert_eq!(
		send(c"SetOffset", InputValue::Color(Color32::rgb(1, 2, 3))),
		mismatch(InputType::Vector)
	);
	send(c"SetOwner", InputValue::Entity(Some(other))).unwrap();
	send(c"SetDamageFilter", InputValue::Entity(Some(other))).unwrap();
	send(c"SetDamageFilter", InputValue::Void).unwrap();
	send(c"Kill", InputValue::String(c"ignored")).unwrap();
	assert_eq!(take_inputs().len(), 8);

	set_accepts(false);
	assert_eq!(send(c"Value", InputValue::Void), Err(InputError::Rejected));
	assert_eq!(take_inputs().len(), 1);

	mocks.target.set_eflags(1);

	let target = entity(&mut mocks.target);

	assert_eq!(
		mocks
			.tools()
			.accept_input(target, c"Value", InputValue::Void, target, target),
		Err(InputError::MarkedForDeletion)
	);
	assert!(take_inputs().is_empty());
}

#[test]
fn inputs_known_to_break_the_server_are_only_sent_unchecked() {
	let mut mocks = mocks();
	let target = entity(&mut mocks.target);
	let world = entity(&mut mocks.world);
	let tools = mocks.tools();
	let script = InputValue::String(c"SpawnEntityGroupFromTable({})");
	let send =
		|target, input: &CStr, value| tools.accept_input(target, input, value, target, target);

	assert_eq!(
		send(target, c"runscriptcode", script),
		Err(InputError::FreesEntities)
	);
	assert_eq!(
		send(world, c"Kill", InputValue::Void),
		Err(InputError::ProtectedEntity)
	);
	assert_eq!(
		send(target, c"SetDamageFilter", InputValue::String(c"!PICKER")),
		Err(InputError::PickerName)
	);
	assert!(take_inputs().is_empty());

	// Other entities may be removed, and only NPC makers' `Spawn` spawns.
	send(target, c"Kill", InputValue::Void).unwrap();
	send(target, c"Spawn", InputValue::Void).unwrap();
	send(target, c"SetDamageFilter", InputValue::String(c"!player")).unwrap();

	// The unchecked form sends them, without an activator or caller.
	// SAFETY: The mock input handler only records the call.
	unsafe {
		tools
			.accept_input_unchecked(target, c"RunScriptCode", script, None, None)
			.unwrap();
	}

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 4);
	assert!(inputs[3].activator.is_null() && inputs[3].caller.is_null());

	mocks.use_chain(c"CTFPlayer");

	let target = entity(&mut mocks.target);
	let tools = mocks.tools();
	let red = InputValue::Color(Color32::rgb(255, 0, 0));

	for kill in [c"Kill", c"KillHierarchy"] {
		assert_eq!(
			tools.accept_input(target, kill, InputValue::Void, target, target),
			Err(InputError::ProtectedEntity)
		);
	}

	tools
		.accept_input(target, c"Color", red, target, target)
		.unwrap();

	let concept = InputValue::String(c"TLK_NOT_A_CONCEPT");

	assert_eq!(
		tools.accept_input(target, c"speakresponseconcept", concept, target, target),
		Err(InputError::UncheckedLookup)
	);
	// SAFETY: The mock input handler only records the call.
	unsafe {
		tools
			.accept_input_unchecked(target, c"SpeakResponseConcept", concept, None, None)
			.unwrap();
	}

	mocks.use_chain(c"CNPCMaker");

	let target = entity(&mut mocks.target);

	assert_eq!(
		mocks
			.tools()
			.accept_input(target, c"Spawn", InputValue::Void, target, target),
		Err(InputError::FreesEntities)
	);
	// `Color` and the unchecked `SpeakResponseConcept` on the player.
	assert_eq!(take_inputs().len(), 2);

	mocks.target.set_eflags(1);

	let target = entity(&mut mocks.target);

	assert_eq!(
		// SAFETY: The input is refused before it reaches the game.
		unsafe {
			mocks
				.tools()
				.accept_input_unchecked(target, c"Color", red, None, None)
		},
		Err(InputError::MarkedForDeletion)
	);
}

fn mocks() -> Mocks {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_COLOR32 as COLOR32,
		_fieldtypes_FIELD_EHANDLE as EHANDLE, _fieldtypes_FIELD_FLOAT as FLOAT,
		_fieldtypes_FIELD_INPUT as INPUT, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_STRING as STRING, _fieldtypes_FIELD_VECTOR as VECTOR,
		_fieldtypes_FIELD_VOID as VOID,
	};

	let world = MockEntity::new(0);
	let target = MockEntity::new(7 | 3 << 16);
	let other = MockEntity::new(9 | 4 << 16);
	let mut base_fields = base_entity_fields().to_vec();

	// A key the map sets, which is not an input.
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut key: sys::typedescription_t = unsafe { std::mem::zeroed() };

	key.fieldType = STRING;
	key.externalName = c"targetname".as_ptr();
	key.flags = FTYPEDESC_KEY;

	base_fields.extend([
		key,
		input(c"Kill", VOID),
		input(c"KillHierarchy", VOID),
		input(c"Color", COLOR32),
		input(c"SetTeam", INTEGER),
		input(c"RunScriptCode", STRING),
		input(c"SetDamageFilter", STRING),
	]);

	let base = map(c"CBaseEntity", base_fields, null_mut());
	let test = map(
		c"CTestEntity",
		vec![
			input(c"Value", INPUT),
			input(c"SetTeam", STRING),
			input(c"Spawn", VOID),
			input(c"SetSpeed", FLOAT),
			input(c"SetOwner", EHANDLE),
			input(c"SetOffset", VECTOR),
			input(c"SetEnabled", BOOLEAN),
		],
		base,
	);
	let base_player = map(c"CBasePlayer", vec![], base);
	let player = map(
		c"CTFPlayer",
		vec![input(c"SpeakResponseConcept", STRING)],
		base_player,
	);
	let maker = map(c"CBaseNPCMaker", vec![input(c"Spawn", VOID)], base);
	let npc_maker = map(c"CNPCMaker", vec![], maker);

	set_datamap(test);

	let maps = vec![test, base, base_player, player, maker, npc_maker];
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let tools_vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex).write(entity_by_index);
			(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value);
		})
	};

	let mut mocks = Mocks {
		tools: leak(sys::IServerTools {
			vtable_: Box::into_raw(tools_vtable),
		}),
		world,
		target,
		other,
		maps,
	};

	WORLD.set(mocks.world.as_ptr());
	POOL_MODE.set(Pool::Works);
	KEYS_SET.set(0);
	set_accepts(true);
	take_inputs();
	mocks
}

/// Pools strings case-insensitively, as `CGameStringPool` does, into the
/// entity's name, as `KeyValue("targetname", ...)` does.
unsafe extern "C" fn set_key_value(
	_: *mut sys::IServerTools,
	entity: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *const c_char,
) -> bool {
	KEYS_SET.set(KEYS_SET.get() + 1);

	// SAFETY: The wrappers pass NUL-terminated keys and values.
	if unsafe { CStr::from_ptr(key) } != c"targetname" {
		return false;
	}

	let mode = POOL_MODE.get();
	let value = match mode {
		// SAFETY: As for the key.
		Pool::Works => unsafe { CStr::from_ptr(value) },

		Pool::FailsAfterWriting | Pool::WritesOtherString => c"something else",
	};

	let pooled = POOL.with_borrow_mut(|pool| {
		let index = pool
			.iter()
			.position(|pooled| pooled.to_bytes().eq_ignore_ascii_case(value.to_bytes()))
			.unwrap_or_else(|| {
				pool.push(value.to_owned());
				pool.len() - 1
			});

		pool[index].as_ptr()
	});

	// SAFETY: Every entity the wrappers pool through is a mock entity, which
	// stores its name at this offset.
	unsafe {
		entity
			.byte_add(MOCK_NAME_OFFSET)
			.cast::<sys::string_t>()
			.write(sys::string_t { pszValue: pooled })
	};

	mode != Pool::FailsAfterWriting
}

#[test]
fn strings_are_pooled_only_for_inputs_that_may_keep_them() {
	let mut mocks = mocks();
	let original = c"original";

	mocks.world.set_name(sys::string_t {
		pszValue: original.as_ptr(),
	});

	let target = entity(&mut mocks.target);
	let tools = mocks.tools();
	let greeting = CString::new("Hello").unwrap();
	let send = |input: &CStr, value| tools.accept_input(target, input, value, target, target);

	send(c"SetDamageFilter", InputValue::String(&greeting)).unwrap();
	send(c"SetDamageFilter", InputValue::String(c"HELLO")).unwrap();
	send(c"Value", InputValue::String(c"hello")).unwrap();

	let pooled = |input: &ReceivedInput| input.string;
	let inputs = take_inputs();
	let first = pooled(&inputs[0]);

	// The entity received the pooled copy, in the case pooled first, and
	// the world kept its name.
	assert_ne!(first, greeting.as_ptr());
	// SAFETY: Pooled strings stay in the thread's pool.
	assert_eq!(unsafe { CStr::from_ptr(first) }, c"Hello");
	assert_eq!(pooled(&inputs[1]), first);
	assert_eq!(pooled(&inputs[2]), first);
	assert_eq!(mocks.world.name().pszValue, original.as_ptr());
	assert_eq!(KEYS_SET.get(), 3);

	// Inputs of other types convert the string during the call, so it is
	// passed as it is.
	let speed = c"2.5";

	send(c"SetSpeed", InputValue::String(speed)).unwrap();
	assert_eq!(pooled(&take_inputs()[0]), speed.as_ptr());
	assert_eq!(KEYS_SET.get(), 3);

	// A null handle reaches a string input as no value.
	send(c"SetDamageFilter", InputValue::Entity(None)).unwrap();
	assert_eq!(take_inputs()[0].field_type, sys::_fieldtypes_FIELD_VOID);
}

#[test]
fn values_reach_accept_input_as_variants_by_address() {
	let mut mocks = mocks();
	let target = entity(&mut mocks.target);
	let other = entity(&mut mocks.other);
	let target_ptr = target.as_ptr();
	let other_ptr = other.as_ptr();
	let other_handle = other.handle().to_raw();
	let tools = mocks.tools();
	let send = |value| tools.accept_input(target, c"value", value, other, target);

	send(InputValue::Void).unwrap();
	send(InputValue::Bool(true)).unwrap();
	send(InputValue::Int(-5)).unwrap();
	send(InputValue::Float(1.5)).unwrap();
	send(InputValue::Vector(Vector::new(1.0, 2.0, 3.0))).unwrap();
	send(InputValue::Color(Color32::new(1, 2, 3, 4))).unwrap();
	send(InputValue::Entity(Some(other))).unwrap();
	send(InputValue::Entity(None)).unwrap();
	send(InputValue::String(c"")).unwrap();

	let inputs = take_inputs();
	let invalid = EntityHandle::INVALID.to_raw();

	// The input is sent by its declared name, so VScript hooks match.
	assert!(inputs.iter().all(|input| {
		input.target == target_ptr
			&& input.name.as_c_str() == c"Value"
			&& input.activator == other_ptr
			&& input.caller == target_ptr
			&& input.output_id == 0
	}));

	let one = 1.0_f32.to_bits();
	let two = 2.0_f32.to_bits();
	let three = 3.0_f32.to_bits();

	assert_eq!(
		inputs
			.iter()
			.map(|input| (input.field_type, input.payload, input.handle))
			.collect::<Vec<_>>(),
		[
			(sys::_fieldtypes_FIELD_VOID, [0; 12], invalid),
			(sys::_fieldtypes_FIELD_BOOLEAN, words([1, 0, 0]), invalid),
			(
				sys::_fieldtypes_FIELD_INTEGER,
				words([(-5_i32).cast_unsigned(), 0, 0]),
				invalid
			),
			(
				sys::_fieldtypes_FIELD_FLOAT,
				words([1.5_f32.to_bits(), 0, 0]),
				invalid
			),
			(
				sys::_fieldtypes_FIELD_VECTOR,
				words([one, two, three]),
				invalid
			),
			(
				sys::_fieldtypes_FIELD_COLOR32,
				words([u32::from_ne_bytes([1, 2, 3, 4]), 0, 0]),
				invalid
			),
			(sys::_fieldtypes_FIELD_EHANDLE, [0; 12], other_handle),
			(sys::_fieldtypes_FIELD_EHANDLE, [0; 12], invalid),
			// An empty string is a null string, as `MAKE_STRING("")` gives.
			(sys::_fieldtypes_FIELD_STRING, [0; 12], invalid),
		]
	);

	// Nothing was pooled for the empty string.
	assert_eq!(KEYS_SET.get(), 0);
}

/// The bytes of three 32-bit words, as the union stores them.
fn words(words: [u32; 3]) -> [u8; 12] {
	let mut bytes = [0; 12];

	for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
		chunk.copy_from_slice(&word.to_ne_bytes());
	}

	bytes
}
