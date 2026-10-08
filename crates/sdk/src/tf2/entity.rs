//! The native members TF2 exposes to VScript on every entity, through the
//! script class of `CBaseEntity`: its solid type, solid flags and collision
//! group, its bounds, its origin, angles, velocity and pushes, and damage.
//!
//! [`TfEntity`] wraps an entity within one engine callback, and calls the
//! members' native bindings, found by name and checked against the SDK's
//! signatures before each call, as [`TfPlayer`] calls `CTFPlayer`'s. Each
//! member runs the game's own logic, which writing the entity's fields would
//! skip: changing the solid flags or the collision group rechecks what the
//! entity collides with and touches, and damage runs the entity's damage
//! filters, the game rules and its `OnTakeDamage`.
//!
//! # VPhysics callbacks
//!
//! Changing whether an entity is solid or a trigger, or its collision group,
//! makes its VPhysics object recheck its collisions, which the game warns "is
//! likely to cause crashes" while VPhysics reports collisions. Change them
//! from other callbacks, such as a think, a frame or a touch. The game itself
//! defers damage dealt during these reports.
//!
//! # Unverified
//!
//! No running server has tested this module yet. Its bindings are checked by
//! name and signature as they are called.
//!
//! [`TfPlayer`]: crate::tf2::player::TfPlayer

#[cfg(test)]
#[path = "../tests/tf2/entity.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::fields::FieldError;
use crate::entities::movement::MoveType;
use crate::entities::solid::{SolidFlags, SolidType};
use crate::math::{QAngle, Vector};
use crate::tf2::collision::TfCollisionGroup;
use crate::tf2::damage::DamageType;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::tf2::script_instances::{ScriptInstance, ScriptInstanceError};
use crate::{Game, Server};
use sdk_raw::tf2::script_binding::{float, handle, int, qangle, vector};
use std::ffi::{CStr, c_int};
use std::ptr::null_mut;

/// The script class that declares the members [`TfEntity`] calls.
const SCRIPT_CLASS: &CStr = c"CBaseEntity";

/// Damage for [`TfEntity::take_damage`] to deal: its amount and kinds, who
/// deals it with what, and how it pushes, as VScript's `TakeDamageCustom`
/// takes it.
///
/// The game's damage filters, among others, read the attacker without
/// checking for one, so damage always has one: without an attacker, the
/// inflictor deals it, and without either, the entity damages itself, as
/// VScript's `TakeDamage` does. Without an inflictor, the attacker inflicts
/// it.
#[derive(Debug, Clone, Copy)]
pub struct Damage<'s> {
	amount: f32,
	damage_type: DamageType,
	custom: c_int,
	attacker: Option<Entity<'s>>,
	inflictor: Option<Entity<'s>>,
	weapon: Option<Entity<'s>>,
	force: Vector,
	position: Vector,
}

impl<'s> Damage<'s> {
	/// `amount` of damage of the kinds `damage_type`, from neither an
	/// attacker, nor an inflictor, nor a weapon, with no custom kind and no
	/// push.
	///
	/// The kinds that push, such as bullets, expect a force and a position:
	/// without them, the game warns in its console the first few times.
	pub const fn new(amount: f32, damage_type: DamageType) -> Self {
		Self {
			amount,
			damage_type,
			custom: 0,
			attacker: None,
			inflictor: None,
			weapon: None,
			force: Vector::new(0.0, 0.0, 0.0),
			position: Vector::new(0.0, 0.0, 0.0),
		}
	}

	/// Who deals the damage, such as a player.
	pub const fn attacker(mut self, attacker: Entity<'s>) -> Self {
		self.attacker = Some(attacker);
		self
	}

	/// The custom kind of the damage, an `ETFDmgCustom` value, such as
	/// [`CUSTOM_DAMAGE_PLASMA`](crate::tf2::damage::CUSTOM_DAMAGE_PLASMA),
	/// which kill icons, ragdolls and many weapons' effects read.
	#[doc(alias("SetDamageCustom"))]
	pub const fn custom(mut self, custom: c_int) -> Self {
		self.custom = custom;
		self
	}

