//! Entity I/O: sending inputs to entities, as map outputs, `ent_fire`, and
//! VScript's `AcceptInput` do.
//!
//! [`ServerTools::accept_input`] sends an input and runs its handler before
//! returning. Before calling into the game, it finds the input the way
//! `AcceptInput` does, so an unknown input or a value the input cannot take
//! is reported as an [`InputError`] instead of a console warning.
//!
//! [`ServerTools::accept_input`]: crate::interfaces::ServerTools::accept_input

#[cfg(test)]
#[path = "tests/inputs.rs"]
mod tests;

use crate::datatables::ServerClass;
use crate::entities::{Entity, EntityHandle};
use crate::math::{Color32, Vector};
use sdk_raw::entities::datamap::FTYPEDESC_INPUT;
use sdk_raw::inputs::{MAX_CONTROL_POINTS, MAX_PREVIOUS_POINTS, NUM_ROBOT_TYPES, Variant};
use sdk_raw::players::{MAX_TEAMS, TF_TEAM_COUNT};
use sdk_raw::tier0::MAX_PATH;
use sdk_raw::util::cstr::copy_cstr;
use std::ffi::{CStr, c_int};
use std::fmt::{self, Display, Formatter};

/// The input that sets one of its target's key values, before the first space
/// of its value, to the rest (`CBaseEntity::InputAddOutput`).
const ADD_OUTPUT_INPUT: &[u8] = b"AddOutput";

/// The exclusive bound on a TF2 building's maximum health as a float: 2^31,
/// from which TF2's conversion of the building's float health back to an
/// `int` is undefined in C++.
const BUILDING_HEALTH_LIMIT: f32 = 2_147_483_648.0;

/// The key prefixes of `CTriggerAreaCapture`'s per-team data, which its
/// `KeyValue` follows with a team number.
const CAPTURE_AREA_TEAM_KEYS: [&[u8]; 4] = [
	b"team_numcap_",
	b"team_cancap_",
	b"team_spawn_",
	b"team_startcap_",
];

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

/// The key prefixes of `CTeamControlPoint`'s per-team data, which its
/// `KeyValue` follows with a team number.
const CONTROL_POINT_TEAM_KEYS: [&[u8]; 6] = [
	b"team_capsound_",
	b"team_model_",
	b"team_timedpoints_",
	b"team_bodygroup_",
	b"team_icon_",
	b"team_overlay_",
];

/// Inputs that remove their target. The engine keeps using a player's or the
/// world's entity, so removing one crashes the server once it is freed.
const KILL_INPUTS: [&[u8]; 2] = [b"Kill", b"KillHierarchy"];

/// The key prefix of `CTeamControlPointMaster`'s team icons, which its
/// `KeyValue` follows with a team number.
const MASTER_TEAM_KEY: &[u8] = b"team_base_icon_";

/// The key value of `m_iMaxHealth` (`CBaseEntity`'s data description).
const MAX_HEALTH_KEY: &[u8] = b"max_health";

/// Spawn inputs of NPC makers, which immediately remove a `prop_physics`
/// blocking the spawn.
const NPC_MAKER_SPAWN_INPUTS: [&[u8]; 4] = [
	b"Spawn",
	b"SpawnNPCInRadius",
	b"SpawnNPCInLine",
	b"SpawnMultiple",
];

/// The procedural entity name that looks through the first player's crosshair
/// without checking that there is one (`FindEntityProcedural`).
const PICKER_NAME: &[u8] = b"!picker";

/// The key value of `CTeamControlPoint::m_iDefaultOwner`, a team number.
const POINT_DEFAULT_OWNER_KEY: &[u8] = b"point_default_owner";

/// The key value of `CTeamControlPoint::m_iPointIndex`.
const POINT_INDEX_KEY: &[u8] = b"point_index";

/// The key prefix of `CTeamControlPoint`'s previous points, which its
/// `KeyValue` follows with a team number and an index, as `<team>_<index>`.
const PREVIOUS_POINT_KEY: &[u8] = b"team_previouspoint_";

/// The key value of the robot type of TF2's
/// `CTFRobotDestruction_RobotSpawn` (`m_spawnData.m_eType`).
const ROBOT_TYPE_KEY: &[u8] = b"type";

/// The input of TF2's buildings (`CBaseObject::InputSetHealth`) that sets
/// their maximum health to its value, as well as their health.
const SET_HEALTH_INPUT: &[u8] = b"SetHealth";

