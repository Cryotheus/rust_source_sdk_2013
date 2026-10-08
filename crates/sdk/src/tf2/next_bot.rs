//! The components of TF2's NextBot actors: how they move (`ILocomotion`), the
//! body they aim and stand with (`IBody`), and what they see (`IVision`).
//!
//! Bots and the other NextBot actors, such as `base_boss`, skeletons and the
//! Halloween bosses, are each an `INextBot` (`game/server/NextBot`), which
//! [`NextBotInterface::of`] finds through the entity's `MyNextBotPointer`, and
//! [`NextBot::interface`](crate::tf2::bots::NextBot::interface) for an actor
//! already wrapped. Its components are called through the generated vtables
//! of their interfaces, so they reach the overrides of the actor's class:
//! `CTFBotLocomotion` for a bot and `NextBotGroundLocomotion` for most other
//! actors.
//!
//! # Moving an actor
//!
//! An actor's own behaviour moves it each tick, and its locomotion carries
//! out each movement once, on the actor's next update. To steer one,
//! [`Locomotion::approach`] its goal and [`Locomotion::face_towards`] it on
//! every tick, from a frame callback; requests from the actor's behaviour on
//! the same tick are averaged with the plugin's. [`Locomotion::drive_to`]
//! moves the actor at once, resolving its collisions on the way.
//!
//! # Lifetimes
//!
//! An actor owns its components and frees them with itself, which
//! [`Server::new`]'s contract defers past `'s`.
//!
//! # Unverified
//!
//! The wrappers follow the generated vtables and Valve's Source SDK 2013, and
//! have not been tested on a live server.

#[cfg(test)]
#[path = "../tests/tf2/next_bot.rs"]
mod tests;

use crate::entities::Entity;
use crate::math::Vector;
use crate::tf2::bots::BotError;
use crate::tf2::nav::NavArea;
use crate::tf2::teams::Team;
use crate::{Game, Server};
use sdk_raw::tf2::nav::TEAM_ANY;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int, c_uint};
use std::marker::PhantomData;
use std::ptr::{NonNull, null, null_mut};

/// Finds an entity's `INextBot`, or `None` if it is not a NextBot actor.
///
/// # Safety
///
/// The server must be running TF2, whose game DLL is built with `NEXT_BOT`,
/// as the generated vtable of `CBaseEntity` is.
pub(crate) unsafe fn next_bot_pointer(entity: Entity<'_>) -> Option<NonNull<sys::INextBot>> {
	let raw = entity.as_ptr();

	// SAFETY: The entity is live for its scope, and the caller vouches that its
	// vtable has the entry at the slot `sdk_raw::tf2::next_bot` reads from the
	// generated binding. The method returns the entity itself, adjusted to its
	// `INextBot` base, or null.
	NonNull::new(unsafe {
		vcall!(raw as sys::CBaseEntity__bindgen_vtable => CBaseEntity_MyNextBotPointer())
	})
}

/// Wraps an entity a component returned, or `None` for null.
///
/// # Safety
///
/// A non-null `raw` must point to a live entity of the server's.
unsafe fn entity_of<'s>(server: Server<'s>, raw: *mut sys::CBaseEntity) -> Option<Entity<'s>> {
	// SAFETY: The caller vouches for the entity.
	NonNull::new(raw).map(|raw| unsafe { Entity::from_live(server, raw) })
}

/// How a body stands (`IBody::PostureType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Posture {
	/// Standing (`STAND`).
	Stand,

	/// Crouching (`CROUCH`).
	Crouch,

	/// Sitting (`SIT`).
	Sit,

	/// Crawling (`CRAWL`).
	Crawl,

	/// Lying down (`LIE`).
	Lie,
}

impl Posture {
	/// Every posture, in native order.
	pub const ALL: [Self; 5] = [Self::Stand, Self::Crouch, Self::Sit, Self::Crawl, Self::Lie];

