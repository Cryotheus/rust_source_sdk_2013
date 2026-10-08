//! Server-side ragdolls: `prop_ragdoll`s whose physics the server simulates,
//! made from an animating entity's current pose, as other Source games make
//! their NPCs' corpses.
//!
//! [`ServerRagdolls::create_server_ragdoll`] calls the game's own
//! `CreateServerRagdoll` (`game/server/physics_prop_ragdoll.cpp`), which TF2
//! does not export, and which [`ServerRagdolls::new`] finds in the game server
//! module, as [`sdk_raw::tf2::ragdolls`] describes. The ragdoll takes the
//! entity's model, skin, body groups and pose, and its velocity, before a
//! death's damage force pushes it.
//!
//! # Lifetime
//!
//! The ragdolls are made without the game's retirement of old ragdolls, so
//! each stays until it is removed: by [`ServerTools::remove`], by the round's
//! cleanup of the map's entities, or by a level change. Each holds an edict
//! for as long, and a ragdoll that falls out of the world never comes to rest.
//! Running out of edicts ends the server with a fatal error, so callers that
//! keep ragdolls should budget them against
//! [`ValveEngine::entity_count`](crate::interfaces::ValveEngine::entity_count).
//!
//! Clients fade a ragdoll out with distance as its `fadescale` key value says,
//! which the game sets to 1. A key value of 0 keeps it drawn at any distance.
//!
//! # TF2's players
//!
//! A TF2 player becomes a `tf_ragdoll` instead, from which each client makes
//! its own ragdoll or gibs, with the effects of the death such as burning or
//! decapitation. A server ragdoll of a player has none of these, nor the
//! player's cosmetics. The game's client ragdoll is not removed: remove the
//! player's `tf_ragdoll` (`m_hRagdoll`) to leave only the server ragdoll, but
//! never point `m_hRagdoll` at a server ragdoll, which TF2's clients take to
//! be a `tf_ragdoll` unchecked.
//!
//! [`TfRagdoll`] reads a `tf_ragdoll`, and sets its [`RagdollFlag`]s before
//! clients make their ragdoll from it.
//! [`TfPlayer::ragdoll`](crate::tf2::player::TfPlayer::ragdoll) finds a
//! player's, and
//! [`TfPlayer::remove_ragdoll`](crate::tf2::player::TfPlayer::remove_ragdoll)
//! removes it.

use crate::datatables::{NetProp, NetPropError, NetValue, NetVar};
use crate::entities::{CollisionGroup, Entity};
use crate::inputs::{InputError, InputValue};
use crate::interfaces::ServerTools;
use crate::math::Vector;
use crate::tf2::PlayerClass;
use crate::tf2::damage::{DamageInfo, DamageType};
use crate::tf2::teams::Team;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::damage::DMG_VEHICLE;
use sdk_raw::tf2::ragdolls::{self as raw, RAGDOLL_MAX_ELEMENTS};
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

/// The input that shows a `prop_ragdoll` (`CRagdollProp::InputTurnOn`), by
/// clearing `EF_NODRAW`, which also has the engine send it to clients again.
const ENABLE_INPUT: &CStr = c"Enable";

