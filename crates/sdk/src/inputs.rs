//! Entity I/O: sending inputs to entities, as map outputs, `ent_fire`, and
//! VScript's `AcceptInput` do.
//!
//! [`ServerTools::accept_input`] sends an input and runs its handler before
//! returning. Before calling into the game, it finds the input the way
//! `AcceptInput` does, so an unknown input or a value the input cannot take
//! is reported as an [`InputError`] instead of a console warning.
//!
//! [`ServerTools::accept_input`]: crate::interfaces::ServerTools::accept_input

use crate::entities::{Entity, EntityHandle, data_fields, data_map_class};
use crate::ffi::borrow_cstr;
use crate::math::{Color32, Vector};
use std::ffi::{CStr, c_int, c_short};
use std::fmt::{self, Display, Formatter};
use std::mem::MaybeUninit;

/// `FTYPEDESC_INPUT` from `public/datamap.h`: the field is an input.
const FTYPEDESC_INPUT: c_short = 0x0008;

/// Inputs that run code the caller chooses or spawn entities from templates.
/// Both can free entities immediately: a template entity that fails to spawn
/// empties the engine's whole pending-deletion list, and VScript can do that,
/// or restart the round, which frees nearly every entity.
const CODE_OR_SPAWN_INPUTS: [&[u8]; 5] = [
	b"RunScriptCode",
	b"RunScriptFile",
	b"CallScriptFunction",
	b"ForceSpawn",
	b"ForceSpawnAtEntityOrigin",
];

/// Spawn inputs of NPC makers, which immediately remove a `prop_physics`
/// blocking the spawn.
const NPC_MAKER_SPAWN_INPUTS: [&[u8]; 4] = [
	b"Spawn",
	b"SpawnNPCInRadius",
	b"SpawnNPCInLine",
	b"SpawnMultiple",
];

/// Inputs that remove their target. The engine keeps using a player's or the
/// world's entity, so removing one crashes the server once it is freed.
const KILL_INPUTS: [&[u8]; 2] = [b"Kill", b"KillHierarchy"];

/// The procedural entity name that looks through the first player's crosshair
/// without checking that there is one (`FindEntityProcedural`).
const PICKER_NAME: &[u8] = b"!picker";

