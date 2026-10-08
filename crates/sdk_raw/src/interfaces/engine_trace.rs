//! Hand-written ABI of `IEngineTrace` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/engine/IEngineTrace.h`, and the contents and masks of traces,
//! from `public/bspflags.h`.

use std::ffi::{CStr, c_uint};

/// The version string `IEngineTrace` is exported and requested under.
///
/// This is `INTERFACEVERSION_ENGINETRACE_SERVER` from
/// `public/engine/IEngineTrace.h`.
#[doc(alias("INTERFACEVERSION_ENGINETRACE_SERVER"))]
pub const VERSION: &CStr = c"EngineTraceServer003";

// The contents of brushes and entities (`CONTENTS_*`), and the masks of
// contents that traces stop at (`MASK_*`), from `public/bspflags.h`. A brush
// can have several contents, and stronger, lower bits take the place of
// weaker brushes in the same leaf.

/// `CONTENTS_EMPTY`: no contents.
pub const CONTENTS_EMPTY: c_uint = 0;

/// `CONTENTS_SOLID`: solid brushes, where an eye is never valid.
pub const CONTENTS_SOLID: c_uint = 0x1;

/// `CONTENTS_WINDOW`: translucent brushes that are not water, such as
/// glass.
pub const CONTENTS_WINDOW: c_uint = 0x2;

/// `CONTENTS_AUX`.
pub const CONTENTS_AUX: c_uint = 0x4;

/// `CONTENTS_GRATE`: alpha-tested grates, which bullets and sight pass
/// through, but solids do not.
pub const CONTENTS_GRATE: c_uint = 0x8;

/// `CONTENTS_SLIME`: slime, a liquid.
pub const CONTENTS_SLIME: c_uint = 0x10;

/// `CONTENTS_WATER`: water.
pub const CONTENTS_WATER: c_uint = 0x20;

/// `CONTENTS_BLOCKLOS`: blocks the line of sight of AI.
pub const CONTENTS_BLOCKLOS: c_uint = 0x40;

/// `CONTENTS_OPAQUE`: cannot be seen through, though it may not be solid.
pub const CONTENTS_OPAQUE: c_uint = 0x80;

/// `LAST_VISIBLE_CONTENTS`: the last of the contents that can be seen.
pub const LAST_VISIBLE_CONTENTS: c_uint = 0x80;

/// `ALL_VISIBLE_CONTENTS`: every content that can be seen, up to
/// [`LAST_VISIBLE_CONTENTS`].
pub const ALL_VISIBLE_CONTENTS: c_uint = LAST_VISIBLE_CONTENTS | (LAST_VISIBLE_CONTENTS - 1);

/// `CONTENTS_TESTFOGVOLUME`.
pub const CONTENTS_TESTFOGVOLUME: c_uint = 0x100;

/// `CONTENTS_UNUSED`.
pub const CONTENTS_UNUSED: c_uint = 0x200;

/// `CONTENTS_UNUSED6`.
pub const CONTENTS_UNUSED6: c_uint = 0x400;

/// `CONTENTS_TEAM1`, TF2's `CONTENTS_REDTEAM`: tells the first team's
/// collisions apart. TF2's RED players block players' movement while
/// `tf_avoidteammates` is on, and rockets, only when the mask has it.
#[doc(alias("CONTENTS_REDTEAM"))]
pub const CONTENTS_TEAM1: c_uint = 0x800;

/// `CONTENTS_TEAM2`, TF2's `CONTENTS_BLUETEAM`: as [`CONTENTS_TEAM1`], for
/// BLU.
#[doc(alias("CONTENTS_BLUETEAM"))]
pub const CONTENTS_TEAM2: c_uint = 0x1000;

/// `CONTENTS_IGNORE_NODRAW_OPAQUE`: ignores [`CONTENTS_OPAQUE`] on surfaces
/// that are not drawn.
pub const CONTENTS_IGNORE_NODRAW_OPAQUE: c_uint = 0x2000;

/// `CONTENTS_MOVEABLE`: entities that push, such as doors and platforms.
pub const CONTENTS_MOVEABLE: c_uint = 0x4000;

