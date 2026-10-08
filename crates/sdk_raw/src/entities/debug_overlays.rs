//! The bits of an entity's `m_debugOverlays`, the `OVERLAY_*` values of
//! `DebugOverlayBits_t` in `game/server/baseentity.h`: what the game draws for
//! an entity to debug it, and buddha mode.
//!
//! The drawing bits are set by commands such as `ent_text`, `ent_bbox` and
//! `ent_pivot`, and draw through the engine's debug overlays, which only a
//! listen server's own client sees. The bits naming NPCs do nothing in games
//! without them, TF2 among them.

use std::ffi::c_int;

/// `OVERLAY_TEXT_BIT`: shows the entity's debug text (`ent_text`).
pub const OVERLAY_TEXT_BIT: c_int = 0x0000_0001;

/// `OVERLAY_NAME_BIT`: shows the entity's name (`ent_name`).
pub const OVERLAY_NAME_BIT: c_int = 0x0000_0002;

/// `OVERLAY_BBOX_BIT`: shows the entity's bounding box (`ent_bbox`).
pub const OVERLAY_BBOX_BIT: c_int = 0x0000_0004;

/// `OVERLAY_PIVOT_BIT`: shows the entity's pivot (`ent_pivot`).
pub const OVERLAY_PIVOT_BIT: c_int = 0x0000_0008;

/// `OVERLAY_MESSAGE_BIT`: shows the entity's messages (`ent_messages`).
pub const OVERLAY_MESSAGE_BIT: c_int = 0x0000_0010;

/// `OVERLAY_ABSBOX_BIT`: shows the entity's absolute bounding box
/// (`ent_absbox`).
pub const OVERLAY_ABSBOX_BIT: c_int = 0x0000_0020;

/// `OVERLAY_RBOX_BIT`: shows the entity's rotated box (`ent_rbox`).
pub const OVERLAY_RBOX_BIT: c_int = 0x0000_0040;

/// `OVERLAY_SHOW_BLOCKSLOS`: shows the entities that block NPCs' sight.
pub const OVERLAY_SHOW_BLOCKSLOS: c_int = 0x0000_0080;

/// `OVERLAY_ATTACHMENTS_BIT`: shows the entity's attachment points
/// (`ent_attachments`).
pub const OVERLAY_ATTACHMENTS_BIT: c_int = 0x0000_0100;

/// `OVERLAY_AUTOAIM_BIT`: shows the entity's autoaim radius
/// (`ent_autoaim`).
pub const OVERLAY_AUTOAIM_BIT: c_int = 0x0000_0200;

/// `OVERLAY_NPC_SELECTED_BIT`: the NPC is selected.
pub const OVERLAY_NPC_SELECTED_BIT: c_int = 0x0000_1000;

/// `OVERLAY_NPC_NEAREST_BIT`: shows the NPC's nearest node.
pub const OVERLAY_NPC_NEAREST_BIT: c_int = 0x0000_2000;

/// `OVERLAY_NPC_ROUTE_BIT`: shows the NPC's route.
pub const OVERLAY_NPC_ROUTE_BIT: c_int = 0x0000_4000;

/// `OVERLAY_NPC_TRIANGULATE_BIT`: shows the NPC's triangulation.
pub const OVERLAY_NPC_TRIANGULATE_BIT: c_int = 0x0000_8000;

/// `OVERLAY_NPC_ZAP_BIT`: destroys the NPC.
pub const OVERLAY_NPC_ZAP_BIT: c_int = 0x0001_0000;

/// `OVERLAY_NPC_ENEMIES_BIT`: shows the NPC's enemies.
pub const OVERLAY_NPC_ENEMIES_BIT: c_int = 0x0002_0000;

/// `OVERLAY_NPC_CONDITIONS_BIT`: shows the NPC's conditions.
pub const OVERLAY_NPC_CONDITIONS_BIT: c_int = 0x0004_0000;

/// `OVERLAY_NPC_SQUAD_BIT`: shows the NPC's squad.
pub const OVERLAY_NPC_SQUAD_BIT: c_int = 0x0008_0000;

/// `OVERLAY_NPC_TASK_BIT`: shows the NPC's task.
pub const OVERLAY_NPC_TASK_BIT: c_int = 0x0010_0000;

/// `OVERLAY_NPC_FOCUS_BIT`: shows a line to the NPC's enemy and target.
pub const OVERLAY_NPC_FOCUS_BIT: c_int = 0x0020_0000;

/// `OVERLAY_NPC_VIEWCONE_BIT`: shows the NPC's view cone.
pub const OVERLAY_NPC_VIEWCONE_BIT: c_int = 0x0040_0000;

/// `OVERLAY_NPC_KILL_BIT`: kills the NPC, running its AI's death.
pub const OVERLAY_NPC_KILL_BIT: c_int = 0x0080_0000;

/// `OVERLAY_WC_CHANGE_ENTITY`: Hammer changed the entity while editing the
/// level live.
pub const OVERLAY_WC_CHANGE_ENTITY: c_int = 0x0100_0000;

/// `OVERLAY_BUDDHA_MODE`: the player takes damage, but damage that would
/// kill them leaves them at 1 health instead (`buddha`).
pub const OVERLAY_BUDDHA_MODE: c_int = 0x0200_0000;

/// `OVERLAY_NPC_STEERING_REGULATIONS`: shows the NPC's steering
/// regulations.
pub const OVERLAY_NPC_STEERING_REGULATIONS: c_int = 0x0400_0000;

/// `OVERLAY_TASK_TEXT_BIT`: shows the names of NPCs' tasks and schedules as
/// they start.
pub const OVERLAY_TASK_TEXT_BIT: c_int = 0x0800_0000;

/// `OVERLAY_PROP_DEBUG`: shows a prop's debug information.
pub const OVERLAY_PROP_DEBUG: c_int = 0x1000_0000;

/// `OVERLAY_NPC_RELATION_BIT`: shows the NPC's relationships.
pub const OVERLAY_NPC_RELATION_BIT: c_int = 0x2000_0000;

/// `OVERLAY_VIEWOFFSET`: shows the entity's view offset (`ent_viewoffset`).
pub const OVERLAY_VIEWOFFSET: c_int = 0x4000_0000;
