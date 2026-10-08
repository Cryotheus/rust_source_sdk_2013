//! An entity's debug overlays (`m_debugOverlays`): what commands such as
//! `ent_text` and `ent_bbox` draw for it, and buddha mode, which keeps a
//! player alive at 1 health.
//!
//! The game reads the bits where it needs them, so setting them runs no game
//! code: a player put in [`BUDDHA_MODE`](DebugOverlays::BUDDHA_MODE) is left
//! at 1 health by the next damage that would kill them, as the `buddha`
//! command does for whoever runs it. Unlike a player's flags, spawning does
//! not clear the bits, so buddha mode outlives a respawn.

#[cfg(test)]
#[path = "../tests/entities/debug_overlays.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::fields::{BaseField, FieldError};
use crate::interfaces::ValveEngine;
use sdk_raw::entities::debug_overlays::{
	OVERLAY_ABSBOX_BIT, OVERLAY_ATTACHMENTS_BIT, OVERLAY_AUTOAIM_BIT, OVERLAY_BBOX_BIT,
	OVERLAY_BUDDHA_MODE, OVERLAY_MESSAGE_BIT, OVERLAY_NAME_BIT, OVERLAY_NPC_CONDITIONS_BIT,
	OVERLAY_NPC_ENEMIES_BIT, OVERLAY_NPC_FOCUS_BIT, OVERLAY_NPC_KILL_BIT, OVERLAY_NPC_NEAREST_BIT,
	OVERLAY_NPC_RELATION_BIT, OVERLAY_NPC_ROUTE_BIT, OVERLAY_NPC_SELECTED_BIT,
	OVERLAY_NPC_SQUAD_BIT, OVERLAY_NPC_STEERING_REGULATIONS, OVERLAY_NPC_TASK_BIT,
	OVERLAY_NPC_TRIANGULATE_BIT, OVERLAY_NPC_VIEWCONE_BIT, OVERLAY_NPC_ZAP_BIT, OVERLAY_PIVOT_BIT,
	OVERLAY_PROP_DEBUG, OVERLAY_RBOX_BIT, OVERLAY_SHOW_BLOCKSLOS, OVERLAY_TASK_TEXT_BIT,
	OVERLAY_TEXT_BIT, OVERLAY_VIEWOFFSET, OVERLAY_WC_CHANGE_ENTITY,
};
use std::ffi::c_int;

/// `m_debugOverlays`.
static DEBUG_OVERLAYS: BaseField<c_int> =
	BaseField::new(c"m_debugOverlays", sys::_fieldtypes_FIELD_INTEGER);

