//! How an entity moves: its flags (`m_fFlags`), move type, velocity, gravity
//! and friction, and the entity it moves with.
//!
//! [`ServerTools::set_move_type`] changes how the game moves an entity.

#[cfg(test)]
#[path = "../tests/entities/movement.rs"]
mod tests;

use crate::entities::fields::{BaseField, FieldError};
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::{ServerTools, ValveEngine};
use crate::math::Vector;
use sdk_raw::entities::find_physics_object_field;

use sdk_raw::entities::flags::{
	FL_AIMTARGET, FL_ANIMDUCKING, FL_ATCONTROLS, FL_BASEVELOCITY, FL_CLIENT, FL_CONVEYOR,
	FL_DISSOLVING, FL_DONTTOUCH, FL_DUCKING, FL_FAKECLIENT, FL_FLY, FL_FROZEN, FL_GODMODE,
	FL_GRAPHED, FL_GRENADE, FL_INRAIN, FL_INWATER, FL_KILLME, FL_NOTARGET, FL_NPC, FL_OBJECT,
	FL_ONFIRE, FL_ONGROUND, FL_ONTRAIN, FL_PARTIALGROUND, FL_STATICPROP, FL_STEPMOVEMENT, FL_SWIM,
	FL_TRANSRAGDOLL, FL_UNBLOCKABLE_BY_PLAYER, FL_WATERJUMP, FL_WORLDBRUSH,
};

use sdk_raw::vcall;
use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

/// `EFL_DIRTY_ABSVELOCITY` from `game/shared/shareddefs.h`: the entity's
/// velocity in the world is yet to be computed from its parent's.
const EFL_DIRTY_ABSVELOCITY: c_int = 1 << 12;

/// `m_vecAbsVelocity`.
static ABS_VELOCITY: BaseField<Vector> =
	BaseField::new(c"m_vecAbsVelocity", sys::_fieldtypes_FIELD_VECTOR);

/// `m_iEFlags`.
static ENGINE_FLAGS: BaseField<c_int> =
	BaseField::new(c"m_iEFlags", sys::_fieldtypes_FIELD_INTEGER);

/// `m_fFlags`.
static FLAGS: BaseField<c_int> = BaseField::new(c"m_fFlags", sys::_fieldtypes_FIELD_INTEGER);

/// `m_flFriction`, the `friction` key value.
static FRICTION: BaseField<f32> = BaseField::new(c"m_flFriction", sys::_fieldtypes_FIELD_FLOAT);

/// `m_flGravity`, the `gravity` key value.
static GRAVITY: BaseField<f32> = BaseField::new(c"m_flGravity", sys::_fieldtypes_FIELD_FLOAT);

/// `m_vecVelocity`, the `velocity` key value.
static LOCAL_VELOCITY: BaseField<Vector> =
	BaseField::new(c"m_vecVelocity", sys::_fieldtypes_FIELD_VECTOR);

/// `m_MoveCollide`.
static MOVE_COLLIDE: BaseField<u8> =
	BaseField::new(c"m_MoveCollide", sys::_fieldtypes_FIELD_CHARACTER);

/// `m_hMoveParent`.
static MOVE_PARENT: BaseField<EntityHandle> =
	BaseField::new(c"m_hMoveParent", sys::_fieldtypes_FIELD_EHANDLE);

/// `m_MoveType`, the `MoveType` key value.
static MOVE_TYPE: BaseField<u8> = BaseField::new(c"m_MoveType", sys::_fieldtypes_FIELD_CHARACTER);

