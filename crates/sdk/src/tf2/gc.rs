//! The types of TF2's messages to the Steam Game Coordinator (GC), for
//! filtering what the game sends through a hook on the
//! [`GameCoordinator`](crate::steam::GameCoordinator).
//!
//! # Strange counters
//!
//! The game raises Strange items' counters, the kill-eater attributes, by
//! asking the GC to: [`INCREMENT_KILL_COUNT_ATTRIBUTE`] for single events,
//! [`INCREMENT_KILL_COUNT_ATTRIBUTE_MULTIPLE`] for the events it batches over
//! 30 seconds, and [`TRACK_UNIQUE_PLAYER_PAIR_EVENT`] for events counted once
//! per pair of players. These are [`STRANGE_PROGRESS`]. The game ignores
//! whether they were sent, and drops a batch once it tried to send it.
//!
//! Contracts record their own kill-eater events: [`QUEST_STRANGE_EVENT`]
//! directly, and possibly [`QUEST_OBJECTIVE_POINTS_CHANGE`] on the GC's side.
//! The game resends unacknowledged contract progress every 30 seconds, for up
//! to 10 minutes.

use crate::steam::MessageType;
use sdk_raw::tf2::gc as raw;

/// `k_EMsgGC_GameServer_LevelInfo`: the map the game server runs.
#[doc(alias("k_EMsgGC_GameServer_LevelInfo"))]
pub const GAME_SERVER_LEVEL_INFO: MessageType = MessageType::new(raw::GAME_SERVER_LEVEL_INFO);

/// `k_EMsgGCGameServerMatchmakingStatus`: the game server's matchmaking state,
/// which a community server sends when its map changes.
#[doc(alias("k_EMsgGCGameServerMatchmakingStatus"))]
pub const GAME_SERVER_MATCHMAKING_STATUS: MessageType =
	MessageType::new(raw::GAME_SERVER_MATCHMAKING_STATUS);

/// `k_EMsgGC_IncrementKillCountAttribute`: one kill-eater event, which raises
/// a Strange item's counter.
#[doc(alias("k_EMsgGC_IncrementKillCountAttribute"))]
pub const INCREMENT_KILL_COUNT_ATTRIBUTE: MessageType =
	MessageType::new(raw::INCREMENT_KILL_COUNT_ATTRIBUTE);

/// `k_EMsgGC_IncrementKillCountAttribute_Multiple`: a batch of kill-eater
/// events, which raise Strange items' counters.
#[doc(alias("k_EMsgGC_IncrementKillCountAttribute_Multiple"))]
pub const INCREMENT_KILL_COUNT_ATTRIBUTE_MULTIPLE: MessageType =
	MessageType::new(raw::INCREMENT_KILL_COUNT_ATTRIBUTE_MULTIPLE);

/// `k_EMsgGCQuestObjective_PointsChange`: a change of a player's contract
/// progress.
#[doc(alias("k_EMsgGCQuestObjective_PointsChange"))]
pub const QUEST_OBJECTIVE_POINTS_CHANGE: MessageType =
	MessageType::new(raw::QUEST_OBJECTIVE_POINTS_CHANGE);

/// `k_EMsgGCQuestStrangeEvent`: a kill-eater event a contract records.
#[doc(alias("k_EMsgGCQuestStrangeEvent"))]
pub const QUEST_STRANGE_EVENT: MessageType = MessageType::new(raw::QUEST_STRANGE_EVENT);

/// The messages that raise Strange items' counters, outside contracts.
pub const STRANGE_PROGRESS: [MessageType; 3] = [
	INCREMENT_KILL_COUNT_ATTRIBUTE,
	INCREMENT_KILL_COUNT_ATTRIBUTE_MULTIPLE,
	TRACK_UNIQUE_PLAYER_PAIR_EVENT,
];

/// `k_EMsgGC_TrackUniquePlayerPairEvent`: a kill-eater event the GC counts
/// once per pair of owner and victim.
#[doc(alias("k_EMsgGC_TrackUniquePlayerPairEvent"))]
pub const TRACK_UNIQUE_PLAYER_PAIR_EVENT: MessageType =
	MessageType::new(raw::TRACK_UNIQUE_PLAYER_PAIR_EVENT);

/// Whether `message` raises a Strange item's counter: one of the
/// [`STRANGE_PROGRESS`] messages, or a contract's [`QUEST_STRANGE_EVENT`].
/// Contract progress, [`QUEST_OBJECTIVE_POINTS_CHANGE`], is not one, though
/// the GC may raise contract-point counters from it on its side.
pub fn is_kill_eater(message: MessageType) -> bool {
	is_strange_progress(message) || message == QUEST_STRANGE_EVENT
}

/// Whether `message` is one of the [`STRANGE_PROGRESS`] messages.
pub fn is_strange_progress(message: MessageType) -> bool {
	STRANGE_PROGRESS.contains(&message)
}