/// The input that passes its integer to the target's `ChangeTeam`
/// (`CBaseEntity::InputSetTeam`).
const SET_TEAM_INPUT: &[u8] = b"SetTeam";

/// The key value of `m_iTeamNum` (`CBaseEntity`'s data description).
const TEAM_NUMBER_KEY: &[u8] = b"teamnumber";

/// The send table of `CTeam`, which every team entity's class derives from.
/// `CTeam` declares no data description.
const TEAM_TABLE: &CStr = c"DT_Team";

/// Inputs that index a game table with a lookup of their value without checking
/// that the lookup succeeded. TF2's `SpeakResponseConcept` reads
/// `g_pszMPConcepts[-1]` for a concept name it does not know.
const UNCHECKED_LOOKUP_INPUTS: [&[u8]; 1] = [b"SpeakResponseConcept"];

/// An input found and checked before being sent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CheckedInput<'s> {
	/// The name the input is declared with, which VScript's `Input<Name>`
	/// hooks are looked up by, case-sensitively.
	name: &'s CStr,

	/// The type the input declares, which decides whether a string value is
	/// pooled and whether a null entity is sent as no value.
	declared: InputType,

	/// Whether the input is one of [`CODE_OR_SPAWN_INPUTS`], or one of
	/// [`NPC_MAKER_SPAWN_INPUTS`] sent to an NPC maker.
	frees_entities: bool,

	/// Whether the input is one of [`KILL_INPUTS`].
	kills: bool,

	/// Whether the input is one of [`UNCHECKED_LOOKUP_INPUTS`].
	looks_up_unchecked: bool,

	/// Whether the input is [`ADD_OUTPUT_INPUT`], which sets a key value of
	/// the target.
	adds_output: bool,

	/// Whether the input is [`SET_HEALTH_INPUT`] sent to a TF2 building,
	/// which stores its value as the building's maximum health.
	sets_max_health: bool,

	/// Whether the input is [`SET_TEAM_INPUT`] taking an integer, which
	/// `CBaseEntity::InputSetTeam` passes to the target's `ChangeTeam`.
	sets_team: bool,

	/// Whether the target is a player or a soundscape. The world is recognized
	/// by its index instead.
	target_is_protected: bool,
}

impl<'s> CheckedInput<'s> {
	/// The name to send the input by: the one it is declared with.
	pub(crate) const fn name(self) -> &'s CStr {
		self.name
	}
}

/// An input was not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
	/// The input name is empty or not ASCII. Input names are ASCII, and
	/// compared ignoring case.
	#[error("the input name is empty or not ASCII")]
	InvalidName,

	/// The target is marked for deletion, as
	/// [`Entity::is_marked_for_deletion`] reports.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// An [`InputValue::Float`] or [`InputValue::Vector`] has a NaN or infinite
	/// component.
	#[error("the value contains a non-finite component")]
	NonFinite,

	/// Neither the target's class nor any of its bases declares an input with
	/// the name, ignoring ASCII case. Key values are not inputs.
	#[error("the entity has no such input")]
	UnknownInput,

	/// The input declares a type the value does not have and is not converted
	/// to, as [`InputValue`] describes.
	#[error("the input takes {expected}, which the value cannot be converted to")]
	TypeMismatch {
		/// The type the input declares.
		expected: InputType,
	},

	/// The input runs code the caller chooses or spawns entities, either of
	/// which can free entities immediately. See
	/// [`ServerTools::accept_input`](crate::interfaces::ServerTools::accept_input).
	#[error("the input can free entities immediately, so it is only sent unchecked")]
	FreesEntities,

	/// The input would remove the world, a player, or a soundscape, which the
	/// game keeps using after they are freed.
	#[error("the input would remove the world, a player, or a soundscape")]
	ProtectedEntity,

	/// The game resolves `"!picker"` through the first player's crosshair
	/// without checking that there is a first player.
	#[error("the value \"!picker\" can make the game dereference a missing player")]
	PickerName,

	/// The input looks its value up in a game table and uses the result without
	/// checking it, such as TF2's `SpeakResponseConcept` with a concept name the
	/// game does not know, which reads out of bounds.
	#[error("the input uses a lookup of its value unchecked, so it is only sent unchecked")]
	UncheckedLookup,

	/// Adding a string to the pool needs the world entity of a loaded map.
	#[error("the string could not be added to the game's string pool")]
	NotPooled,

	/// The input would set a maximum health the game cannot handle: 0 or
	/// less, which game code divides integers by, crashing for 0, or for one
	/// of TF2's buildings, a value that rounds to 2^31 or more as a float,
	/// which TF2 converts back to an `int` in a way C++ leaves undefined.
	///
	/// `SetHealth` sets a TF2 building's maximum health as well as its
	/// health, and `AddOutput` with the `max_health` key sets any entity's.
	/// TF2's Horseless Headless Horsemann divides integers by its maximum
	/// health while it moves, on the server, and the client of a building's
	/// builder by the building's, in its building-status HUD. A value whose
	/// conversion to an `int` the C library leaves undefined, such as a float
	/// beyond `int`'s range or a string of too many digits, is refused too.
	#[error("the input would set a maximum health the game cannot handle")]
	InvalidMaxHealth,

	/// `AddOutput` would set a key value the game cannot handle, other than
	/// the maximum health: a number the game indexes an array with unchecked,
	/// or the team number of a player or a team entity, as
	/// [`ServerTools::set_key_value`](crate::interfaces::ServerTools::set_key_value)
	/// lists.
	#[error("the input would set a key value the game cannot handle")]
	InvalidKeyValue,

	/// `SetTeam` would put an entity in a team the game cannot handle. A
	/// player's `ChangeTeam` checks the team itself, but other entities take
	/// any team number, which the game then indexes its per-team arrays with
	/// unchecked, so it must lie within 0 to 3, the teams of TF2 and of
	/// Source SDK 2013's templates. A team entity's number must not change at
	/// all, since clients look teams up by it.
	#[error("the input would put the entity in a team the game cannot handle")]
	InvalidTeam,

	/// `AcceptInput` refused the input, although the checks before the call
	/// expected it to accept it.
	#[error("the entity rejected the input")]
	Rejected,
}