	/// The posture with this number, or `None` for any other.
	pub const fn from_raw(raw: sys::IBody_PostureType) -> Option<Self> {
		match raw {
			sys::IBody_PostureType_STAND => Some(Self::Stand),
			sys::IBody_PostureType_CROUCH => Some(Self::Crouch),
			sys::IBody_PostureType_SIT => Some(Self::Sit),
			sys::IBody_PostureType_CRAWL => Some(Self::Crawl),
			sys::IBody_PostureType_LIE => Some(Self::Lie),
			_ => None,
		}
	}

	/// The posture's number.
	pub const fn to_raw(self) -> sys::IBody_PostureType {
		match self {
			Self::Stand => sys::IBody_PostureType_STAND,
			Self::Crouch => sys::IBody_PostureType_CROUCH,
			Self::Sit => sys::IBody_PostureType_SIT,
			Self::Crawl => sys::IBody_PostureType_CRAWL,
			Self::Lie => sys::IBody_PostureType_LIE,
		}
	}
}

/// How excited a body is (`IBody::ArousalType`), which some actors' bodies
/// animate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arousal {
	/// Calm (`NEUTRAL`).
	Neutral,

	/// Alert (`ALERT`).
	Alert,

	/// In a fight (`INTENSE`).
	Intense,
}

impl Arousal {
	/// Every arousal level, in native order.
	pub const ALL: [Self; 3] = [Self::Neutral, Self::Alert, Self::Intense];

	/// The arousal level with this number, or `None` for any other.
	pub const fn from_raw(raw: sys::IBody_ArousalType) -> Option<Self> {
		match raw {
			sys::IBody_ArousalType_NEUTRAL => Some(Self::Neutral),
			sys::IBody_ArousalType_ALERT => Some(Self::Alert),
			sys::IBody_ArousalType_INTENSE => Some(Self::Intense),
			_ => None,
		}
	}

	/// The arousal level's number.
	pub const fn to_raw(self) -> sys::IBody_ArousalType {
		match self {
			Self::Neutral => sys::IBody_ArousalType_NEUTRAL,
			Self::Alert => sys::IBody_ArousalType_ALERT,
			Self::Intense => sys::IBody_ArousalType_INTENSE,
		}
	}
}

/// How much a request to aim a body's head matters
/// (`IBody::LookAtPriorityType`). A body ignores requests of lower priority
/// than the one it is carrying out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LookAtPriority {
	/// Idle looking around (`BORING`).
	Boring,

	/// Something worth a look, such as where an enemy was last seen
	/// (`INTERESTING`).
	Interesting,

	/// A danger (`IMPORTANT`).
	Important,

	/// An active threat (`CRITICAL`).
	Critical,

	/// Nothing interrupts it (`MANDATORY`).
	Mandatory,
}

impl LookAtPriority {
	/// The priority's number.
	pub const fn to_raw(self) -> sys::IBody_LookAtPriorityType {
		match self {
			Self::Boring => sys::IBody_LookAtPriorityType_BORING,
			Self::Interesting => sys::IBody_LookAtPriorityType_INTERESTING,
			Self::Important => sys::IBody_LookAtPriorityType_IMPORTANT,
			Self::Critical => sys::IBody_LookAtPriorityType_CRITICAL,
			Self::Mandatory => sys::IBody_LookAtPriorityType_MANDATORY,
		}
	}
}

/// The `IVision::FieldOfViewCheckType` for whether a check of sight counts
/// the field of view.
const fn field_of_view_check(use_field_of_view: bool) -> sys::IVision_FieldOfViewCheckType {
	if use_field_of_view {
		sys::IVision_FieldOfViewCheckType_USE_FOV
	} else {
		sys::IVision_FieldOfViewCheckType_DISREGARD_FOV
	}
}

/// A NextBot actor's `INextBot`, within the engine callback `'s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NextBotInterface<'s> {
	bot: NonNull<sys::INextBot>,
	_scope: PhantomData<&'s ()>,
}

