//! Turns off one client's achievements until its next level load, as
//! `sv_cheats 1` does, without changing the server's own `sv_cheats`.
//!
//! # Why one frame is enough
//!
//! TF2's achievement manager runs only in the client: the server compiles no
//! TF achievements (`game/shared/tf/achievements_tf.cpp:10`), and can only
//! send progress in `AchievementEvent` user messages. Every client frame,
//! `CAchievementMgr::Update` latches `m_bCheatsEverOn` once the client's copy
//! of `sv_cheats` is set (`game/shared/achievementmgr.cpp:436-454`, called
//! from `CHLClient::HudUpdate` at `game/client/cdll_client_int.cpp:1308`).
//! Only `LevelInitPreEntity` clears it (`achievementmgr.cpp:481-483`). While
//! it is set, `CheckAchievementsEnabled` refuses achievements, their
//! progress, and TF's per-class stat records, printing `Achievements
//! disabled: cheats turned on in this app session.` to the client's console
//! (`achievementmgr.cpp:1102-1117`). The replay achievements, which are
//! always enabled, are the exception, as with the server's own `sv_cheats`.
//!
//! [`AchievementLock::lock`] therefore takes a [`ClientCheats`] lease that
//! shows the client `sv_cheats` set for one frame, then restores it, as the
//! [`cheats`](crate::net::cheats) module describes. Any lease the client
//! confirmed has the same effect, such as one running cheat commands.
//!
//! # Wiring
//!
//! Drive the [`ClientCheats`] as its module describes, and make every other
//! sender of `sv_cheats` to clients, such as an existing cheat-command
//! runner, use that same coordinator before wiring a lock in: a lock is only
//! confirmed if nothing else sends the client `sv_cheats` during its window.
//!
//! Despite the console message, the flag lasts for a level
//! (`achievementmgr.h:149`): the client clears it when it loads the next map
//! (`cdll_client_int.cpp:1605-1633`) or reconnects. Lock clients:
//!
//! - from [`GameEventId::PlayerActivate`] on every level, after clearing the
//!   lock and the coordinator at level shutdown, since clients keep their
//!   user IDs across a level change;
//! - with [`AchievementLock::lock_all`] once the plugin loads, since clients
//!   already connected get no `PlayerActivate`, and once it is unpaused,
//!   since pausing requires [`ClientCheats::restore_all`], which cancels the
//!   locks still pending.
//!
//! A lock that ended without confirmation reports [`LockState::Failed`], and
//! is only tried again by another call to [`AchievementLock::lock`] or
//! [`AchievementLock::lock_all`].
//!
//! # Limits
//!
//! - A lock cannot be undone: the client's achievements stay off until it
//!   loads another level, even if the plugin no longer wants them off.
//! - For about one round trip, the client may change its own cheat-flagged
//!   variables and run client-side cheat commands, as with any lease, and
//!   longer if its answer cannot be seen. Retail clients have no command that
//!   unlocks achievements.
//! - The lock is enforced by the client. A modified client can ignore it, and
//!   external tools can unlock achievements anyway: it matches what
//!   `sv_cheats 1` does, and is no anti-cheat.
//! - Bots and SourceTV earn no achievements, and a listen server's host shares
//!   the server's `sv_cheats`, so they cannot be locked.
//!
//! The achievement manager's latch is in the SDK's public client code, cited
//! above. That the client applies `sv_cheats` from the server, and runs that
//! frame between the lease's messages and the restore, is behaviour of the
//! closed-source engine, observed on TF2's 64-bit Windows server at
//! `sv_cheats 0` with a retail client: the lock is confirmed, the client's
//! `sv_cheats` returns to 0, and its next achievement event prints the
//! message above. Linux GNU servers, and delayed or lost packets, are
//! untested.
//!
//! [`GameEventId::PlayerActivate`]: crate::tf2::game_events::GameEventId::PlayerActivate

use crate::interfaces::GameClient;
use crate::net::cheats::{Begun, CheatsError, ClientCheats, LeaseId, Purpose};
use crate::players::UserId;
use crate::{Game, Server};

/// What locking one client did.
pub type LockResult = Result<LockState, LockError>;

/// The clients whose achievements a plugin turned off this level.
///
/// It records which lease locks each client, and reads the rest from the
/// [`ClientCheats`] that coordinates the leases, which the plugin keeps and
/// drives as its module describes. Like it, a lock holds only plain data.
#[doc(alias("m_bCheatsEverOn"))]
#[derive(Debug, Default)]
pub struct AchievementLock {
	requests: Vec<(UserId, Request)>,
}

impl AchievementLock {
	/// A lock that knows no client, which a `static` can hold.
	pub const fn new() -> Self {
		Self {
			requests: Vec::new(),
		}
	}

	/// Forgets every client, as at level shutdown, when [`ClientCheats::clear`]
	/// is called too.
	pub fn clear(&mut self) {
		self.requests.clear();
	}

	/// Forgets a client, as once it disconnects. Returns whether it was known.
	pub fn forget(&mut self, user_id: UserId) -> bool {
		let known = self.requests.len();

		self.requests.retain(|&(id, _)| id != user_id);
		self.requests.len() != known
	}

	/// The lease this lock last took for the client with `user_id`, to match
	/// against [`CheatsEvent::Finished`] for how it ended, or `None` if it
	/// took none since the client was last forgotten or cleared.
	///
	/// [`CheatsEvent::Finished`]: crate::net::cheats::CheatsEvent::Finished
	pub fn lease(&self, user_id: UserId) -> Option<LeaseId> {
		self.requests
			.iter()
			.find_map(|&(id, request)| match request {
				Request::Lease(lease) if id == user_id => Some(lease),
				_ => None,
			})
	}

