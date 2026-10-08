//! TF2 sounds played to chosen clients without a position, as TF2's announcer
//! plays its lines, sound script entries played from an entity, and the
//! precaching of sound script entries.
//!
//! [`broadcast`] sends each recipient TF2's `teamplay_broadcast_audio` game
//! event, which the client answers by playing the sound from its own view.
//! The client resolves the name, so it can be a sound script entry, such as
//! `Announcer.RoundBegins5Seconds`, or a sample path. [`emit_script_sound`]
//! plays a sound script entry from an entity, as the game plays its own. For
//! a sample played from an entity or a point, to chosen recipients, use
//! [`EngineSound::emit_sound`](crate::interfaces::EngineSound::emit_sound),
//! which takes precached samples only. To make a player speak a voice line,
//! use [`voice`](crate::tf2::voice).
//!
//! # Precaching
//!
//! The server's precache table lists the samples clients load ahead of play.
//! It is emptied at every level change, so a plugin precaches its sounds
//! again for each map: samples through
//! [`EngineSound::precache_sound`](crate::interfaces::EngineSound::precache_sound),
//! and every sample of a sound script entry through [`precache_script_sound`].
//! TF2 precaches the sounds it plays itself, such as every voice line its
//! response rules can choose, when the level starts.
//!
//! # Bots
//!
//! Bots and other fake clients have no client, so they hear nothing.
//! [`broadcast`] skips them.
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, a client has been observed to play a
//! broadcast sound script entry, `Game.YourTeamWon`, as its handler in the
//! SDK does (`game/client/clientmode_shared.cpp`): from its own view, not
//! positioned in the world. Whether a client loads on demand a sample the
//! server never precached has not been tested, nor have Linux servers.

#[cfg(test)]
#[path = "../tests/tf2/sound.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::engine_sound::SoundFlags;
use crate::interfaces::game_event::CreateEventError;
use crate::net::messages::GameEvent;
use crate::net::{EncodeError, NetChannel, NetMessage, Reliability};
use crate::tf2::game_events::GameEventId;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::user_messages::Recipients;
use crate::{Game, InterfaceError, Server};
use std::ffi::{CStr, c_int};

/// The `team` of a `teamplay_broadcast_audio` event that clients on every team
/// play.
const ANY_TEAM: c_int = 255;

/// The game event TF2's `BroadcastSound` fires.
const BROADCAST_EVENT: &CStr = GameEventId::TeamplayBroadcastAudio.name_cstr();

/// The longest sound name, in bytes, that [`broadcast`] sends.
///
/// An `svc_GameEvent` carries at most 2047 bits of event. TF2 describes
/// `teamplay_broadcast_audio` (`resource/modevents.res`) as a byte `team`,
/// the string `sound`, and the shorts `additional_flags` and `player`, which
/// with the event's 9-bit ID take 49 bits besides the name, leaving 249
/// bytes for the name and its terminator.
pub const MAX_SOUND_LEN: usize = 248;

/// The `player` of a `teamplay_broadcast_audio` event that names no player.
const NO_PLAYER: c_int = -1;

/// Why [`broadcast`] could not send its event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BroadcastError {
	/// The game event manager did not create `teamplay_broadcast_audio`,
	/// which it refuses for an event the game did not declare or no listener
	/// asked for. A TF2 client asks for it while it connects, by sending its
	/// listeners to the engine (`clc_ListenEvents`), so until a human client
	/// has done so, this can fail even though a recipient has a player and a
	/// channel.
	#[error(transparent)]
	Create(#[from] CreateEventError),

	/// The sound's name is longer than [`MAX_SOUND_LEN`] bytes, or the encoded
	/// event does not fit in an `svc_GameEvent`, which carries fewer than 2048
	/// bits. The name's length is checked before anything else.
	#[error(transparent)]
	Encode(#[from] EncodeError),

	/// An interface needed to send the event is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The entity given for the sound's voice has no edict, so no client
	/// knows it.
	#[error("the voice's entity is not networked, so no client knows it")]
	NotNetworked,

	/// The game event manager did not encode the event, which it refuses for
	/// an event without a description.
	#[error("the game event manager could not encode the event")]
	NotSerialized,

	/// The server does not run TF2.
	#[error("broadcast sounds require a TF2 server")]
	NotTf2,
}

/// Why [`emit_script_sound`] could not play a sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EmitError {
	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The server does not run TF2.
	#[error("script sounds require a TF2 server")]
	NotTf2,

	/// The binding adapter of the entity's `EmitSound` reported failure.
	#[error("the native EmitSound method rejected its arguments")]
	Rejected,

	/// The entity's script class descriptors lack `EmitSound`, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native EmitSound method")]
	UnsupportedMethod,
}

impl From<BindingError> for EmitError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Why [`precache_script_sound`] could not precache a sound.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrecacheError {
	/// An interface needed to find the world is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// No map is loaded, its entities have not spawned yet, or the world is
	/// marked for deletion.
	#[error("the world entity, which the precache is made through, is missing")]
	NoWorld,