bitflags::bitflags! {
	/// An entity's flags (`m_fFlags`), the `FL_*` values from
	/// `public/const.h`, as multiplayer games define them.
	///
	/// Flags read from an entity keep every bit, including those without a
	/// constant here.
	#[doc(alias("m_fFlags"))]
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct EntityFlags: c_int {
		/// `FL_AIMTARGET`: aim assistance may aim at the entity.
		#[doc(alias("FL_AIMTARGET"))]
		const AIM_TARGET = FL_AIMTARGET;

		/// `FL_ANIMDUCKING`: the player is crouching or standing up, or fully
		/// crouched along with [`DUCKING`](Self::DUCKING).
		#[doc(alias("FL_ANIMDUCKING"))]
		const ANIM_DUCKING = FL_ANIMDUCKING;

		/// `FL_ATCONTROLS`: the player cannot move, but keeps its inputs, to
		/// control another entity.
		#[doc(alias("FL_ATCONTROLS"))]
		const AT_CONTROLS = FL_ATCONTROLS;

		/// `FL_BASEVELOCITY`: base velocity was applied this frame.
		#[doc(alias("FL_BASEVELOCITY"))]
		const BASE_VELOCITY = FL_BASEVELOCITY;

		/// `FL_CLIENT`: the entity is a player.
		#[doc(alias("FL_CLIENT"))]
		const CLIENT = FL_CLIENT;

		/// `FL_CONVEYOR`: the entity is a conveyor.
		#[doc(alias("FL_CONVEYOR"))]
		const CONVEYOR = FL_CONVEYOR;

		/// `FL_DISSOLVING`: the entity is dissolving.
		#[doc(alias("FL_DISSOLVING"))]
		const DISSOLVING = FL_DISSOLVING;

		/// `FL_DONTTOUCH`: the entity touches nothing.
		#[doc(alias("FL_DONTTOUCH"))]
		const DONT_TOUCH = FL_DONTTOUCH;

		/// `FL_DUCKING`: the player is fully crouched, or standing up from it.
		#[doc(alias("FL_DUCKING"))]
		const DUCKING = FL_DUCKING;

		/// `FL_FAKECLIENT`: the player is a bot.
		#[doc(alias("FL_FAKECLIENT"))]
		const FAKE_CLIENT = FL_FAKECLIENT;

		/// `FL_FLY`: the entity moves without needing to be on the ground.
		#[doc(alias("FL_FLY"))]
		const FLY = FL_FLY;

		/// `FL_FROZEN`: the player cannot move or look around.
		#[doc(alias("FL_FROZEN"))]
		const FROZEN = FL_FROZEN;

		/// `FL_GODMODE`: the player takes no damage.
		#[doc(alias("FL_GODMODE"))]
		const GOD_MODE = FL_GODMODE;

		/// `FL_GRAPHED`: the entity blocks a connection of the navigation
		/// graph.
		#[doc(alias("FL_GRAPHED"))]
		const GRAPHED = FL_GRAPHED;

		/// `FL_GRENADE`: the entity is a grenade.
		#[doc(alias("FL_GRENADE"))]
		const GRENADE = FL_GRENADE;

		/// `FL_INRAIN`: the entity stands in rain.
		#[doc(alias("FL_INRAIN"))]
		const IN_RAIN = FL_INRAIN;

		/// `FL_INWATER`: the entity is in water.
		#[doc(alias("FL_INWATER"))]
		const IN_WATER = FL_INWATER;

		/// `FL_KILLME`: the entity is marked for deletion.
		#[doc(alias("FL_KILLME"))]
		const KILL_ME = FL_KILLME;

		/// `FL_NOTARGET`: enemies do not target the entity.
		#[doc(alias("FL_NOTARGET"))]
		const NO_TARGET = FL_NOTARGET;

		/// `FL_NPC`: the entity is an NPC.
		#[doc(alias("FL_NPC"))]
		const NPC = FL_NPC;

		/// `FL_OBJECT`: NPCs see the entity.
		#[doc(alias("FL_OBJECT"))]
		const OBJECT = FL_OBJECT;

		/// `FL_ONFIRE`: the entity is on fire.
		#[doc(alias("FL_ONFIRE"))]
		const ON_FIRE = FL_ONFIRE;

		/// `FL_ONGROUND`: the entity rests on the ground.
		#[doc(alias("FL_ONGROUND"))]
		const ON_GROUND = FL_ONGROUND;

		/// `FL_ONTRAIN`: the player controls a train.
		#[doc(alias("FL_ONTRAIN"))]
		const ON_TRAIN = FL_ONTRAIN;

		/// `FL_PARTIALGROUND`: not all of the entity's corners are on the
		/// ground.
		#[doc(alias("FL_PARTIALGROUND"))]
		const PARTIAL_GROUND = FL_PARTIALGROUND;

		/// `FL_STATICPROP`: the entity is a static prop.
		#[doc(alias("FL_STATICPROP"))]
		const STATIC_PROP = FL_STATICPROP;

		/// `FL_STEPMOVEMENT`: the entity's step movement does no processing.
		#[doc(alias("FL_STEPMOVEMENT"))]
		const STEP_MOVEMENT = FL_STEPMOVEMENT;

		/// `FL_SWIM`: the entity moves without needing to be on the ground,
		/// but stays in water.
		#[doc(alias("FL_SWIM"))]
		const SWIM = FL_SWIM;

		/// `FL_TRANSRAGDOLL`: the entity is turning into a client-side
		/// ragdoll.
		#[doc(alias("FL_TRANSRAGDOLL"))]
		const TRANS_RAGDOLL = FL_TRANSRAGDOLL;

		/// `FL_UNBLOCKABLE_BY_PLAYER`: players cannot block the entity as it
		/// pushes them.
		#[doc(alias("FL_UNBLOCKABLE_BY_PLAYER"))]
		const UNBLOCKABLE_BY_PLAYER = FL_UNBLOCKABLE_BY_PLAYER;

		/// `FL_WATERJUMP`: the player is jumping out of water.
		#[doc(alias("FL_WATERJUMP"))]
		const WATER_JUMP = FL_WATERJUMP;

		/// `FL_WORLDBRUSH`: the entity is a brush that is part of the world.
		#[doc(alias("FL_WORLDBRUSH"))]
		const WORLD_BRUSH = FL_WORLDBRUSH;

		// Bits without a constant here, which games may set.
		const _ = !0;
	}
}

