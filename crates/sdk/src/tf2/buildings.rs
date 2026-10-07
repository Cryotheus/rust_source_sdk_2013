//! TF2's buildings: the sentry guns, dispensers and teleporters engineers
//! build, the sappers spies place on them, and the dispensers payload carts
//! and game modes place, all of whose classes derive from `CBaseObject`.
//!
//! [`Building`] reads what every building networks, such as its builder, its
//! level and whether it is sapped, and acts on it: it detonates or removes
//! it, and sends it the inputs maps send. [`Sentry`], [`Dispenser`] and
//! [`Teleporter`] read what their kinds add. [`PlayerBuildings`] finds a
//! player's buildings and the one an engineer carries, and removes them.
//!
//! A building's health is an entity's: [`Entity::health`] reads it, and
//! [`ServerTools::send_health_input`] sends the `SetHealth`, `AddHealth` and
//! `RemoveHealth` inputs buildings declare.
//!
//! [`sdk_raw::tf2::buildings`] holds TF2's numbers for buildings. Where
//! buildings may be placed is [`objects`](super::objects)'. [`BuildingClass`]
//! names the buildings' C++ classes, whose vtables [`building_vtables`] finds
//! for `metamod_source`'s building hooks.
//!
//! # Blueprints
//!
//! While an engineer or spy places a building, it exists as a blueprint,
//! which the builder's weapon keeps track of: [`Building::is_placing`] is
//! true for it. Blueprints are not detonated or removed, which the weapon
//! does itself as the builder puts it away.
//!
//! [`Entity::health`]: crate::entities::Entity::health
//! [`ServerTools::send_health_input`]: crate::interfaces::ServerTools::send_health_input

#[cfg(test)]
#[path = "../tests/tf2/buildings.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError, NetVar};
use crate::entities::Entity;
use crate::inputs::{InputError, InputValue};
use crate::tf2::objects::ObjectKind;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::buildings as raw;
use sdk_raw::tf2::script_binding::{BOOL, boolean};
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};

/// The classes of buildings, as patterns
/// [`ServerTools::find_by_class_name`](crate::interfaces::ServerTools::find_by_class_name)
/// matches: those engineers and spies build start with `obj_`, and the others
/// are payload carts', Player Destruction's and Robot Destruction's
/// dispensers.
const CLASSES: [&CStr; 4] = [
	c"obj_*",
	c"mapobj_cart_dispenser",
	c"pd_dispenser",
	c"rd_robot_dispenser",
];

/// The class of sappers, and of their blueprints.
const SAPPER_CLASS: &CStr = c"obj_attachment_sapper";

/// Why a building operation failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildingError {
	/// A VScript method could not be called.
	#[error(transparent)]
	Binding(#[from] BindingError),

	/// The building is a blueprint its builder is placing.
	#[error("the building is a blueprint its builder is placing")]
	Blueprint,

	/// The building is being destroyed: the game is running its death, which
	/// removes it.
	#[error("the building is being destroyed")]
	Dying,

	/// An input could not be sent.
	#[error(transparent)]
	Input(#[from] InputError),

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The building, or player, is already marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// A networked variable could not be read.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The entity is not a TF2 building.
	#[error("the entity is not a TF2 building")]
	NotBuilding,

	/// The building is not of the kind the view requires.
	#[error("the building is not a {0:?}")]
	NotKind(BuildingKind),

	/// The entity is not a TF2 player.
	#[error("the entity is not a TF2 player")]
	NotTfPlayer,

	/// The building's `m_iObjectType` is not one of TF2's building types.
	#[error("the building's type {0} is not one of TF2's")]
	UnknownKind(c_int),

	/// The building's `m_iState` is not one of its kind's states.
	#[error("the building's state {0} is not one of its kind's")]
	UnknownState(c_int),

	/// The server does not run Team Fortress 2.
	#[error("building operations require Team Fortress 2")]
	WrongGame,
}

/// What a building is (`m_iObjectType`), as TF2 numbers them
/// (`ObjectType_t`).
#[doc(alias("ObjectType_t", "GetType", "m_iObjectType"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildingKind {
	/// A dispenser: an engineer's, or one a payload cart or game mode places.
	#[doc(alias("OBJ_DISPENSER", "CObjectDispenser", "obj_dispenser"))]
	Dispenser,

	/// A spy's sapper, on a building or, in Mann vs. Machine, a robot.
	#[doc(alias("OBJ_ATTACHMENT_SAPPER", "CObjectSapper", "obj_attachment_sapper"))]
	Sapper,

	/// A sentry gun, mini-sentries included.
	#[doc(alias("OBJ_SENTRYGUN", "CObjectSentrygun", "obj_sentrygun"))]
	Sentry,

	/// A teleporter entrance or exit.
	#[doc(alias("OBJ_TELEPORTER", "CObjectTeleporter", "obj_teleporter"))]
	Teleporter,
}

impl BuildingKind {
	/// Every kind, in the order TF2 numbers them.
	pub const ALL: [Self; 4] = [
		Self::Dispenser,
		Self::Teleporter,
		Self::Sentry,
		Self::Sapper,
	];

	/// The kind TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::OBJ_DISPENSER => Self::Dispenser,
			raw::OBJ_TELEPORTER => Self::Teleporter,
			raw::OBJ_SENTRYGUN => Self::Sentry,
			raw::OBJ_ATTACHMENT_SAPPER => Self::Sapper,
			_ => return None,
		})
	}

	/// The kind as one of the buildings engineers place from a blueprint, or
	/// `None` for a sapper.
	pub const fn object_kind(self) -> Option<ObjectKind> {
		match self {
			Self::Dispenser => Some(ObjectKind::Dispenser),
			Self::Sentry => Some(ObjectKind::Sentry),
			Self::Teleporter => Some(ObjectKind::Teleporter),
			Self::Sapper => None,
		}
	}

	/// TF2's number for the kind.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Dispenser => raw::OBJ_DISPENSER,
			Self::Teleporter => raw::OBJ_TELEPORTER,
			Self::Sentry => raw::OBJ_SENTRYGUN,
			Self::Sapper => raw::OBJ_ATTACHMENT_SAPPER,
		}
	}
}

