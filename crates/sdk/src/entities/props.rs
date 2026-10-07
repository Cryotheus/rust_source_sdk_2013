//! Animated props: `prop_dynamic` and the classes that share its entity,
//! `CDynamicProp` (`game/server/props.cpp`), which show a model, play its
//! animations, and can be hidden and made to stop colliding.
//!
//! [`DynamicProp::spawn`] spawns one as [`DynamicPropSpawn`] describes,
//! through [`EntitySpawn`]. Its model must be precached already: a prop
//! precaches its model as it spawns, and a model the level has not precached
//! could overflow the engine's model table, which stops the server.
//! [`Server::precache_model`](crate::Server::precache_model) precaches one
//! while the level loads. [`EntitySpawn`] spawns props with other key values,
//! such as `modelscale` or `rendercolor`.
//!
//! # Unverified
//!
//! The wrappers follow Valve's Source SDK 2013 and have not been tested on a
//! live server.

#[cfg(test)]
#[path = "../tests/entities/props.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::spawn::{EntitySpawn, SpawnError};
use crate::inputs::{InputError, InputValue};
use crate::interfaces::{ModelInfo, ServerTools};
use crate::math::{QAngle, Vector};
use std::ffi::{CStr, CString, c_int};

/// The data map class of every animated prop.
const DYNAMIC_PROP_CLASS: &CStr = c"CDynamicProp";

/// The data map class of `prop_dynamic_ornament`.
const ORNAMENT_PROP_CLASS: &CStr = c"COrnamentProp";

/// Why an animated prop could not be spawned, wrapped or controlled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PropError {
	/// The entity is not of the class the wrapper or method takes.
	#[error("the entity is not a {class:?}")]
	WrongClass {
		/// The data map class it takes.
		class: &'static CStr,
	},

	/// The model given is not precached, or is a dynamic model.
	#[error("the model {model:?} is not precached")]
	ModelNotPrecached {
		/// The model.
		model: CString,
	},

	/// An input was not sent, or the prop rejected it.
	#[error(transparent)]
	Input(#[from] InputError),

	/// The game created no prop, the prop refused a key value and was
	/// removed, or it removed itself as it spawned.
	#[error(transparent)]
	Spawn(#[from] SpawnError),
}

/// An animated prop (`CDynamicProp`) within the current engine callback: a
/// `prop_dynamic`, `prop_dynamic_override` or `prop_dynamic_ornament`, or one
/// of the classes deriving from them, such as doors.
///
/// Once spawned, `prop_dynamic_override` is renamed `prop_dynamic`.
#[doc(alias("prop_dynamic", "CDynamicProp"))]
#[derive(Debug, Clone, Copy)]
pub struct DynamicProp<'s> {
	tools: ServerTools<'s>,
	entity: Entity<'s>,
}

impl<'s> DynamicProp<'s> {
	/// Wraps `entity`. Fails with [`PropError::WrongClass`] unless its data
	/// maps include `CDynamicProp`'s.
	pub fn new(tools: ServerTools<'s>, entity: Entity<'s>) -> Result<Self, PropError> {
		if entity.is_a(DYNAMIC_PROP_CLASS) {
			Ok(Self { tools, entity })
		} else {
			Err(PropError::WrongClass {
				class: DYNAMIC_PROP_CLASS,
			})
		}
	}

	/// Spawns and activates a prop as `spawn` describes.
	///
	/// Fails, before creating it, with [`PropError::ModelNotPrecached`] for a
	/// model that is not precached, and with [`SpawnError::NonFinite`] for a
	/// position or angle that is not finite. Fails with
	/// [`SpawnError::RemovedItself`] if the prop removed itself as it spawned,
	/// as a `prop_dynamic` does with a model meant for a physics prop.
	pub fn spawn(
		tools: ServerTools<'s>,
		models: ModelInfo<'_>,
		spawn: &DynamicPropSpawn<'_>,
	) -> Result<Self, PropError> {
		if models.model_index(spawn.model).is_none() {
			return Err(PropError::ModelNotPrecached {
				model: spawn.model.to_owned(),
			});
		}

		let mut prop = EntitySpawn::new(spawn.class.class_name())
			.model(spawn.model)
			.origin(spawn.origin)
			.angles(spawn.angles)
			.key(c"solid", &number_value(spawn.solid.to_raw()))
			.key(c"skin", &number_value(spawn.skin))
			.key(c"StartDisabled", flag_value(spawn.start_disabled));

		if let Some(animation) = spawn.animation {
			prop = prop.key(c"DefaultAnim", animation);
		}

		if let Some(name) = spawn.name {
			prop = prop.name(name);
		}

		// SAFETY: `CDynamicProp`'s constructor only sets its members. Its
		// `Spawn` precaches its model, which the check above found precached,
		// and two sounds props share; sets its model; removes only itself,
		// through deferred deletion, for a model it refuses; plays its default
		// animation, whose `OnAnimationBegun` output fires through the event
		// queue; and makes its physics shadow and bone followers. Its
		// `Activate` only warns of a health on a prop that takes no damage,
		// finds its lighting origins by name, and, for an ornament, finds the
		// entity it follows by name.
		let entity = unsafe { prop.spawn(tools) }?;

		Ok(Self { tools, entity })
	}