/// How the game moves an entity (`MoveType_t`).
#[doc(alias("MoveType_t", "m_MoveType"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoveType {
	/// `MOVETYPE_NONE`: never moves.
	#[doc(alias("MOVETYPE_NONE"))]
	None,

	/// `MOVETYPE_ISOMETRIC`: an unused player movement.
	#[doc(alias("MOVETYPE_ISOMETRIC"))]
	Isometric,

	/// `MOVETYPE_WALK`: walks on the ground, as players do.
	#[doc(alias("MOVETYPE_WALK"))]
	Walk,

	/// `MOVETYPE_STEP`: steps with gravity, as NPCs do.
	#[doc(alias("MOVETYPE_STEP"))]
	Step,

	/// `MOVETYPE_FLY`: flies without gravity, colliding with what it hits.
	#[doc(alias("MOVETYPE_FLY"))]
	Fly,

	/// `MOVETYPE_FLYGRAVITY`: flies with gravity, as thrown objects do.
	#[doc(alias("MOVETYPE_FLYGRAVITY"))]
	FlyGravity,

	/// `MOVETYPE_VPHYSICS`: simulated by the physics engine.
	#[doc(alias("MOVETYPE_VPHYSICS"))]
	VPhysics,

	/// `MOVETYPE_PUSH`: pushes what it moves into, as doors do, and ignores
	/// the world.
	#[doc(alias("MOVETYPE_PUSH"))]
	Push,

	/// `MOVETYPE_NOCLIP`: flies through everything, as with the `noclip`
	/// command.
	#[doc(alias("MOVETYPE_NOCLIP"))]
	NoClip,

	/// `MOVETYPE_LADDER`: climbs a ladder.
	#[doc(alias("MOVETYPE_LADDER"))]
	Ladder,

	/// `MOVETYPE_OBSERVER`: an observer's movement.
	#[doc(alias("MOVETYPE_OBSERVER"))]
	Observer,

	/// `MOVETYPE_CUSTOM`: the entity's own movement.
	#[doc(alias("MOVETYPE_CUSTOM"))]
	Custom,
}

