//! Tests of `sv_cheats` leases through a mock engine: which clients can hold
//! one, what their channels are sent, and the restore built from the engine's
//! console variable registry.

use source_sdk_2013::commands::CommandFlags;
use source_sdk_2013::net::NetMessage;
use source_sdk_2013::net::cheats::{
	Begun, CheatsError, CheatsEvent, LeaseOutcome, LeaseStatus, Purpose,
};
use source_sdk_2013::net::incoming::Verdict;
use source_sdk_2013::net::messages::SetConVar;
use source_sdk_2013::test_support::net::cheats::{
	Decoded, MockClient, MockEngine, MockVar, query_cookie,
};
use source_sdk_2013::test_support::net_leases::{cheats, confirm, spoof};
use source_sdk_2013::test_support::players::user;

/// Variables a mock registry lists after `sv_cheats`.
const VARS: [MockVar; 7] = [
	MockVar::var(
		c"host_timescale",
		c"1.0",
		c"0.5",
		CommandFlags::REPLICATED.union(CommandFlags::CHEAT),
	),
	MockVar::command(c"status"),
	MockVar::var(
		c"tf_grapplinghook_move_speed",
		c"750",
		c"750",
		CommandFlags::REPLICATED.union(CommandFlags::CHEAT),
	),
	MockVar::var(c"mp_timelimit", c"0", c"30", CommandFlags::NOTIFY),
	MockVar::var(
		c"sv_gravity",
		c"800",
		c"400",
		CommandFlags::REPLICATED.union(CommandFlags::NOTIFY),
	),
	MockVar::var(c"r_drawothermodels", c"1", c"2", CommandFlags::CHEAT),
	MockVar::var(
		c"tf_avoidteammates",
		c"1",
		c"0",
		CommandFlags::REPLICATED.union(CommandFlags::CHEAT),
	),
];

#[test]
fn begin_refuses_clients_without_their_own_sv_cheats() {
	let bot = MockClient {
		fake: true,
		..MockClient::active(3)
	};
	let tv = MockClient {
		hltv: true,
		..MockClient::active(4)
	};
	let host = MockClient {
		loopback: true,
		..MockClient::active(5)
	};
	let unreachable = MockClient {
		no_channel: true,
		..MockClient::active(6)
	};
	let mock = MockEngine::new(
		&[MockClient::default(), bot, tv, host, unreachable],
		c"0",
		&[],
	);
	let mut cheats = cheats();

	for (id, error) in [
		(3, CheatsError::FakeClient),
		(4, CheatsError::FakeClient),
		(5, CheatsError::Loopback),
		(6, CheatsError::NoChannel),
		(9, CheatsError::NoClient(user(9))),
	] {
		assert_eq!(
			cheats.begin(mock.server(), user(id), Purpose::Observe),
			Err(error)
		);
	}

	for slot in 1..5 {
		assert!(mock.take_sent(slot).is_empty());
	}

	assert_eq!(cheats.client(user(3)), None);
}

#[test]
fn engine_leases_wait_for_activity_and_restore_all_reaches_open_clients() {
	let connecting = MockClient {
		active: false,
		..MockClient::active(2)
	};
	let mock = MockEngine::new(&[connecting, MockClient::active(3)], c"0", &[]);
	let mut cheats = cheats();
	let Ok(Begun::Lease(waiting)) = cheats.begin(mock.server(), user(2), Purpose::Observe) else {
		panic!("a lease");
	};
	let Ok(Begun::Lease(sent)) = cheats.begin(mock.server(), user(3), Purpose::Observe) else {
		panic!("a lease");
	};

	assert!(mock.take_sent(0).is_empty());
	assert_eq!(mock.take_sent(1).len(), 1);
	assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
	assert!(mock.take_sent(0).is_empty());

	mock.set_client(0, MockClient::active(2));
	assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
	assert_eq!(mock.take_sent(0).len(), 1);
	assert_eq!(cheats.lease(waiting), Some(LeaseStatus::Sent));

	// The second client left before restoring.
	mock.set_client(1, MockClient::default());

	assert_eq!(
		cheats.restore_all(mock.server()),
		Ok(vec![
			CheatsEvent::Finished {
				user_id: user(2),
				lease: waiting,
				outcome: LeaseOutcome::Cancelled,
			},
			CheatsEvent::Finished {
				user_id: user(3),
				lease: sent,
				outcome: LeaseOutcome::Cancelled,
			},
		])
	);
	assert_eq!(
		mock.take_sent(0),
		[vec![Decoded::SetConVar(vec![(
			c"sv_cheats".into(),
			c"0".into()
		)])]]
	);
	assert!(mock.take_sent(1).is_empty());
	assert_eq!(cheats.client(user(3)), None);
	assert!(cheats.forget(user(2)));
}

#[test]
fn engine_restores_that_do_not_fit_fall_back_to_sv_cheats() {
	let mock = MockEngine::new(&[MockClient::active(2)], c"0", &VARS);
	let mut cheats = cheats();
	let Ok(Begun::Lease(lease)) = cheats.begin(mock.server(), user(2), Purpose::Observe) else {
		panic!("a lease");
	};
	let cookie = query_cookie(&mock.take_sent(0)[0]);
	let alone = SetConVar {
		convars: &[(c"sv_cheats", c"0")],
	}
	.encode()
	.unwrap()
	.len();

	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	mock.room(0, alone);

	let events = cheats.on_game_frame(mock.server()).unwrap();

	assert!(matches!(
		events[..],
		[
			CheatsEvent::RestoreFailed { .. },
			CheatsEvent::Finished {
				outcome: LeaseOutcome::Confirmed,
				..
			}
		]
	));
	assert_eq!(
		mock.take_sent(0),
		[vec![Decoded::SetConVar(vec![(
			c"sv_cheats".into(),
			c"0".into()
		)])]]
	);
	assert_eq!(cheats.lease(lease), None);

	mock.room(0, usize::MAX);
	assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
	assert_eq!(mock.take_sent(0).len(), 1);
	assert!(!cheats.client(user(2)).unwrap().restore_pending);
}

#[test]
fn leases_reach_the_engine_and_restore_from_its_registry() {
	let player = MockClient::active(2);
	let mock = MockEngine::new(&[MockClient::default(), player], c"0", &VARS);
	let mut cheats = cheats();
	let Ok(Begun::Lease(lease)) = cheats.begin(mock.server(), user(2), Purpose::Observe) else {
		panic!("a lease");
	};
	let sent = mock.take_sent(1);
	let cookie = query_cookie(&sent[0]);

	assert_eq!(sent, [spoof(cookie, &[])]);
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);

	// The server's sv_cheats changed meanwhile: the restore sends it as it is.
	mock.set_cheats(c"2");

	assert_eq!(
		cheats.on_game_frame(mock.server()),
		Ok(vec![CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}])
	);
	assert_eq!(
		mock.take_sent(1),
		[vec![Decoded::SetConVar(vec![
			(c"sv_cheats".into(), c"2".into()),
			(c"host_timescale".into(), c"0.5".into()),
			(c"tf_avoidteammates".into(), c"0".into()),
		])]]
	);

	// Server cheats: nothing is sent.
	assert_eq!(
		cheats.begin(mock.server(), user(2), Purpose::Observe),
		Ok(Begun::ServerCheats)
	);
	assert!(mock.take_sent(1).is_empty());
	assert_eq!(cheats.on_game_frame(mock.server()), Ok(Vec::new()));
}