impl From<ObjectKind> for BuildingKind {
	fn from(kind: ObjectKind) -> Self {
		match kind {
			ObjectKind::Dispenser => Self::Dispenser,
			ObjectKind::Sentry => Self::Sentry,
			ObjectKind::Teleporter => Self::Teleporter,
		}
	}
}

bitflags::bitflags! {
	/// A building's object flags (`m_fObjectFlags`, `OF_*`). Unknown bits are
	/// preserved.
	#[doc(alias("m_fObjectFlags", "GetObjectFlags"))]
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct ObjectFlags: c_int {
		/// Lets the builder place another of the building from the same
		/// blueprint, as spies place sappers.
		#[doc(alias("OF_ALLOW_REPEAT_PLACEMENT"))]
		const ALLOW_REPEAT_PLACEMENT = raw::OF_ALLOW_REPEAT_PLACEMENT;

		/// The building is placed on another, as sappers are.
		#[doc(alias("OF_MUST_BE_BUILT_ON_ATTACHMENT"))]
		const BUILT_ON_ATTACHMENT = raw::OF_MUST_BE_BUILT_ON_ATTACHMENT;

		/// The building has no model, as the dispensers Player Destruction and
		/// Robot Destruction attach to players and robots, and sentries do not
		/// target it.
		#[doc(alias("OF_DOESNT_HAVE_A_MODEL"))]
		const NO_MODEL = raw::OF_DOESNT_HAVE_A_MODEL;

		/// One of the dispensers Player Destruction attaches to its team
		/// leaders.
		#[doc(alias("OF_PLAYER_DESTRUCTION"))]
		const PLAYER_DESTRUCTION = raw::OF_PLAYER_DESTRUCTION;

		const _ = !0;
	}
}