/// Why a server ragdoll could not be created.
#[derive(Debug, thiserror::Error)]
pub enum RagdollError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The entity's model index could not be read.
	#[error(transparent)]
	ModelIndex(#[from] NetPropError),

	/// The entity's model is not a studio model with a collision model of 1 to
	/// [`RAGDOLL_MAX_ELEMENTS`] solids, from which the game would make a
	/// ragdoll without physics.
	#[error("the entity's model has no collision model a ragdoll can be made from")]
	NoCollisionModel,

	/// The entity is not a `CBaseAnimating`: its networked class does not
	/// derive from `DT_BaseAnimating`.
	#[error("the entity is not an animating entity")]
	NotAnimating,

	/// `CreateServerRagdoll` returned no ragdoll.
	#[error("the game created no ragdoll")]
	NotCreated,

	/// The new ragdoll refused the input that shows it, so it was removed
	/// again.
	#[error("the new ragdoll could not be shown")]
	NotShown(#[source] InputError),

	/// The game server module's `CreateServerRagdoll` was not found, as
	/// [`sdk_raw::tf2::ragdolls`] describes.
	#[error("the game's CreateServerRagdoll could not be found")]
	Unresolved,

	/// The server does not run TF2.
	#[error("server ragdolls require a TF2 server")]
	UnsupportedGame,
}

/// A flag of a [`TfRagdoll`], which decides how clients make the ragdoll of a
/// player's death.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RagdollFlag {
	/// Burns away to ash, as the victims of some of the Pyro's weapons do
	/// (`m_bBecomeAsh`).
	#[doc(alias("m_bBecomeAsh"))]
	BecomeAsh,

	/// Burns (`m_bBurning`).
	#[doc(alias("m_bBurning"))]
	Burning,

	/// Fades out as a cloaked Spy does (`m_bCloaked`).
	#[doc(alias("m_bCloaked"))]
	Cloaked,

	/// Bursts in a red mist as it gibs, as the victims of the Classic's
	/// critical hits do (`m_bCritOnHardHit`).
	#[doc(alias("m_bCritOnHardHit"))]
	CritOnHardHit,

	/// Crackles with electricity (`m_bElectrocuted`).
	#[doc(alias("m_bElectrocuted"))]
	Electrocuted,

	/// A Spy's feigned death (`m_bFeignDeath`).
	#[doc(alias("m_bFeignDeath"))]
	FeignDeath,

	/// Bursts into gibs instead of a ragdoll (`m_bGib`).
	#[doc(alias("m_bGib"))]
	Gib,

	/// Turns to gold, as the victims of golden weapons do (`m_bGoldRagdoll`).
	#[doc(alias("m_bGoldRagdoll"))]
	Gold,

	/// Freezes into an ice statue, as the Spy-cicle's victims do
	/// (`m_bIceRagdoll`).
	#[doc(alias("m_bIceRagdoll"))]
	Ice,

	/// The player died on the ground (`m_bOnGround`). Clients play most death
	/// animations only for such a ragdoll.
	#[doc(alias("m_bOnGround"))]
	OnGround,

	/// The player was disguised (`m_bWasDisguised`), so a feigned death shows
	/// the disguise.
	#[doc(alias("m_bWasDisguised"))]
	WasDisguised,
}

impl RagdollFlag {
	/// Every flag.
	pub const ALL: [Self; 11] = [
		Self::BecomeAsh,
		Self::Burning,
		Self::Cloaked,
		Self::CritOnHardHit,
		Self::Electrocuted,
		Self::FeignDeath,
		Self::Gib,
		Self::Gold,
		Self::Ice,
		Self::OnGround,
		Self::WasDisguised,
	];

	/// The name of the flag's networked variable.
	pub const fn net_prop_name(self) -> &'static CStr {
		match self {
			Self::BecomeAsh => c"m_bBecomeAsh",
			Self::Burning => c"m_bBurning",
			Self::Cloaked => c"m_bCloaked",
			Self::CritOnHardHit => c"m_bCritOnHardHit",
			Self::Electrocuted => c"m_bElectrocuted",
			Self::FeignDeath => c"m_bFeignDeath",
			Self::Gib => c"m_bGib",
			Self::Gold => c"m_bGoldRagdoll",
			Self::Ice => c"m_bIceRagdoll",
			Self::OnGround => c"m_bOnGround",
			Self::WasDisguised => c"m_bWasDisguised",
		}
	}
}

/// TF2's `CreateServerRagdoll`, within the current engine callback.
#[doc(alias("CreateServerRagdoll"))]
#[derive(Debug, Clone, Copy)]
pub struct ServerRagdolls<'s> {
	raw: raw::ServerRagdolls,
	server: Server<'s>,
}

impl<'s> ServerRagdolls<'s> {
	/// Finds `CreateServerRagdoll` in the game server module. Fails with
	/// [`RagdollError::UnsupportedGame`] outside TF2, and with
	/// [`RagdollError::Unresolved`] if it is not found.
	///
	/// Once a call has found it, later calls reuse it without inspecting the
	/// module again, as
	/// [`ServerRagdolls::cached`](sdk_raw::tf2::ragdolls::ServerRagdolls::cached)
	/// describes. This relies on Source never unloading that module while
	/// plugins are loaded. The first call inspects the whole module, which on
	/// Windows means scanning its code, so make it from a callback where that
	/// pause does not matter, such as the plugin's load or a level start.
	pub fn new(server: Server<'s>) -> Result<Self, RagdollError> {
		if server.game() != Game::TeamFortress2 {
			return Err(RagdollError::UnsupportedGame);
		}

		// SAFETY: `Server::new` guarantees that the game server module, whose
		// factory this is, stays loaded through the callback. A cached resolution
		// for the same factory and module base was made in this same image: Source
		// never unloads the game server module while plugins are loaded, since
		// Metamod:Source and the engine unload plugins first, and the cache, a
		// static of this plugin, is unloaded with it.
		let raw = unsafe { raw::ServerRagdolls::cached(server.game_server_factory().as_raw()) }
			.map_err(|_| RagdollError::Unresolved)?;

		Ok(Self { raw, server })
	}