/// `CONTENTS_AREAPORTAL`: area portals. This and the contents after it
/// cannot be seen, and do not take the place of other brushes.
pub const CONTENTS_AREAPORTAL: c_uint = 0x8000;

/// `CONTENTS_PLAYERCLIP`: clips that block players alone.
pub const CONTENTS_PLAYERCLIP: c_uint = 0x1_0000;

/// `CONTENTS_MONSTERCLIP`: clips that block NPCs alone.
pub const CONTENTS_MONSTERCLIP: c_uint = 0x2_0000;

/// `CONTENTS_CURRENT_0`: a current towards 0 degrees, which can be added to
/// other contents.
pub const CONTENTS_CURRENT_0: c_uint = 0x4_0000;

/// `CONTENTS_CURRENT_90`: a current towards 90 degrees.
pub const CONTENTS_CURRENT_90: c_uint = 0x8_0000;

/// `CONTENTS_CURRENT_180`: a current towards 180 degrees.
pub const CONTENTS_CURRENT_180: c_uint = 0x10_0000;

/// `CONTENTS_CURRENT_270`: a current towards 270 degrees.
pub const CONTENTS_CURRENT_270: c_uint = 0x20_0000;

/// `CONTENTS_CURRENT_UP`: an upward current.
pub const CONTENTS_CURRENT_UP: c_uint = 0x40_0000;

/// `CONTENTS_CURRENT_DOWN`: a downward current.
pub const CONTENTS_CURRENT_DOWN: c_uint = 0x80_0000;

/// `CONTENTS_ORIGIN`: origin brushes, removed when the map is compiled.
pub const CONTENTS_ORIGIN: c_uint = 0x100_0000;

/// `CONTENTS_MONSTER`: never on a brush. The game's trace filters only let a
/// trace hit an entity other than a brush, such as a player or a building,
/// when its mask has it.
pub const CONTENTS_MONSTER: c_uint = 0x200_0000;

/// `CONTENTS_DEBRIS`: debris.
pub const CONTENTS_DEBRIS: c_uint = 0x400_0000;

/// `CONTENTS_DETAIL`: detail brushes, which the map's visibility ignores.
pub const CONTENTS_DETAIL: c_uint = 0x800_0000;

/// `CONTENTS_TRANSLUCENT`: set on brushes with a translucent surface.
pub const CONTENTS_TRANSLUCENT: c_uint = 0x1000_0000;

/// `CONTENTS_LADDER`: ladders.
pub const CONTENTS_LADDER: c_uint = 0x2000_0000;

/// `CONTENTS_HITBOX`: traces hit models' hitboxes rather than their
/// collision models.
pub const CONTENTS_HITBOX: c_uint = 0x4000_0000;

/// `MASK_ALL`: every contents flag, so that a trace stops at whatever it
/// meets, the brushes of triggers included.
pub const MASK_ALL: c_uint = 0xFFFF_FFFF;

/// `MASK_SOLID`: everything that is normally solid, entities other than
/// brushes included.
pub const MASK_SOLID: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_MONSTER | CONTENTS_GRATE;

/// `MASK_PLAYERSOLID`: everything that blocks players' movement.
pub const MASK_PLAYERSOLID: c_uint = CONTENTS_SOLID
	| CONTENTS_MOVEABLE
	| CONTENTS_PLAYERCLIP
	| CONTENTS_WINDOW
	| CONTENTS_MONSTER
	| CONTENTS_GRATE;

/// `MASK_NPCSOLID`: everything that blocks NPCs' movement.
pub const MASK_NPCSOLID: c_uint = CONTENTS_SOLID
	| CONTENTS_MOVEABLE
	| CONTENTS_MONSTERCLIP
	| CONTENTS_WINDOW
	| CONTENTS_MONSTER
	| CONTENTS_GRATE;

/// `MASK_WATER`: the liquids, in which water physics apply.
pub const MASK_WATER: c_uint = CONTENTS_WATER | CONTENTS_MOVEABLE | CONTENTS_SLIME;