use sdk_raw::tf2::objects::ObjectVtables;
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// A TF2 building, scoped to one engine callback.
#[doc(alias("CBaseObject"))]
#[derive(Debug, Clone, Copy)]
pub struct Building<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> Building<'s> {
	/// Wraps a building, or fails with [`BuildingError::WrongGame`] unless the
	/// server runs TF2, and [`BuildingError::NotBuilding`] unless `entity`'s
	/// datamaps include `CBaseObject`.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BuildingError> {
		if server.game() != Game::TeamFortress2 {
			return Err(BuildingError::WrongGame);
		}

		if !entity.has_data_map_class(c"CBaseObject") {
			return Err(BuildingError::NotBuilding);
		}

		Ok(Self { server, entity })
	}

	/// Every building on the server, sappers and blueprints included, but not
	/// those already marked for deletion, which the game has stopped keeping
	/// track of.
	pub fn all(server: Server<'s>) -> Result<impl Iterator<Item = Self>, BuildingError> {
		if server.game() != Game::TeamFortress2 {
			return Err(BuildingError::WrongGame);
		}

		let tools = server.server_tools()?;

		Ok(CLASSES
			.into_iter()
			.flat_map(move |class| {
				std::iter::successors(tools.find_by_class_name(None, class), move |&entity| {
					tools.find_by_class_name(Some(entity), class)
				})
			})
			.filter(|entity| !entity.is_marked_for_deletion())
			.filter_map(move |entity| Self::new(server, entity).ok()))
	}

	/// Reads one of the building's `bool` networked variables.
	fn bool(self, name: &CStr) -> Result<bool, BuildingError> {
		self.get(name)
	}

	/// The player who built the building (`m_hBuilder`), or `None` for a
	/// building without one, such as one a map placed, or if they no longer
	/// exist.
	#[doc(alias("m_hBuilder", "GetBuilder"))]
	pub fn builder(self) -> Result<Option<Entity<'s>>, BuildingError> {
		self.handle(c"m_hBuilder")
	}

	/// The building a sapper is placed on (`m_hBuiltOnEntity`), or `None` for
	/// a building on none, or if it no longer exists.
	#[doc(alias("m_hBuiltOnEntity", "GetParentObject"))]
	pub fn built_on(self) -> Result<Option<Entity<'s>>, BuildingError> {
		self.handle(c"m_hBuiltOnEntity")
	}

	/// Fails unless the building may be detonated or removed: one that is
	/// neither marked for deletion, dying, nor a blueprint.
	fn check_removable(self) -> Result<(), BuildingError> {
		if self.entity.is_marked_for_deletion() {
			return Err(BuildingError::MarkedForDeletion);
		}

		if self.is_dying() {
			return Err(BuildingError::Dying);
		}

		if self.is_placing()? {
			return Err(BuildingError::Blueprint);
		}

		Ok(())
	}

	/// How much of the building is built (`m_flPercentageConstructed`), from 0
	/// to 1.
	#[doc(alias("m_flPercentageConstructed", "GetPercentageConstructed"))]
	pub fn construction_progress(self) -> Result<f32, BuildingError> {
		self.get(c"m_flPercentageConstructed")
	}

	/// Removes the building silently, as its builder's leaving the game does
	/// (`CBaseObject::DestroyObject`): without exploding or gibs, an
	/// `object_detonated` or `object_destroyed` event, or its `OnDestroyed`
	/// output. An engineer carrying it puts it away, and switches to their
	/// last weapon.
	///
	/// Fails with [`BuildingError::MarkedForDeletion`],
	/// [`BuildingError::Dying`] or [`BuildingError::Blueprint`] for a
	/// building being removed already, or that is a blueprint.
	#[doc(alias("DestroyObject"))]
	pub fn destroy(self) -> Result<(), BuildingError> {
		self.check_removable()?;

		let building = self.entity.as_ptr();

		// SAFETY: `new` found `CBaseObject` in the building's datamaps, so it is
		// one, whose entity base `sdk_raw::entities::health` asserts is at offset
		// zero. The building is live during `'s`, on the main thread, and neither
		// marked for deletion nor dying. `DestroyObject` removes it through the
		// engine's deferred deletion, as it does the screens it shows.
		unsafe {
			vcall!(building as sys::CBaseObject__bindgen_vtable => CBaseObject_DestroyObject())
		};

		Ok(())
	}

	/// Destroys the building as its builder's PDA does
	/// (`CBaseObject::DetonateObject`): it explodes into gibs, which drop
	/// metal, fires its `OnDestroyed` output and an `object_detonated` event,
	/// and is removed. An engineer carrying it loses it.
	///
	/// Fails as [`Self::destroy`] does.
	#[doc(alias("DetonateObject"))]
	pub fn detonate(self) -> Result<(), BuildingError> {
		self.check_removable()?;

		let building = self.entity.as_ptr();

		// SAFETY: As for `destroy`. `DetonateObject` kills the building, as the
		// game does, which runs map outputs and other plugins' hooks, spawns
		// gibs, and removes it through the engine's deferred deletion.
		unsafe {
			vcall!(building as sys::CBaseObject__bindgen_vtable => CBaseObject_DetonateObject())
		};

		Ok(())
	}

	/// Disables the building, as its `Disable` input does: it stops working
	/// as a sapper makes it, until the game updates its disabled state again,
	/// as it does when a sapper on it is removed, or its `Enable` input.
	#[doc(alias("Disable", "InputDisable"))]
	pub fn disable(self) -> Result<(), BuildingError> {
		self.input(c"Disable", InputValue::Void)
	}

	/// Enables a disabled building, as its `Enable` input does, unless it is
	/// sapped, or disabled by the Cow Mangler, or its team lost the round.
	#[doc(alias("Enable", "InputEnable"))]
	pub fn enable(self) -> Result<(), BuildingError> {
		self.input(c"Enable", InputValue::Void)
	}

	/// The building.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The building's object flags (`m_fObjectFlags`).
	#[doc(alias("m_fObjectFlags", "GetObjectFlags"))]
	pub fn flags(self) -> Result<ObjectFlags, BuildingError> {
		Ok(ObjectFlags::from_bits_retain(self.int(c"m_fObjectFlags")?))
	}

	/// Reads one of the building's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, BuildingError> {
		Ok(self.net_prop(name)?.get(self.entity)?)
	}

	/// Resolves one of the building's networked entity handles.
	fn handle(self, name: &CStr) -> Result<Option<Entity<'s>>, BuildingError> {
		let handle = self.net_prop(name)?.get_handle(self.entity)?;

		Ok(self.server.server_tools()?.entity_by_handle(handle))
	}

	/// Hides the building and disables it, as its `Hide` input does, until
	/// its `Show` input.
	#[doc(alias("Hide", "InputHide"))]
	pub fn hide(self) -> Result<(), BuildingError> {
		self.input(c"Hide", InputValue::Void)
	}

	/// The highest level the building reached (`m_iHighestUpgradeLevel`),
	/// which a building redeployed after being carried builds up to.
	#[doc(alias("m_iHighestUpgradeLevel", "GetHighestUpgradeLevel"))]
	pub fn highest_level(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iHighestUpgradeLevel")
	}

	/// Sends one of `CBaseObject`'s inputs, with the building as its activator
	/// and caller, which none of them reads.
	fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), BuildingError> {
		Ok(self.server.server_tools()?.accept_input(
			self.entity,
			name,
			value,
			self.entity,
			self.entity,
		)?)
	}

	/// Reads one of the building's `int` networked variables.
	fn int(self, name: &CStr) -> Result<c_int, BuildingError> {
		self.get(name)
	}

	/// Whether an engineer carries the building (`m_bCarried`).
	#[doc(alias("m_bCarried", "IsCarried"))]
	pub fn is_carried(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bCarried")
	}

	/// Whether the building is being built (`m_bBuilding`), as a blueprint
	/// placed, and a carried building redeployed, are.
	#[doc(alias("m_bBuilding", "IsBuilding"))]
	pub fn is_constructing(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bBuilding")
	}

	/// Whether the building is disabled (`m_bDisabled`), as sapping, the Cow
	/// Mangler, its `Disable` input, and the end of a round its team lost
	/// make it.
	#[doc(alias("m_bDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bDisabled")
	}

	/// Whether the building is one of the disposable sentries Mann vs.
	/// Machine's upgrades let engineers build (`m_bDisposableBuilding`).
	#[doc(alias("m_bDisposableBuilding", "IsDisposableBuilding"))]
	pub fn is_disposable(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bDisposableBuilding")
	}

	/// Whether the game is running the building's death
	/// (`CBaseObject::IsDying`), which removes it.
	#[doc(alias("IsDying", "m_bDying"))]
	pub fn is_dying(self) -> bool {
		let building = self.entity.as_ptr();

		// SAFETY: As for `destroy`. `IsDying` only reads the building.
		unsafe { vcall!(building as sys::CBaseObject__bindgen_vtable => CBaseObject_IsDying()) }
	}

	/// Whether the building is a mini-building (`m_bMiniBuilding`), as the
	/// Gunslinger's sentries are.
	#[doc(alias("m_bMiniBuilding", "IsMiniBuilding"))]
	pub fn is_mini(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bMiniBuilding")
	}

	/// Whether the building is a blueprint its builder is placing
	/// (`m_bPlacing`).
	#[doc(alias("m_bPlacing", "IsPlacing"))]
	pub fn is_placing(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bPlacing")
	}

	/// Whether the Cow Mangler's charged shot disabled the building
	/// (`m_bPlasmaDisable`), for a few seconds.
	#[doc(alias("m_bPlasmaDisable", "IsPlasmaDisabled"))]
	pub fn is_plasma_disabled(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bPlasmaDisable")
	}

	/// Whether an engineer is redeploying the building after carrying it
	/// (`m_bCarryDeploy`), which builds it faster.
	#[doc(alias("m_bCarryDeploy", "IsRedeploying"))]
	pub fn is_redeploying(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bCarryDeploy")
	}

	/// Whether a sapper is on the building (`m_bHasSapper`).
	#[doc(alias("m_bHasSapper", "HasSapper"))]
	pub fn is_sapped(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bHasSapper")
	}

	/// What the building is (`m_iObjectType`).
	#[doc(alias("m_iObjectType", "GetType", "ObjectType"))]
	pub fn kind(self) -> Result<BuildingKind, BuildingError> {
		let raw = self.int(c"m_iObjectType")?;

		BuildingKind::from_raw(raw).ok_or(BuildingError::UnknownKind(raw))
	}

	/// The building's level (`m_iUpgradeLevel`), from 1.
	#[doc(alias("m_iUpgradeLevel", "GetUpgradeLevel"))]
	pub fn level(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iUpgradeLevel")
	}

	/// The highest level the building may be upgraded to
	/// (`CBaseObject::GetMaxUpgradeLevel`): 1 for mini-sentries and
	/// disposable sentries, and 3 for other buildings.
	#[doc(alias("GetMaxUpgradeLevel"))]
	pub fn max_level(self) -> c_int {
		let building = self.entity.as_ptr();

		// SAFETY: As for `destroy`. `GetMaxUpgradeLevel` only reads the building.
		unsafe {
			vcall!(building as sys::CBaseObject__bindgen_vtable => CBaseObject_GetMaxUpgradeLevel())
		}
	}

	/// The building's mode (`m_iObjectMode`), which tells apart the variants
	/// of some kinds, as TF2 numbers them: `MODE_TELEPORTER_*`,
	/// `MODE_SENTRYGUN_*` or `MODE_SAPPER_*` in [`sdk_raw::tf2::buildings`].
	/// [`Teleporter::end`] reads a teleporter's.
	#[doc(alias("m_iObjectMode", "GetObjectMode"))]
	pub fn mode(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iObjectMode")
	}

	/// Resolves one of the building's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, BuildingError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?)
	}

	/// The sapper on the building, or `None` if none is. A sapper's blueprint
	/// is on no building, and a sapper marked for deletion is skipped.
	#[doc(alias("GetSapper", "GetObjectOfTypeOnMe"))]
	pub fn sapper(self) -> Result<Option<Self>, BuildingError> {
		let tools = self.server.server_tools()?;
		let handle = self.entity.handle();
		let mut found = tools.find_by_class_name(None, SAPPER_CLASS);

		while let Some(entity) = found {
			if !entity.is_marked_for_deletion()
				&& let Ok(sapper) = Self::new(self.server, entity)
				&& sapper.net_prop(c"m_hBuiltOnEntity")?.get_handle(entity)? == handle
			{
				return Ok(Some(sapper));
			}

			found = tools.find_by_class_name(Some(entity), SAPPER_CLASS);
		}

		Ok(None)
	}

	/// Sets how solid the building is to players, as its `SetSolidToPlayer`
	/// input does, which moves it into the collision group that makes it so.
	#[doc(alias("SetSolidToPlayer", "InputSetSolidToPlayer", "SetSolidToPlayers"))]
	pub fn set_solid_to_players(self, solid: SolidToPlayers) -> Result<(), BuildingError> {
		self.input(c"SetSolidToPlayer", InputValue::Int(solid.to_raw()))
	}

	/// Shows a hidden building, as its `Show` input does, and enables it as
	/// [`Self::enable`] does.
	#[doc(alias("Show", "InputShow"))]
	pub fn show(self) -> Result<(), BuildingError> {
		self.input(c"Show", InputValue::Void)
	}

	/// The building's team (`m_iTeamNum`), which is its builder's.
	#[doc(alias("m_iTeamNum", "GetTeamNumber"))]
	pub fn team(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iTeamNum")
	}

	/// The metal put into the building's next level (`m_iUpgradeMetal`).
	#[doc(alias("m_iUpgradeMetal"))]
	pub fn upgrade_metal(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iUpgradeMetal")
	}

	/// The metal the building's next level takes (`m_iUpgradeMetalRequired`).
	#[doc(alias("m_iUpgradeMetalRequired", "GetUpgradeMetalRequired"))]
	pub fn upgrade_metal_required(self) -> Result<c_int, BuildingError> {
		self.int(c"m_iUpgradeMetalRequired")
	}

	/// Whether the map placed the building (`m_bWasMapPlaced`), rather than
	/// a player.
	#[doc(alias("m_bWasMapPlaced"))]
	pub fn was_map_placed(self) -> Result<bool, BuildingError> {
		self.bool(c"m_bWasMapPlaced")
	}
}