	/// Creates a permanent server ragdoll of `animating` in its current pose,
	/// pushed by `info`'s damage force, and shows it.
	///
	/// The force is applied to the physics object of `force_bone`, a solid of
	/// the model's collision model such as a dead player's `m_nForceBone`, and
	/// spread over the others from that object's position. With `None`, or a
	/// bone the collision model does not have, it is spread over every object
	/// from `info`'s damage position, unless that is the origin, which leaves
	/// the ragdoll unpushed. `info`'s `DMG_VEHICLE` bit is ignored, which would
	/// take a path of the game's meant for its vehicles and NPCs.
	///
	/// The ragdoll is put in `group`, such as [`CollisionGroup::Debris`], with
	/// which players, bullets, projectiles and other ragdolls pass through it,
	/// and has `animating` as its owner. It copies `animating`'s effects, such
	/// as a dead TF2 player's `EF_NODRAW`, which would keep the engine from
	/// sending it to clients, so this sends it the `Enable` input, which clears
	/// `EF_NODRAW`: the ragdoll is always shown. If it refuses the input, it is
	/// removed again and [`RagdollError::NotShown`] returned.
	///
	/// Fails with [`RagdollError::MarkedForDeletion`] or
	/// [`RagdollError::NotAnimating`] for such an `animating`, and with
	/// [`RagdollError::NoCollisionModel`] unless its model is a studio model
	/// whose collision model has 1 to [`RAGDOLL_MAX_ELEMENTS`] solids. The
	/// [module documentation](self#lifetime) describes how long the ragdoll
	/// stays.
	#[doc(alias("CreateServerRagdoll"))]
	pub fn create_server_ragdoll(
		self,
		animating: Entity<'s>,
		force_bone: Option<usize>,
		info: &DamageInfo,
		group: CollisionGroup,
	) -> Result<Entity<'s>, RagdollError> {
		if animating.is_marked_for_deletion() {
			return Err(RagdollError::MarkedForDeletion);
		}

		if !animating
			.server_class()
			.and_then(|class| class.table())
			.is_some_and(|table| table.derives_from_named(c"DT_BaseAnimating"))
		{
			return Err(RagdollError::NotAnimating);
		}

		let tools = self.server.server_tools()?;
		let model_info = self.server.model_info()?;

		let NetValue::Int(model) = self
			.server
			.server_game_dll()?
			.entity_net_prop(animating, c"m_nModelIndex")?
			.value(animating)?
		else {
			return Err(RagdollError::NoCollisionModel);
		};

		// The game reads the bones of the studio model, and makes a physics
		// object for each solid of its collision model.
		let solids = model_info
			.vcollide_solid_count(model)
			.filter(|solids| (1..=RAGDOLL_MAX_ELEMENTS).contains(solids))
			.filter(|_| model_info.studio_name(model).is_some())
			.ok_or(RagdollError::NoCollisionModel)?;

		let force_bone = force_bone
			.and_then(|bone| c_int::try_from(bone).ok())
			.filter(|bone| (0..solids).contains(bone))
			.unwrap_or(raw::NO_FORCE_BONE);

		let mut info = info.clone();

		info.set_damage_type(info.damage_type() - DamageType::from_bits_retain(DMG_VEHICLE as u32));

