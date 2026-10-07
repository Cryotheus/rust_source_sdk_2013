//! TF2's objective entities: the map entities that time rounds, hold and
//! capture control points, carry flags and push payload carts, and the
//! round wins they end in.
//!
//! Each wrapper checks the entity's class when it is created, as
//! [`RoundTimer::new`] does, and then reads the entity's state from its
//! networked variables, or its data description for entities that are not
//! networked, and changes it through the inputs a map's logic sends, with
//! [`ServerTools::accept_input`]'s checks. An input runs its handler before
//! returning, so the change is in effect when the method returns, as far as
//! the handler makes it: handlers ignore some inputs in some states, as each
//! method documents. The wrapper is both the activator and the caller of
//! the inputs it sends, unless a method takes them.
//!
//! - Rounds: [`RoundTimer`] (`team_round_timer`), [`KothLogic`]
//!   (`tf_logic_koth`), whose timers are round timers too, and [`RoundWin`]
//!   (`game_round_win`).
//! - Control points: [`ControlPoint`] (`team_control_point`), the
//!   [`CaptureArea`] (`trigger_capture_area`) players capture it in, the
//!   [`ControlPointMaster`] (`team_control_point_master`) that wins rounds
//!   with the points, its mini-rounds, [`ControlPointRound`]
//!   (`team_control_point_round`), and the [`ObjectiveResource`]
//!   (`tf_objective_resource`) that networks the points' state.
//! - Flags: [`CaptureFlag`] (`item_teamflag`) and the [`CaptureZone`]
//!   (`func_capturezone`) players capture it in.
//! - Payload: [`TrainWatcher`] (`team_train_watcher`), which tracks a cart.
//!
//! Map logic, other plugins and the game's own code send the same inputs
//! and change the same variables, so what a wrapper reads can change after
//! any call into the game. A round reset that resets the map, as most do,
//! removes the map's objective entities and spawns them again from the map,
//! so what a wrapper changes lasts until then.
//!
//! [`ServerTools::accept_input`]: crate::interfaces::ServerTools::accept_input

mod capture_area;
mod control_points;
mod flags;
mod objective_resource;
mod round_timer;
mod train_watcher;

use crate::datatables::{NetProp, NetPropError, NetVar, Storage};
use crate::entities::Entity;
use crate::inputs::{InputError, InputValue};
use crate::{Game, InterfaceError, Server};
use std::ffi::{CStr, CString, c_int};

pub use capture_area::CaptureArea;
pub use control_points::{CaptureWins, ControlPoint, ControlPointMaster, ControlPointRound};
pub use flags::{CaptureFlag, CaptureZone, FlagStatus, FlagType};
pub use objective_resource::ObjectiveResource;
pub use round_timer::{KothLogic, RoundTimer, RoundTimerOutput, RoundWin, TimerState, WinReason};
pub use train_watcher::TrainWatcher;

