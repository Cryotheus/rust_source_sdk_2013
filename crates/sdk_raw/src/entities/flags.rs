//! The flags of an entity's `m_fFlags`, the `FL_*` values from
//! `public/const.h`, as multiplayer games, TF2 among them, define them.
//!
//! The first ten describe players' movement, which the game predicts on
//! clients, and are sent to them. The others are the server's.

use std::ffi::c_int;

/// `FL_AIMTARGET`: aim assistance may aim at the entity.
pub const FL_AIMTARGET: c_int = 1 << 17;

/// `FL_ANIMDUCKING`: the player is crouching or standing up, or fully
/// crouched while also [`FL_DUCKING`].
pub const FL_ANIMDUCKING: c_int = 1 << 2;

/// `FL_ATCONTROLS`: the player cannot move, but keeps its inputs, to control
/// another entity.
pub const FL_ATCONTROLS: c_int = 1 << 7;

/// `FL_BASEVELOCITY`: base velocity was applied this frame.
pub const FL_BASEVELOCITY: c_int = 1 << 24;

/// `FL_CLIENT`: the entity is a player.
pub const FL_CLIENT: c_int = 1 << 8;

/// `FL_CONVEYOR`: the entity is a conveyor, which moves what stands on it.
pub const FL_CONVEYOR: c_int = 1 << 13;

/// `FL_DISSOLVING`: the entity is dissolving.
pub const FL_DISSOLVING: c_int = 1 << 29;

/// `FL_DONTTOUCH`: the entity touches nothing, and stops touching what it
/// touched when it was set.
pub const FL_DONTTOUCH: c_int = 1 << 23;

/// `FL_DUCKING`: the player is fully crouched, or standing up from it.
pub const FL_DUCKING: c_int = 1 << 1;

/// `FL_FAKECLIENT`: the player is a bot, whose client the server simulates.
pub const FL_FAKECLIENT: c_int = 1 << 9;

/// `FL_FLY`: the entity moves without needing to be on the ground.
pub const FL_FLY: c_int = 1 << 11;

/// `FL_FROZEN`: the player cannot move or look around.
pub const FL_FROZEN: c_int = 1 << 6;

/// `FL_GODMODE`: the player takes no damage, as with the `god` command.
pub const FL_GODMODE: c_int = 1 << 15;

/// `FL_GRAPHED`: the entity blocks a connection of the navigation graph.
pub const FL_GRAPHED: c_int = 1 << 20;

/// `FL_GRENADE`: the entity is a grenade.
pub const FL_GRENADE: c_int = 1 << 21;

/// `FL_INRAIN`: the entity stands in rain.
pub const FL_INRAIN: c_int = 1 << 5;

/// `FL_INWATER`: the entity is in water.
pub const FL_INWATER: c_int = 1 << 10;

/// `FL_KILLME`: the entity is marked for deletion.
pub const FL_KILLME: c_int = 1 << 27;

/// `FL_NOTARGET`: enemies do not target the entity, as with the `notarget`
/// command.
pub const FL_NOTARGET: c_int = 1 << 16;

/// `FL_NPC`: the entity is an NPC.
pub const FL_NPC: c_int = 1 << 14;

/// `FL_OBJECT`: NPCs see the entity, as they do missiles.
pub const FL_OBJECT: c_int = 1 << 26;

/// `FL_ONFIRE`: the entity is on fire.
pub const FL_ONFIRE: c_int = 1 << 28;

/// `FL_ONGROUND`: the entity rests on the ground.
pub const FL_ONGROUND: c_int = 1 << 0;

/// `FL_ONTRAIN`: the player controls a train, so its movement is ignored.
pub const FL_ONTRAIN: c_int = 1 << 4;

/// `FL_PARTIALGROUND`: not all of the entity's corners are on the ground.
pub const FL_PARTIALGROUND: c_int = 1 << 18;

/// `FL_STATICPROP`: the entity is a static prop.
pub const FL_STATICPROP: c_int = 1 << 19;

/// `FL_STEPMOVEMENT`: the entity's step movement does no processing.
pub const FL_STEPMOVEMENT: c_int = 1 << 22;

/// `FL_SWIM`: the entity moves without needing to be on the ground, but
/// stays in water.
pub const FL_SWIM: c_int = 1 << 12;

/// `FL_TRANSRAGDOLL`: the entity is turning into a client-side ragdoll.
pub const FL_TRANSRAGDOLL: c_int = 1 << 30;

/// `FL_UNBLOCKABLE_BY_PLAYER`: players cannot block the entity as it pushes
/// them.
pub const FL_UNBLOCKABLE_BY_PLAYER: c_int = 1 << 31;

/// `FL_WATERJUMP`: the player is jumping out of water.
pub const FL_WATERJUMP: c_int = 1 << 3;

/// `FL_WORLDBRUSH`: the entity is a brush that is part of the world, which
/// does not move and is never removed.
pub const FL_WORLDBRUSH: c_int = 1 << 25;
