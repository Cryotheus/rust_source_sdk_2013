//! Hand-written header values of TF2's game modes, which the generated
//! bindings omit: the game and HUD types of `game/shared/tf/tf_shareddefs.h`,
//! the holidays of `game/shared/econ/econ_item_constants.h`, and the Halloween
//! scenarios of `game/shared/tf/tf_gamerules.h`, all from anonymous or unused
//! enums.

use std::ffi::c_int;

/// `HALLOWEEN_SCENARIO_DOOMSDAY`: Carnival of Carnage's.
pub const HALLOWEEN_SCENARIO_DOOMSDAY: c_int = 5;

/// `HALLOWEEN_SCENARIO_HIGHTOWER`: Helltower's, whose players use spells.
pub const HALLOWEEN_SCENARIO_HIGHTOWER: c_int = 4;

/// `HALLOWEEN_SCENARIO_LAKESIDE`: Ghost Fort's, with Merasmus.
pub const HALLOWEEN_SCENARIO_LAKESIDE: c_int = 3;

/// `HALLOWEEN_SCENARIO_MANN_MANOR`: Mann Manor's, with the Horseless Headless
/// Horsemann.
pub const HALLOWEEN_SCENARIO_MANN_MANOR: c_int = 1;

/// `HALLOWEEN_SCENARIO_NONE`: no Halloween scenario.
pub const HALLOWEEN_SCENARIO_NONE: c_int = 0;

/// `HALLOWEEN_SCENARIO_VIADUCT`: Eyeaduct's, with Monoculus.
pub const HALLOWEEN_SCENARIO_VIADUCT: c_int = 2;

/// `kHoliday_AprilFools`: April Fools' Day.
pub const HOLIDAY_APRIL_FOOLS: c_int = 11;

/// `kHoliday_Christmas`: Christmas, the Smissmas.
pub const HOLIDAY_CHRISTMAS: c_int = 3;

/// `kHoliday_CommunityUpdate`: a community update.
pub const HOLIDAY_COMMUNITY_UPDATE: c_int = 4;

/// `kHoliday_EOTL`: the End of the Line update.
pub const HOLIDAY_EOTL: c_int = 5;

/// `kHoliday_FullMoon`: a full moon.
pub const HOLIDAY_FULL_MOON: c_int = 8;

/// `kHoliday_Halloween`: Halloween.
pub const HOLIDAY_HALLOWEEN: c_int = 2;

/// `kHoliday_HalloweenOrFullMoon`: Halloween or a full moon.
pub const HOLIDAY_HALLOWEEN_OR_FULL_MOON: c_int = 9;

/// `kHoliday_HalloweenOrFullMoonOrValentines`: Halloween, a full moon, or
/// Valentine's Day.
pub const HOLIDAY_HALLOWEEN_OR_FULL_MOON_OR_VALENTINES: c_int = 10;

/// `kHoliday_MeetThePyro`: the Meet the Pyro event.
pub const HOLIDAY_MEET_THE_PYRO: c_int = 7;

/// `kHoliday_None`: no holiday.
pub const HOLIDAY_NONE: c_int = 0;

/// `kHoliday_Soldier`: the Soldier event.
pub const HOLIDAY_SOLDIER: c_int = 12;

/// `kHoliday_Summer`: the Summer event.
pub const HOLIDAY_SUMMER: c_int = 13;

/// `kHoliday_TFBirthday`: TF2's birthday.
pub const HOLIDAY_TF_BIRTHDAY: c_int = 1;

/// `kHoliday_Valentines`: Valentine's Day.
pub const HOLIDAY_VALENTINES: c_int = 6;

/// `TF_GAMETYPE_ARENA`: Arena.
pub const TF_GAMETYPE_ARENA: c_int = 4;

/// `TF_GAMETYPE_CP`: control points, also King of the Hill's.
pub const TF_GAMETYPE_CP: c_int = 2;

/// `TF_GAMETYPE_CTF`: Capture the Flag, also Mannpower's.
pub const TF_GAMETYPE_CTF: c_int = 1;

/// `TF_GAMETYPE_ESCORT`: Payload and Payload Race.
pub const TF_GAMETYPE_ESCORT: c_int = 3;

/// `TF_GAMETYPE_MVM`: Mann vs. Machine.
pub const TF_GAMETYPE_MVM: c_int = 5;

/// `TF_GAMETYPE_PASSTIME`: PASS Time.
pub const TF_GAMETYPE_PASSTIME: c_int = 7;

/// `TF_GAMETYPE_PD`: Player Destruction.
pub const TF_GAMETYPE_PD: c_int = 8;

/// `TF_GAMETYPE_RD`: Robot Destruction.
pub const TF_GAMETYPE_RD: c_int = 6;

/// `TF_GAMETYPE_UNDEFINED`: a level whose objectives set no other game type,
/// such as a Special Delivery level, or one without objectives.
pub const TF_GAMETYPE_UNDEFINED: c_int = 0;

/// `TF_HUDTYPE_ARENA`: Arena's HUD.
pub const TF_HUDTYPE_ARENA: c_int = 4;

/// `TF_HUDTYPE_CP`: the control points' HUD.
pub const TF_HUDTYPE_CP: c_int = 2;

/// `TF_HUDTYPE_CTF`: Capture the Flag's HUD.
pub const TF_HUDTYPE_CTF: c_int = 1;

/// `TF_HUDTYPE_ESCORT`: Payload's HUD.
pub const TF_HUDTYPE_ESCORT: c_int = 3;

/// `TF_HUDTYPE_TRAINING`: the training levels' HUD.
pub const TF_HUDTYPE_TRAINING: c_int = 5;

/// `TF_HUDTYPE_UNDEFINED`: the HUD of the game type.
pub const TF_HUDTYPE_UNDEFINED: c_int = 0;