		// SAFETY: The game server module stays loaded through the callback, which
		// runs on the main thread (conditions 1 and 3 of `Server::new`), and the
		// game creates entities in plugin callbacks, none of which is known to
		// run within VPhysics' simulation or its callbacks, from which the game
		// defers its damage and removals. `animating` is live during `'s`, not
		// marked for deletion, and a `CBaseAnimating`, whose networked class
		// derives from `DT_BaseAnimating`, at the address of its `CBaseEntity`,
		// as `sdk_raw::tf2` asserts for the bases of TF2's players. Its model is
		// a studio model with 1 to `RAGDOLL_MAX_ELEMENTS` solids. The damage copy
		// lives through the call. `CreateServerRagdoll` creates the ragdoll
		// without spawning it (`CreateNoSpawn`), copies the model and effects,
		// sets its owner, sets up `animating`'s bones twice, creates the physics
		// objects (`InitRagdoll`), creates and spawns an `env_entity_dissolver`
		// if `animating` is dissolving, and sets the collision bounds. None of
		// this frees an entity, so condition 4 holds.
		let ragdoll = unsafe {
			self.raw.create(
				NonNull::new(animating.as_ptr()).unwrap().cast(),
				force_bone,
				NonNull::new(info.as_ptr().cast_mut()).unwrap(),
				group.to_raw(),
				false,
			)
		}
		.ok_or(RagdollError::NotCreated)?;

		// SAFETY: The game added the new ragdoll to the entity list, and frees it
		// only through deferred deletion, after `'s`.
		let ragdoll = unsafe { Entity::from_raw(ragdoll) };

		show(tools, ragdoll)?;

		Ok(ragdoll)
	}
}

/// What a TF2 player's death leaves for clients to make a ragdoll or gibs
/// from (`tf_ragdoll`, `CTFRagdoll`), within one engine callback.
///
/// The game creates one as a player dies, which clients make their own
/// ragdoll from when they first receive it, and which the player's
/// [`ragdoll`](crate::tf2::player::TfPlayer::ragdoll) refers to until their
/// next death. Clients read it once, so changing it only matters before it
/// is first sent to them, in the frame of the death, such as from a hook of
/// the death that runs after the game's.
#[doc(alias("tf_ragdoll", "CTFRagdoll"))]
#[derive(Debug, Clone, Copy)]
pub struct TfRagdoll<'s> {
	server: Server<'s>,
	ragdoll: Entity<'s>,
}

impl<'s> TfRagdoll<'s> {
	/// Wraps a player's ragdoll, or returns [`TfRagdollError::NotTfRagdoll`]
	/// unless the server runs TF2 and `ragdoll`'s server class is
	/// `CTFRagdoll`.
	pub fn new(server: Server<'s>, ragdoll: Entity<'s>) -> Result<Self, TfRagdollError> {
		if server.game() != Game::TeamFortress2
			|| !ragdoll
				.server_class()
				.is_some_and(|class| class.name() == c"CTFRagdoll")
		{
			return Err(TfRagdollError::NotTfRagdoll);
		}

		Ok(Self { server, ragdoll })
	}

	/// The class the player died as (`m_iClass`), or `None` for
	/// `TF_CLASS_UNDEFINED`.
	///
	/// Fails with [`TfRagdollError::UnknownClass`] for any other value that is
	/// no playable class.
	#[doc(alias("m_iClass"))]
	pub fn class(self) -> Result<Option<PlayerClass>, TfRagdollError> {
		match self.get::<c_int>(c"m_iClass")? {
			0 => Ok(None),

			raw => PlayerClass::from_raw(raw)
				.map(Some)
				.ok_or(TfRagdollError::UnknownClass(raw)),
		}
	}

	/// The kind of the death's damage (`m_iDamageCustom`), a `TF_DMG_CUSTOM_*`
	/// value such as a headshot's, from which clients choose the death
	/// animation. The game clears it when the death animation should stop.
	#[doc(alias("m_iDamageCustom"))]
	pub fn damage_custom(self) -> Result<c_int, TfRagdollError> {
		self.get(c"m_iDamageCustom")
	}