/// An entity of one of the objective classes, which the wrappers read and
/// send inputs to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Objective<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> Objective<'s> {
	/// Wraps `entity` if the server runs TF2 and one of the entity's data
	/// descriptions is `class`'s, or fails with
	/// [`ObjectiveError::WrongClass`] naming `expected`.
	pub(crate) fn new(
		server: Server<'s>,
		entity: Entity<'s>,
		class: &CStr,
		expected: &'static str,
	) -> Result<Self, ObjectiveError> {
		if server.game() != Game::TeamFortress2 || !entity.has_data_map_class(class) {
			return Err(ObjectiveError::WrongClass { expected });
		}

		Ok(Self { server, entity })
	}

	/// Reads a boolean field of `class`'s own data description.
	pub(crate) fn bool_field(
		self,
		class: &'static CStr,
		field: &'static CStr,
	) -> Result<bool, ObjectiveError> {
		let offset = self.field_offset(class, field, &[sys::_fieldtypes_FIELD_BOOLEAN], 1, 0)?;

		// SAFETY: `field_offset` found a `bool` field of the class at the offset,
		// in the live entity, which is read without forming a reference, as the
		// game writes it too.
		Ok(unsafe { self.entity.as_ptr().byte_add(offset).cast::<u8>().read() } != 0)
	}

	/// Fails with [`ObjectiveError::MarkedForDeletion`] if the entity is
	/// marked for deletion.
	pub(crate) fn check_live(self) -> Result<(), ObjectiveError> {
		if self.entity.is_marked_for_deletion() {
			Err(ObjectiveError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	/// The game time, in seconds.
	pub(crate) fn current_time(self) -> Result<f32, ObjectiveError> {
		self.server
			.player_info_manager()?
			.global_vars()
			.map(|globals| globals.current_time())
			.ok_or(ObjectiveError::NoGlobals)
	}

	/// Reads element `index` of the networked array `name`.
	pub(crate) fn element<T: NetVar>(self, name: &CStr, index: usize) -> Result<T, ObjectiveError> {
		Ok(self.net_prop(name)?.element(index)?.get(self.entity)?)
	}

	/// The entity.
	pub(crate) const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The offset of element `index` of the field named `field`, of one of
	/// `types`, that `class`'s own data description declares with elements of
	/// `size` bytes, aligned to `size` and within the 64 KiB the game's
	/// entities fit in. A field that is not an array has one element.
	fn field_offset(
		self,
		class: &'static CStr,
		field: &'static CStr,
		types: &[sys::fieldtype_t],
		size: usize,
		index: usize,
	) -> Result<usize, ObjectiveError> {
		self.check_live()?;

		self.entity
			.data_maps()
			.find(|map| map.class_name() == Some(class))
			.and_then(|map| {
				map.fields().iter().find(|declared| {
					declared.name() == Some(field) && types.contains(&declared.fieldType)
				})
			})
			.filter(|declared| {
				let count = usize::from(declared.fieldSize);

				index < count
					&& usize::try_from(declared.fieldSizeInBytes).ok() == size.checked_mul(count)
			})
			.and_then(|declared| declared.offset()?.checked_add(index.checked_mul(size)?))
			.filter(|offset| {
				offset.is_multiple_of(size)
					&& offset.checked_add(size).is_some_and(|end| end <= 65_536)
			})
			.ok_or(ObjectiveError::UnsupportedLayout { class, field })
	}

	/// Reads the networked boolean `name`, stored as a `bool` or an `int`.
	pub(crate) fn flag(self, name: &CStr) -> Result<bool, ObjectiveError> {
		Ok(flag(self.net_prop(name)?, self.entity)?)
	}

	/// Reads element `index` of the networked array of booleans `name`, stored
	/// as `bool`s or `int`s.
	pub(crate) fn flag_element(self, name: &CStr, index: usize) -> Result<bool, ObjectiveError> {
		Ok(flag(self.net_prop(name)?.element(index)?, self.entity)?)
	}

	/// Reads element `index` of a float array of `class`'s own data
	/// description, as [`Self::float_field`] reads a float.
	pub(crate) fn float_element(
		self,
		class: &'static CStr,
		field: &'static CStr,
		index: usize,
	) -> Result<f32, ObjectiveError> {
		let offset = self.field_offset(
			class,
			field,
			&[sys::_fieldtypes_FIELD_FLOAT, sys::_fieldtypes_FIELD_TIME],
			size_of::<f32>(),
			index,
		)?;

		// SAFETY: As for `bool_field`, for a `float` field.
		Ok(unsafe { self.entity.as_ptr().byte_add(offset).cast::<f32>().read() })
	}

	/// Reads a float field of `class`'s own data description, declared as a
	/// float or a game time.
	pub(crate) fn float_field(
		self,
		class: &'static CStr,
		field: &'static CStr,
	) -> Result<f32, ObjectiveError> {
		self.float_element(class, field, 0)
	}

	/// Reads the networked variable `name`.
	pub(crate) fn get<T: NetVar>(self, name: &CStr) -> Result<T, ObjectiveError> {
		Ok(self.net_prop(name)?.get(self.entity)?)
	}

	/// The entity the networked handle `name` refers to, or `None` if it refers
	/// to none, or to an entity that no longer exists.
	pub(crate) fn handle_entity(self, name: &CStr) -> Result<Option<Entity<'s>>, ObjectiveError> {
		let handle = self.net_prop(name)?.get_handle(self.entity)?;

		Ok(self.server.server_tools()?.entity_by_handle(handle))
	}

	/// Sends the input `name` with `value`, with the entity as its activator
	/// and caller.
	pub(crate) fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), ObjectiveError> {
		self.input_from(name, value, self.entity, self.entity)
	}

	/// Sends the input `name` with `value`, from `activator` and `caller`.
	pub(crate) fn input_from(
		self,
		name: &CStr,
		value: InputValue<'_>,
		activator: Entity<'_>,
		caller: Entity<'_>,
	) -> Result<(), ObjectiveError> {
		let tools = self.server.server_tools()?;

		Ok(tools.accept_input(self.entity, name, value, activator, caller)?)
	}

	/// Reads an integer field of `class`'s own data description.
	pub(crate) fn int_field(
		self,
		class: &'static CStr,
		field: &'static CStr,
	) -> Result<c_int, ObjectiveError> {
		let offset = self.field_offset(
			class,
			field,
			&[sys::_fieldtypes_FIELD_INTEGER],
			size_of::<c_int>(),
			0,
		)?;

		// SAFETY: As for `bool_field`, for an `int` field.
		Ok(unsafe { self.entity.as_ptr().byte_add(offset).cast::<c_int>().read() })
	}

	/// The entity's key value `key`, as
	/// [`ServerTools::key_value`](crate::interfaces::ServerTools::key_value)
	/// reads it.
	pub(crate) fn key_value(self, key: &CStr) -> Result<Option<CString>, ObjectiveError> {
		self.check_live()?;

		Ok(self.server.server_tools()?.key_value(self.entity, key))
	}

	/// The networked variable `name` of the entity.
	pub(crate) fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, ObjectiveError> {
		self.check_live()?;

		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?)
	}

	/// The server.
	pub(crate) const fn server(self) -> Server<'s> {
		self.server
	}

	/// Sets the entity's key value `key`, as
	/// [`ServerTools::set_key_value`](crate::interfaces::ServerTools::set_key_value)
	/// does, or fails with [`ObjectiveError::KeyValueRejected`].
	pub(crate) fn set_key_value(
		self,
		key: &'static CStr,
		value: &CStr,
	) -> Result<(), ObjectiveError> {
		self.check_live()?;

		if self
			.server
			.server_tools()?
			.set_key_value(self.entity, key, value)
		{
			Ok(())
		} else {
			Err(ObjectiveError::KeyValueRejected { key })
		}
	}

	/// The text of the string key value `key`, which `class`'s own data
	/// description declares, or [`ObjectiveError::UnsupportedLayout`] if the
	/// entity has no such key.
	pub(crate) fn string_key(
		self,
		class: &'static CStr,
		key: &'static CStr,
	) -> Result<CString, ObjectiveError> {
		self.key_value(key)?
			.ok_or(ObjectiveError::UnsupportedLayout { class, field: key })
	}
}

