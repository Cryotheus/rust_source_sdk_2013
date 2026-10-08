//! TF2's numbers for its projectiles: the kinds of projectiles weapons fire
//! (`ProjectileType_t`) from `game/shared/tf/tf_shareddefs.h`, which arrows
//! network, and the modes of pipebombs from
//! `game/shared/tf/tf_weapon_grenade_pipebomb.h`.

use std::ffi::c_int;

/// `TF_GL_MODE_CANNONBALL`: a Loose Cannon's cannonball.
pub const TF_GL_MODE_CANNONBALL: c_int = 3;

/// `TF_GL_MODE_REGULAR`: a grenade launcher's grenade, and the mode of the
/// jars, balls and spells that derive from pipebombs.
pub const TF_GL_MODE_REGULAR: c_int = 0;

/// `TF_GL_MODE_REMOTE_DETONATE`: a stickybomb launcher's stickybomb.
pub const TF_GL_MODE_REMOTE_DETONATE: c_int = 1;

/// `TF_GL_MODE_REMOTE_DETONATE_PRACTICE`: the Sticky Jumper's stickybomb,
/// which does no damage.
pub const TF_GL_MODE_REMOTE_DETONATE_PRACTICE: c_int = 2;

/// `TF_NUM_PROJECTILES`, the number of projectile types.
pub const TF_NUM_PROJECTILES: c_int = 31;

/// `TF_PROJECTILE_ARROW`, the Huntsman's and Fortified Compound's arrows.
pub const TF_PROJECTILE_ARROW: c_int = 8;

/// `TF_PROJECTILE_BREAD_MONSTER`, the bread monsters an unused throwable
/// leaves where it hits, an effect rather than a projectile.
pub const TF_PROJECTILE_BREAD_MONSTER: c_int = 28;

/// `TF_PROJECTILE_BREADMONSTER_JARATE`, the Self-Aware Beauty Mark's jars.
pub const TF_PROJECTILE_BREADMONSTER_JARATE: c_int = 24;

/// `TF_PROJECTILE_BREADMONSTER_MADMILK`, the Mutated Milk's jars.
pub const TF_PROJECTILE_BREADMONSTER_MADMILK: c_int = 25;

/// `TF_PROJECTILE_BUILDING_REPAIR_BOLT`, the Rescue Ranger's bolts.
pub const TF_PROJECTILE_BUILDING_REPAIR_BOLT: c_int = 18;

/// `TF_PROJECTILE_BULLET`, the type of hitscan weapons, which fire no
/// projectile.
pub const TF_PROJECTILE_BULLET: c_int = 1;

/// `TF_PROJECTILE_CANNONBALL`, the Loose Cannon's cannonballs.
pub const TF_PROJECTILE_CANNONBALL: c_int = 17;

/// `TF_PROJECTILE_CLEAVER`, the Flying Guillotine's cleavers.
pub const TF_PROJECTILE_CLEAVER: c_int = 15;

/// `TF_PROJECTILE_ENERGY_BALL`, the Cow Mangler 5000's shots.
pub const TF_PROJECTILE_ENERGY_BALL: c_int = 12;

/// `TF_PROJECTILE_ENERGY_RING`, the Righteous Bison's and Pomson 6000's
/// shots.
pub const TF_PROJECTILE_ENERGY_RING: c_int = 13;

/// `TF_PROJECTILE_FESTIVE_ARROW`, the Festive Huntsman's arrows.
pub const TF_PROJECTILE_FESTIVE_ARROW: c_int = 19;

/// `TF_PROJECTILE_FESTIVE_HEALING_BOLT`, the Festive Crusader's Crossbow's
/// bolts.
pub const TF_PROJECTILE_FESTIVE_HEALING_BOLT: c_int = 23;

/// `TF_PROJECTILE_FESTIVE_JAR`, the Festive Jarate's jars.
pub const TF_PROJECTILE_FESTIVE_JAR: c_int = 22;

/// `TF_PROJECTILE_FLAME_BALL`, the Dragon's Fury's fireballs.
pub const TF_PROJECTILE_FLAME_BALL: c_int = 30;

/// `TF_PROJECTILE_FLAME_ROCKET`, an unused flaming rocket.
pub const TF_PROJECTILE_FLAME_ROCKET: c_int = 9;

/// `TF_PROJECTILE_FLARE`, flare guns' flares.
pub const TF_PROJECTILE_FLARE: c_int = 6;

/// `TF_PROJECTILE_GRAPPLINGHOOK`, the Grappling Hook's hooks.
pub const TF_PROJECTILE_GRAPPLINGHOOK: c_int = 26;

/// `TF_PROJECTILE_HEALING_BOLT`, the Crusader's Crossbow's bolts.
pub const TF_PROJECTILE_HEALING_BOLT: c_int = 11;

/// `TF_PROJECTILE_JAR`, Jarate's jars.
pub const TF_PROJECTILE_JAR: c_int = 7;

/// `TF_PROJECTILE_JAR_GAS`, the Gas Passer's jars.
pub const TF_PROJECTILE_JAR_GAS: c_int = 29;

/// `TF_PROJECTILE_JAR_MILK`, Mad Milk's jars.
pub const TF_PROJECTILE_JAR_MILK: c_int = 10;

/// `TF_PROJECTILE_NONE`, no projectile.
pub const TF_PROJECTILE_NONE: c_int = 0;

/// `TF_PROJECTILE_PIPEBOMB`, grenade launchers' grenades.
pub const TF_PROJECTILE_PIPEBOMB: c_int = 3;

/// `TF_PROJECTILE_PIPEBOMB_PRACTICE`, the Sticky Jumper's stickybombs.
pub const TF_PROJECTILE_PIPEBOMB_PRACTICE: c_int = 14;

/// `TF_PROJECTILE_PIPEBOMB_REMOTE`, stickybomb launchers' stickybombs.
pub const TF_PROJECTILE_PIPEBOMB_REMOTE: c_int = 4;

/// `TF_PROJECTILE_ROCKET`, rocket launchers' rockets.
pub const TF_PROJECTILE_ROCKET: c_int = 2;

/// `TF_PROJECTILE_SENTRY_ROCKET`, level 3 sentries' rockets.
pub const TF_PROJECTILE_SENTRY_ROCKET: c_int = 27;

/// `TF_PROJECTILE_SPELL`, spellbooks' spells.
pub const TF_PROJECTILE_SPELL: c_int = 21;

/// `TF_PROJECTILE_STICKY_BALL`, a ball the server's code never fires: the
/// Sandman and the Wrap Assassin launch their balls themselves.
pub const TF_PROJECTILE_STICKY_BALL: c_int = 16;

/// `TF_PROJECTILE_SYRINGE`, syringe guns' syringes.
pub const TF_PROJECTILE_SYRINGE: c_int = 5;

/// `TF_PROJECTILE_THROWABLE`, unused throwables.
pub const TF_PROJECTILE_THROWABLE: c_int = 20;