/// The type of value an input declares, which [`InputValue`]s are converted
/// to.
#[doc(alias("fieldtype_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputType {
	/// The input ignores its value.
	#[doc(alias("FIELD_VOID"))]
	Void,

	/// A boolean, which integers, floats, and strings convert to.
	#[doc(alias("FIELD_BOOLEAN"))]
	Bool,

	/// An integer, which floats and strings convert to.
	#[doc(alias("FIELD_INTEGER"))]
	Int,

	/// A float, which integers and strings convert to.
	#[doc(alias("FIELD_FLOAT"))]
	Float,

	/// A string, which entities convert to as their name. [`InputValue::Void`]
	/// arrives as an empty string.
	#[doc(alias("FIELD_STRING"))]
	String,

	/// A vector, which strings convert to.
	#[doc(alias("FIELD_VECTOR"))]
	Vector,

	/// A color, which strings convert to.
	#[doc(alias("FIELD_COLOR32"))]
	Color,

	/// An entity handle, which strings convert to by a name search.
	#[doc(alias("FIELD_EHANDLE"))]
	Entity,

	/// The handler reads the value as it arrives (`FIELD_INPUT`).
	#[doc(alias("FIELD_INPUT"))]
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

/// The value an input carries (`variant_t`).
///
/// `AcceptInput` converts the value to the type the input declares, as
/// [`InputType`] describes: strings to every type but [`InputType::Other`],
/// as map I/O relies on, integers and floats to each other and to booleans,
/// and entities to their name.
#[doc(alias("variant_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputValue<'a> {
	/// No value, as map I/O sends for an empty parameter. Inputs that take a
	/// string receive an empty one.
	#[default]
	Void,

	/// `Bool` converts to no other type.
	Bool(bool),

	/// An integer, which converts to floats and booleans.
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

	/// `Color` converts to no other type.
	Color(Color32),

	/// An entity, or `None` for a null handle. Inputs that take a string
	/// receive the entity's name, or an empty string for `None`.
	Entity(Option<Entity<'a>>),
}

impl InputValue<'_> {
	/// Whether every float component is finite. Values without floats are.
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
		match self {
			Self::Void => Variant::Void,
			Self::Bool(value) => Variant::Bool(value),
			Self::Int(value) => Variant::Int(value),
			Self::Float(value) => Variant::Float(value),
			Self::String(_) => Variant::String(string),
			Self::Vector(value) => Variant::Vector([value.x, value.y, value.z]),
			Self::Color(value) => Variant::Color(value.into()),

			Self::Entity(entity) => Variant::Entity(
				entity
					.map_or(EntityHandle::INVALID, Entity::handle)
					.to_raw(),
			),
		}
		.to_raw()
	}
}