	/// Turns off the achievements of the client with `user_id` until it loads
	/// another level, by taking a lease from `cheats`.
	///
	/// Nothing new is sent if the client is locked, being locked, or refused,
	/// or if the server's own `sv_cheats` is set, which every client sees. A
	/// client whose lock failed, or that was last found under the server's
	/// own `sv_cheats`, is tried again.
	///
	/// Fails outside TF2, for the clients [`ClientCheats::begin`] refuses,
	/// such as bots, and when the lease's messages cannot be sent.
	pub fn lock(
		&mut self,
		cheats: &mut ClientCheats,
		server: Server<'_>,
		user_id: UserId,
	) -> LockResult {
		if server.game() != Game::TeamFortress2 {
			return Err(LockError::UnsupportedGame);
		}

		match self.state(cheats, user_id) {
			None | Some(LockState::Failed | LockState::ServerCheats) => {}
			Some(state) => return Ok(state),
		}

		let (request, state) = match cheats.begin(server, user_id, Purpose::Observe)? {
			Begun::Lease(lease) => (Request::Lease(lease), LockState::Pending),
			Begun::ServerCheats => (Request::ServerCheats, LockState::ServerCheats),
		};

		self.forget(user_id);
		self.requests.push((user_id, request));

		Ok(state)
	}

	/// [`Self::lock`]s every connected client but bots and SourceTV, and
	/// returns what happened to each. Clients still connecting are locked
	/// once they are in the game.
	///
	/// Call it once the plugin loads, and once it is unpaused, since those
	/// clients get no [`GameEventId::PlayerActivate`] then.
	///
	/// Fails outside TF2, and when the engine's game server cannot be found.
	///
	/// [`GameEventId::PlayerActivate`]: crate::tf2::game_events::GameEventId::PlayerActivate
	pub fn lock_all(
		&mut self,
		cheats: &mut ClientCheats,
		server: Server<'_>,
	) -> Result<Vec<(UserId, LockResult)>, LockError> {
		if server.game() != Game::TeamFortress2 {
			return Err(LockError::UnsupportedGame);
		}

		let user_ids: Vec<UserId> = server
			.valve_engine()
			.map_err(CheatsError::from)?
			.game_server()
			.ok_or(CheatsError::NoServer)?
			.clients()
			.filter(|client| client.is_connected() && !client.is_fake() && !client.is_hltv())
			.filter_map(GameClient::user_id)
			.collect();

		Ok(user_ids
			.into_iter()
			.map(|user_id| (user_id, self.lock(cheats, server, user_id)))
			.collect())
	}

	/// How far the client with `user_id` is locked, or `None` if this lock
	/// never tried, and `cheats` knows of no lease of the client that was
	/// confirmed or refused.
	pub fn state(&self, cheats: &ClientCheats, user_id: UserId) -> Option<LockState> {
		let client = cheats.client(user_id);

		if client.is_some_and(|client| client.confirmed) {
			return Some(LockState::Locked);
		}

		let request = self
			.requests
			.iter()
			.find(|&&(id, _)| id == user_id)
			.map(|&(_, request)| request);

		match request {
			Some(Request::Lease(lease)) if cheats.lease(lease).is_some() => {
				Some(LockState::Pending)
			}

			_ if client.is_some_and(|client| client.refused) => Some(LockState::Refused),
			Some(Request::Lease(_)) => Some(LockState::Failed),
			Some(Request::ServerCheats) => Some(LockState::ServerCheats),
			None => None,
		}
	}
}

/// Why a client's achievements could not be turned off.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockError {
	/// The server is not running Team Fortress 2.
	#[error("achievement locks require Team Fortress 2")]
	UnsupportedGame,

	/// The lease could not begin.
	#[error(transparent)]
	Cheats(#[from] CheatsError),
}

/// How far a client's achievements are turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LockState {
	/// The lease is waiting to be sent, or for the client's answer and
	/// restore.
	Pending,

	/// The client confirmed a lease, as [`ClientStatus::confirmed`] tells: its
	/// achievements are off until it loads another level.
	///
	/// [`ClientStatus::confirmed`]: crate::net::cheats::ClientStatus::confirmed
	Locked,

	/// The server's own `sv_cheats` was set when the client was last locked,
	/// which every client sees. That turns off the client's achievements if
	/// it stays set for one of the client's frames, which nothing confirms:
	/// locking the client again checks again, and takes a lease once the
	/// server's `sv_cheats` is off.
	ServerCheats,

	/// The client answered a lease that its `sv_cheats` was not set, as
	/// [`ClientStatus::refused`] tells: it ignores the value, as a modified
	/// client may. Locking it again sends nothing until the coordinator is
	/// cleared.
	///
	/// [`ClientStatus::refused`]: crate::net::cheats::ClientStatus::refused
	Refused,

	/// The lease ended without confirmation: the client did not answer in
	/// time, its answer could not be read, the lease's messages did not fit,
	/// [`ClientCheats::restore_all`] cancelled it, the server's own
	/// `sv_cheats` was set when it was due, or the client left. Match
	/// [`AchievementLock::lease`] against
	/// [`CheatsEvent::Finished`](crate::net::cheats::CheatsEvent::Finished)
	/// for which. Locking the client again takes a new lease.
	Failed,
}

/// What a lock did for one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
	/// Took this lease.
	Lease(LeaseId),

	/// Found the server's own `sv_cheats` set.
	ServerCheats,
}
