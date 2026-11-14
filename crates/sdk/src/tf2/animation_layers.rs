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
//! [`AnimationLayers::hold_gesture`] holds a gesture still at a chosen point,
//! as a pose: it finds the layer through the entity's datamap
//! (`m_AnimOverlay`) and stops it advancing and being removed.
//! [`AnimationLayers::set_gesture_playback_rate`] finds it the same way and
//! changes how fast it plays.
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

const _: () = assert!(size_of::<Layer>() == 0x50);
const _: () = assert!(std::mem::offset_of!(Layer, activity) == 0x34);

/// The offset of the layer count in the layers' `CUtlVector`, after its
/// `CUtlMemory` (a pointer and two `int`s).
const COUNT_OFFSET: usize = 16;

/// `ANIM_LAYER_ACTIVE`: the layer plays.
const LAYER_ACTIVE: c_int = 0x1;

/// `ANIM_LAYER_AUTOKILL`: the layer is removed once it has played.
const LAYER_AUTOKILL: c_int = 0x2;

/// `ANIM_LAYER_DYING`: the layer fades out to be removed.
const LAYER_DYING: c_int = 0x20;

/// `ANIM_LAYER_KILLME`: the layer is being removed.
const LAYER_KILLME: c_int = 0x4;

/// The most layers an entity is trusted to have.
const MAX_LAYERS: usize = 15;