impl MoveType {
	/// The move type a `MoveType_t` value stands for, or `None` for a value
	/// past `MOVETYPE_CUSTOM`.
	pub const fn from_raw(value: u8) -> Option<Self> {
		Some(match value as sys::MoveType_t {
			sys::MoveType_t_MOVETYPE_NONE => Self::None,
			sys::MoveType_t_MOVETYPE_ISOMETRIC => Self::Isometric,
			sys::MoveType_t_MOVETYPE_WALK => Self::Walk,
			sys::MoveType_t_MOVETYPE_STEP => Self::Step,
			sys::MoveType_t_MOVETYPE_FLY => Self::Fly,
			sys::MoveType_t_MOVETYPE_FLYGRAVITY => Self::FlyGravity,
			sys::MoveType_t_MOVETYPE_VPHYSICS => Self::VPhysics,
			sys::MoveType_t_MOVETYPE_PUSH => Self::Push,
			sys::MoveType_t_MOVETYPE_NOCLIP => Self::NoClip,
			sys::MoveType_t_MOVETYPE_LADDER => Self::Ladder,
			sys::MoveType_t_MOVETYPE_OBSERVER => Self::Observer,
			sys::MoveType_t_MOVETYPE_CUSTOM => Self::Custom,
			_ => return None,
		})
	}

	/// The move type's `MoveType_t` value.
	pub const fn to_raw(self) -> u8 {
		(match self {
			Self::None => sys::MoveType_t_MOVETYPE_NONE,
			Self::Isometric => sys::MoveType_t_MOVETYPE_ISOMETRIC,
			Self::Walk => sys::MoveType_t_MOVETYPE_WALK,
			Self::Step => sys::MoveType_t_MOVETYPE_STEP,
			Self::Fly => sys::MoveType_t_MOVETYPE_FLY,
			Self::FlyGravity => sys::MoveType_t_MOVETYPE_FLYGRAVITY,
			Self::VPhysics => sys::MoveType_t_MOVETYPE_VPHYSICS,
			Self::Push => sys::MoveType_t_MOVETYPE_PUSH,
			Self::NoClip => sys::MoveType_t_MOVETYPE_NOCLIP,
			Self::Ladder => sys::MoveType_t_MOVETYPE_LADDER,
			Self::Observer => sys::MoveType_t_MOVETYPE_OBSERVER,
			Self::Custom => sys::MoveType_t_MOVETYPE_CUSTOM,
		}) as u8
	}
}

impl<'s> Entity<'s> {
	/// The entity's velocity in the world, in units per second
	/// (`GetAbsVelocity`), or `None` while the game has yet to compute it from
	/// the entity's move parent, which it does as something asks for it.
	#[doc(alias("GetAbsVelocity", "m_vecAbsVelocity"))]
	pub fn abs_velocity(self) -> Result<Option<Vector>, FieldError> {
		if ENGINE_FLAGS.read(self)? & EFL_DIRTY_ABSVELOCITY == 0 {
			return ABS_VELOCITY.read(self).map(Some);
		}

		// Without a parent, the game computes it as the local velocity.
		match self.move_parent()? {
			Some(_) => Ok(None),
			None => LOCAL_VELOCITY.read(self).map(Some),
		}
	}

	/// The entity's flags (`m_fFlags`).
	#[doc(alias("GetFlags", "m_fFlags"))]
	pub fn flags(self) -> Result<EntityFlags, FieldError> {
		FLAGS.read(self).map(EntityFlags::from_bits_retain)
	}