	/// Attaches an ornament to `target`, which it then follows, merged with
	/// its bones, and shows it (`SetAttached`).
	///
	/// Fails with [`PropError::WrongClass`] unless the prop is a
	/// `prop_dynamic_ornament`.
	#[doc(alias("SetAttached"))]
	pub fn attach(self, target: Entity<'_>) -> Result<(), PropError> {
		self.check_ornament()?;

		// `SetAttached` finds its target by name, which `!activator` names.
		Ok(self.tools.accept_input(
			self.entity,
			c"SetAttached",
			InputValue::String(c"!activator"),
			target,
			self.entity,
		)?)
	}

	/// Fails with [`PropError::WrongClass`] unless the prop is a
	/// `prop_dynamic_ornament`.
	fn check_ornament(self) -> Result<(), PropError> {
		if self.entity.is_a(ORNAMENT_PROP_CLASS) {
			Ok(())
		} else {
			Err(PropError::WrongClass {
				class: ORNAMENT_PROP_CLASS,
			})
		}
	}

	/// Detaches an ornament from what it follows, and hides it and makes it
	/// not solid, as it was before it was attached (`Detach`).
	///
	/// Fails with [`PropError::WrongClass`] unless the prop is a
	/// `prop_dynamic_ornament`.
	#[doc(alias("Detach"))]
	pub fn detach(self) -> Result<(), PropError> {
		self.check_ornament()?;
		self.input(c"Detach", InputValue::Void)
	}

	/// Makes the prop not solid, without changing how it collides otherwise
	/// (`DisableCollision`).
	#[doc(alias("DisableCollision"))]
	pub fn disable_collision(self) -> Result<(), PropError> {
		self.input(c"DisableCollision", InputValue::Void)
	}

	/// Makes the prop solid again, after [`Self::disable_collision`]
	/// (`EnableCollision`).
	#[doc(alias("EnableCollision"))]
	pub fn enable_collision(self) -> Result<(), PropError> {
		self.input(c"EnableCollision", InputValue::Void)
	}

	/// The prop's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Hides the prop, which keeps colliding (`TurnOff`).
	#[doc(alias("TurnOff", "Disable"))]
	pub fn hide(self) -> Result<(), PropError> {
		self.input(c"TurnOff", InputValue::Void)
	}

	/// Sends the prop an input, with itself as the activator and caller.
	fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), PropError> {
		Ok(self
			.tools
			.accept_input(self.entity, name, value, self.entity, self.entity)?)
	}

	/// Plays the sequence or activity named `animation` once it reaches it
	/// from the one playing, then returns to the default animation, if the
	/// prop has one (`SetAnimation`). The prop plays its first sequence
	/// instead if its model has none of that name.
	#[doc(alias("SetAnimation"))]
	pub fn set_animation(self, animation: &CStr) -> Result<(), PropError> {
		self.input(c"SetAnimation", InputValue::String(animation))
	}

	/// Sets the animation the prop returns to once one it plays ends
	/// (`SetDefaultAnimation`). It does not start it.
	#[doc(alias("SetDefaultAnimation", "DefaultAnim"))]
	pub fn set_default_animation(self, animation: &CStr) -> Result<(), PropError> {
		self.input(c"SetDefaultAnimation", InputValue::String(animation))
	}

	/// Shows the prop again, after [`Self::hide`] or spawning it disabled
	/// (`TurnOn`).
	#[doc(alias("TurnOn", "Enable"))]
	pub fn show(self) -> Result<(), PropError> {
		self.input(c"TurnOn", InputValue::Void)
	}
}