/// A C++ class of TF2's buildings. Each has a vtable of its own, which hooks
/// of the buildings' methods patch: [`building_vtables`] finds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildingClass {
	/// A payload cart's dispenser.
	#[doc(alias("CObjectCartDispenser", "mapobj_cart_dispenser"))]
	CartDispenser,

	/// An engineer's dispenser.
	#[doc(alias("CObjectDispenser", "obj_dispenser"))]
	Dispenser,

	/// The dispenser Player Destruction's team leaders carry.
	#[doc(alias("CPlayerDestructionDispenser", "pd_dispenser"))]
	PlayerDestructionDispenser,

	/// The dispenser of a Robot Destruction robot.
	#[doc(alias("CRobotDispenser", "rd_robot_dispenser"))]
	RobotDispenser,

	/// A spy's sapper.
	#[doc(alias("CObjectSapper", "obj_attachment_sapper"))]
	Sapper,

	/// A sentry gun, mini-sentries and disposable sentries included.
	#[doc(alias("CObjectSentrygun", "obj_sentrygun"))]
	Sentry,

	/// A teleporter entrance or exit.
	#[doc(alias("CObjectTeleporter", "obj_teleporter"))]
	Teleporter,
}

impl BuildingClass {
	/// Every building class, in the order [`BuildingVtables::all`] gives their
	/// vtables.
	pub const ALL: [Self; 7] = [
		Self::CartDispenser,
		Self::Dispenser,
		Self::PlayerDestructionDispenser,
		Self::RobotDispenser,
		Self::Sapper,
		Self::Sentry,
		Self::Teleporter,
	];