/// The value an input carries (`variant_t`).
///
/// `AcceptInput` converts the value to the type the input declares, as
/// [`InputType`] describes: strings to every type but [`InputType::Other`],
/// as map I/O relies on, integers and floats to each other and to booleans,
/// and entities to their name.
#[doc(alias = "variant_t")]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputValue<'a> {
	/// No value, as map I/O sends for an empty parameter. Inputs that take a
	/// string receive an empty one.
	#[default]
	Void,

	/// `Bool` converts to no other type.
	Bool(bool),

	Int(c_int),

	/// Checked to be finite. A string the game parses as a float is not.
	Float(f32),

	/// A string. Converting to other types uses the C library: `atoi` for
	/// integers (`"1.9"` is 1), `atof` for floats, `atoi(s) != 0` for booleans
	/// (`"true"` is false), `"x y z"` or `"[x y z]"` for vectors, `"r g b a"`
	/// for colors, and a name search for entities.
	///
	/// Inputs that take a string, or any value, may keep it, so it is added to
	/// the game's string pool first, where it stays until the level ends. The
	/// pool compares strings case-insensitively, so such an input may receive
	/// an equal string, in another case, that was pooled before. A level holds
	/// about 65,000 pooled strings at most, and the game frees a pooled string
	/// at the next round restart if it equals the name of a removed entity a
	/// template spawned with a unique name suffix (`&0001`).
	String(&'a CStr),

	/// Checked to be finite. A string the game parses as a vector is not.
	Vector(Vector),

	Color(Color32),

	/// An entity, or `None` for a null handle. Inputs that take a string
	/// receive the entity's name, or an empty string for `None`.
	Entity(Option<Entity<'a>>),
}

impl InputValue<'_> {
	const fn field_type(self) -> sys::fieldtype_t {
		match self {
			Self::Void => sys::_fieldtypes_FIELD_VOID,
			Self::Bool(_) => sys::_fieldtypes_FIELD_BOOLEAN,
			Self::Int(_) => sys::_fieldtypes_FIELD_INTEGER,
			Self::Float(_) => sys::_fieldtypes_FIELD_FLOAT,
			Self::String(_) => sys::_fieldtypes_FIELD_STRING,
			Self::Vector(_) => sys::_fieldtypes_FIELD_VECTOR,
			Self::Color(_) => sys::_fieldtypes_FIELD_COLOR32,
			Self::Entity(_) => sys::_fieldtypes_FIELD_EHANDLE,
		}
	}

	fn is_finite(self) -> bool {
		match self {
			Self::Float(value) => value.is_finite(),
			Self::Vector(value) => value.is_finite(),
			_ => true,
		}
	}

	/// Builds the `variant_t` the game's setters would, with `string` as the
	/// form of a string value to pass.
	fn to_variant(self, string: sys::string_t) -> sys::variant_t {
		// SAFETY: Zero is valid for every field: a null string, a null handle
		// index, and `FIELD_VOID`.
		let mut variant = unsafe { MaybeUninit::<sys::variant_t>::zeroed().assume_init() };
		let mut handle = EntityHandle::INVALID;

		match self {
			Self::Void => {}
			Self::Bool(value) => variant.__bindgen_anon_1.bVal = value,
			Self::Int(value) => variant.__bindgen_anon_1.iVal = value,
			Self::Float(value) => variant.__bindgen_anon_1.flVal = value,
			Self::String(_) => variant.__bindgen_anon_1.iszVal = string,
			Self::Vector(value) => variant.__bindgen_anon_1.vecVal = [value.x, value.y, value.z],
			Self::Color(value) => variant.__bindgen_anon_1.rgbaVal = value.into(),
			Self::Entity(entity) => handle = entity.map_or(EntityHandle::INVALID, Entity::handle),
		}

		// As `CHandle`'s default constructor leaves it for other types.
		variant.eVal._base.m_Index = handle.to_raw();
		variant.fieldType = self.field_type();
		variant
	}
}

/// The type of value an input declares, which [`InputValue`]s are converted
/// to.
#[doc(alias = "fieldtype_t")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputType {
	/// The input ignores its value.
	Void,
	Bool,
	Int,
	Float,
	String,
	Vector,
	Color,
	Entity,

	/// The handler reads the value as it arrives (`FIELD_INPUT`).
	Any,

	/// A type no [`InputValue`] converts to, by its `fieldtype_t`.
	Other(i64),
}

impl InputType {
	fn from_raw(raw: sys::fieldtype_t) -> Self {
		match raw {
			sys::_fieldtypes_FIELD_VOID => Self::Void,
			sys::_fieldtypes_FIELD_BOOLEAN => Self::Bool,
			sys::_fieldtypes_FIELD_INTEGER => Self::Int,
			sys::_fieldtypes_FIELD_FLOAT => Self::Float,
			sys::_fieldtypes_FIELD_STRING => Self::String,
			sys::_fieldtypes_FIELD_VECTOR => Self::Vector,
			sys::_fieldtypes_FIELD_COLOR32 => Self::Color,
			sys::_fieldtypes_FIELD_EHANDLE => Self::Entity,
			sys::_fieldtypes_FIELD_INPUT => Self::Any,
			other => Self::Other(i64::from(other)),
		}
	}

	/// Whether `AcceptInput` accepts a value for this type: the value already
	/// has it, `variant_t::Convert` converts it, or the input takes a string
	/// and gets no value.
	fn accepts(self, value: InputValue<'_>) -> bool {
		use InputValue as V;

		matches!(
			(self, value),
			(Self::Void | Self::Any, _)
				| (Self::String, V::Void | V::String(_) | V::Entity(_))
				| (
					Self::Bool,
					V::Bool(_) | V::Int(_) | V::Float(_) | V::String(_)
				)
				| (
					Self::Int | Self::Float,
					V::Int(_) | V::Float(_) | V::String(_)
				)
				| (Self::Vector, V::Vector(_) | V::String(_))
				| (Self::Color, V::Color(_) | V::String(_))
				| (Self::Entity, V::Entity(_) | V::String(_))
		)
	}