impl<'s> NextBotInterface<'s> {
	/// Wraps an actor's `INextBot`.
	///
	/// # Safety
	///
	/// `bot` must point to the `INextBot` of a live NextBot actor of TF2's,
	/// which stays allocated for all of `'s`.
	pub const unsafe fn from_raw(bot: NonNull<sys::INextBot>) -> Self {
		Self {
			bot,
			_scope: PhantomData,
		}
	}

	/// The `INextBot` of an entity (`MyNextBotPointer`), or `None` if it is
	/// not a NextBot actor, as human players and most entities are not.
	/// Fails with [`BotError::UnsupportedGame`] unless the server runs TF2.
	#[doc(alias("MyNextBotPointer"))]
	pub fn of(server: Server<'s>, entity: Entity<'s>) -> Result<Option<Self>, BotError> {
		if server.game() != Game::TeamFortress2 {
			return Err(BotError::UnsupportedGame);
		}

		// SAFETY: The server runs TF2, and the actor frees its `INextBot` only
		// with itself, which `Server::new`'s contract defers past `'s`.
		Ok(unsafe { next_bot_pointer(entity) }.map(|bot| unsafe { Self::from_raw(bot) }))
	}

	/// The actor's body (`GetBodyInterface`).
	#[doc(alias("GetBodyInterface"))]
	pub fn body(self) -> Body<'s> {
		let bot = self.bot.as_ptr().cast_const();

		// SAFETY: The bot is live, and its vtable is laid out as the generated
		// binding's. Every NextBot actor has a body, which it frees only with
		// itself.
		let body =
			unsafe { vcall!(bot as sys::INextBot__bindgen_vtable => INextBot_GetBodyInterface()) };

		Body {
			body: NonNull::new(body).expect("NextBot actors always have a body"),
			_scope: PhantomData,
		}
	}

	/// The actor's entity (`GetEntity`).
	#[doc(alias("GetEntity"))]
	pub fn entity(self, server: Server<'s>) -> Entity<'s> {
		let bot = self.bot.as_ptr().cast_const();

		// SAFETY: As for `body`. The bot is its own entity.
		let entity =
			unsafe { vcall!(bot as sys::INextBot__bindgen_vtable => INextBot_GetEntity()) };

		// SAFETY: An actor's entity is live while the actor is.
		unsafe { entity_of(server, entity.cast()) }.expect("NextBot actors are entities")
	}

	/// The actor's locomotion (`GetLocomotionInterface`).
	#[doc(alias("GetLocomotionInterface"))]
	pub fn locomotion(self) -> Locomotion<'s> {
		let bot = self.bot.as_ptr().cast_const();

		// SAFETY: As for `body`, for the actor's locomotion.
		let locomotion = unsafe {
			vcall!(bot as sys::INextBot__bindgen_vtable => INextBot_GetLocomotionInterface())
		};

		Locomotion {
			locomotion: NonNull::new(locomotion).expect("NextBot actors always have a locomotion"),
			_scope: PhantomData,
		}
	}

	/// The actor's vision (`GetVisionInterface`).
	#[doc(alias("GetVisionInterface"))]
	pub fn vision(self) -> Vision<'s> {
		let bot = self.bot.as_ptr().cast_const();

		// SAFETY: As for `body`, for the actor's vision.
		let vision = unsafe {
			vcall!(bot as sys::INextBot__bindgen_vtable => INextBot_GetVisionInterface())
		};

		Vision {
			vision: NonNull::new(vision).expect("NextBot actors always have a vision"),
			_scope: PhantomData,
		}
	}

	/// The actor's raw pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::INextBot {
		self.bot.as_ptr()
	}
}

/// Calls a method of a component through its generated vtable.
macro_rules! component_call {
	($self:ident . $field:ident as $Vtable:ty => $method:ident($($argument:expr),* $(,)?)) => {{
		let this = $self.$field.as_ptr();

		// SAFETY: The component is live for `'s`, as the module documentation
		// describes, and its vtable is laid out as the generated binding's.
		// The method runs the actor's own code, on the main thread.
		unsafe { vcall!(this as $Vtable => $method($($argument),*)) }
	}};
}