	/// The kind of building the class's are.
	pub const fn kind(self) -> BuildingKind {
		match self {
			Self::CartDispenser
			| Self::Dispenser
			| Self::PlayerDestructionDispenser
			| Self::RobotDispenser => BuildingKind::Dispenser,

			Self::Sapper => BuildingKind::Sapper,
			Self::Sentry => BuildingKind::Sentry,
			Self::Teleporter => BuildingKind::Teleporter,
		}
	}

	/// The class's undecorated C++ name, by which its run-time type
	/// information is found.
	pub const fn name(self) -> &'static str {
		match self {
			Self::CartDispenser => "CObjectCartDispenser",
			Self::Dispenser => "CObjectDispenser",
			Self::PlayerDestructionDispenser => "CPlayerDestructionDispenser",
			Self::RobotDispenser => "CRobotDispenser",
			Self::Sapper => "CObjectSapper",
			Self::Sentry => "CObjectSentrygun",
			Self::Teleporter => "CObjectTeleporter",
		}
	}
}

/// Why the building classes' vtables could not all be found.
#[derive(Debug, thiserror::Error)]
pub enum BuildingVtableError {
	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The class has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2's building class `{}`", .0.name())]
	NotFound(BuildingClass),

	/// The server does not run Team Fortress 2.
	#[error("building hooks require Team Fortress 2")]
	WrongGame,
}

impl From<util::Error> for BuildingVtableError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// The primary vtables of every building class in this server's game module.
/// Intended for the Metamod adapter; no building is retained.
#[derive(Debug, Clone, Copy)]
pub struct BuildingVtables<'s> {
	vtables: [NonNull<*mut c_void>; 7],
	_scope: PhantomData<&'s Server<'s>>,
}

impl BuildingVtables<'_> {
	/// The vtables of the classes of [`BuildingClass::ALL`], in its order.
	pub const fn all(self) -> [NonNull<*mut c_void>; 7] {
		self.vtables
	}

	/// The vtable of `class`.
	pub const fn get(self, class: BuildingClass) -> NonNull<*mut c_void> {
		self.vtables[class as usize]
	}
}

/// A dispenser: an engineer's, or one a payload cart, Player Destruction or
/// Robot Destruction places.
#[doc(alias("CObjectDispenser"))]
#[derive(Debug, Clone, Copy)]
pub struct Dispenser<'s>(Building<'s>);

impl<'s> Dispenser<'s> {
	/// Wraps a dispenser, or fails with [`BuildingError::NotKind`] unless
	/// `building`'s datamaps include `CObjectDispenser`.
	pub fn new(building: Building<'s>) -> Result<Self, BuildingError> {
		if !building.entity.has_data_map_class(c"CObjectDispenser") {
			return Err(BuildingError::NotKind(BuildingKind::Dispenser));
		}

		Ok(Self(building))
	}

	/// The dispenser as a building.
	pub const fn building(self) -> Building<'s> {
		self.0
	}

	/// Whether a payload cart carries the dispenser (`mapobj_cart_dispenser`).
	#[doc(alias("CObjectCartDispenser", "mapobj_cart_dispenser"))]
	pub fn is_cart(self) -> bool {
		self.0.entity.has_data_map_class(c"CObjectCartDispenser")
	}

	/// The metal the dispenser holds (`m_iAmmoMetal`), which it gives players.
	#[doc(alias("m_iAmmoMetal"))]
	pub fn metal(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iAmmoMetal")
	}

	/// What the dispenser is doing (`m_iState`).
	#[doc(alias("m_iState"))]
	pub fn state(self) -> Result<DispenserState, BuildingError> {
		let raw = self.0.int(c"m_iState")?;

		DispenserState::from_raw(raw).ok_or(BuildingError::UnknownState(raw))
	}
}

/// What a dispenser is doing (`m_iState`), as TF2 numbers it
/// (`DISPENSER_STATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispenserState {
	/// Built, or being built.
	#[doc(alias("DISPENSER_STATE_IDLE"))]
	Idle,

	/// Playing its upgrade animation.
	#[doc(alias("DISPENSER_STATE_UPGRADING"))]
	Upgrading,
}

