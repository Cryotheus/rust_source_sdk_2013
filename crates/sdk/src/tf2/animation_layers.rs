//! Animation layers: gestures that play over an animating entity's own
//! sequence, as a TF2 player's attacks play over their run.
//!
//! [`AnimationLayers::restart_gesture`] calls the game's own
//! `CBaseAnimatingOverlay::RestartGesture`, which [`AnimationLayers::new`]
//! finds in the game server module, as [`sdk_raw::tf2::animation_layers`]
//! describes. Only the entities whose networked class derives from
//! `DT_BaseAnimatingOverlay` have layers: among them the game's flex entities
//! (`funCBaseFlex`), NextBots, buildings and players, but not its props. The
//! entity's `StudioFrameAdvance`
//! ([`Animating::studio_frame_advance`](crate::tf2::animating::Animating::studio_frame_advance))
//! plays its layers on and removes those that finished, so call it as the
//! entity animates, unless the game does, as for its players and NextBots.
//!
//! Clients show the layers of an entity the server animates, but not those of
//! TF2's players, whose clients animate them on their own.
//!
//! # Unverified
//!
//! That clients show the layers of a `funCBaseFlex` has not been seen on a
//! live server.

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::tf2::animation_layers as raw;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The send table every entity with animation layers derives from.
const OVERLAY_TABLE: &CStr = c"DT_BaseAnimatingOverlay";

/// Why a gesture could not be played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnimationLayerError {
	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The entity has no animation layers: its networked class does not derive
	/// from `DT_BaseAnimatingOverlay`.
	#[error("the entity has no animation layers")]
	NoLayers,

	/// The game server module's `RestartGesture` was not found, as
	/// [`sdk_raw::tf2::animation_layers`] describes.
	#[error("the game's CBaseAnimatingOverlay::RestartGesture could not be found")]
	Unresolved,

	/// The server does not run TF2.
	#[error("animation layers require a TF2 server")]
	UnsupportedGame,
}

/// The game's animation layer functions, within the current engine callback.
#[derive(Debug, Clone, Copy)]
pub struct AnimationLayers<'s> {
	raw: raw::AnimationLayers,
	callback: PhantomData<Server<'s>>,
}

impl<'s> AnimationLayers<'s> {
	/// Finds `RestartGesture` in the game server module. Fails with
	/// [`AnimationLayerError::UnsupportedGame`] outside TF2, and with
	/// [`AnimationLayerError::Unresolved`] if it is not found.
	///
	/// Once a call has found it, later calls reuse it without inspecting the
	/// module again, as
	/// [`AnimationLayers::cached`](sdk_raw::tf2::animation_layers::AnimationLayers::cached)
	/// describes. This relies on Source never unloading that module while
	/// plugins are loaded. The first call inspects the whole module, which on
	/// Windows means scanning its code, so make it from a callback where that
	/// pause does not matter, such as the plugin's load or a level start.
	pub fn new(server: Server<'s>) -> Result<Self, AnimationLayerError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AnimationLayerError::UnsupportedGame);
		}

		// SAFETY: `Server::new` guarantees that the game server module, whose
		// factory this is, stays loaded through the callback. A cached resolution
		// for the same factory and module base was made in this same image: Source
		// never unloads the game server module while plugins are loaded, since
		// Metamod:Source and the engine unload plugins first, and the cache, a
		// static of this plugin, is unloaded with it.
		let raw = unsafe { raw::AnimationLayers::cached(server.game_server_factory().as_raw()) }
			.map_err(|_| AnimationLayerError::Unresolved)?;

		Ok(Self {
			raw,
			callback: PhantomData,
		})
	}

	/// Plays the gesture of `activity`, such as the one
	/// [`Animating::lookup_activity`](crate::tf2::animating::Animating::lookup_activity)
	/// finds for `ACT_MP_ATTACK_STAND_MELEE`, on `entity` from its start:
	/// restarts the layer that plays it already, or, if none does and
	/// `add_if_missing` is set, adds a layer for the sequence the entity's model
	/// picks for the activity, which `autokill` has fade out and be removed
	/// once it has played. A model without a sequence for the activity gets no
	/// layer, and the game says so in the console as a developer message. An
	/// entity has room for 15 layers.
	///
	/// Fails with [`AnimationLayerError::MarkedForDeletion`] or
	/// [`AnimationLayerError::NoLayers`] for such an `entity`.
	#[doc(alias("RestartGesture", "AddGesture"))]
	pub fn restart_gesture(
		self,
		entity: Entity<'s>,
		activity: c_int,
		add_if_missing: bool,
		autokill: bool,
	) -> Result<(), AnimationLayerError> {
		if entity.is_marked_for_deletion() {
			return Err(AnimationLayerError::MarkedForDeletion);
		}

		if !entity
			.server_class()
			.and_then(|class| class.table())
			.is_some_and(|table| table.derives_from_named(OVERLAY_TABLE))
		{
			return Err(AnimationLayerError::NoLayers);
		}

		// SAFETY: `Server::new` keeps the game server module, which this was
		// resolved in, loaded through the callback, which runs on the main thread,
		// and the entity is a live `CBaseAnimatingOverlay`, as its networked class
		// says, not marked for deletion.
		unsafe {
			self.raw.restart_gesture(
				NonNull::new(entity.as_ptr()).unwrap(),
				activity,
				add_if_missing,
				autokill,
			)
		};
		Ok(())
	}
}