/// Calls a method that returns a `const Vector &`, and copies the vector.
macro_rules! component_vector {
	($self:ident . $field:ident as $Vtable:ty => $method:ident()) => {{
		let vector = component_call!($self.$field as $Vtable => $method());

		// SAFETY: The method returns a reference to a vector the component or
		// a static holds, copied before anything can change it.
		Vector::from(unsafe { vector.read() })
	}};
}

/// How a NextBot actor moves (`ILocomotion`), within the engine callback
/// `'s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Locomotion<'s> {
	locomotion: NonNull<sys::ILocomotion>,
	_scope: PhantomData<&'s ()>,
}

impl<'s> Locomotion<'s> {
	/// Moves the actor towards `goal` on its next update (`Approach`), as one
	/// request among those made on the same tick, which are averaged by their
	/// `weight`. The [module documentation](self#moving-an-actor) explains
	/// how to steer an actor with it.
	#[doc(alias("Approach"))]
	pub fn approach(self, goal: Vector, weight: f32) {
		let goal = sys::Vector::from(goal);

		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_Approach(&raw const goal, weight));
	}

	/// Clears the actor's stuck status (`ClearStuckStatus`), giving `reason`
	/// to its debug output.
	#[doc(alias("ClearStuckStatus"))]
	pub fn clear_stuck_status(self, reason: &CStr) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_ClearStuckStatus(reason.as_ptr()));
	}

	/// The fall height that kills the actor (`GetDeathDropHeight`).
	#[doc(alias("GetDeathDropHeight"))]
	pub fn death_drop_height(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetDeathDropHeight())
	}

	/// The speed the actor tries to move at (`GetDesiredSpeed`).
	#[doc(alias("GetDesiredSpeed"))]
	pub fn desired_speed(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetDesiredSpeed())
	}

	/// Moves the actor to `position` at once (`DriveTo`), stopping short where
	/// it collides on the way.
	#[doc(alias("DriveTo"))]
	pub fn drive_to(self, position: Vector) {
		let position = sys::Vector::from(position);

		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_DriveTo(&raw const position));
	}

	/// Turns the actor towards `target` (`FaceTowards`), as far as its turn
	/// rate allows on its next update.
	#[doc(alias("FaceTowards"))]
	pub fn face_towards(self, target: Vector) {
		let target = sys::Vector::from(target);

		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_FaceTowards(&raw const target));
	}

	/// Where the actor touches the ground, which it moves by (`GetFeet`).
	#[doc(alias("GetFeet"))]
	pub fn feet(self) -> Vector {
		component_vector!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetFeet())
	}

	/// The entity the actor stands on, or `None` if it is in the air
	/// (`GetGround`).
	#[doc(alias("GetGround"))]
	pub fn ground(self, server: Server<'s>) -> Option<Entity<'s>> {
		let ground = component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetGround());

		// SAFETY: The ground is a live entity, or null.
		unsafe { entity_of(server, ground) }
	}

	/// The direction the actor moves in across the ground, as a unit vector in
	/// the x-y plane, which it keeps while it stands still
	/// (`GetGroundMotionVector`).
	#[doc(alias("GetGroundMotionVector"))]
	pub fn ground_motion_vector(self) -> Vector {
		component_vector!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetGroundMotionVector())
	}

	/// The normal of the ground the actor stands on (`GetGroundNormal`).
	#[doc(alias("GetGroundNormal"))]
	pub fn ground_normal(self) -> Vector {
		component_vector!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetGroundNormal())
	}

	/// The actor's speed in the x-y plane (`GetGroundSpeed`).
	#[doc(alias("GetGroundSpeed"))]
	pub fn ground_speed(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetGroundSpeed())
	}

	/// Whether the actor may path through `area` (`IsAreaTraversable`), as
	/// it is not blocked for the actor's team.
	#[doc(alias("IsAreaTraversable"))]
	pub fn is_area_traversable(self, area: NavArea<'_>) -> bool {
		let area = area.as_ptr().cast::<sys::CNavArea>().cast_const();

		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsAreaTraversable(area))
	}

	/// Whether the actor tried to move very recently (`IsAttemptingToMove`).
	#[doc(alias("IsAttemptingToMove"))]
	pub fn is_attempting_to_move(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsAttemptingToMove())
	}

	/// Whether the actor is jumping or climbing (`IsClimbingOrJumping`).
	#[doc(alias("IsClimbingOrJumping"))]
	pub fn is_climbing_or_jumping(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsClimbingOrJumping())
	}

	/// Whether the actor stands on something (`IsOnGround`).
	#[doc(alias("IsOnGround"))]
	pub fn is_on_ground(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsOnGround())
	}

	/// Whether the actor runs rather than walks (`IsRunning`).
	#[doc(alias("IsRunning"))]
	pub fn is_running(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsRunning())
	}

	/// Whether the actor has been trying to move without getting anywhere
	/// (`IsStuck`).
	#[doc(alias("IsStuck"))]
	pub fn is_stuck(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsStuck())
	}

	/// Whether the actor is on a ladder, or getting on or off one
	/// (`IsUsingLadder`).
	#[doc(alias("IsUsingLadder"))]
	pub fn is_using_ladder(self) -> bool {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_IsUsingLadder())
	}

	/// Makes the actor jump straight up (`Jump`). Actors that cannot jump,
	/// such as `base_boss`, ignore it.
	#[doc(alias("Jump"))]
	pub fn jump(self) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_Jump());
	}

	/// The actor's highest acceleration (`GetMaxAcceleration`).
	#[doc(alias("GetMaxAcceleration"))]
	pub fn max_acceleration(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetMaxAcceleration())
	}

	/// The actor's highest deceleration (`GetMaxDeceleration`).
	#[doc(alias("GetMaxDeceleration"))]
	pub fn max_deceleration(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetMaxDeceleration())
	}

	/// The highest the actor can jump (`GetMaxJumpHeight`).
	#[doc(alias("GetMaxJumpHeight"))]
	pub fn max_jump_height(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetMaxJumpHeight())
	}

	/// The direction the actor moves in, as a unit vector, which it keeps
	/// while it stands still (`GetMotionVector`).
	#[doc(alias("GetMotionVector"))]
	pub fn motion_vector(self) -> Vector {
		component_vector!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetMotionVector())
	}

	/// Makes the actor run (`Run`).
	#[doc(alias("Run"))]
	pub fn run(self) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_Run());
	}

	/// The actor's running speed (`GetRunSpeed`).
	#[doc(alias("GetRunSpeed"))]
	pub fn run_speed(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetRunSpeed())
	}

	/// Sets the speed the actor tries to move at (`SetDesiredSpeed`), which
	/// `run`, `walk` and `stop` also set.
	#[doc(alias("SetDesiredSpeed"))]
	pub fn set_desired_speed(self, speed: f32) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_SetDesiredSpeed(speed));
	}

	/// Sets the speed the actor never exceeds, whatever its desired speed
	/// (`SetSpeedLimit`).
	#[doc(alias("SetSpeedLimit"))]
	pub fn set_speed_limit(self, speed: f32) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_SetSpeedLimit(speed));
	}

	/// The actor's speed (`GetSpeed`).
	#[doc(alias("GetSpeed"))]
	pub fn speed(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetSpeed())
	}

	/// The speed the actor never exceeds (`GetSpeedLimit`).
	#[doc(alias("GetSpeedLimit"))]
	pub fn speed_limit(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetSpeedLimit())
	}

	/// The highest step the actor climbs without jumping (`GetStepHeight`).
	#[doc(alias("GetStepHeight"))]
	pub fn step_height(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetStepHeight())
	}

	/// Makes the actor stop (`Stop`).
	#[doc(alias("Stop"))]
	pub fn stop(self) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_Stop());
	}

	/// How long the actor has been stuck, in seconds (`GetStuckDuration`).
	#[doc(alias("GetStuckDuration"))]
	pub fn stuck_duration(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetStuckDuration())
	}

	/// The z of the unit normal of the steepest slope the actor walks up
	/// (`GetTraversableSlopeLimit`).
	#[doc(alias("GetTraversableSlopeLimit"))]
	pub fn traversable_slope_limit(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetTraversableSlopeLimit())
	}

	/// The actor's velocity (`GetVelocity`).
	#[doc(alias("GetVelocity"))]
	pub fn velocity(self) -> Vector {
		component_vector!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetVelocity())
	}

	/// Makes the actor walk (`Walk`).
	#[doc(alias("Walk"))]
	pub fn walk(self) {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_Walk());
	}

	/// The actor's walking speed (`GetWalkSpeed`).
	#[doc(alias("GetWalkSpeed"))]
	pub fn walk_speed(self) -> f32 {
		component_call!(self.locomotion as sys::ILocomotion__bindgen_vtable => ILocomotion_GetWalkSpeed())
	}

	/// The locomotion's raw pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::ILocomotion {
		self.locomotion.as_ptr()
	}
}