	/// Whether the input's handler, or its field, may keep a string value.
	/// Every other type converts it before the handler runs.
	const fn keeps_strings(self) -> bool {
		matches!(self, Self::String | Self::Any)
	}
}

impl Display for InputType {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::Void => f.write_str("no value"),
			Self::Bool => f.write_str("a boolean"),
			Self::Int => f.write_str("an integer"),
			Self::Float => f.write_str("a float"),
			Self::String => f.write_str("a string"),
			Self::Vector => f.write_str("a vector"),
			Self::Color => f.write_str("a color"),
			Self::Entity => f.write_str("an entity"),
			Self::Any => f.write_str("any value"),
			Self::Other(raw) => write!(f, "field type {raw}"),
		}
	}
}

/// An input was not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
	/// Input names are ASCII, and compared ignoring case.
	#[error("the input name is empty or not ASCII")]
	InvalidName,

	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	#[error("the value contains a non-finite component")]
	NonFinite,

	#[error("the entity has no such input")]
	UnknownInput,

	#[error("the input takes {expected}, which the value cannot be converted to")]
	TypeMismatch { expected: InputType },

	/// The input runs code the caller chooses or spawns entities, either of
	/// which can free entities immediately. See
	/// [`ServerTools::accept_input`](crate::interfaces::ServerTools::accept_input).
	#[error("the input can free entities immediately, so it is only sent unchecked")]
	FreesEntities,

	/// The input would remove a player or the world, which the engine keeps
	/// using after they are freed.
	#[error("the input would remove a player or the world")]
	ProtectedEntity,

	/// The game resolves `"!picker"` through the first player's crosshair
	/// without checking that there is a first player.
	#[error("the value \"!picker\" can make the game dereference a missing player")]
	PickerName,

	/// Adding a string to the pool needs the world entity of a loaded map.
	#[error("the string could not be added to the game's string pool")]
	NotPooled,

	/// `AcceptInput` refused the input, although the checks before the call
	/// expected it to accept it.
	#[error("the entity rejected the input")]
	Rejected,
}

/// An input found and checked before being sent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CheckedInput<'s> {
	/// The name the input is declared with, which VScript's `Input<Name>`
	/// hooks are looked up by, case-sensitively.
	name: &'s CStr,
	declared: InputType,
	frees_entities: bool,
	kills: bool,
	target_is_player: bool,
}

impl<'s> CheckedInput<'s> {
	pub(crate) const fn name(self) -> &'s CStr {
		self.name
	}
}

/// Finds the input `AcceptInput` would dispatch and checks that it takes the
/// value, as `AcceptInput` would.
pub(crate) fn check_input<'s>(
	target: Entity<'s>,
	name: &CStr,
	value: InputValue<'_>,
) -> Result<CheckedInput<'s>, InputError> {
	let name = name.to_bytes();

	if name.is_empty() || !name.is_ascii() {
		return Err(InputError::InvalidName);
	}

	if target.is_marked_for_deletion() {
		return Err(InputError::MarkedForDeletion);
	}

	if !value.is_finite() {
		return Err(InputError::NonFinite);
	}

	let mut found = None;
	let mut is_player = false;
	let mut is_npc_maker = false;

	// `AcceptInput` searches from the entity's own class towards its bases, and
	// takes the first input with the name, ignoring ASCII case.
	for map in target.data_maps() {
		let class = data_map_class(map).map_or(&[][..], CStr::to_bytes);

		is_player |= class == b"CBasePlayer";
		is_npc_maker |= class == b"CBaseNPCMaker";

		if found.is_some() {
			continue;
		}

		found = data_fields(map).iter().find_map(|field| {
			// SAFETY: Input names are string literals of the game DLL.
			let external = unsafe { borrow_cstr::<'s>(field.externalName) }?;

			(field.flags & FTYPEDESC_INPUT != 0 && external.to_bytes().eq_ignore_ascii_case(name))
				.then_some((external, field.fieldType))
		});
	}

	let (declared_name, declared) = found.ok_or(InputError::UnknownInput)?;
	let declared = InputType::from_raw(declared);

	if !declared.accepts(value) {
		return Err(InputError::TypeMismatch { expected: declared });
	}

	let is_any = |names: &[&[u8]]| names.iter().any(|known| known.eq_ignore_ascii_case(name));

	Ok(CheckedInput {
		name: declared_name,
		declared,
		frees_entities: is_any(&CODE_OR_SPAWN_INPUTS)
			|| (is_npc_maker && is_any(&NPC_MAKER_SPAWN_INPUTS)),
		kills: is_any(&KILL_INPUTS),
		target_is_player: is_player,
	})
}