/// Why an objective entity could not be read or controlled.
#[derive(Debug, thiserror::Error)]
pub enum ObjectiveError {
	/// The engine's globals, which hold the game time, are unavailable.
	#[error("the engine's globals are unavailable")]
	NoGlobals,

	/// An input was not sent, or the entity rejected it.
	#[error(transparent)]
	Input(#[from] InputError),

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The entity refused a key value, because it has no such key or the
	/// value is one [`ServerTools::set_key_value`] refuses.
	///
	/// [`ServerTools::set_key_value`]: crate::interfaces::ServerTools::set_key_value
	#[error("the entity refused the key value {key:?}")]
	KeyValueRejected {
		/// The key.
		key: &'static CStr,
	},

	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// A control point index lies outside the game's
	/// [`MAX_CONTROL_POINTS`](sdk_raw::inputs::MAX_CONTROL_POINTS).
	#[error("control point {index} is out of range")]
	PointOutOfRange {
		/// The index.
		index: usize,
	},

	/// A variable holds a value the SDK does not know for it, such as a
	/// state added by a game update.
	#[error("{name:?} holds {value}, which the SDK does not know")]
	UnknownValue {
		/// The variable.
		name: &'static CStr,

		/// Its value.
		value: c_int,
	},

	/// The class's data description lacks a field of the expected name and
	/// type at a usable offset.
	#[error("the data description of {class:?} lacks a usable {field:?}")]
	UnsupportedLayout {
		/// The class declaring the field.
		class: &'static CStr,

		/// The field.
		field: &'static CStr,
	},

	/// The entity is not of the expected class, or the server does not run
	/// TF2.
	#[error("the entity is not a TF2 {expected}")]
	WrongClass {
		/// The expected entity, by its class name in maps.
		expected: &'static str,
	},
}

/// The first entity of the class `class_name` in the entity list that `wrap`
/// accepts, or `None` if there is none. Entities marked for deletion, which
/// the wrappers neither read nor send inputs to, are skipped.
pub(crate) fn find_by_class<'s, T>(
	server: Server<'s>,
	class_name: &CStr,
	wrap: impl Fn(Server<'s>, Entity<'s>) -> Result<T, ObjectiveError>,
) -> Result<Option<T>, ObjectiveError> {
	let tools = server.server_tools()?;
	let mut found = tools.find_by_class_name(None, class_name);

	while let Some(entity) = found {
		if !entity.is_marked_for_deletion()
			&& let Ok(wrapped) = wrap(server, entity)
		{
			return Ok(Some(wrapped));
		}

		found = tools.find_by_class_name(Some(entity), class_name);
	}

	Ok(None)
}

/// The first entity named `name` in the entity list that `wrap` accepts, or
/// `None` if there is none, skipping those marked for deletion as
/// [`find_by_class`] does.
pub(crate) fn find_by_name<'s, T>(
	server: Server<'s>,
	name: &CStr,
	wrap: impl Fn(Server<'s>, Entity<'s>) -> Result<T, ObjectiveError>,
) -> Result<Option<T>, ObjectiveError> {
	let tools = server.server_tools()?;
	let mut found = tools.find_by_name(None, name);

	while let Some(entity) = found {
		if !entity.is_marked_for_deletion()
			&& let Ok(wrapped) = wrap(server, entity)
		{
			return Ok(Some(wrapped));
		}

		found = tools.find_by_name(Some(entity), name);
	}

	Ok(None)
}

/// Reads a networked boolean of `entity`, which the game declares as a `bool`
/// or as an `int` of one bit, such as the objective resource's
/// `m_bCPIsVisible`.
pub(crate) fn flag(prop: NetProp<'_>, entity: Entity<'_>) -> Result<bool, NetPropError> {
	match prop.storage() {
		Storage::I32 | Storage::U32 => Ok(prop.get::<i32>(entity)? != 0),
		_ => Ok(prop.get::<u8>(entity)? != 0),
	}
}