	/// The server does not run TF2.
	#[error("script sound precaching requires a TF2 server")]
	NotTf2,

	/// The binding adapter of the world's `PrecacheScriptSound` reported
	/// failure.
	#[error("the native PrecacheScriptSound method rejected its arguments")]
	Rejected,

	/// The world's script class descriptors lack `PrecacheScriptSound`, or
	/// its signature differs from the SDK's.
	#[error("the game does not expose the expected native PrecacheScriptSound method")]
	UnsupportedMethod,
}

impl From<BindingError> for PrecacheError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Plays a sound to every client `recipients` names, from the client's own
/// view, as TF2's announcer lines play. Returns how many clients' channels
/// queued it.
///
/// This builds the `teamplay_broadcast_audio` event that TF2's
/// `CTFGameRules::BroadcastSound` fires (`game/shared/tf/tf_gamerules.cpp`),
/// but instead of firing it to every client, sends it to each recipient as an
/// `svc_GameEvent`, in the stream the event's description asks for. The
/// client plays `sound`, a sound script entry or a sample path of at most
/// [`MAX_SOUND_LEN`] bytes, with `flags` added, through
/// `C_BaseEntity::EmitSound` from `SOUND_FROM_LOCAL_PLAYER`, so it does not
/// fade with distance. The server neither resolves nor checks the name.
///
/// # Teams
///
/// The client plays the event only for its `team`
/// (`ClientModeShared::FireGameEvent` in `game/client/clientmode_shared.cpp`):
/// a client plays a team number if it is on that team, or spectates a player
/// on it in first-person, chase or point-of-interest mode. Team 0 plays only
/// for a client that finds no team of its own (`GetLocalTeam` is null) and
/// spectates a player on team 0, which almost never happens. Team 255 plays
/// for every client whatever its team, and is what this sends, so
/// `recipients` alone choose who hears the sound.
///
/// # Voice
///
/// `voice_of` fills the event's `player` field. If it is a TF2 player, and
/// the sound plays on `CHAN_VOICE`, each client pitches the sound as it
/// pitches that player's voice (`C_TFPlayer::ClientAdjustStartSoundParams`),
/// such as for Pyrovision or a voice-changing spell. `None` sends -1, which no
/// client adjusts the sound for. Clients adjust the sound for TF2 players
/// only, so any other networked entity changes nothing, as `None` does.
///
/// # Recipients
///
/// Only recipients with a player entity and a net channel get the event.
/// Bots, SourceTV and empty slots have no channel, and a client still
/// connecting has no player until the game creates one
/// (`ClientPutInServer`), so all of them are skipped. If none is left, this
/// returns `Ok(0)` before creating the event, which the game event manager
/// refuses to create while no client listens for it, as on a server with
/// only bots. A recipient whose stream has no room left for the event is not
/// counted.
///
/// [`Recipients::reliable`] has no effect here: the event goes in the stream
/// its description asks for, as when the game fires it.
///
/// The event is sent, never fired, so no listener on the server sees it.
/// SourceTV relays the fired event to its spectators and demos
/// (`CTFHLTVDirector::GetModEvents` in `game/server/tf/tf_hltvdirector.cpp`),
/// but SourceTV demos and Replay recordings never contain a broadcast.
///
/// # Errors
///
/// The sound's length and `voice_of` are checked first, so an empty or
/// bot-only `recipients` fails as any other would: with
/// [`BroadcastError::Encode`] for a sound longer than [`MAX_SOUND_LEN`] bytes,
/// and with [`BroadcastError::NotNetworked`] for a `voice_of` without an
/// edict.
#[doc(alias("BroadcastSound", "teamplay_broadcast_audio"))]
pub fn broadcast(
	server: Server<'_>,
	recipients: &Recipients,
	sound: &CStr,
	flags: SoundFlags,
	voice_of: Option<Entity<'_>>,
) -> Result<usize, BroadcastError> {
	if server.game() != Game::TeamFortress2 {
		return Err(BroadcastError::NotTf2);
	}

	EncodeError::check_len("sound", sound.count_bytes(), MAX_SOUND_LEN)?;

	let player = match voice_of {
		Some(player) => player.edict().ok_or(BroadcastError::NotNetworked)?.index(),
		None => NO_PLAYER,
	};
	let engine = server.valve_engine()?;
	let channels: Vec<NetChannel<'_>> = recipients
		.players()
		.iter()
		.filter_map(|&index| engine.edict_of_index(index))
		.filter(|edict| edict.entity().is_some())
		.filter_map(|edict| engine.net_channel(edict))
		.collect();

	if channels.is_empty() {
		return Ok(0);
	}

	let events = server.game_events()?;
	let mut event = events.create_event(BROADCAST_EVENT)?;

	event.set_int(c"team", ANY_TEAM);
	event.set_string(c"sound", sound);
	event.set_int(c"additional_flags", flags.bits());
	event.set_int(c"player", player);

	// The engine, which the SDK does not include, sends a fired event to each
	// client in the stream `IGameEvent::IsReliable` names.
	let reliability = match event.as_event().is_reliable() {
		true => Reliability::Reliable,
		false => Reliability::Unreliable,
	};
	let data = events
		.serialize_event(event.as_event())
		.ok_or(BroadcastError::NotSerialized)?;

	drop(event);

	let message = GameEvent { data: &data }.encode()?;

	Ok(channels
		.into_iter()
		.filter(|channel| channel.send_encoded(&message, reliability).is_ok())
		.count())
}