impl DispenserState {
	/// The state TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::DISPENSER_STATE_IDLE => Self::Idle,
			raw::DISPENSER_STATE_UPGRADING => Self::Upgrading,
			_ => return None,
		})
	}

	/// TF2's number for the state.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Idle => raw::DISPENSER_STATE_IDLE,
			Self::Upgrading => raw::DISPENSER_STATE_UPGRADING,
		}
	}
}

/// A TF2 player's buildings, scoped to one engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerBuildings<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerBuildings<'s> {
	/// Wraps a player's buildings, or fails with [`BuildingError::WrongGame`]
	/// unless the server runs TF2, and [`BuildingError::NotTfPlayer`] unless
	/// `player`'s datamaps include `CTFPlayer`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, BuildingError> {
		if server.game() != Game::TeamFortress2 {
			return Err(BuildingError::WrongGame);
		}

		if !player.has_data_map_class(c"CTFPlayer") {
			return Err(BuildingError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// The buildings the player built (`m_hBuilder`), sappers and blueprints
	/// included, as [`Building::all`] finds them.
	pub fn all(self) -> Result<impl Iterator<Item = Building<'s>>, BuildingError> {
		let player = self.player.handle();

		Ok(Building::all(self.server)?.filter(move |building| {
			building
				.net_prop(c"m_hBuilder")
				.ok()
				.and_then(|builder| builder.get_handle(building.entity).ok())
				== Some(player)
		}))
	}

	/// Calls one of `CTFPlayer`'s VScript methods on the player.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn call(
		self,
		method: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, BuildingError> {
		if self.player.is_marked_for_deletion() {
			return Err(BuildingError::MarkedForDeletion);
		}

		// SAFETY: The caller vouches for the method.
		Ok(unsafe { binding::call(self.player, c"CTFPlayer", method, arguments, result) }?)
	}

	/// The building the player, an engineer, carries (`m_hCarriedObject`), or
	/// `None` if they carry none.
	#[doc(alias("m_hCarriedObject", "GetCarriedObject", "m_bCarryingObject"))]
	pub fn carried(self) -> Result<Option<Building<'s>>, BuildingError> {
		let handle = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, c"m_hCarriedObject")?
			.get_handle(self.player)?;

		let Some(entity) = self.server.server_tools()?.entity_by_handle(handle) else {
			return Ok(None);
		};

		Building::new(self.server, entity).map(Some)
	}

	/// Whether the player, a spy, placed a sapper in the last 3 seconds
	/// (`CTFPlayer::IsPlacingSapper`).
	#[doc(alias("IsPlacingSapper"))]
	pub fn is_placing_sapper(self) -> Result<bool, BuildingError> {
		// SAFETY: `IsPlacingSapper` only reads the player.
		let result = unsafe { self.call(c"IsPlacingSapper", &mut [], BOOL) }?;

		// SAFETY: The checked return type selects the bool union member.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Whether a sapper the player, a spy, placed is sapping a building
	/// (`CTFPlayer::IsSapping`).
	#[doc(alias("IsSapping"))]
	pub fn is_sapping(self) -> Result<bool, BuildingError> {
		// SAFETY: `IsSapping` only reads the player.
		let result = unsafe { self.call(c"IsSapping", &mut [], BOOL) }?;

		// SAFETY: As for `is_placing_sapper`.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Picks up the building the player, an engineer, looks at, as their
	/// alternate attack with the Wrench does
	/// (`CTFPlayer::TryToPickupBuilding`). Returns whether they picked one
	/// up: they do not while they carry one, taunt or cannot pick buildings
	/// up, or if none of theirs is in front of them.
	#[doc(alias("TryToPickupBuilding"))]
	pub fn pick_up(self) -> Result<bool, BuildingError> {
		// SAFETY: `TryToPickupBuilding` checks the player itself, as VScript
		// calls it, and carrying a building removes nothing.
		let result = unsafe { self.call(c"TryToPickupBuilding", &mut [], BOOL) }?;

		// SAFETY: As for `is_placing_sapper`.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// The player.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Removes the buildings the game counts as the player's, as it does when
	/// they change class (`CTFPlayer::RemoveAllObjects`), firing an
	/// `object_removed` event for each. With `explode`, each is detonated, as
	/// [`Building::detonate`] does, and otherwise removed silently.
	#[doc(alias("RemoveAllObjects"))]
	pub fn remove_all(self, explode: bool) -> Result<(), BuildingError> {
		// SAFETY: `RemoveAllObjects` removes the buildings, and their gibs'
		// spawning, through the engine's deferred deletion, as VScript calls it.
		unsafe { self.call(c"RemoveAllObjects", &mut [boolean(explode)], binding::VOID) }?;

		Ok(())
	}
}

/// A sentry gun, mini-sentries and disposable sentries included.
#[doc(alias("CObjectSentrygun"))]
#[derive(Debug, Clone, Copy)]
pub struct Sentry<'s>(Building<'s>);

impl<'s> Sentry<'s> {
	/// Wraps a sentry, or fails with [`BuildingError::NotKind`] unless
	/// `building`'s datamaps include `CObjectSentrygun`.
	pub fn new(building: Building<'s>) -> Result<Self, BuildingError> {
		if !building.entity.has_data_map_class(c"CObjectSentrygun") {
			return Err(BuildingError::NotKind(BuildingKind::Sentry));
		}

		Ok(Self(building))
	}

	/// The kills the sentry assisted in (`m_iAssists`).
	#[doc(alias("m_iAssists"))]
	pub fn assists(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iAssists")
	}

	/// The sentry as a building.
	pub const fn building(self) -> Building<'s> {
		self.0
	}

	/// The entity the sentry targets (`m_hEnemy`), or `None` if none.
	#[doc(alias("m_hEnemy", "GetEnemy"))]
	pub fn enemy(self) -> Result<Option<Entity<'s>>, BuildingError> {
		self.0.handle(c"m_hEnemy")
	}

	/// Whether the sentry's builder aims it with the Wrangler
	/// (`m_bPlayerControlled`).
	#[doc(alias("m_bPlayerControlled", "IsPlayerControlled"))]
	pub fn is_wrangled(self) -> Result<bool, BuildingError> {
		self.0.bool(c"m_bPlayerControlled")
	}

	/// The players the sentry killed (`m_iKills`).
	#[doc(alias("m_iKills"))]
	pub fn kills(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iKills")
	}

	/// The rockets the sentry holds (`m_iAmmoRockets`), which only level 3
	/// sentries fire.
	#[doc(alias("m_iAmmoRockets"))]
	pub fn rockets(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iAmmoRockets")
	}

	/// The shells the sentry holds (`m_iAmmoShells`).
	#[doc(alias("m_iAmmoShells"))]
	pub fn shells(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iAmmoShells")
	}

	/// The shield the Wrangler gives the sentry while it aims it
	/// (`m_nShieldLevel`), which reduces the damage it takes: 0 without one.
	#[doc(alias("m_nShieldLevel", "GetShieldLevel"))]
	pub fn shield_level(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_nShieldLevel")
	}

	/// What the sentry is doing (`m_iState`).
	#[doc(alias("m_iState"))]
	pub fn state(self) -> Result<SentryState, BuildingError> {
		let raw = self.0.int(c"m_iState")?;

		SentryState::from_raw(raw).ok_or(BuildingError::UnknownState(raw))
	}
}