bitflags::bitflags! {
	/// What the game draws for an entity to debug it, and buddha mode
	/// (`m_debugOverlays`), the `OVERLAY_*` values of `DebugOverlayBits_t` in
	/// `game/server/baseentity.h`.
	///
	/// The drawings go through the engine's debug overlays, which only a
	/// listen server's own client sees. The bits naming NPCs do nothing in
	/// games without them, TF2 among them. Overlays read from an entity keep
	/// every bit, including those without a constant here.
	#[doc(alias("m_debugOverlays", "DebugOverlayBits_t"))]
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct DebugOverlays: c_int {
		/// `OVERLAY_ABSBOX_BIT`: shows the absolute bounding box
		/// (`ent_absbox`).
		#[doc(alias("OVERLAY_ABSBOX_BIT"))]
		const ABS_BOX = OVERLAY_ABSBOX_BIT;

		/// `OVERLAY_ATTACHMENTS_BIT`: shows the attachment points
		/// (`ent_attachments`).
		#[doc(alias("OVERLAY_ATTACHMENTS_BIT"))]
		const ATTACHMENTS = OVERLAY_ATTACHMENTS_BIT;

		/// `OVERLAY_AUTOAIM_BIT`: shows the autoaim radius (`ent_autoaim`).
		#[doc(alias("OVERLAY_AUTOAIM_BIT"))]
		const AUTOAIM = OVERLAY_AUTOAIM_BIT;

		/// `OVERLAY_BBOX_BIT`: shows the bounding box (`ent_bbox`).
		#[doc(alias("OVERLAY_BBOX_BIT"))]
		const BOUNDING_BOX = OVERLAY_BBOX_BIT;

		/// `OVERLAY_BUDDHA_MODE`: the player takes damage, but damage that
		/// would kill them leaves them at 1 health instead (`buddha`).
		#[doc(alias("OVERLAY_BUDDHA_MODE", "buddha"))]
		const BUDDHA_MODE = OVERLAY_BUDDHA_MODE;

		/// `OVERLAY_MESSAGE_BIT`: shows the inputs and outputs the entity
		/// sends and receives (`ent_messages`).
		#[doc(alias("OVERLAY_MESSAGE_BIT"))]
		const MESSAGES = OVERLAY_MESSAGE_BIT;

		/// `OVERLAY_NAME_BIT`: shows the entity's name (`ent_name`).
		#[doc(alias("OVERLAY_NAME_BIT"))]
		const NAME = OVERLAY_NAME_BIT;

		/// `OVERLAY_NPC_CONDITIONS_BIT`: shows the NPC's conditions.
		#[doc(alias("OVERLAY_NPC_CONDITIONS_BIT"))]
		const NPC_CONDITIONS = OVERLAY_NPC_CONDITIONS_BIT;

		/// `OVERLAY_NPC_ENEMIES_BIT`: shows the NPC's enemies.
		#[doc(alias("OVERLAY_NPC_ENEMIES_BIT"))]
		const NPC_ENEMIES = OVERLAY_NPC_ENEMIES_BIT;

		/// `OVERLAY_NPC_FOCUS_BIT`: shows a line to the NPC's enemy and target.
		#[doc(alias("OVERLAY_NPC_FOCUS_BIT"))]
		const NPC_FOCUS = OVERLAY_NPC_FOCUS_BIT;

		/// `OVERLAY_NPC_KILL_BIT`: kills the NPC, running its AI's death.
		#[doc(alias("OVERLAY_NPC_KILL_BIT"))]
		const NPC_KILL = OVERLAY_NPC_KILL_BIT;

		/// `OVERLAY_NPC_NEAREST_BIT`: shows the NPC's nearest node.
		#[doc(alias("OVERLAY_NPC_NEAREST_BIT"))]
		const NPC_NEAREST = OVERLAY_NPC_NEAREST_BIT;

		/// `OVERLAY_NPC_RELATION_BIT`: shows the NPC's relationships.
		#[doc(alias("OVERLAY_NPC_RELATION_BIT"))]
		const NPC_RELATIONS = OVERLAY_NPC_RELATION_BIT;

		/// `OVERLAY_NPC_ROUTE_BIT`: shows the NPC's route.
		#[doc(alias("OVERLAY_NPC_ROUTE_BIT"))]
		const NPC_ROUTE = OVERLAY_NPC_ROUTE_BIT;

		/// `OVERLAY_NPC_SELECTED_BIT`: the NPC is selected.
		#[doc(alias("OVERLAY_NPC_SELECTED_BIT"))]
		const NPC_SELECTED = OVERLAY_NPC_SELECTED_BIT;

		/// `OVERLAY_NPC_SQUAD_BIT`: shows the NPC's squad.
		#[doc(alias("OVERLAY_NPC_SQUAD_BIT"))]
		const NPC_SQUAD = OVERLAY_NPC_SQUAD_BIT;

		/// `OVERLAY_NPC_STEERING_REGULATIONS`: shows the NPC's steering
		/// regulations.
		#[doc(alias("OVERLAY_NPC_STEERING_REGULATIONS"))]
		const NPC_STEERING_REGULATIONS = OVERLAY_NPC_STEERING_REGULATIONS;

		/// `OVERLAY_NPC_TASK_BIT`: shows the NPC's task.
		#[doc(alias("OVERLAY_NPC_TASK_BIT"))]
		const NPC_TASK = OVERLAY_NPC_TASK_BIT;

		/// `OVERLAY_NPC_TRIANGULATE_BIT`: shows the NPC's triangulation.
		#[doc(alias("OVERLAY_NPC_TRIANGULATE_BIT"))]
		const NPC_TRIANGULATE = OVERLAY_NPC_TRIANGULATE_BIT;

		/// `OVERLAY_NPC_VIEWCONE_BIT`: shows the NPC's view cone.
		#[doc(alias("OVERLAY_NPC_VIEWCONE_BIT"))]
		const NPC_VIEW_CONE = OVERLAY_NPC_VIEWCONE_BIT;

		/// `OVERLAY_NPC_ZAP_BIT`: destroys the NPC.
		#[doc(alias("OVERLAY_NPC_ZAP_BIT"))]
		const NPC_ZAP = OVERLAY_NPC_ZAP_BIT;

		/// `OVERLAY_PIVOT_BIT`: shows the pivot (`ent_pivot`).
		#[doc(alias("OVERLAY_PIVOT_BIT"))]
		const PIVOT = OVERLAY_PIVOT_BIT;

		/// `OVERLAY_PROP_DEBUG`: shows a prop's debug information.
		#[doc(alias("OVERLAY_PROP_DEBUG"))]
		const PROP_DEBUG = OVERLAY_PROP_DEBUG;

		/// `OVERLAY_RBOX_BIT`: shows the rotated box (`ent_rbox`).
		#[doc(alias("OVERLAY_RBOX_BIT"))]
		const ROTATED_BOX = OVERLAY_RBOX_BIT;

		/// `OVERLAY_SHOW_BLOCKSLOS`: shows the entities that block NPCs'
		/// sight.
		#[doc(alias("OVERLAY_SHOW_BLOCKSLOS"))]
		const SHOW_BLOCKS_SIGHT = OVERLAY_SHOW_BLOCKSLOS;

		/// `OVERLAY_TASK_TEXT_BIT`: shows the names of NPCs' tasks and
		/// schedules as they start.
		#[doc(alias("OVERLAY_TASK_TEXT_BIT"))]
		const TASK_TEXT = OVERLAY_TASK_TEXT_BIT;

		/// `OVERLAY_TEXT_BIT`: shows the entity's debug text (`ent_text`).
		#[doc(alias("OVERLAY_TEXT_BIT"))]
		const TEXT = OVERLAY_TEXT_BIT;

		/// `OVERLAY_VIEWOFFSET`: shows the view offset (`ent_viewoffset`).
		#[doc(alias("OVERLAY_VIEWOFFSET"))]
		const VIEW_OFFSET = OVERLAY_VIEWOFFSET;

		/// `OVERLAY_WC_CHANGE_ENTITY`: Hammer changed the entity while
		/// editing the level live.
		#[doc(alias("OVERLAY_WC_CHANGE_ENTITY"))]
		const WC_CHANGE_ENTITY = OVERLAY_WC_CHANGE_ENTITY;
	}
}

impl<'s> Entity<'s> {
	/// The entity's debug overlays (`m_debugOverlays`).
	#[doc(alias("m_debugOverlays"))]
	pub fn debug_overlays(self) -> Result<DebugOverlays, FieldError> {
		DEBUG_OVERLAYS
			.read(self)
			.map(DebugOverlays::from_bits_retain)
	}

	/// Sets the entity's debug overlays (`m_debugOverlays`), as the `buddha`
	/// and `ent_*` commands do.
	///
	/// The member is not networked, so `engine` only records a change no
	/// client is sent.
	#[doc(alias("m_debugOverlays"))]
	pub fn set_debug_overlays(
		self,
		engine: ValveEngine<'_>,
		overlays: DebugOverlays,
	) -> Result<(), FieldError> {
		DEBUG_OVERLAYS.write(engine, self, overlays.bits())
	}
}