/// Refuses inputs known to free entities immediately or to crash the server.
pub(crate) fn check_guards(
	target: Entity<'_>,
	input: CheckedInput<'_>,
	value: InputValue<'_>,
) -> Result<(), InputError> {
	if input.frees_entities {
		return Err(InputError::FreesEntities);
	}

	if input.kills && (input.target_is_player || target.index() == Some(0)) {
		return Err(InputError::ProtectedEntity);
	}

	if let InputValue::String(string) = value
		&& string.to_bytes().eq_ignore_ascii_case(PICKER_NAME)
	{
		return Err(InputError::PickerName);
	}

	Ok(())
}

/// Builds the `variant_t` for a value sent to a checked input, pooling a
/// string the input may keep through `pool`.
pub(crate) fn to_variant(
	input: CheckedInput<'_>,
	value: InputValue<'_>,
	pool: impl FnOnce(&CStr) -> Option<sys::string_t>,
) -> Result<sys::variant_t, InputError> {
	let null = sys::string_t {
		pszValue: std::ptr::null(),
	};

	let (value, string) = match value {
		// `MAKE_STRING("")` and `AllocPooledString("")` both give a null string.
		InputValue::String(string) if string.is_empty() => (value, null),
		InputValue::String(string) if input.declared.keeps_strings() => {
			(value, pool(string).ok_or(InputError::NotPooled)?)
		}

		// Converted before the handler runs, so it is only read during the call.
		InputValue::String(string) => (
			value,
			sys::string_t {
				pszValue: string.as_ptr(),
			},
		),

		// `Convert` leaves a null handle as a handle, which string handlers read
		// as the text `<<null entity>>`, so it is sent as no value instead.
		InputValue::Entity(None) if input.declared == InputType::String => (InputValue::Void, null),

		_ => (value, null),
	};

	Ok(value.to_variant(string))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entities::FTYPEDESC_KEY;
	use crate::entities::test_support::{
		MOCK_NAME_OFFSET, MockEntity, ReceivedInput, base_entity_fields, data_map as map, field,
		leak, set_accepts, set_datamap, take_inputs,
	};
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use crate::interfaces::ServerTools;
	use crate::server::Game;
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

	unsafe extern "C" fn entity_by_index(
		_: *mut sys::IServerTools,
		index: c_int,
	) -> *mut sys::CBaseEntity {
		if index == 0 { WORLD.get() } else { null_mut() }
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

		if unsafe { CStr::from_ptr(key) } != c"targetname" {
			return false;
		}

		let mode = POOL_MODE.get();
		let value = match mode {
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

		unsafe {
			entity
				.byte_add(MOCK_NAME_OFFSET)
				.cast::<sys::string_t>()
				.write(sys::string_t { pszValue: pooled })
		};

		mode != Pool::FailsAfterWriting
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
			unsafe { ServerTools::from_raw(NonNull::new(self.tools).unwrap(), Game::TeamFortress2) }
		}

		/// Makes mock entities report the chain whose most-derived class is
		/// `class`.
		fn use_chain(&mut self, class: &CStr) {
			let map = self
				.maps
				.iter()
				.copied()
				.find(|&map| unsafe { CStr::from_ptr((*map).dataClassName) } == class)
				.unwrap();

			set_datamap(map);
		}
	}

	/// A mock entity, for as long as the mocks live.
	fn entity(mock: &mut MockEntity) -> Entity<'static> {
		unsafe { Entity::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
	}

	fn input(name: &'static CStr, field_type: sys::fieldtype_t) -> sys::typedescription_t {
		let mut input = field();

		input.fieldType = field_type;
		input.externalName = name.as_ptr();
		input.flags = FTYPEDESC_INPUT;
		input
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
		let mut key = field();

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
		let player = map(c"CTFPlayer", vec![], base_player);
		let maker = map(c"CBaseNPCMaker", vec![input(c"Spawn", VOID)], base);
		let npc_maker = map(c"CNPCMaker", vec![], maker);

		set_datamap(test);

		let maps = vec![test, base, base_player, player, maker, npc_maker];
		let tools_vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
						.write(entity_by_index);
					(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value);
				},
			)
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

	/// The bytes of three 32-bit words, as the union stores them.
	fn words(words: [u32; 3]) -> [u8; 12] {
		let mut bytes = [0; 12];

		for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
			chunk.copy_from_slice(&word.to_ne_bytes());
		}

		bytes
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
		assert_eq!(send(c"", InputValue::Void), Err(InputError::InvalidName));
		assert_eq!(
			send(c"K\xc3\xafll", InputValue::Void),
			Err(InputError::InvalidName)
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

		assert_eq!(
			send(c"SetSpeed", InputValue::Float(f32::NAN)),
			Err(InputError::NonFinite)
		);
		assert_eq!(
			send(
				c"SetOffset",
				InputValue::Vector(Vector::new(0.0, f32::INFINITY, 0.0))
			),
			Err(InputError::NonFinite)
		);
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

		// The unchecked form still finds the input and checks the value.
		unsafe {
			tools
				.accept_input_unchecked(target, c"RunScriptCode", script, None, None)
				.unwrap();
			assert_eq!(
				tools.accept_input_unchecked(
					target,
					c"RunScriptCode",
					InputValue::Int(1),
					None,
					None
				),
				Err(InputError::TypeMismatch {
					expected: InputType::String
				})
			);
			assert_eq!(
				tools.accept_input_unchecked(target, c"", InputValue::Void, None, None),
				Err(InputError::InvalidName)
			);
			assert_eq!(
				tools.accept_input_unchecked(
					target,
					c"SetSpeed",
					InputValue::Float(f32::NAN),
					None,
					None
				),
				Err(InputError::NonFinite)
			);
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

		mocks.use_chain(c"CNPCMaker");

		let target = entity(&mut mocks.target);

		assert_eq!(
			mocks
				.tools()
				.accept_input(target, c"Spawn", InputValue::Void, target, target),
			Err(InputError::FreesEntities)
		);
		assert_eq!(take_inputs().len(), 1);

		mocks.target.set_eflags(1);

		let target = entity(&mut mocks.target);

		assert_eq!(
			unsafe {
				mocks
					.tools()
					.accept_input_unchecked(target, c"Color", red, None, None)
			},
			Err(InputError::MarkedForDeletion)
		);
	}

	#[test]
	fn declared_types_describe_themselves() {
		assert_eq!(
			InputType::from_raw(sys::_fieldtypes_FIELD_INPUT),
			InputType::Any
		);
		assert_eq!(
			InputType::from_raw(sys::_fieldtypes_FIELD_TIME),
			InputType::Other(16)
		);
		assert_eq!(InputType::Other(16).to_string(), "field type 16");
		assert!(!InputType::Other(16).accepts(InputValue::String(c"1")));
		assert_eq!(
			InputError::TypeMismatch {
				expected: InputType::Color
			}
			.to_string(),
			"the input takes a color, which the value cannot be converted to"
		);
	}
}
