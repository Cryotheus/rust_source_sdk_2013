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
#[doc(alias = "m_bCheatsEverOn")]
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::net::cheats::test_support::{MockClient, MockEngine, query_cookie, response};
	use crate::net::cheats::{CheatsEvent, CheatsOptions, LeaseOutcome};
	use crate::net::incoming::Verdict;
	use crate::server::InterfaceFactory;
	use std::ffi::{c_char, c_int, c_void};
	use std::num::NonZero;
	use std::time::Duration;

	#[test]
	fn any_confirmed_lease_locks_the_client() {
		let mock = MockEngine::new(&[MockClient::active(2)], c"0", &[]);
		let mut cheats = ClientCheats::default();
		let lock = AchievementLock::default();
		let commands = Purpose::Commands(vec![c"cl_soundscape_flush".into()]);

		cheats.begin(mock.server(), user(2), commands).unwrap();

		let cookie = query_cookie(&mock.take_sent(0)[0]);

		assert_eq!(lock.state(&cheats, user(2)), None);
		cheats.on_response(user(2), &response(cookie, 0, c"1"));
		cheats.on_game_frame(mock.server()).unwrap();
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::Locked));
		assert_eq!(lock.lease(user(2)), None);
	}

	#[test]
	fn bots_and_other_games_are_refused() {
		let bot = MockClient {
			fake: true,
			..MockClient::active(3)
		};
		let mock = MockEngine::new(&[bot], c"0", &[]);
		let mut cheats = ClientCheats::default();
		let mut lock = AchievementLock::new();

		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(3)),
			Err(LockError::Cheats(CheatsError::FakeClient))
		);
		assert_eq!(lock.state(&cheats, user(3)), None);

		unsafe extern "C" fn no_interfaces(_: *const c_char, _: *mut c_int) -> *mut c_void {
			std::ptr::null_mut()
		}

		let factory = InterfaceFactory::new(no_interfaces);
		let scope = ();

		// SAFETY: The factories export nothing, and nothing is looked up.
		let server = unsafe { Server::new(factory, factory, Game::SourceSdk2013, &scope) };

		assert_eq!(
			lock.lock(&mut cheats, server, user(3)),
			Err(LockError::UnsupportedGame)
		);
		assert_eq!(
			lock.lock_all(&mut cheats, server),
			Err(LockError::UnsupportedGame)
		);
	}

	#[test]
	fn clients_are_locked_once_per_level() {
		let mock = MockEngine::new(&[MockClient::active(2)], c"0", &[]);
		let mut cheats = ClientCheats::default();
		let mut lock = AchievementLock::new();

		assert_eq!(lock.state(&cheats, user(2)), None);
		assert_eq!(lock.lease(user(2)), None);
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);

		let lease = lock.lease(user(2)).unwrap();
		let sent = mock.take_sent(0);
		let cookie = query_cookie(&sent[0]);

		assert_eq!(sent.len(), 1);

		// Locking again while pending sends nothing.
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);
		assert!(mock.take_sent(0).is_empty());

		// The answer locks the client once its restore is sent.
		assert_eq!(
			cheats.on_response(user(2), &response(cookie, 0, c"1")),
			Verdict::Block
		);
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::Pending));
		assert_eq!(
			cheats.on_game_frame(mock.server()),
			Ok(vec![CheatsEvent::Finished {
				user_id: user(2),
				lease,
				outcome: LeaseOutcome::Confirmed,
			}])
		);
		assert_eq!(mock.take_sent(0).len(), 1);
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::Locked));
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Locked)
		);
		assert!(mock.take_sent(0).is_empty());

		// A new level: both forget, and the client is locked again, after it
		// is sent the server's values once more.
		cheats.restore_all(mock.server()).unwrap();
		cheats.clear();
		lock.clear();
		assert_eq!(lock.state(&cheats, user(2)), None);
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);
		assert_ne!(lock.lease(user(2)), Some(lease));
		assert!(mock.take_sent(0).is_empty());
		assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
		assert_eq!(mock.take_sent(0).len(), 2);
	}

	#[test]
	fn failed_locks_are_tried_again() {
		let mock = MockEngine::new(&[MockClient::active(2)], c"0", &[]);
		let mut cheats = ClientCheats::new(CheatsOptions {
			confirm_timeout: Duration::ZERO,
			max_attempts: NonZero::<u8>::MIN,
			retry_delay: Duration::ZERO,
		});
		let mut lock = AchievementLock::new();

		lock.lock(&mut cheats, mock.server(), user(2)).unwrap();

		// No answer within the (zero) timeout, on the only attempt.
		let events = cheats.on_game_frame(mock.server()).unwrap();

		assert_eq!(
			events,
			[CheatsEvent::Finished {
				user_id: user(2),
				lease: lock.lease(user(2)).unwrap(),
				outcome: LeaseOutcome::TimedOut,
			}]
		);
		assert_eq!(mock.take_sent(0).len(), 2);
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::Failed));

		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);
		assert_eq!(mock.take_sent(0).len(), 1);
		assert!(lock.forget(user(2)));
		assert_eq!(lock.state(&cheats, user(2)), None);
	}

	#[test]
	fn lock_all_locks_every_connected_player() {
		let connecting = MockClient {
			active: false,
			..MockClient::active(3)
		};
		let bot = MockClient {
			fake: true,
			..MockClient::active(4)
		};
		let host = MockClient {
			loopback: true,
			..MockClient::active(5)
		};
		let mock = MockEngine::new(
			&[
				MockClient::active(2),
				connecting,
				bot,
				MockClient::default(),
				host,
			],
			c"0",
			&[],
		);
		let mut cheats = ClientCheats::default();
		let mut lock = AchievementLock::new();

		assert_eq!(
			lock.lock_all(&mut cheats, mock.server()),
			Ok(vec![
				(user(2), Ok(LockState::Pending)),
				(user(3), Ok(LockState::Pending)),
				(user(5), Err(LockError::Cheats(CheatsError::Loopback))),
			])
		);
		assert_eq!(mock.take_sent(0).len(), 1);

		// The client still connecting is sent its lease once in the game.
		assert!(mock.take_sent(1).is_empty());
		mock.set_client(1, MockClient::active(3));
		assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
		assert_eq!(mock.take_sent(1).len(), 1);

		// Again, as after unpausing: pending clients are left as they are.
		assert_eq!(
			lock.lock_all(&mut cheats, mock.server()),
			Ok(vec![
				(user(2), Ok(LockState::Pending)),
				(user(3), Ok(LockState::Pending)),
				(user(5), Err(LockError::Cheats(CheatsError::Loopback))),
			])
		);
		assert!(mock.take_sent(0).is_empty());
	}

	#[test]
	fn refusing_clients_are_not_tried_again() {
		let mock = MockEngine::new(&[MockClient::active(2)], c"0", &[]);
		let mut cheats = ClientCheats::default();
		let mut lock = AchievementLock::new();

		lock.lock(&mut cheats, mock.server(), user(2)).unwrap();

		let cookie = query_cookie(&mock.take_sent(0)[0]);

		cheats.on_response(user(2), &response(cookie, 0, c"0"));
		assert_eq!(
			cheats.on_game_frame(mock.server()),
			Ok(vec![CheatsEvent::Finished {
				user_id: user(2),
				lease: lock.lease(user(2)).unwrap(),
				outcome: LeaseOutcome::Refused,
			}])
		);
		assert_eq!(mock.take_sent(0).len(), 1);
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::Refused));
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Refused)
		);
		assert!(mock.take_sent(0).is_empty());

		// The refusal lasts until the coordinator is cleared.
		cheats.clear();
		lock.clear();
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);
	}

	#[test]
	fn server_cheats_are_checked_again() {
		let mock = MockEngine::new(&[MockClient::active(2)], c"1", &[]);
		let mut cheats = ClientCheats::default();
		let mut lock = AchievementLock::new();

		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::ServerCheats)
		);
		assert!(mock.take_sent(0).is_empty());
		assert_eq!(lock.state(&cheats, user(2)), Some(LockState::ServerCheats));
		assert_eq!(lock.lease(user(2)), None);

		// Still set: nothing is sent.
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::ServerCheats)
		);
		assert!(mock.take_sent(0).is_empty());

		// Cleared, maybe before the client's frame saw it: a lease is taken.
		mock.set_cheats(c"0");
		assert_eq!(
			lock.lock(&mut cheats, mock.server(), user(2)),
			Ok(LockState::Pending)
		);
		assert_eq!(mock.take_sent(0).len(), 1);
	}

	fn user(id: u16) -> UserId {
		UserId::new(id).unwrap()
	}
}