	/// The ragdoll's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.ragdoll
	}

	/// Whether the ragdoll has `flag`.
	pub fn flag(self, flag: RagdollFlag) -> Result<bool, TfRagdollError> {
		self.get(flag.net_prop_name())
	}

	/// The damage force of the death (`m_vecForce`), with which clients push
	/// the ragdoll.
	#[doc(alias("m_vecForce"))]
	pub fn force(self) -> Result<Vector, TfRagdollError> {
		self.get(c"m_vecForce")
	}

	/// The bone the death's force pushes (`m_nForceBone`).
	#[doc(alias("m_nForceBone"))]
	pub fn force_bone(self) -> Result<c_int, TfRagdollError> {
		self.get(c"m_nForceBone")
	}

	/// Reads one of the ragdoll's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, TfRagdollError> {
		Ok(self.net_prop(name)?.get(self.ragdoll)?)
	}

	/// How much the player's hands were scaled (`m_flHandScale`), as some
	/// conditions and taunts scale them.
	#[doc(alias("m_flHandScale"))]
	pub fn hand_scale(self) -> Result<f32, TfRagdollError> {
		self.get(c"m_flHandScale")
	}

	/// How much the player's head was scaled (`m_flHeadScale`).
	#[doc(alias("m_flHeadScale"))]
	pub fn head_scale(self) -> Result<f32, TfRagdollError> {
		self.get(c"m_flHeadScale")
	}

	/// Resolves one of the ragdoll's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, TfRagdollError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.ragdoll, name)?)
	}

	/// Where the player died (`m_vecRagdollOrigin`).
	#[doc(alias("m_vecRagdollOrigin"))]
	pub fn origin(self) -> Result<Vector, TfRagdollError> {
		self.get(c"m_vecRagdollOrigin")
	}

	/// The player who died (`m_hPlayer`), or `None` if they no longer exist.
	#[doc(alias("m_hPlayer"))]
	pub fn player(self) -> Result<Option<Entity<'s>>, TfRagdollError> {
		let handle = self.net_prop(c"m_hPlayer")?.get_handle(self.ragdoll)?;

		Ok(self.server.server_tools()?.entity_by_handle(handle))
	}

	/// Sets or clears one of the ragdoll's flags, and records the change so
	/// the engine sends it to clients. Clients read the flags once, as they
	/// make the ragdoll, so set them before the ragdoll is first sent, in the
	/// frame of the death.
	///
	/// Fails with [`TfRagdollError::MarkedForDeletion`] for a ragdoll marked
	/// for deletion.
	pub fn set_flag(self, flag: RagdollFlag, value: bool) -> Result<(), TfRagdollError> {
		if self.ragdoll.is_marked_for_deletion() {
			return Err(TfRagdollError::MarkedForDeletion);
		}

		let engine = self.server.valve_engine()?;

		// SAFETY: The game sets each flag either way itself, and clients
		// combine any of them.
		unsafe {
			self.net_prop(flag.net_prop_name())?
				.set(engine, self.ragdoll, value)
		}?;

		Ok(())
	}

	/// The player's team as they died (`m_iTeam`).
	///
	/// Fails with [`TfRagdollError::UnknownTeam`] for a value that is not one
	/// of TF2's teams.
	#[doc(alias("m_iTeam"))]
	pub fn team(self) -> Result<Team, TfRagdollError> {
		let raw = self.get::<c_int>(c"m_iTeam")?;

		Team::from_raw(raw).ok_or(TfRagdollError::UnknownTeam(raw))
	}

	/// How much the player's torso was scaled (`m_flTorsoScale`).
	#[doc(alias("m_flTorsoScale"))]
	pub fn torso_scale(self) -> Result<f32, TfRagdollError> {
		self.get(c"m_flTorsoScale")
	}

	/// The player's velocity as they died (`m_vecRagdollVelocity`), which
	/// clients receive clamped to 2048 units per second in each axis.
	#[doc(alias("m_vecRagdollVelocity"))]
	pub fn velocity(self) -> Result<Vector, TfRagdollError> {
		self.get(c"m_vecRagdollVelocity")
	}
}

/// Why a [`TfRagdoll`] could not be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum TfRagdollError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The ragdoll is marked for deletion.
	#[error("the ragdoll is marked for deletion")]
	MarkedForDeletion,

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The entity is not a TF2 player's ragdoll, or the server does not run
	/// TF2.
	#[error("the entity is not a tf_ragdoll")]
	NotTfRagdoll,

	/// The ragdoll's `m_iClass` holds neither `TF_CLASS_UNDEFINED` nor a
	/// playable class.
	#[error("the ragdoll's class {0} is not one of TF2's playable classes")]
	UnknownClass(c_int),

	/// The ragdoll's `m_iTeam` is not one of TF2's teams.
	#[error("the ragdoll's team {0} is not one of TF2's")]
	UnknownTeam(c_int),
}

/// Sends `ragdoll` the input that shows it, or removes it if it refuses.
fn show(tools: ServerTools<'_>, ragdoll: Entity<'_>) -> Result<(), RagdollError> {
	if let Err(error) =
		tools.accept_input(ragdoll, ENABLE_INPUT, InputValue::Void, ragdoll, ragdoll)
	{
		// A ragdoll is not one of the entities `remove` protects.
		let _ = tools.remove(ragdoll);

		return Err(RagdollError::NotShown(error));
	}

	Ok(())
}