	/// The push the damage gives, from which the game computes the knockback
	/// and how a ragdoll flies.
	pub const fn force(mut self, force: Vector) -> Self {
		self.force = force;
		self
	}

	/// What inflicts the damage, such as a rocket or a sentry gun.
	pub const fn inflictor(mut self, inflictor: Entity<'s>) -> Self {
		self.inflictor = Some(inflictor);
		self
	}

	/// Where the damage hits, in the world.
	pub const fn position(mut self, position: Vector) -> Self {
		self.position = position;
		self
	}

	/// The weapon that deals the damage, whose attributes the game applies.
	pub const fn weapon(mut self, weapon: Entity<'s>) -> Self {
		self.weapon = Some(weapon);
		self
	}
}

/// An entity of a TF2 server, through the native members `CBaseEntity`
/// exposes to VScript, within one engine callback.
///
/// Keep the entity's [`EntityHandle`](crate::entities::EntityHandle) across
/// callbacks, and wrap it again in each.
#[derive(Debug, Clone, Copy)]
pub struct TfEntity<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> TfEntity<'s> {
	/// Wraps an entity, or returns [`TfEntityError::NotTf2`] unless the server
	/// runs TF2.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, TfEntityError> {
		if server.game() != Game::TeamFortress2 {
			return Err(TfEntityError::NotTf2);
		}

		Ok(Self { server, entity })
	}

	/// Adds `flags` to the entity's solid flags, as
	/// [`set_solid_flags`](Self::set_solid_flags) does.
	#[doc(alias("AddSolidFlags"))]
	pub fn add_solid_flags(self, flags: SolidFlags) -> Result<(), TfEntityError> {
		self.call_with_flags(c"AddSolidFlags", flags)
	}

	/// Spins the entity faster by `impulse`, about its own x, y and z axes, in
	/// degrees per second (`ApplyLocalAngularVelocityImpulse`): through its
	/// VPhysics object if VPhysics moves it, or else by changing its angular
	/// velocity.
	///
	/// The game discards a spin it thinks too fast, warning in its console.
	/// Fails with [`TfEntityError::NonFinite`] for an impulse that is not
	/// finite, and with [`TfEntityError::NoPhysicsObject`] for an entity that
	/// VPhysics moves without a physics object, before the game is called.
	#[doc(alias("ApplyLocalAngularVelocityImpulse"))]
	pub fn apply_angular_impulse(self, impulse: Vector) -> Result<(), TfEntityError> {
		self.call_with_impulse(c"ApplyLocalAngularVelocityImpulse", impulse)
	}

	/// Pushes the entity, adding `impulse` to its velocity in the world, in
	/// units per second (`ApplyAbsVelocityImpulse`): through its VPhysics
	/// object if VPhysics moves it, or else by changing its velocity.
	///
	/// This is `CBaseEntity`'s push: [`TfPlayer::apply_impulse`] pushes a
	/// player as TF2 pushes them, scaled by their condition and attributes.
	/// The game clamps a push it thinks too fast, and discards a far faster
	/// one, warning in its console. Fails as
	/// [`apply_angular_impulse`](Self::apply_angular_impulse) does.
	///
	/// [`TfPlayer::apply_impulse`]: crate::tf2::player::TfPlayer::apply_impulse
	#[doc(alias("ApplyAbsVelocityImpulse"))]
	pub fn apply_impulse(self, impulse: Vector) -> Result<(), TfEntityError> {
		self.call_with_impulse(c"ApplyAbsVelocityImpulse", impulse)
	}

	/// Calls a member of the entity that returns nothing.
	///
	/// # Safety
	///
	/// As for [`binding::call`], with a member of `CBaseEntity` that accepts
	/// `arguments`.
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<(), TfEntityError> {
		self.check_live()?;

		// SAFETY: The caller vouches for the member and its arguments.
		unsafe { binding::call(self.entity, SCRIPT_CLASS, name, arguments, binding::VOID) }?;

		Ok(())
	}

	/// Calls one of the members that change the entity's solid flags with
	/// `flags`.
	fn call_with_flags(self, name: &CStr, flags: SolidFlags) -> Result<(), TfEntityError> {
		// SAFETY: The solid flag members change the flags of the entity's
		// collision property, and update its collision rules, partition and
		// touches when that changes whether it is solid or a trigger.
		unsafe { self.call(name, &mut [int(c_int::from(flags.bits()))]) }
	}

	/// Calls one of the members that push the entity with `impulse`, after
	/// checking it and the entity's physics object, which the members use
	/// without checking for one.
	fn call_with_impulse(self, name: &CStr, impulse: Vector) -> Result<(), TfEntityError> {
		if !impulse.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		if self.entity.move_type()? == Some(MoveType::VPhysics)
			&& !self.entity.has_physics_object()?
		{
			return Err(TfEntityError::NoPhysicsObject);
		}

		let impulse = sys::Vector::from(impulse);

		// SAFETY: The member reads the finite impulse during the call, and
		// pushes the entity through its physics object, which it has if
		// VPhysics moves it, or through its velocity otherwise.
		unsafe { self.call(name, &mut [vector(&impulse)]) }
	}

	/// Calls one of the members that set the velocity of the entity's
	/// physics object with `velocity`, after checking it and that the entity
	/// has the object, without which the members do nothing.
	fn call_with_physics_velocity(
		self,
		name: &CStr,
		velocity: Vector,
	) -> Result<(), TfEntityError> {
		if !velocity.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		if !self.entity.has_physics_object()? {
			return Err(TfEntityError::NoPhysicsObject);
		}

		let velocity = sys::Vector::from(velocity);

		// SAFETY: The member checks for the physics object, and reads the
		// finite velocity during the call.
		unsafe { self.call(name, &mut [vector(&velocity)]) }
	}

	/// Fails with [`TfEntityError::MarkedForDeletion`] if the entity is marked
	/// for deletion.
	fn check_live(self) -> Result<(), TfEntityError> {
		if self.entity.is_marked_for_deletion() {
			Err(TfEntityError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	/// The wrapped entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The script instance of `entity`, made if it has none.
	fn instance(self, entity: Entity<'s>) -> Result<sys::HSCRIPT, TfEntityError> {
		Ok(ScriptInstance::of(self.server, entity)?.as_raw())
	}

	/// Removes `flags` from the entity's solid flags, as
	/// [`set_solid_flags`](Self::set_solid_flags) does.
	#[doc(alias("RemoveSolidFlags"))]
	pub fn remove_solid_flags(self, flags: SolidFlags) -> Result<(), TfEntityError> {
		self.call_with_flags(c"RemoveSolidFlags", flags)
	}

	/// Turns the entity in the world (`SetAbsAngles`), and its angles relative
	/// to its move parent to match.
	///
	/// Unlike a teleport, clients interpolate the turn, as they do moves made
	/// with [`set_abs_origin`](Self::set_abs_origin). Fails with
	/// [`TfEntityError::NonFinite`] for angles that are not finite, before the
	/// game is called.
	#[doc(alias("SetAbsAngles"))]
	pub fn set_abs_angles(self, angles: QAngle) -> Result<(), TfEntityError> {
		if ![angles.pitch, angles.yaw, angles.roll]
			.iter()
			.all(|angle| angle.is_finite())
		{
			return Err(TfEntityError::NonFinite);
		}

		let angles = sys::QAngle::from(angles);

		// SAFETY: The member reads the finite angles during the call, and stores
		// them, the entity's local angles, which it computes from its move
		// parent's, and the time of the change, which clients interpolate from.
		unsafe { self.call(c"SetAbsAngles", &mut [qangle(&angles)]) }
	}

	/// Moves the entity in the world (`SetAbsOrigin`), and its origin relative
	/// to its move parent to match.
	///
	/// Unlike a teleport, which makes clients snap the entity to its new
	/// origin, clients interpolate the move, so stepping an entity a little
	/// each tick looks smooth. The entity touches nothing it passes through on
	/// the way. Fails with [`TfEntityError::NonFinite`] for an origin that is
	/// not finite, before the game is called.
	#[doc(alias("SetAbsOrigin"))]
	pub fn set_abs_origin(self, origin: Vector) -> Result<(), TfEntityError> {
		if !origin.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		let origin = sys::Vector::from(origin);

		// SAFETY: The member reads the finite origin during the call, and stores
		// it, the entity's local origin, which it computes from its move
		// parent's, and the time of the change, which clients interpolate from.
		unsafe { self.call(c"SetAbsOrigin", &mut [vector(&origin)]) }
	}

	/// Sets the entity's velocity in the world, in units per second
	/// (`SetAbsVelocity`), and its velocity relative to its move parent to
	/// match.
	///
	/// An entity that VPhysics moves takes its velocity from its physics
	/// object instead: set it with
	/// [`set_physics_velocity`](Self::set_physics_velocity). Fails with
	/// [`TfEntityError::NonFinite`] for a velocity that is not finite, before
	/// the game is called.
	#[doc(alias("SetAbsVelocity"))]
	pub fn set_abs_velocity(self, velocity: Vector) -> Result<(), TfEntityError> {
		if !velocity.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		let velocity = sys::Vector::from(velocity);

		// SAFETY: The member reads the finite velocity during the call, and
		// stores it, and the entity's local velocity, which it computes from
		// its move parent's.
		unsafe { self.call(c"SetAbsVelocity", &mut [vector(&velocity)]) }
	}

	/// Sets the entity's collision group (`SetCollisionGroup`), from which the
	/// game decides what it collides with, and rechecks its collisions if it
	/// changes.
	#[doc(alias("SetCollisionGroup", "m_CollisionGroup"))]
	pub fn set_collision_group(self, group: TfCollisionGroup) -> Result<(), TfEntityError> {
		// SAFETY: The member stores the group, and updates the entity's
		// collision rules if it changed.
		unsafe { self.call(c"SetCollisionGroup", &mut [int(group.to_raw())]) }
	}

	/// Sets how fast the entity's VPhysics object spins, about the entity's
	/// own x, y and z axes, in degrees per second (`SetPhysAngularVelocity`).
	///
	/// Fails as [`set_physics_velocity`](Self::set_physics_velocity) does.
	#[doc(alias("SetPhysAngularVelocity"))]
	pub fn set_physics_angular_velocity(self, velocity: Vector) -> Result<(), TfEntityError> {
		self.call_with_physics_velocity(c"SetPhysAngularVelocity", velocity)
	}

	/// Sets the velocity of the entity's VPhysics object in the world, in
	/// units per second (`SetPhysVelocity`), which moves an entity that
	/// VPhysics moves, such as a physics prop.
	///
	/// Fails with [`TfEntityError::NonFinite`] for a velocity that is not
	/// finite, and with [`TfEntityError::NoPhysicsObject`] for an entity
	/// without a physics object, before the game is called.
	#[doc(alias("SetPhysVelocity"))]
	pub fn set_physics_velocity(self, velocity: Vector) -> Result<(), TfEntityError> {
		self.call_with_physics_velocity(c"SetPhysVelocity", velocity)
	}

	/// Sets the entity's bounds, relative to its origin (`SetSize`): the
	/// collision box of an entity whose solid type is a box, and the box
	/// within which clients and triggers see it.
	///
	/// Fails with [`TfEntityError::NonFinite`] for bounds that are not finite,
	/// and with [`TfEntityError::InvalidBounds`] for a minimum above its
	/// maximum, for which the game would stop the server, before the game is
	/// called.
	#[doc(alias("SetSize", "UTIL_SetSize"))]
	pub fn set_size(self, mins: Vector, maxs: Vector) -> Result<(), TfEntityError> {
		if !mins.is_finite() || !maxs.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		if mins.cmpgt(*maxs).any() {
			return Err(TfEntityError::InvalidBounds);
		}

		let (mins, maxs) = (sys::Vector::from(mins), sys::Vector::from(maxs));

		// SAFETY: The member reads the finite, ordered bounds during the call,
		// and stores them in the entity's collision property.
		unsafe { self.call(c"SetSize", &mut [vector(&mins), vector(&maxs)]) }
	}

	/// Sets how the entity collides (`SetSolid`), such as by a box aligned with
	/// the world's axes, whose size [`set_size`](Self::set_size) sets.
	///
	/// The game rechecks what the entity collides with and touches, and ends its
	/// touches if it is no longer solid. A VPhysics object the entity already
	/// has is kept as it is, and none is made: an entity that spawns without
	/// one, such as a prop spawned [not solid](SolidType::None), collides by
	/// the new type alone.
	#[doc(alias("SetSolid", "m_nSolidType"))]
	pub fn set_solid(self, solid: SolidType) -> Result<(), TfEntityError> {
		// SAFETY: The member stores the solid type in the entity's collision
		// property, and updates its collision rules, partition and touches.
		unsafe { self.call(c"SetSolid", &mut [int(c_int::from(solid.to_raw()))]) }
	}

	/// Sets the entity's solid flags (`SetSolidFlags`): whether it is solid, a
	/// trigger, and how it collides.
	///
	/// When the change makes the entity solid or not, or a trigger or not, the
	/// game rechecks what it collides with and touches, and ends its touches
	/// if they no longer apply.
	#[doc(alias("SetSolidFlags", "m_usSolidFlags"))]
	pub fn set_solid_flags(self, flags: SolidFlags) -> Result<(), TfEntityError> {
		self.call_with_flags(c"SetSolidFlags", flags)
	}

	/// Damages the entity (`TakeDamageCustom`), as the game's own damage
	/// does: through its damage filter, the game rules, and its
	/// `OnTakeDamage`, which may kill it.
	///
	/// The entities `damage` names get [`ScriptInstance`]s if they have none,
	/// which give them script scopes too. Fails with
	/// [`TfEntityError::NonFinite`] for an amount, force or position that is
	/// not finite, before the game is called. The game's damage hooks, such as
	/// a plugin's, run within the call.
	#[doc(alias("TakeDamage", "TakeDamageCustom", "TakeDamageEx"))]
	pub fn take_damage(self, damage: Damage<'s>) -> Result<(), TfEntityError> {
		if !damage.amount.is_finite() || !damage.force.is_finite() || !damage.position.is_finite() {
			return Err(TfEntityError::NonFinite);
		}

		self.check_live()?;

		let attacker = damage.attacker.or(damage.inflictor).unwrap_or(self.entity);
		let inflictor = damage.inflictor.unwrap_or(attacker);
		let weapon = damage
			.weapon
			.map(|weapon| self.instance(weapon))
			.transpose()?;
		let (force, position) = (
			sys::Vector::from(damage.force),
			sys::Vector::from(damage.position),
		);

		let mut arguments = [
			handle(self.instance(inflictor)?),
			handle(self.instance(attacker)?),
			handle(weapon.unwrap_or(null_mut())),
			vector(&force),
			vector(&position),
			float(damage.amount),
			int(damage.damage_type.bits() as c_int),
			int(damage.custom),
		];

		// SAFETY: The member resolves the instances to their entities, a null
		// weapon to none, and deals the finite damage through the game's damage
		// path, which frees entities only through deferred deletion.
		unsafe { self.call(c"TakeDamageCustom", &mut arguments) }
	}
}

/// Why a native member of an entity could not be called.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TfEntityError {
	/// A member of the entity that the call checks could not be read.
	#[error(transparent)]
	Field(#[from] FieldError),

	/// A minimum of the bounds lies above its maximum.
	#[error("a minimum of the bounds lies above its maximum")]
	InvalidBounds,

	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The entity has no VPhysics object, which the call needs: to set the
	/// object's velocity, or to push an entity that VPhysics moves.
	#[error("the entity has no physics object")]
	NoPhysicsObject,

	/// A vector or amount given has a component that is NaN or infinite.
	#[error("a value given is NaN or infinite")]
	NonFinite,

	/// The server does not run TF2.
	#[error("entity natives require TF2")]
	NotTf2,

	/// The native binding of the member reported failure.
	#[error("the native method rejected its arguments")]
	Rejected,

	/// An entity the damage names has no script instance, and none could be
	/// made.
	#[error(transparent)]
	ScriptInstance(#[from] ScriptInstanceError),

	/// The entity's script class descriptors lack the member, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,
}

impl From<BindingError> for TfEntityError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}