/// The classes of an entity that decide which key values ([`check_key`]) and
/// team numbers ([`check_team_change`]) the game cannot handle for it.
#[derive(Debug, Clone, Copy, Default)]
struct TargetClasses {
	/// Whether a data description of the entity's class or its bases is TF2's
	/// `CBaseObject`'s, whose health TF2 keeps as a float.
	building: bool,

	/// As for `building`, `CTriggerAreaCapture`'s.
	capture_area: bool,

	/// As for `building`, `CTeamControlPoint`'s.
	control_point: bool,

	/// As for `building`, `CTeamControlPointMaster`'s.
	control_point_master: bool,

	/// As for `building`, `CBasePlayer`'s.
	player: bool,

	/// As for `building`, TF2's `CTFRobotDestruction_RobotSpawn`'s.
	robot_spawn: bool,

	/// Whether the entity's send table derives from [`TEAM_TABLE`].
	team: bool,
}

impl TargetClasses {
	/// Finds the classes of `entity`.
	fn of(entity: Entity<'_>) -> Self {
		let mut target = Self {
			team: entity
				.server_class()
				.and_then(ServerClass::table)
				.is_some_and(|table| table.derives_from_named(TEAM_TABLE)),
			..Self::default()
		};

		for map in entity.data_maps() {
			let class = match map.class_name().map(CStr::to_bytes) {
				Some(b"CBaseObject") => &mut target.building,
				Some(b"CBasePlayer") => &mut target.player,
				Some(b"CTeamControlPoint") => &mut target.control_point,
				Some(b"CTeamControlPointMaster") => &mut target.control_point_master,
				Some(b"CTFRobotDestruction_RobotSpawn") => &mut target.robot_spawn,
				Some(b"CTriggerAreaCapture") => &mut target.capture_area,
				_ => continue,
			};

			*class = true;
		}

		target
	}
}

/// What C's `atoi` returns for `string`, as [`scan_int`] reads it, or 0 for
/// no digits. Returns `None` for an integer beyond `int`'s range, for which
/// `atoi` is undefined.
fn atoi(string: &[u8]) -> Option<c_int> {
	scan_int(string).map_or(Some(0), |(value, _)| value)
}

/// Checks the key value an `AddOutput` input sets with `value`, as
/// [`check_key`] does. `CBaseEntity::InputAddOutput` copies the string it
/// receives into a buffer of `MAX_PATH` bytes, and passes the part before the
/// first space to `KeyValue` as the key, and the rest as the value, with its
/// colons replaced by commas, at which `atoi` stops either way.
fn check_added_key_value(target: Entity<'_>, value: InputValue<'_>) -> Result<(), InputError> {
	let name;

	let received = match value {
		InputValue::String(string) => string.to_bytes(),

		// String inputs receive an entity's name (`variant_t::Convert`).
		InputValue::Entity(Some(entity)) => {
			// SAFETY: The name field belongs to the live entity, and holds null
			// or a pooled string, which is copied immediately.
			name = entity
				.name_field()
				.and_then(|field| unsafe { copy_cstr(field.read().pszValue) })
				.unwrap_or_default();

			name.to_bytes()
		}

		_ => return Ok(()),
	};

	let copied = &received[..received.len().min(MAX_PATH - 1)];

	let Some(space) = copied.iter().position(|&byte| byte == b' ') else {
		return Ok(());
	};

	check_key(
		TargetClasses::of(target),
		&copied[..space],
		&copied[space + 1..],
	)
}