/// What a sentry is doing (`m_iState`), as TF2 numbers it
/// (`SENTRY_STATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SentryState {
	/// Firing at its enemy.
	#[doc(alias("SENTRY_STATE_ATTACKING"))]
	Attacking,

	/// Being built, carried, or disabled.
	#[doc(alias("SENTRY_STATE_INACTIVE"))]
	Inactive,

	/// Turning, looking for an enemy.
	#[doc(alias("SENTRY_STATE_SEARCHING"))]
	Searching,

	/// Playing its upgrade animation.
	#[doc(alias("SENTRY_STATE_UPGRADING"))]
	Upgrading,
}

impl SentryState {
	/// The state TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::SENTRY_STATE_INACTIVE => Self::Inactive,
			raw::SENTRY_STATE_SEARCHING => Self::Searching,
			raw::SENTRY_STATE_ATTACKING => Self::Attacking,
			raw::SENTRY_STATE_UPGRADING => Self::Upgrading,
			_ => return None,
		})
	}

	/// TF2's number for the state.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Inactive => raw::SENTRY_STATE_INACTIVE,
			Self::Searching => raw::SENTRY_STATE_SEARCHING,
			Self::Attacking => raw::SENTRY_STATE_ATTACKING,
			Self::Upgrading => raw::SENTRY_STATE_UPGRADING,
		}
	}
}

/// How solid a building is to players' movement, as the `SolidToPlayer` key
/// value and input number it (`OBJSOLIDTYPE`). Every building blocks the
/// other team's players.
#[doc(alias("OBJSOLIDTYPE", "SolidToPlayer"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolidToPlayers {
	/// As solid as the `SolidToPlayerMovement` of the building's kind in
	/// `scripts/objects.txt` makes it, or as [`Self::No`] if that is 0 or
	/// missing.
	#[doc(alias("SOLID_TO_PLAYER_USE_DEFAULT"))]
	Default,

	/// Its own team's players pass through it, pushed away from its center
	/// ([`TfCollisionGroup::Object`](super::collision::TfCollisionGroup::Object)).
	#[doc(alias("SOLID_TO_PLAYER_NO"))]
	No,

	/// Blocks every player
	/// ([`TfCollisionGroup::ObjectSolidToPlayerMovement`](super::collision::TfCollisionGroup::ObjectSolidToPlayerMovement)).
	#[doc(alias("SOLID_TO_PLAYER_YES"))]
	Yes,
}

impl SolidToPlayers {
	/// TF2's number for the solidity.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Default => raw::SOLID_TO_PLAYER_USE_DEFAULT,
			Self::Yes => raw::SOLID_TO_PLAYER_YES,
			Self::No => raw::SOLID_TO_PLAYER_NO,
		}
	}
}

/// A teleporter entrance or exit.
#[doc(alias("CObjectTeleporter"))]
#[derive(Debug, Clone, Copy)]
pub struct Teleporter<'s>(Building<'s>);

impl<'s> Teleporter<'s> {
	/// Wraps a teleporter, or fails with [`BuildingError::NotKind`] unless
	/// `building`'s datamaps include `CObjectTeleporter`.
	pub fn new(building: Building<'s>) -> Result<Self, BuildingError> {
		if !building.entity.has_data_map_class(c"CObjectTeleporter") {
			return Err(BuildingError::NotKind(BuildingKind::Teleporter));
		}

		Ok(Self(building))
	}