	/// The friction the entity moves with, as a scale of the server's
	/// (`m_flFriction`), the `friction` key value. Only players' movement
	/// uses it.
	#[doc(alias("GetFriction", "m_flFriction"))]
	pub fn friction(self) -> Result<f32, FieldError> {
		FRICTION.read(self)
	}

	/// The gravity the entity moves with, as a scale of the server's
	/// (`m_flGravity`), the `gravity` key value, where 0 also stands for the
	/// server's own.
	#[doc(alias("GetGravity", "m_flGravity"))]
	pub fn gravity(self) -> Result<f32, FieldError> {
		GRAVITY.read(self)
	}

	/// Whether the entity has a VPhysics object (`m_pPhysicsObject`): physics
	/// props have one once spawned, as do the players, brushes and others whose
	/// shadows block what VPhysics simulates.
	///
	/// Fails with [`FieldError::NotFound`] unless `CBaseEntity`'s datamap
	/// declares the object as the game does, at an aligned, plausible offset.
	#[doc(alias("VPhysicsGetObject", "m_pPhysicsObject"))]
	pub fn has_physics_object(self) -> Result<bool, FieldError> {
		static OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = match OFFSET.get() {
			Some(&offset) => offset,

			None => {
				let offset = find_physics_object_field(self.data_maps()).ok_or_else(|| {
					FieldError::NotFound {
						name: c"m_pPhysicsObject".to_owned(),
					}
				})?;

				*OFFSET.get_or_init(|| offset)
			}
		};

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base, and is aligned for
		// the pointer, which is read without forming a reference, as the game
		// writes it too.
		let object = unsafe {
			self.as_ptr()
				.byte_add(offset)
				.cast::<*const c_void>()
				.read()
		};

		Ok(!object.is_null())
	}

	/// The entity's velocity relative to its move parent, or in the world
	/// without one, in units per second (`m_vecVelocity`).
	#[doc(alias("GetLocalVelocity", "m_vecVelocity"))]
	pub fn local_velocity(self) -> Result<Vector, FieldError> {
		LOCAL_VELOCITY.read(self)
	}

	/// How the entity reacts when it flies into something (`m_MoveCollide`), or
	/// `None` for a reaction the SDK does not know.
	#[doc(alias("GetMoveCollide", "m_MoveCollide"))]
	pub fn move_collide(self) -> Result<Option<MoveCollide>, FieldError> {
		MOVE_COLLIDE.read(self).map(MoveCollide::from_raw)
	}

	/// The entity the entity moves with (`m_hMoveParent`), such as the player
	/// wearing an item, or `None` if it moves on its own.
	#[doc(alias("GetMoveParent", "moveparent", "m_hMoveParent"))]
	pub fn move_parent(self) -> Result<Option<EntityHandle>, FieldError> {
		let parent = MOVE_PARENT.read(self)?;

		Ok(parent.is_valid().then_some(parent))
	}

	/// How the game moves the entity (`m_MoveType`), or `None` for a move type
	/// the SDK does not know.
	#[doc(alias("GetMoveType", "m_MoveType"))]
	pub fn move_type(self) -> Result<Option<MoveType>, FieldError> {
		MOVE_TYPE.read(self).map(MoveType::from_raw)
	}

	/// Sets the friction the entity moves with (`m_flFriction`), as
	/// `CBaseEntity::SetFriction` does.
	#[doc(alias("SetFriction", "m_flFriction"))]
	pub fn set_friction(self, engine: ValveEngine<'_>, friction: f32) -> Result<(), FieldError> {
		FRICTION.write(engine, self, friction)
	}

	/// Sets the gravity the entity moves with (`m_flGravity`), as
	/// `CBaseEntity::SetGravity` does.
	#[doc(alias("SetGravity", "m_flGravity"))]
	pub fn set_gravity(self, engine: ValveEngine<'_>, gravity: f32) -> Result<(), FieldError> {
		GRAVITY.write(engine, self, gravity)
	}
}