/// The body of a NextBot actor (`IBody`), within the engine callback `'s`:
/// where its head aims, how it stands, and its hull.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Body<'s> {
	body: NonNull<sys::IBody>,
	_scope: PhantomData<&'s ()>,
}

impl<'s> Body<'s> {
	/// Aims the actor's head at an entity, following it for `duration`
	/// seconds (`AimHeadTowards`), unless the head is carrying out a request
	/// of higher `priority`.
	#[doc(alias("AimHeadTowards"))]
	pub fn aim_head_at(self, subject: Entity<'_>, priority: LookAtPriority, duration: f32) {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_AimHeadTowards1(
			subject.as_ptr(),
			priority.to_raw(),
			duration,
			null_mut(),
			null(),
		));
	}

	/// Aims the actor's head at a position for `duration` seconds
	/// (`AimHeadTowards`), unless the head is carrying out a request of higher
	/// `priority`.
	#[doc(alias("AimHeadTowards"))]
	pub fn aim_head_towards(self, position: Vector, priority: LookAtPriority, duration: f32) {
		let position = sys::Vector::from(position);

		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_AimHeadTowards(
			&raw const position,
			priority.to_raw(),
			duration,
			null_mut(),
			null(),
		));
	}

	/// How excited the actor is, or `None` for a level the game does not
	/// define (`GetArousal`).
	#[doc(alias("GetArousal"))]
	pub fn arousal(self) -> Option<Arousal> {
		Arousal::from_raw(
			component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetArousal()),
		)
	}

	/// The posture the actor is in, or `None` for one the game does not define
	/// (`GetActualPosture`).
	#[doc(alias("GetActualPosture"))]
	pub fn actual_posture(self) -> Option<Posture> {
		Posture::from_raw(
			component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetActualPosture()),
		)
	}

	/// The collision group the actor moves with (`GetCollisionGroup`).
	#[doc(alias("GetCollisionGroup"))]
	pub fn collision_group(self) -> c_uint {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetCollisionGroup())
	}

	/// The height of the actor's hull while it crouches
	/// (`GetCrouchHullHeight`).
	#[doc(alias("GetCrouchHullHeight"))]
	pub fn crouch_hull_height(self) -> f32 {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetCrouchHullHeight())
	}

	/// The posture the actor is trying to take, or `None` for one the game
	/// does not define (`GetDesiredPosture`).
	#[doc(alias("GetDesiredPosture"))]
	pub fn desired_posture(self) -> Option<Posture> {
		Posture::from_raw(
			component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetDesiredPosture()),
		)
	}

	/// Where the actor sees from (`GetEyePosition`).
	#[doc(alias("GetEyePosition"))]
	pub fn eye_position(self) -> Vector {
		component_vector!(self.body as sys::IBody__bindgen_vtable => IBody_GetEyePosition())
	}

	/// The corners of the actor's hull in its current posture, relative to its
	/// feet (`GetHullMins` and `GetHullMaxs`).
	#[doc(alias("GetHullMins", "GetHullMaxs"))]
	pub fn hull(self) -> (Vector, Vector) {
		(
			component_vector!(self.body as sys::IBody__bindgen_vtable => IBody_GetHullMins()),
			component_vector!(self.body as sys::IBody__bindgen_vtable => IBody_GetHullMaxs()),
		)
	}

	/// The height of the actor's hull in its current posture
	/// (`GetHullHeight`).
	#[doc(alias("GetHullHeight"))]
	pub fn hull_height(self) -> f32 {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetHullHeight())
	}

	/// The width of the actor's hull in the x-y plane (`GetHullWidth`).
	#[doc(alias("GetHullWidth"))]
	pub fn hull_width(self) -> f32 {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetHullWidth())
	}

	/// Whether the actor's head has reached the target it was last asked to
	/// aim at (`IsHeadAimingOnTarget`).
	#[doc(alias("IsHeadAimingOnTarget"))]
	pub fn is_head_aiming_on_target(self) -> bool {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_IsHeadAimingOnTarget())
	}

	/// Whether the actor is in the posture it is trying to take
	/// (`IsInDesiredPosture`).
	#[doc(alias("IsInDesiredPosture"))]
	pub fn is_in_desired_posture(self) -> bool {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_IsInDesiredPosture())
	}

	/// Whether the actor's posture lets it move (`IsPostureMobile`).
	#[doc(alias("IsPostureMobile"))]
	pub fn is_posture_mobile(self) -> bool {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_IsPostureMobile())
	}

	/// Sets how excited the actor is (`SetArousal`). Bodies that do not
	/// animate it ignore it.
	#[doc(alias("SetArousal"))]
	pub fn set_arousal(self, arousal: Arousal) {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_SetArousal(arousal.to_raw()));
	}

	/// Asks the actor to take a posture (`SetDesiredPosture`). Bodies with
	/// one posture ignore it.
	#[doc(alias("SetDesiredPosture"))]
	pub fn set_desired_posture(self, posture: Posture) {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_SetDesiredPosture(posture.to_raw()));
	}

	/// The contents the actor's movement collides with (`GetSolidMask`).
	#[doc(alias("GetSolidMask"))]
	pub fn solid_mask(self) -> c_uint {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetSolidMask())
	}

	/// The height of the actor's hull while it stands (`GetStandHullHeight`).
	#[doc(alias("GetStandHullHeight"))]
	pub fn stand_hull_height(self) -> f32 {
		component_call!(self.body as sys::IBody__bindgen_vtable => IBody_GetStandHullHeight())
	}

	/// The direction the actor looks in, as a unit vector (`GetViewVector`).
	#[doc(alias("GetViewVector"))]
	pub fn view_vector(self) -> Vector {
		component_vector!(self.body as sys::IBody__bindgen_vtable => IBody_GetViewVector())
	}

	/// The body's raw pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IBody {
		self.body.as_ptr()
	}
}

