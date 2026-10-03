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

use crate::entities::{Entity, EntityHandle};
use crate::math::{Color32, Vector};
use sdk_raw::entities::datamap::FTYPEDESC_INPUT;
use sdk_raw::inputs::Variant;
use std::ffi::{CStr, c_int};
use std::fmt::{self, Display, Formatter};

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

/// Inputs that remove their target. The engine keeps using a player's or the
/// world's entity, so removing one crashes the server once it is freed.
const KILL_INPUTS: [&[u8]; 2] = [b"Kill", b"KillHierarchy"];

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

/// Refuses inputs known to free entities immediately or to crash the server.
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
	let mut is_protected = false;
	let mut is_npc_maker = false;

	// `AcceptInput` searches from the entity's own class towards its bases, and
	// takes the first input with the name, ignoring ASCII case.
	for map in target.data_maps() {
		let class = map.class_name().map_or(&[][..], CStr::to_bytes);

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
		target_is_protected: is_protected,
	})
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
