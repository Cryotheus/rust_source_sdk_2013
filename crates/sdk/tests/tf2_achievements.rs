#![cfg(feature = "tf2")]

//! Tests of locking clients' achievements through `sv_cheats` leases sent
//! by the mock engine.

use source_sdk_2013::net::cheats::{CheatsError, CheatsEvent, ClientCheats, LeaseOutcome};
use source_sdk_2013::net::incoming::Verdict;
use source_sdk_2013::test_support::net::cheats::{MockClient, MockEngine, query_cookie, response};
use source_sdk_2013::test_support::players::user;
use source_sdk_2013::tf2::achievements::{AchievementLock, LockError, LockState};

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