/// What a NextBot actor sees (`IVision`), within the engine callback `'s`:
/// its sight checks, its field of view, and the entities it knows of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Vision<'s> {
	vision: NonNull<sys::IVision>,
	_scope: PhantomData<&'s ()>,
}

impl<'s> Vision<'s> {
	/// Makes the actor aware of an entity (`AddKnownEntity`), as though it
	/// had seen it.
	#[doc(alias("AddKnownEntity"))]
	pub fn add_known_entity(self, entity: Entity<'_>) {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_AddKnownEntity(entity.as_ptr()));
	}

	/// The actor's field of view, in degrees (`GetFieldOfView`).
	#[doc(alias("GetFieldOfView"))]
	pub fn field_of_view(self) -> f32 {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_GetFieldOfView())
	}

	/// Makes the actor forget every entity it knows of
	/// (`ForgetAllKnownEntities`).
	#[doc(alias("ForgetAllKnownEntities"))]
	pub fn forget_all_known_entities(self) {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_ForgetAllKnownEntities());
	}

	/// Makes the actor forget an entity (`ForgetEntity`).
	#[doc(alias("ForgetEntity"))]
	pub fn forget_entity(self, entity: Entity<'_>) {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_ForgetEntity(entity.as_ptr()));
	}

	/// Whether the actor can see an entity: in range, not hidden, and, if
	/// `use_field_of_view`, within its field of view (`IsAbleToSee`).
	#[doc(alias("IsAbleToSee"))]
	pub fn is_able_to_see(self, entity: Entity<'_>, use_field_of_view: bool) -> bool {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_IsAbleToSee(
			entity.as_ptr(),
			field_of_view_check(use_field_of_view),
			null_mut(),
		))
	}

	/// Whether the actor can see a position: in range, in a clear line of
	/// sight, and, if `use_field_of_view`, within its field of view
	/// (`IsAbleToSee`).
	#[doc(alias("IsAbleToSee"))]
	pub fn is_able_to_see_position(self, position: Vector, use_field_of_view: bool) -> bool {
		let position = sys::Vector::from(position);

		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_IsAbleToSee1(
			&raw const position,
			field_of_view_check(use_field_of_view),
		))
	}

	/// Whether a position is within the actor's field of view
	/// (`IsInFieldOfView`).
	#[doc(alias("IsInFieldOfView"))]
	pub fn is_in_field_of_view(self, position: Vector) -> bool {
		let position = sys::Vector::from(position);

		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_IsInFieldOfView(&raw const position))
	}

	/// Whether nothing blocks the actor's line of sight to a position
	/// (`IsLineOfSightClear`).
	#[doc(alias("IsLineOfSightClear"))]
	pub fn is_line_of_sight_clear(self, position: Vector) -> bool {
		let position = sys::Vector::from(position);

		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_IsLineOfSightClear(&raw const position))
	}

	/// How many known entities of `team`, or of any team for `None`, the
	/// actor knows of: only those it sees now if `only_visible`, and within
	/// `range` if given (`GetKnownCount`).
	#[doc(alias("GetKnownCount"))]
	pub fn known_count(self, team: Option<Team>, only_visible: bool, range: Option<f32>) -> c_int {
		let team = team.map_or(TEAM_ANY, Team::to_raw);

		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_GetKnownCount(
			team,
			only_visible,
			range.unwrap_or(-1.0),
		))
	}

	/// How far the actor sees (`GetMaxVisionRange`).
	#[doc(alias("GetMaxVisionRange"))]
	pub fn max_vision_range(self) -> f32 {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_GetMaxVisionRange())
	}

	/// The entity the actor counts as its most dangerous threat, among those
	/// it sees now if `only_visible`, or `None` (`GetPrimaryKnownThreat`).
	#[doc(alias("GetPrimaryKnownThreat"))]
	pub fn primary_known_threat(
		self,
		server: Server<'s>,
		only_visible: bool,
	) -> Option<Entity<'s>> {
		let known = component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_GetPrimaryKnownThreat(only_visible));

		if known.is_null() {
			return None;
		}

		// SAFETY: The known entity is one the vision holds, whose vtable is
		// laid out as the generated binding's. The method resolves its handle.
		let entity = unsafe {
			vcall!(known as sys::CKnownEntity__bindgen_vtable => CKnownEntity_GetEntity())
		};

		// SAFETY: A resolved handle is a live entity, or null.
		unsafe { entity_of(server, entity) }
	}

	/// Sets the actor's field of view, in degrees (`SetFieldOfView`).
	#[doc(alias("SetFieldOfView"))]
	pub fn set_field_of_view(self, degrees: f32) {
		component_call!(self.vision as sys::IVision__bindgen_vtable => IVision_SetFieldOfView(degrees));
	}

	/// The vision's raw pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IVision {
		self.vision.as_ptr()
	}
}