/// `MASK_OPAQUE`: everything that blocks light.
pub const MASK_OPAQUE: c_uint = CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_OPAQUE;

/// `MASK_OPAQUE_AND_NPCS`: [`MASK_OPAQUE`], and entities other than brushes.
pub const MASK_OPAQUE_AND_NPCS: c_uint = MASK_OPAQUE | CONTENTS_MONSTER;

/// `MASK_BLOCKLOS`: everything that blocks the line of sight of AI.
pub const MASK_BLOCKLOS: c_uint = CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_BLOCKLOS;

/// `MASK_BLOCKLOS_AND_NPCS`: [`MASK_BLOCKLOS`], and entities other than
/// brushes.
pub const MASK_BLOCKLOS_AND_NPCS: c_uint = MASK_BLOCKLOS | CONTENTS_MONSTER;

/// `MASK_VISIBLE`: everything that blocks players' line of sight.
pub const MASK_VISIBLE: c_uint = MASK_OPAQUE | CONTENTS_IGNORE_NODRAW_OPAQUE;

/// `MASK_VISIBLE_AND_NPCS`: [`MASK_VISIBLE`], and entities other than
/// brushes.
pub const MASK_VISIBLE_AND_NPCS: c_uint = MASK_OPAQUE_AND_NPCS | CONTENTS_IGNORE_NODRAW_OPAQUE;

/// `MASK_SHOT`: what bullets hit, entities by their hitboxes.
pub const MASK_SHOT: c_uint = CONTENTS_SOLID
	| CONTENTS_MOVEABLE
	| CONTENTS_MONSTER
	| CONTENTS_WINDOW
	| CONTENTS_DEBRIS
	| CONTENTS_HITBOX;

/// `MASK_SHOT_HULL`: what weapons that do not trace lines, such as melee
/// weapons, hit, grates included.
pub const MASK_SHOT_HULL: c_uint = CONTENTS_SOLID
	| CONTENTS_MOVEABLE
	| CONTENTS_MONSTER
	| CONTENTS_WINDOW
	| CONTENTS_DEBRIS
	| CONTENTS_GRATE;

/// `MASK_SHOT_PORTAL`: solids but grates.
pub const MASK_SHOT_PORTAL: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_MONSTER;

/// `MASK_SOLID_BRUSHONLY`: every brush a solid collides with.
pub const MASK_SOLID_BRUSHONLY: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_GRATE;

/// `MASK_PLAYERSOLID_BRUSHONLY`: every brush that blocks players' movement.
pub const MASK_PLAYERSOLID_BRUSHONLY: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_PLAYERCLIP | CONTENTS_GRATE;

/// `MASK_NPCSOLID_BRUSHONLY`: every brush that blocks NPCs' movement.
pub const MASK_NPCSOLID_BRUSHONLY: c_uint =
	CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_WINDOW | CONTENTS_MONSTERCLIP | CONTENTS_GRATE;

/// `MASK_NPCWORLDSTATIC`: the world's brushes that block NPCs, without those
/// that move, as NPCs' routes are rebuilt with.
pub const MASK_NPCWORLDSTATIC: c_uint =
	CONTENTS_SOLID | CONTENTS_WINDOW | CONTENTS_MONSTERCLIP | CONTENTS_GRATE;

/// `MASK_SPLITAREAPORTAL`: what can split area portals.
pub const MASK_SPLITAREAPORTAL: c_uint = CONTENTS_WATER | CONTENTS_SLIME;

/// `MASK_CURRENT`: every current.
pub const MASK_CURRENT: c_uint = CONTENTS_CURRENT_0
	| CONTENTS_CURRENT_90
	| CONTENTS_CURRENT_180
	| CONTENTS_CURRENT_270
	| CONTENTS_CURRENT_UP
	| CONTENTS_CURRENT_DOWN;

/// `MASK_DEADSOLID`: what blocks corpses, which the game does not use.
pub const MASK_DEADSOLID: c_uint =
	CONTENTS_SOLID | CONTENTS_PLAYERCLIP | CONTENTS_WINDOW | CONTENTS_GRATE;
