//! Hand-written values of TF2's bosses, which the generated bindings do not
//! describe: the Halloween team, the class names of the Halloween bosses and
//! skeletons, and the skeletons' types.

use crate::players::TF_TEAM_COUNT;
use std::ffi::{CStr, c_int};

/// The class name of `base_boss`, `CTFBaseBoss`, a NextBot actor with a
/// model, health and speed, and no behaviour of its own.
pub const BASE_BOSS: &CStr = c"base_boss";

/// The data map class of `base_boss` and Mann vs. Machine's tank
/// (`tank_boss`), which derives from it.
pub const BASE_BOSS_CLASS: &CStr = c"CTFBaseBoss";

/// The class name of Monoculus, `CEyeballBoss`.
pub const EYEBALL_BOSS: &CStr = c"eyeball_boss";

/// The class name of the Horseless Headless Horsemann, `CHeadlessHatman`.
pub const HEADLESS_HATMAN: &CStr = c"headless_hatman";

/// The class name of Merasmus, `CMerasmus`.
pub const MERASMUS: &CStr = c"merasmus";

/// The class name of the monster resource, `CMonsterResource`, which
/// networks the boss health bar. TF2's game rules create one with each
/// level.
pub const MONSTER_RESOURCE: &CStr = c"monster_resource";

/// The data map class of the monster resource.
pub const MONSTER_RESOURCE_CLASS: &CStr = c"CMonsterResource";

/// `CZombie::SkeletonType_t` (`game/server/tf/halloween/zombie/zombie.h`):
/// the skeletons' types.
pub mod skeleton {
	use std::ffi::c_int;

	/// A skeleton king: twice the size, with 1,000 health and a crown.
	pub const SKELETON_KING: c_int = 1;

	/// A small skeleton.
	pub const SKELETON_MINI: c_int = 2;

	/// An ordinary skeleton, with 50 health.
	pub const SKELETON_NORMAL: c_int = 0;
}

/// The team the game spawns its own Halloween bosses and skeletons on, which
/// is neither RED nor BLU (`TF_TEAM_HALLOWEEN` in
/// `game/shared/tf/tf_shareddefs.h`). It shares its number, 5, with
/// `TF_TEAM_AUTOASSIGN`, past the four teams
/// [`TF_TEAM_COUNT`](crate::players::TF_TEAM_COUNT) counts, so no team entity
/// has it.
pub const TF_TEAM_HALLOWEEN: c_int = TF_TEAM_COUNT + 1;

/// The class name of a skeleton, `CZombie`.
pub const TF_ZOMBIE: &CStr = c"tf_zombie";

/// The class name of a skeleton spawner, `CZombieSpawner`.
pub const TF_ZOMBIE_SPAWNER: &CStr = c"tf_zombie_spawner";

/// The data map class of a skeleton spawner.
pub const TF_ZOMBIE_SPAWNER_CLASS: &CStr = c"CZombieSpawner";