/// The datamap member holding an entity's layers, a
/// `CUtlVector<CAnimationLayer>`: its elements' pointer, then its allocation
/// count and grow size, then its count at [`COUNT_OFFSET`].
const OVERLAY_MEMBER: &CStr = c"m_AnimOverlay";

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

	/// The model has no gesture for the activity, so no layer plays it.
	#[error("the model has no gesture for the activity")]
	NoGesture,

	/// A layer's playback rate must be finite and nonnegative.
	#[error("a layer's playback rate must be finite and nonnegative")]
	InvalidPlaybackRate,

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

	/// The active layer of `entity`, not being removed, that plays a gesture
	/// of `activity`, as `RestartGesture` looks for one.
	fn gesture_layer(
		self,
		entity: Entity<'s>,
		activity: c_int,
	) -> Result<Option<NonNull<Layer>>, AnimationLayerError> {
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

		let offset = entity
			.data_member_offset(OVERLAY_MEMBER)
			.ok_or(AnimationLayerError::NoLayers)?;

		// SAFETY: The entity is a live `CBaseAnimatingOverlay`, as its networked
		// class says, whose datamap places its layers' `CUtlVector` at `offset`:
		// the elements' pointer first, the count at `COUNT_OFFSET`. They are read
		// on the main thread, where the game changes them.
		let (layers, count) = unsafe {
			let vector = entity.as_ptr().byte_add(offset);
			(
				vector.cast::<*mut Layer>().read(),
				vector.byte_add(COUNT_OFFSET).cast::<c_int>().read(),
			)
		};

		let count = usize::try_from(count).unwrap_or(0).min(MAX_LAYERS);
		let Some(layers) = NonNull::new(layers) else {
			return Ok(None);
		};

		Ok((0..count)
			.map(|index| {
				// SAFETY: The vector holds `count` layers from `layers`.
				unsafe { layers.add(index) }
			})
			.find(|layer| {
				// SAFETY: As above, the layer is live.
				let layer = unsafe { layer.as_ref() };

				layer.flags & LAYER_ACTIVE != 0
					&& layer.flags & (LAYER_KILLME | LAYER_DYING) == 0
					&& layer.activity == activity
			}))
	}

	/// Holds the gesture of `activity` on `entity` still at `cycle`, from 0 to
	/// 1, at full weight: adds it as [`Self::restart_gesture`] does if no
	/// layer plays it, then stops its layer advancing, fading, and being
	/// removed once played, and sets its cycle. Call it as the entity
	/// animates: the game may remove the layer, as TF2's buildings remove
	/// every gesture as they start upgrading (`RemoveAllGestures`), and a
	/// layer being removed is left to fade while a new one is added. The
	/// playback rate is not networked, so clients are told of a change only.
	///
	/// Fails as [`Self::restart_gesture`] does, and with
	/// [`AnimationLayerError::NoGesture`] if the model has no gesture for the
	/// activity.
	pub fn hold_gesture(
		self,
		server: Server<'s>,
		entity: Entity<'s>,
		activity: c_int,
		cycle: f32,
	) -> Result<(), AnimationLayerError> {
		let cycle = cycle.clamp(0.0, 1.0);
		let layer = match self.gesture_layer(entity, activity)? {
			Some(layer) => layer,

			None => {
				self.restart_gesture(entity, activity, true, false)?;
				self.gesture_layer(entity, activity)?
					.ok_or(AnimationLayerError::NoGesture)?
			}
		};

		// SAFETY: `gesture_layer` found the layer among the live entity's
		// layers, laid out as `Layer`, on the main thread, and nothing else
		// holds a reference to it during this call. Fields are written through
		// the pointer, as the game writes them.
		let changed = unsafe {
			let layer = layer.as_ptr();
			let changed = (*layer).cycle != cycle
				|| (*layer).prev_cycle != cycle
				|| (*layer).weight != 1.0
				|| (*layer).playback_rate != 0.0
				|| (*layer).flags & LAYER_AUTOKILL != 0;

			(*layer).flags &= !LAYER_AUTOKILL;
			(*layer).sequence_finished = false;
			(*layer).cycle = cycle;
			(*layer).prev_cycle = cycle;
			(*layer).weight = 1.0;
			(*layer).playback_rate = 0.0;
			(*layer).blend_in = 0.0;
			(*layer).blend_out = 0.0;
			changed
		};

		if changed
			&& let Some(edict) = entity.edict()
			&& let Ok(engine) = server.valve_engine()
		{
			edict.full_state_changed(engine);
		}

		Ok(())
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

	/// Sets the playback rate of the layer that plays the gesture of
	/// `activity` on `entity`, if one does, and says whether one did: at 0.5
	/// it plays at half speed, and a layer the game added with autokill takes
	/// twice as long to be removed. The game sets a layer's rate as it adds the
	/// layer, usually to 1, and leaves it: a restarted layer keeps its rate,
	/// but a gesture the game adds again gets a new layer, so call it as the
	/// entity animates. The rate is not networked; clients play a layer the
	/// server animates from the cycles it sends, so they see the new speed.
	///
	/// Fails with [`AnimationLayerError::InvalidPlaybackRate`] for a negative or
	/// non-finite `rate`, and with [`AnimationLayerError::MarkedForDeletion`] or
	/// [`AnimationLayerError::NoLayers`] for such an `entity`.
	#[doc(alias("m_flPlaybackRate", "SetLayerPlaybackRate"))]
	pub fn set_gesture_playback_rate(
		self,
		entity: Entity<'s>,
		activity: c_int,
		rate: f32,
	) -> Result<bool, AnimationLayerError> {
		if !rate.is_finite() || rate < 0.0 {
			return Err(AnimationLayerError::InvalidPlaybackRate);
		}

		let Some(layer) = self.gesture_layer(entity, activity)? else {
			return Ok(false);
		};

		// SAFETY: `gesture_layer` found the layer among the live entity's
		// layers, laid out as `Layer`, on the main thread, and nothing else
		// holds a reference to it during this call. The rate is written through
		// the pointer, as the game writes it.
		unsafe { (*layer.as_ptr()).playback_rate = rate };
		Ok(true)
	}
}

/// A layer, as the server's `CAnimationLayer` lays it out on both supported
/// targets (`game/server/BaseAnimatingOverlay.h`): 0x50 bytes, with the
/// activity at 0x34, as [`sdk_raw::tf2::animation_layers`]' Windows
/// signature walks them.
#[repr(C)]
struct Layer {
	flags: c_int,
	sequence_finished: bool,
	looping: bool,
	sequence: c_int,
	cycle: f32,
	prev_cycle: f32,
	weight: f32,
	playback_rate: f32,
	blend_in: f32,
	blend_out: f32,
	kill_rate: f32,
	kill_delay: f32,
	layer_anim_time: f32,
	layer_fade_out_time: f32,
	activity: c_int,
	priority: c_int,
	order: c_int,
	last_event_check: f32,
	last_access: f32,
	owner: *mut std::ffi::c_void,
}