/// Refuses inputs known to free entities immediately, or to crash the server
/// or its clients.
pub(crate) fn check_guards(
	target: Entity<'_>,
	input: CheckedInput<'_>,
	value: InputValue<'_>,
) -> Result<(), InputError> {
	if input.frees_entities {
		return Err(InputError::FreesEntities);
	}

	if input.kills && (input.target_is_protected || target.index() == Some(0)) {
		return Err(InputError::ProtectedEntity);
	}

	if input.looks_up_unchecked {
		return Err(InputError::UncheckedLookup);
	}

	if input.sets_max_health
		&& !converted_int(value).is_some_and(|max| is_valid_max_health(max, true))
	{
		return Err(InputError::InvalidMaxHealth);
	}

	if input.adds_output {
		check_added_key_value(target, value)?;
	}

	if input.sets_team {
		check_team_change(target, value)?;
	}

	if let InputValue::String(string) = value
		&& string.to_bytes().eq_ignore_ascii_case(PICKER_NAME)
	{
		return Err(InputError::PickerName);
	}

	Ok(())
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
	let mut is_building = false;
	let mut is_protected = false;
	let mut is_npc_maker = false;

	// `AcceptInput` searches from the entity's own class towards its bases, and
	// takes the first input with the name, ignoring ASCII case.
	for map in target.data_maps() {
		let class = map.class_name().map_or(&[][..], CStr::to_bytes);

		is_building |= class == b"CBaseObject";
		is_protected |= class == b"CBasePlayer" || class == b"CEnvSoundscape";
		is_npc_maker |= class == b"CBaseNPCMaker";

		if found.is_some() {
			continue;
		}

		found = map.fields().iter().find_map(|field| {
			let external = field.external_name()?;

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
		looks_up_unchecked: is_any(&UNCHECKED_LOOKUP_INPUTS),
		adds_output: is_any(&[ADD_OUTPUT_INPUT]),
		sets_max_health: is_building && is_any(&[SET_HEALTH_INPUT]),
		sets_team: declared == InputType::Int && is_any(&[SET_TEAM_INPUT]),
		target_is_protected: is_protected,
	})
}

/// Checks that the game can handle `value` for `key` on an entity of
/// `target`'s classes, as `KeyValue` reads them, and fails with
/// [`InputError::InvalidMaxHealth`] for a maximum health it cannot handle, or
/// [`InputError::InvalidKeyValue`] for the other values
/// [`ServerTools::set_key_value`] refuses.
///
/// [`ServerTools::set_key_value`]: crate::interfaces::ServerTools::set_key_value
fn check_key(target: TargetClasses, key: &[u8], value: &[u8]) -> Result<(), InputError> {
	let below =
		|number: Option<c_int>, limit: c_int| number.is_some_and(|n| (0..limit).contains(&n));
	let checked = |valid: bool| valid.then_some(()).ok_or(InputError::InvalidKeyValue);

	// Class overrides of `KeyValue` match these prefixes case-sensitively,
	// before `CBaseEntity::KeyValue` sees the key, and index their per-team
	// data with the number after them, read with `atoi`, unchecked.
	let team_suffix =
		|prefixes: &[&[u8]]| prefixes.iter().find_map(|prefix| key.strip_prefix(*prefix));

	if target.control_point {
		if let Some(rest) = key.strip_prefix(PREVIOUS_POINT_KEY) {
			return checked(is_valid_previous_point(rest));
		}

		if let Some(rest) = team_suffix(&CONTROL_POINT_TEAM_KEYS) {
			return checked(below(atoi(rest), TF_TEAM_COUNT));
		}
	}

	if target.capture_area
		&& let Some(rest) = team_suffix(&CAPTURE_AREA_TEAM_KEYS)
	{
		return checked(below(atoi(rest), TF_TEAM_COUNT));
	}

	if target.control_point_master
		&& let Some(rest) = key.strip_prefix(MASTER_TEAM_KEY)
	{
		return checked(below(atoi(rest), MAX_TEAMS));
	}

	// `CBaseEntity::KeyValue` cuts the key at its first `#`, which Hammer
	// appends to repeated keys, and `ParseKeyvalue` compares the rest with the
	// keys of the data description ignoring case, and reads integers with
	// `atoi`.
	let key = key.split(|&byte| byte == b'#').next().unwrap_or_default();
	let is = |name: &[u8]| key.eq_ignore_ascii_case(name);
	let value = atoi(value);

	if is(MAX_HEALTH_KEY) {
		return value
			.is_some_and(|max| is_valid_max_health(max, target.building))
			.then_some(())
			.ok_or(InputError::InvalidMaxHealth);
	}

	// A player stays in its old team's list of players, which keeps pointing
	// to it once it is freed, and clients look team entities up by their team
	// number, unchecked.
	if is(TEAM_NUMBER_KEY) {
		return checked(!target.player && !target.team && below(value, TF_TEAM_COUNT));
	}

	if target.control_point && is(POINT_INDEX_KEY) {
		return checked(below(value, MAX_CONTROL_POINTS));
	}

	if target.control_point && is(POINT_DEFAULT_OWNER_KEY) {
		return checked(below(value, TF_TEAM_COUNT));
	}

	if target.robot_spawn && is(ROBOT_TYPE_KEY) {
		return checked(below(value, NUM_ROBOT_TYPES));
	}

	Ok(())
}

/// Checks that the game can handle `value` for `key` on `target`, as
/// [`ServerTools::set_key_value`] describes, with the errors of
/// [`check_key`].
///
/// [`ServerTools::set_key_value`]: crate::interfaces::ServerTools::set_key_value
pub(crate) fn check_key_value(
	target: Entity<'_>,
	key: &CStr,
	value: &CStr,
) -> Result<(), InputError> {
	check_key(TargetClasses::of(target), key.to_bytes(), value.to_bytes())
}

/// Refuses a `SetTeam` input with `value` that would put `target` in a team
/// the game cannot handle, as [`InputError::InvalidTeam`] describes.
/// `variant_t::Convert` converts the value as [`converted_int`] does.
fn check_team_change(target: Entity<'_>, value: InputValue<'_>) -> Result<(), InputError> {
	let target = TargetClasses::of(target);

	let valid = target.player
		|| (!target.team
			&& converted_int(value).is_some_and(|team| (0..TF_TEAM_COUNT).contains(&team)));

	valid.then_some(()).ok_or(InputError::InvalidTeam)
}

/// The `int` `AcceptInput` converts `value` to for an input taking an
/// integer, as `variant_t::Convert` does: floats with a C cast, which
/// truncates, and strings with `atoi`.
///
/// Returns `None` for other values, and for conversions the C library leaves
/// undefined, of floats and decimal strings beyond `int`'s range.
fn converted_int(value: InputValue<'_>) -> Option<c_int> {
	match value {
		InputValue::Int(value) => Some(value),

		InputValue::Float(value) => (-2_147_483_648.0..2_147_483_648.0)
			.contains(&value)
			.then_some(value as c_int),

		InputValue::String(string) => atoi(string.to_bytes()),
		_ => None,
	}
}

/// Whether the game can handle `max` as an entity's maximum health: it is
/// positive, and for a TF2 building, rounds below 2^31 as a float.
fn is_valid_max_health(max: c_int, building: bool) -> bool {
	max > 0 && (!building || (max as f32) < BUILDING_HEALTH_LIMIT)
}

/// Whether `CTeamControlPoint::KeyValue` indexes its data within bounds for
/// a key of [`PREVIOUS_POINT_KEY`] followed by `rest`, which it reads with
/// `sscanf(rest, "%d_%d", &team, &index)`. The team is left uninitialized,
/// and the index 0, if their conversions fail.
fn is_valid_previous_point(rest: &[u8]) -> bool {
	let Some((Some(team), rest)) = scan_int(rest) else {
		return false;
	};

	let index = match rest.strip_prefix(b"_").and_then(scan_int) {
		Some((index, _)) => index,
		None => Some(0),
	};

	(0..TF_TEAM_COUNT).contains(&team)
		&& index.is_some_and(|index| (0..MAX_PREVIOUS_POINTS).contains(&index))
}

/// Reads a decimal `int` as C's `atoi` and `scanf`'s `%d` do: the digits
/// after any white space and an optional sign, up to the first other byte.
/// Returns the integer, or `None` beyond `int`'s range, for which both are
/// undefined, and the bytes after it, or `None` if there are no digits.
fn scan_int(string: &[u8]) -> Option<(Option<c_int>, &[u8])> {
	let start = string
		.iter()
		// C's `isspace` also counts the vertical tab.
		.position(|&byte| !byte.is_ascii_whitespace() && byte != 0x0B)
		.unwrap_or(string.len());

	let (negative, digits) = match &string[start..] {
		[b'-', digits @ ..] => (true, digits),
		[b'+', digits @ ..] => (false, digits),
		digits => (false, digits),
	};

	let count = digits
		.iter()
		.take_while(|byte| byte.is_ascii_digit())
		.count();

	if count == 0 {
		return None;
	}

	let value = digits[..count].iter().try_fold(0, |value: c_int, &digit| {
		let digit = c_int::from(digit - b'0');

		value.checked_mul(10).and_then(|value| {
			if negative {
				value.checked_sub(digit)
			} else {
				value.checked_add(digit)
			}
		})
	});

	Some((value, &digits[count..]))
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