	/// The teleporter as a building.
	pub const fn building(self) -> Building<'s> {
		self.0
	}

	/// Which end the teleporter is (`m_iObjectMode`).
	#[doc(alias("m_iObjectMode", "IsEntrance", "IsExit"))]
	pub fn end(self) -> Result<TeleporterEnd, BuildingError> {
		Ok(match self.0.mode()? {
			raw::MODE_TELEPORTER_ENTRANCE => TeleporterEnd::Entrance,
			_ => TeleporterEnd::Exit,
		})
	}

	/// How long the teleporter's current recharge lasts, in seconds
	/// (`m_flCurrentRechargeDuration`), which its level decides.
	#[doc(alias("m_flCurrentRechargeDuration"))]
	pub fn recharge_duration(self) -> Result<f32, BuildingError> {
		self.0.get(c"m_flCurrentRechargeDuration")
	}

	/// The game time at which the teleporter is recharged
	/// (`m_flRechargeTime`), or a past one if it is.
	#[doc(alias("m_flRechargeTime"))]
	pub fn recharge_time(self) -> Result<f32, BuildingError> {
		self.0.get(c"m_flRechargeTime")
	}

	/// What the teleporter is doing (`m_iState`).
	#[doc(alias("m_iState"))]
	pub fn state(self) -> Result<TeleporterState, BuildingError> {
		let raw = self.0.int(c"m_iState")?;

		TeleporterState::from_raw(raw).ok_or(BuildingError::UnknownState(raw))
	}

	/// The players the teleporter teleported (`m_iTimesUsed`).
	#[doc(alias("m_iTimesUsed"))]
	pub fn times_used(self) -> Result<c_int, BuildingError> {
		self.0.int(c"m_iTimesUsed")
	}

	/// The yaw, in degrees, from an entrance towards its exit
	/// (`m_flYawToExit`), at which it draws its direction arrow.
	#[doc(alias("m_flYawToExit"))]
	pub fn yaw_to_exit(self) -> Result<f32, BuildingError> {
		self.0.get(c"m_flYawToExit")
	}
}

/// Which end of a teleporter a teleporter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeleporterEnd {
	/// The entrance, which teleports players away.
	#[doc(alias("MODE_TELEPORTER_ENTRANCE", "TTYPE_ENTRANCE"))]
	Entrance,

	/// The exit, at which players arrive.
	#[doc(alias("MODE_TELEPORTER_EXIT", "TTYPE_EXIT"))]
	Exit,
}

/// What a teleporter is doing (`m_iState`), as TF2 numbers it
/// (`TELEPORTER_STATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeleporterState {
	/// Being built.
	#[doc(alias("TELEPORTER_STATE_BUILDING"))]
	Building,

	/// Built, without its other end.
	#[doc(alias("TELEPORTER_STATE_IDLE"))]
	Idle,

	/// Built with its other end, and charged.
	#[doc(alias("TELEPORTER_STATE_READY"))]
	Ready,

	/// An exit about to receive a player.
	#[doc(alias("TELEPORTER_STATE_RECEIVING"))]
	Receiving,

	/// An exit releasing the player it received.
	#[doc(alias("TELEPORTER_STATE_RECEIVING_RELEASE"))]
	ReceivingRelease,

	/// Recharging after a teleport.
	#[doc(alias("TELEPORTER_STATE_RECHARGING"))]
	Recharging,

	/// An entrance teleporting a player away.
	#[doc(alias("TELEPORTER_STATE_SENDING"))]
	Sending,

	/// Playing its upgrade animation.
	#[doc(alias("TELEPORTER_STATE_UPGRADING"))]
	Upgrading,
}

impl TeleporterState {
	/// The state TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::TELEPORTER_STATE_BUILDING => Self::Building,
			raw::TELEPORTER_STATE_IDLE => Self::Idle,
			raw::TELEPORTER_STATE_READY => Self::Ready,
			raw::TELEPORTER_STATE_SENDING => Self::Sending,
			raw::TELEPORTER_STATE_RECEIVING => Self::Receiving,
			raw::TELEPORTER_STATE_RECEIVING_RELEASE => Self::ReceivingRelease,
			raw::TELEPORTER_STATE_RECHARGING => Self::Recharging,
			raw::TELEPORTER_STATE_UPGRADING => Self::Upgrading,
			_ => return None,
		})
	}

	/// TF2's number for the state.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Building => raw::TELEPORTER_STATE_BUILDING,
			Self::Idle => raw::TELEPORTER_STATE_IDLE,
			Self::Ready => raw::TELEPORTER_STATE_READY,
			Self::Sending => raw::TELEPORTER_STATE_SENDING,
			Self::Receiving => raw::TELEPORTER_STATE_RECEIVING,
			Self::ReceivingRelease => raw::TELEPORTER_STATE_RECEIVING_RELEASE,
			Self::Recharging => raw::TELEPORTER_STATE_RECHARGING,
			Self::Upgrading => raw::TELEPORTER_STATE_UPGRADING,
		}
	}
}

/// Finds the primary vtable of every building class in the game server
/// module, before any hook is installed, from the classes' run-time type
/// information. Missing or ambiguous information is an error: no class is
/// left out. Call once, such as while loading: it reads the whole module, a
/// few times, however many classes there are.
///
/// The search only checks that each vtable holds code at
/// [`IS_PLACEMENT_POS_VALID_SLOT`](sdk_raw::tf2::objects::IS_PLACEMENT_POS_VALID_SLOT),
/// which every building class has.
pub fn building_vtables(server: Server<'_>) -> Result<BuildingVtables<'_>, BuildingVtableError> {
	if server.game() != Game::TeamFortress2 {
		return Err(BuildingVtableError::WrongGame);
	}

	// SAFETY: The game server factory is the game module's `CreateInterface`,
	// and the Server's callback scope keeps the module loaded while its
	// sections are inspected (`Server::new` condition 1).
	let search = unsafe { ObjectVtables::load(server.game_server_factory().as_raw()) }?;
	let found = search.find_all(&BuildingClass::ALL.map(BuildingClass::name));
	let mut vtables = [NonNull::dangling(); 7];

	for ((vtable, class), pointer) in vtables.iter_mut().zip(BuildingClass::ALL).zip(found) {
		*vtable = pointer.ok_or(BuildingVtableError::NotFound(class))?;
	}

	Ok(BuildingVtables {
		vtables,
		_scope: PhantomData,
	})
}