/// Which class [`DynamicProp::spawn`] spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DynamicPropClass {
	/// `prop_dynamic`, which removes itself as it spawns if its model's
	/// `prop_data` makes it a physics prop's, or is invalid.
	#[doc(alias("prop_dynamic"))]
	#[default]
	Dynamic,

	/// `prop_dynamic_override`, which takes any model.
	#[doc(alias("prop_dynamic_override"))]
	Override,

	/// `prop_dynamic_ornament`, which spawns hidden and not solid, until
	/// [`DynamicProp::attach`] attaches it to an entity it then follows.
	#[doc(alias("prop_dynamic_ornament"))]
	Ornament,
}

impl DynamicPropClass {
	/// The class's name, which creates it.
	pub const fn class_name(self) -> &'static CStr {
		match self {
			Self::Dynamic => c"prop_dynamic",
			Self::Override => c"prop_dynamic_override",
			Self::Ornament => c"prop_dynamic_ornament",
		}
	}
}

/// What [`DynamicProp::spawn`] spawns: an animated prop's key values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicPropSpawn<'a> {
	/// Its class.
	pub class: DynamicPropClass,

	/// Its model, such as `models/props_gameplay/resupply_locker.mdl`, which
	/// must be precached already (`model`).
	pub model: &'a CStr,

	/// Where it is (`origin`).
	pub origin: Vector,

	/// Which way it faces (`angles`).
	pub angles: QAngle,

	/// How it collides (`solid`).
	pub solid: PropSolid,

	/// The sequence or activity it plays from the start, and returns to once
	/// another ends, or `None` to stand still (`DefaultAnim`).
	pub animation: Option<&'a CStr>,

	/// Its model's skin (`skin`).
	pub skin: c_int,

	/// Whether it starts hidden, until [`DynamicProp::show`]
	/// (`StartDisabled`).
	pub start_disabled: bool,

	/// Its name, by which inputs and outputs find it, or `None` for none
	/// (`targetname`).
	pub name: Option<&'a CStr>,
}

impl<'a> DynamicPropSpawn<'a> {
	/// A `prop_dynamic` showing `model` at `origin`, facing along the X axis,
	/// colliding with its model's collision mesh, standing still in its
	/// first skin, shown, and unnamed.
	pub const fn new(model: &'a CStr, origin: Vector) -> Self {
		Self {
			class: DynamicPropClass::Dynamic,
			model,
			origin,
			angles: QAngle {
				pitch: 0.0,
				yaw: 0.0,
				roll: 0.0,
			},
			solid: PropSolid::Vphysics,
			animation: None,
			skin: 0,
			start_disabled: false,
			name: None,
		}
	}
}

/// How a prop collides (`SolidType_t`), as the prop's `solid` key value
/// gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PropSolid {
	/// Not at all (`SOLID_NONE`). A [`DynamicPropClass::Dynamic`] prop then
	/// takes an oriented box that is not solid, which still bounds it as it
	/// turns.
	#[doc(alias("SOLID_NONE"))]
	None,

	/// By its model's bounding box, aligned with the world's axes
	/// (`SOLID_BBOX`).
	#[doc(alias("SOLID_BBOX"))]
	BoundingBox,

	/// By its model's collision mesh (`SOLID_VPHYSICS`), as maps' props
	/// default to. A model without one does not collide.
	#[doc(alias("SOLID_VPHYSICS"))]
	#[default]
	Vphysics,
}

impl PropSolid {
	/// The `SolidType_t` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::None => sys::SolidType_t_SOLID_NONE as c_int,
			Self::BoundingBox => sys::SolidType_t_SOLID_BBOX as c_int,
			Self::Vphysics => sys::SolidType_t_SOLID_VPHYSICS as c_int,
		}
	}
}

/// `1` or `0`, as boolean key values are given.
fn flag_value(flag: bool) -> &'static CStr {
	if flag { c"1" } else { c"0" }
}

/// A number as a key value, in decimal.
fn number_value(number: c_int) -> CString {
	CString::new(number.to_string()).expect("numbers contain no NUL")
}
