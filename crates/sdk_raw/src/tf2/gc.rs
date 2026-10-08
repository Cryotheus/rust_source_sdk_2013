//! The types of TF2's messages to the Steam Game Coordinator (GC), from the
//! `EGCItemMsg` and `ETFGCMsg` enums of the SDK's `econ_gcmessages.proto` and
//! `tf_gcmessages.proto`.
//!
//! These are message ids, without [`PROTOBUF_FLAG`], which the game sets in
//! the type of each of these when it sends it.
//!
//! [`PROTOBUF_FLAG`]: crate::steam::PROTOBUF_FLAG

/// `k_EMsgGC_GameServer_LevelInfo`: the map the game server runs
/// (`tf_gcmessages.proto:114`).
#[doc(alias("k_EMsgGC_GameServer_LevelInfo"))]
pub const GAME_SERVER_LEVEL_INFO: u32 = 5700;

/// `k_EMsgGCGameServerMatchmakingStatus`: the game server's matchmaking state,
/// which a community server sends when its map changes
/// (`tf_gcmessages.proto:183`).
#[doc(alias("k_EMsgGCGameServerMatchmakingStatus"))]
pub const GAME_SERVER_MATCHMAKING_STATUS: u32 = 6295;

/// `k_EMsgGC_IncrementKillCountAttribute`: one kill-eater event, which raises
/// a Strange item's counter (`econ_gcmessages.proto:180`). The game sends it
/// at once for the events it does not batch, and for kills tracked by items
/// that are not equipped.
#[doc(alias("k_EMsgGC_IncrementKillCountAttribute"))]
pub const INCREMENT_KILL_COUNT_ATTRIBUTE: u32 = 1071;

/// `k_EMsgGC_IncrementKillCountAttribute_Multiple`: a batch of kill-eater
/// events, which raise Strange items' counters, which the game sends every 30
/// seconds (`econ_gcmessages.proto:217`).
#[doc(alias("k_EMsgGC_IncrementKillCountAttribute_Multiple"))]
pub const INCREMENT_KILL_COUNT_ATTRIBUTE_MULTIPLE: u32 = 1097;

/// `k_EMsgGCQuestObjective_PointsChange`: a change of a player's contract
/// progress (`econ_gcmessages.proto:345`).
#[doc(alias("k_EMsgGCQuestObjective_PointsChange"))]
pub const QUEST_OBJECTIVE_POINTS_CHANGE: u32 = 2562;

/// `k_EMsgGCQuestStrangeEvent`: a kill-eater event a contract records, such
/// as points contributed to friends' contracts (`tf_gcmessages.proto:321`).
#[doc(alias("k_EMsgGCQuestStrangeEvent"))]
pub const QUEST_STRANGE_EVENT: u32 = 6577;

/// `k_EMsgGC_TrackUniquePlayerPairEvent`: a kill-eater event the GC counts
/// once per pair of owner and victim (`econ_gcmessages.proto:198`).
#[doc(alias("k_EMsgGC_TrackUniquePlayerPairEvent"))]
pub const TRACK_UNIQUE_PLAYER_PAIR_EVENT: u32 = 1084;