/// Plays a sound script entry, such as `TFPlayer.Decapitated`, from `entity`,
/// as the game's own `CBaseEntity::EmitSound` does: positioned at the entity,
/// on the entry's channel, at its volume, pitch and sound level, to the
/// clients that can hear the entity's origin (`CPASAttenuationFilter`).
///
/// This calls the `EmitSound` member that `CBaseEntity` exposes to VScript
/// (`ScriptEmitSound`, `game/shared/SoundEmitterSystem.cpp`) on the entity,
/// through its native binding, without a script VM. The member returns
/// nothing, so an unknown entry is not reported as an error; the game plays
/// nothing. The entry's samples must be precached, as TF2 precaches the
/// sounds it plays itself, or as [`precache_script_sound`] does.
///
/// The game's filter does not use prediction rules, so unlike the game's
/// predicted sounds, it reaches a player whose command the game is running.
/// The engine's `EmitSound` runs before this returns, with any plugin hook
/// on it.
#[doc(alias("EmitSound", "ScriptEmitSound"))]
pub fn emit_script_sound(
	server: Server<'_>,
	entity: Entity<'_>,
	name: &CStr,
) -> Result<(), EmitError> {
	if server.game() != Game::TeamFortress2 {
		return Err(EmitError::NotTf2);
	}

	if entity.is_marked_for_deletion() {
		return Err(EmitError::MarkedForDeletion);
	}

	// SAFETY: The checked `CBaseEntity` member looks the entry up and sends
	// the sound through the engine, which copies what it keeps. It creates and
	// frees no entities, and reads the name only during the call.
	unsafe {
		binding::call(
			entity,
			c"CBaseEntity",
			c"EmitSound",
			&mut [binding::string(name)],
			binding::VOID,
		)?;
	}

	Ok(())
}

/// Precaches every sample of a sound script entry, such as `Scout.Thanks01`,
/// for the current level, as the game's `CBaseEntity::PrecacheScriptSound`
/// does.
///
/// A name that is no entry is precached as a sample path if it contains
/// `.wav` in any case, or `.mp3` in lowercase only
/// (`CSoundEmitterSystem::PrecacheScriptSound` in
/// `game/shared/SoundEmitterSystem.cpp`), so give sample paths a lowercase
/// extension.
///
/// This calls the `PrecacheScriptSound` member that `CBaseEntity` exposes to
/// VScript (`game/server/baseentity.cpp`) on the world entity, through its
/// native binding, without a script VM. The member returns nothing, so an
/// unknown entry is not reported as an error; the game prints
/// `PrecacheScriptSound '...' failed, no such sound script entry` to the
/// developer console instead, the first time each name fails.
///
/// # Timing
///
/// The world exists once the map's entities have spawned; before that, this
/// fails with [`PrecacheError::NoWorld`]. The game allows precaching until
/// the end of its own `ServerActivate`, which closes the window
/// (`CBaseEntity::SetAllowPrecache(false)` in `game/server/gameinterface.cpp`),
/// so precache in a hook that runs after the game's `LevelInit`, or before
/// the game's `ServerActivate`. A callback that runs after the game's
/// `ServerActivate`, such as a post-hook of it, is already late. Unlike
/// VScript's global `PrecacheScriptSound`, the member does not reopen the
/// window, so a late call still precaches each sample, but the game warns
/// `Late precache of ...` for each one the engine reports as not precached
/// yet (`CBaseEntity::PrecacheSound`).
#[doc(alias("PrecacheScriptSound"))]
pub fn precache_script_sound(server: Server<'_>, name: &CStr) -> Result<(), PrecacheError> {
	if server.game() != Game::TeamFortress2 {
		return Err(PrecacheError::NotTf2);
	}

	let world = server
		.server_tools()?
		.entity_by_index(0)
		.filter(|world| !world.is_marked_for_deletion())
		.ok_or(PrecacheError::NoWorld)?;

	// SAFETY: The checked `CBaseEntity` member looks the entry up and adds its
	// samples to the engine's precache table, which copies them. It creates
	// and frees no entities, and reads the name only during the call.
	unsafe {
		binding::call(
			world,
			c"CBaseEntity",
			c"PrecacheScriptSound",
			&mut [binding::string(name)],
			binding::VOID,
		)?;
	}

	Ok(())
}