/// How an entity that flies reacts when it hits something
/// (`MoveCollide_t`).
#[doc(alias("MoveCollide_t", "m_MoveCollide"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MoveCollide {
	/// `MOVECOLLIDE_DEFAULT`: stops, or slides, as its move type does.
	#[doc(alias("MOVECOLLIDE_DEFAULT"))]
	#[default]
	Default,

	/// `MOVECOLLIDE_FLY_BOUNCE`: bounces off, as elastic as the surface, with
	/// friction.
	#[doc(alias("MOVECOLLIDE_FLY_BOUNCE"))]
	FlyBounce,

	/// `MOVECOLLIDE_FLY_CUSTOM`: lets the entity's `Touch` change its velocity.
	#[doc(alias("MOVECOLLIDE_FLY_CUSTOM"))]
	FlyCustom,

	/// `MOVECOLLIDE_FLY_SLIDE`: slides along the surface, with friction.
	#[doc(alias("MOVECOLLIDE_FLY_SLIDE"))]
	FlySlide,
}

impl MoveCollide {
	/// The reaction a `MoveCollide_t` value stands for, or `None` for a value
	/// past `MOVECOLLIDE_FLY_SLIDE`.
	pub const fn from_raw(value: u8) -> Option<Self> {
		Some(match value as sys::MoveCollide_t {
			sys::MoveCollide_t_MOVECOLLIDE_DEFAULT => Self::Default,
			sys::MoveCollide_t_MOVECOLLIDE_FLY_BOUNCE => Self::FlyBounce,
			sys::MoveCollide_t_MOVECOLLIDE_FLY_CUSTOM => Self::FlyCustom,
			sys::MoveCollide_t_MOVECOLLIDE_FLY_SLIDE => Self::FlySlide,
			_ => return None,
		})
	}

	/// The reaction's `MoveCollide_t` value.
	pub const fn to_raw(self) -> u8 {
		(match self {
			Self::Default => sys::MoveCollide_t_MOVECOLLIDE_DEFAULT,
			Self::FlyBounce => sys::MoveCollide_t_MOVECOLLIDE_FLY_BOUNCE,
			Self::FlyCustom => sys::MoveCollide_t_MOVECOLLIDE_FLY_CUSTOM,
			Self::FlySlide => sys::MoveCollide_t_MOVECOLLIDE_FLY_SLIDE,
		}) as u8
	}
}

impl<'s> ServerTools<'s> {
	/// Sets how the game moves `entity`, keeping how it reacts when it hits
	/// something, through `CBaseEntity::SetMoveType`, which also updates its
	/// collision rules and how often it is simulated.
	#[doc(alias("SetMoveType", "SetEntityMoveType"))]
	pub fn set_move_type(self, entity: Entity<'_>, move_type: MoveType) {
		// SAFETY: `Server::new` guarantees the interface is live, and the entity
		// is live. The game only sets the entity's members and its collision
		// rules.
		unsafe {
			vcall!(self.as_ptr() => IServerTools_SetMoveType(entity.as_ptr(), c_int::from(move_type.to_raw())))
		};
	}

	/// Sets how the game moves `entity` and how it reacts when it hits
	/// something, as [`Self::set_move_type`] does.
	#[doc(alias("SetMoveType"))]
	pub fn set_move_type_and_collide(
		self,
		entity: Entity<'_>,
		move_type: MoveType,
		collide: MoveCollide,
	) {
		// SAFETY: As for `set_move_type`.
		unsafe {
			vcall!(self.as_ptr() => IServerTools_SetMoveType1(
				entity.as_ptr(),
				c_int::from(move_type.to_raw()),
				c_int::from(collide.to_raw()),
			))
		};
	}
}
