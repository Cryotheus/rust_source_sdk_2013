//! Tests of `ClientCheats`' leases: the blocks clients are sent, how their
//! answers are read, timeouts and retries, restores, and clients coming and
//! going, mostly over a fake engine link with injected time.

use super::*;
use crate::net::incoming::mock_incoming_message;

use crate::test_support::net::cheats::{
	Decoded, MockClient, MockEngine, decode, query_cookie, response,
};

use crate::test_support::net::incoming::{respond_cvar_value, unreadable};
use crate::test_support::net_leases::{cheats, confirm, spoof};
use crate::test_support::players::user;

/// The engine as a test describes it, recording what is sent.
struct FakeLink {
	/// Connected clients, and whether each is active.
	clients: Vec<(UserId, bool)>,

	/// Sends of more bits than this fail.
	room: usize,

	/// What each send was given, decoded.
	sent: Vec<(UserId, Vec<Decoded>)>,

	server_cheats: bool,

	values: Vec<(CString, CString)>,
}

impl FakeLink {
	/// Active clients with these user IDs, and a server whose `sv_cheats` is
	/// 0, with `host_timescale` at 0.5.
	fn new(user_ids: &[UserId]) -> Self {
		Self {
			clients: user_ids.iter().map(|&user_id| (user_id, true)).collect(),
			room: usize::MAX,
			sent: Vec::new(),
			server_cheats: false,
			values: vec![
				(c"sv_cheats".into(), c"0".into()),
				(c"host_timescale".into(), c"0.5".into()),
			],
		}
	}

	fn take_sent(&mut self) -> Vec<(UserId, Vec<Decoded>)> {
		take(&mut self.sent)
	}
}

impl Link for FakeLink {
	type Peer = UserId;

	fn peer(&mut self, user_id: UserId) -> Option<(UserId, bool)> {
		self.clients.iter().copied().find(|&(id, _)| id == user_id)
	}

	fn restore_values(&mut self) -> &[(CString, CString)] {
		&self.values
	}

	fn send(&mut self, peer: UserId, bits: &BitWriter) -> Result<(), SendError> {
		if bits.len() > self.room {
			return Err(SendError::TooLarge { bits: bits.len() });
		}

		self.sent.push((peer, decode(bits)));
		Ok(())
	}

	fn server_cheats(&mut self) -> bool {
		self.server_cheats
	}
}

#[test]
fn a_client_that_reports_cheats_off_refused() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	assert_eq!(
		cheats.on_response(user(2), &response(cookie, 0, c"0")),
		Verdict::Block
	);
	assert!(!cheats.client(user(2)).unwrap().refused);

	// Refusals are not retried, but the client is still restored.
	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Refused,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert_eq!(
		cheats.client(user(2)),
		Some(ClientStatus {
			confirmed: false,
			pending: 0,
			refused: true,
			restore_pending: false,
		})
	);
}

#[test]
fn a_confirmed_lease_is_restored_on_the_next_frame_with_cheat_variables() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let commands = Purpose::Commands(vec![c"r_cleardecals".into(), c"cl_soundscape_flush".into()]);
	let Begun::Lease(lease) = cheats
		.begin_with(&mut link, user(2), commands, now)
		.unwrap()
	else {
		panic!("a lease");
	};
	let sent = link.take_sent();
	let cookie = query_cookie(&sent[0].1);

	// One block: sv_cheats first, the commands, then the query.
	assert_eq!(sent.len(), 1);
	assert_eq!(
		sent[0].1,
		spoof(cookie, &[c"r_cleardecals", c"cl_soundscape_flush"])
	);
	assert!(COOKIES.contains(&cookie));
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));
	assert_eq!(
		cheats.client(user(2)),
		Some(ClientStatus {
			confirmed: false,
			pending: 1,
			refused: false,
			restore_pending: true,
		})
	);

	// Nothing is sent while waiting for the answer.
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 1)), []);
	assert!(link.take_sent().is_empty());

	// The answer confirms the lease once the restore follows it.
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));
	assert!(!cheats.client(user(2)).unwrap().confirmed);
	assert!(link.take_sent().is_empty());

	// A repeated answer, or one saying otherwise, is blocked and changes
	// nothing.
	assert_eq!(
		cheats.on_response(user(2), &response(cookie, 0, c"0")),
		Verdict::Block
	);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));

	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 1)),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert_eq!(cheats.lease(lease), None);
	assert_eq!(
		cheats.client(user(2)),
		Some(ClientStatus {
			confirmed: true,
			pending: 0,
			refused: false,
			restore_pending: false,
		})
	);

	// A repeated answer is still blocked, but changes nothing.
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 2)), []);
	assert!(link.take_sent().is_empty());
}

#[test]
fn a_late_answer_before_the_restore_still_confirms() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	// Past the timeout, but no frame has restored the client yet.
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 10)),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
}

#[test]
fn a_restore_that_cannot_be_sent_at_all_keeps_the_window_open() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	link.room = 0;

	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::RestoreFailed {
			user_id: user(2),
			error: SendError::TooLarge {
				bits: restore_bits(&link.values).len(),
			},
		}]
	);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));

	link.room = usize::MAX;

	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
}

#[test]
fn a_restore_too_large_sends_sv_cheats_first_then_the_rest() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);

	// Room for `sv_cheats` alone.
	link.room = restore_bits(&link.values[..1]).len();

	let events = cheats.on_game_frame_with(&mut link, now);
	let full = restore_bits(&link.values).len();

	assert_eq!(
		events,
		[
			CheatsEvent::RestoreFailed {
				user_id: user(2),
				error: SendError::TooLarge { bits: full },
			},
			CheatsEvent::Finished {
				user_id: user(2),
				lease,
				outcome: LeaseOutcome::Confirmed,
			},
		]
	);
	assert_eq!(
		link.take_sent(),
		[(
			user(2),
			vec![Decoded::SetConVar(vec![(c"sv_cheats".into(), c"0".into())])]
		)]
	);
	assert!(cheats.client(user(2)).unwrap().restore_pending);

	// New leases wait for the rest of the restore.
	let Ok(Begun::Lease(next)) = cheats.begin_with(&mut link, user(2), Purpose::Observe, now)
	else {
		panic!("a lease");
	};

	assert_eq!(cheats.lease(next), Some(LeaseStatus::Queued));
	assert!(link.take_sent().is_empty());

	// Still no room: nothing is sent, and it is tried again.
	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::RestoreFailed {
			user_id: user(2),
			error: SendError::TooLarge { bits: full },
		}]
	);
	assert!(link.take_sent().is_empty());

	link.room = usize::MAX;

	assert_eq!(cheats.on_game_frame_with(&mut link, now), []);

	let sent = link.take_sent();

	assert_eq!(sent[0], (user(2), restore()));
	assert_eq!(sent[1], (user(2), spoof(query_cookie(&sent[1].1), &[])));
}

#[test]
fn a_spoof_that_does_not_fit_is_refused_without_trace() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);

	link.room = 8;

	assert!(matches!(
		cheats.begin_with(&mut link, user(2), Purpose::Observe, now),
		Err(CheatsError::Send(SendError::TooLarge { .. }))
	));
	assert_eq!(cheats.client(user(2)), None);
	assert!(link.take_sent().is_empty());
}

#[test]
fn a_timeout_requeues_the_other_leases_without_counting_them() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (first, _) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);
	let (second, cookie) = start(
		&mut cheats,
		&mut link,
		user(2),
		Purpose::Observe,
		at(now, 2),
	);

	// The first lease timed out, which restores the client and cuts the
	// second short. The second goes out again at once, after the restore.
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 3)),
		[CheatsEvent::Retrying {
			user_id: user(2),
			lease: first,
			attempt: 2,
		}]
	);

	let sent = link.take_sent();
	let again = query_cookie(&sent[1].1);

	assert_eq!(sent, [(user(2), restore()), (user(2), spoof(again, &[]))]);
	assert_ne!(again, cookie);
	assert_eq!(cheats.lease(first), Some(LeaseStatus::Queued));
	assert_eq!(cheats.lease(second), Some(LeaseStatus::Sent));

	// Its earlier answer came after the restore, so it proves nothing.
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	assert_eq!(cheats.lease(second), Some(LeaseStatus::Sent));

	// Its new answer confirms it, without waiting for its timeout.
	assert_eq!(confirm(&mut cheats, user(2), again), Verdict::Block);
	assert_eq!(cheats.lease(second), Some(LeaseStatus::Answered));
}

#[test]
fn an_unreadable_variable_still_confirms_by_order() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	// 3: the variable does not allow queries, so no value comes back.
	assert_eq!(
		cheats.on_response(user(2), &response(cookie, 3, c"")),
		Verdict::Block
	);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
}

#[test]
fn answers_must_match_the_client_cookie_and_name() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2), user(3)]);
	let (lease, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	assert_eq!(confirm(&mut cheats, user(3), cookie), Verdict::Continue);
	assert_eq!(confirm(&mut cheats, user(2), cookie - 1), Verdict::Continue);
	assert_eq!(confirm(&mut cheats, user(2), 7), Verdict::Continue);

	let other_name = Incoming::RespondCvarValue {
		cookie,
		status: QUERY_CVAR_VALUE_INTACT,
		name: c"sv_pure".into(),
		value: c"1".into(),
	};

	assert_eq!(cheats.on_response(user(2), &other_name), Verdict::Continue);
	assert_eq!(
		cheats.on_response(user(2), &Incoming::CmdKeyValues),
		Verdict::Continue
	);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));

	// Names are compared as the engine compares variable names.
	let upper = Incoming::RespondCvarValue {
		cookie,
		status: QUERY_CVAR_VALUE_INTACT,
		name: c"SV_CHEATS".into(),
		value: c"1".into(),
	};

	assert_eq!(cheats.on_response(user(2), &upper), Verdict::Block);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));
}

#[test]
fn answers_reach_leases_through_the_incoming_hook() {
	let mock = MockEngine::new(&[MockClient::active(2)], c"0", &[]);
	let mut cheats = cheats();
	let client = mock.game_client(0);
	// SAFETY: The mock messages are leaked, and answer the only virtual call
	// `on_incoming` makes of them.
	let incoming = |kind, raw| unsafe { mock_incoming_message(kind, raw, client) };
	let foreign = respond_cvar_value(7, QUERY_CVAR_VALUE_INTACT, c"sv_cheats", c"1");

	// No answer is expected yet.
	assert_eq!(
		cheats.on_incoming(incoming(IncomingKind::RespondCvarValue, foreign)),
		Verdict::Continue
	);

	let Ok(Begun::Lease(lease)) = cheats.begin(mock.server(), user(2), Purpose::Observe) else {
		panic!("a lease");
	};
	let cookie = query_cookie(&mock.take_sent(0)[0]);
	let answer = respond_cvar_value(cookie, QUERY_CVAR_VALUE_INTACT, c"sv_cheats", c"1");

	// Other kinds of messages, and answers to other queries, pass.
	assert_eq!(
		cheats.on_incoming(incoming(IncomingKind::StringCmd, answer)),
		Verdict::Continue
	);
	assert_eq!(
		cheats.on_incoming(incoming(IncomingKind::RespondCvarValue, foreign)),
		Verdict::Continue
	);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));

	assert_eq!(
		cheats.on_incoming(incoming(IncomingKind::RespondCvarValue, answer)),
		Verdict::Block
	);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));
	assert_eq!(
		cheats.on_game_frame(mock.server()),
		Ok(vec![CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}])
	);

	// A repeated answer is still blocked.
	assert_eq!(
		cheats.on_incoming(incoming(IncomingKind::RespondCvarValue, answer)),
		Verdict::Block
	);
}

/// Seconds after `start`.
fn at(start: Instant, seconds: u64) -> Instant {
	start + Duration::from_secs(seconds)
}

#[test]
fn clients_that_leave_cancel_their_leases() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2), user(3)]);
	let (lease, _) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);
	let (kept, _) = start(&mut cheats, &mut link, user(3), Purpose::Observe, now);

	link.clients.retain(|&(id, _)| id != user(2));

	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Cancelled,
		}]
	);
	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.client(user(2)), None);
	assert_eq!(cheats.lease(kept), Some(LeaseStatus::Sent));
}

#[test]
fn leases_of_one_client_share_its_restore() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (first, first_cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);
	let commands = Purpose::Commands(vec![c"cl_soundscape_flush".into()]);
	let (second, second_cookie) = start(&mut cheats, &mut link, user(2), commands, at(now, 1));

	assert_ne!(first_cookie, second_cookie);
	assert_eq!(confirm(&mut cheats, user(2), first_cookie), Verdict::Block);

	// The second lease's query is still unanswered.
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 1)), []);
	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.lease(first), Some(LeaseStatus::Answered));

	assert_eq!(confirm(&mut cheats, user(2), second_cookie), Verdict::Block);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 2)),
		[
			CheatsEvent::Finished {
				user_id: user(2),
				lease: first,
				outcome: LeaseOutcome::Confirmed,
			},
			CheatsEvent::Finished {
				user_id: user(2),
				lease: second,
				outcome: LeaseOutcome::Confirmed,
			},
		]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
}

#[test]
fn leases_wait_for_the_client_to_be_active() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[]);

	link.clients.push((user(2), false));

	let Ok(Begun::Lease(lease)) = cheats.begin_with(&mut link, user(2), Purpose::Observe, now)
	else {
		panic!("a lease");
	};

	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Queued));
	assert!(!cheats.client(user(2)).unwrap().restore_pending);

	// Waiting does not time out.
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 60)), []);
	assert!(link.take_sent().is_empty());

	link.clients[0].1 = true;

	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 61)), []);

	let sent = link.take_sent();
	let cookie = query_cookie(&sent[0].1);

	assert_eq!(sent, [(user(2), spoof(cookie, &[]))]);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));
}

#[test]
fn queued_sends_that_fail_are_retried_then_fail() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[]);

	link.clients.push((user(2), false));

	let Ok(Begun::Lease(lease)) = cheats.begin_with(&mut link, user(2), Purpose::Observe, now)
	else {
		panic!("a lease");
	};

	link.clients[0].1 = true;
	link.room = 0;

	let refused = |bits| SendError::TooLarge { bits };
	let size = spoof_bits([(0, &Purpose::Observe)]).unwrap().len();

	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Retrying {
			user_id: user(2),
			lease,
			attempt: 2,
		}]
	);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 1)),
		[CheatsEvent::Retrying {
			user_id: user(2),
			lease,
			attempt: 3,
		}]
	);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 3)),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::SendFailed(refused(size)),
		}]
	);
	assert!(link.take_sent().is_empty());
}

/// The restore of a [`FakeLink::new`] server.
fn restore() -> Vec<Decoded> {
	vec![Decoded::SetConVar(vec![
		(c"sv_cheats".into(), c"0".into()),
		(c"host_timescale".into(), c"0.5".into()),
	])]
}

#[test]
fn restore_all_cancels_what_was_not_answered() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2), user(3), user(4)]);
	let (answered, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);
	let (unanswered, _) = start(&mut cheats, &mut link, user(3), Purpose::Observe, now);

	link.clients[2].1 = false;

	let Ok(Begun::Lease(queued)) = cheats.begin_with(&mut link, user(4), Purpose::Observe, now)
	else {
		panic!("a lease");
	};

	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);

	let events = cheats.restore_all_with(&mut link, now);

	assert_eq!(
		events,
		[
			CheatsEvent::Finished {
				user_id: user(2),
				lease: answered,
				outcome: LeaseOutcome::Confirmed,
			},
			CheatsEvent::Finished {
				user_id: user(3),
				lease: unanswered,
				outcome: LeaseOutcome::Cancelled,
			},
			CheatsEvent::Finished {
				user_id: user(4),
				lease: queued,
				outcome: LeaseOutcome::Cancelled,
			},
		]
	);

	// Only the clients shown sv_cheats set are restored.
	assert_eq!(
		link.take_sent(),
		[(user(2), restore()), (user(3), restore())]
	);
	assert!(cheats.client(user(2)).unwrap().confirmed);
	assert_eq!(cheats.client(user(3)).unwrap().pending, 0);

	// Clearing forgets what was confirmed, and the client never shown
	// sv_cheats set. The others are restored again on the next level.
	cheats.clear();

	for id in [2, 3] {
		assert_eq!(
			cheats.client(user(id)),
			Some(ClientStatus {
				confirmed: false,
				pending: 0,
				refused: false,
				restore_pending: true,
			})
		);
	}

	assert_eq!(cheats.client(user(4)), None);
	assert!(cheats.forget(user(2)));
	assert!(!cheats.forget(user(4)));
}

#[test]
fn restore_all_completes_a_partial_restore() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (_, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);

	// Room for `sv_cheats` alone.
	link.room = restore_bits(&link.values[..1]).len();
	cheats.on_game_frame_with(&mut link, now);
	assert_eq!(link.take_sent().len(), 1);
	assert!(cheats.client(user(2)).unwrap().restore_pending);

	link.room = usize::MAX;

	assert_eq!(cheats.restore_all_with(&mut link, now), []);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert!(!cheats.client(user(2)).unwrap().restore_pending);
}

#[test]
fn restores_missed_at_level_shutdown_are_sent_once_active_on_the_next_level() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2), user(3)]);
	let (missed, cookie) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);
	let (restored, other) = start(&mut cheats, &mut link, user(3), Purpose::Observe, now);

	assert_eq!(confirm(&mut cheats, user(3), other), Verdict::Block);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(3),
			lease: restored,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
	assert_eq!(link.take_sent(), [(user(3), restore())]);
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);

	// At level shutdown, the first client's restore does not fit at all.
	link.room = 0;

	assert_eq!(
		cheats.restore_all_with(&mut link, now),
		[
			CheatsEvent::RestoreFailed {
				user_id: user(2),
				error: SendError::TooLarge {
					bits: restore_bits(&link.values).len(),
				},
			},
			CheatsEvent::Finished {
				user_id: user(2),
				lease: missed,
				outcome: LeaseOutcome::Confirmed,
			},
		]
	);

	cheats.clear();
	link.room = usize::MAX;

	// Changing levels, the clients are not active, and are sent nothing.
	for client in &mut link.clients {
		client.1 = false;
	}

	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 10)), []);
	assert!(link.take_sent().is_empty());
	assert!(cheats.client(user(2)).unwrap().restore_pending);

	// Once active, both are told the server's values again, including the
	// one restored during the level, and new leases wait for that.
	for client in &mut link.clients {
		client.1 = true;
	}

	let Ok(Begun::Lease(next)) =
		cheats.begin_with(&mut link, user(2), Purpose::Observe, at(now, 10))
	else {
		panic!("a lease");
	};

	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.lease(next), Some(LeaseStatus::Queued));
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 10)), []);

	let sent = link.take_sent();

	assert_eq!(
		sent,
		[
			(user(2), restore()),
			(user(2), spoof(query_cookie(&sent[1].1), &[])),
			(user(3), restore()),
		]
	);
	assert!(!cheats.client(user(3)).unwrap().restore_pending);
}

#[test]
fn restores_split_into_messages_of_255_and_skip_what_does_not_fit() {
	let names: Vec<CString> = (0..300)
		.map(|index| CString::new(format!("tf_var_{index}")).unwrap())
		.collect();
	let mut values = vec![(c"sv_cheats".to_owned(), c"0".to_owned())];
	let long = CString::new(vec![b'9'; MAX_CONVAR_LEN + 1]).unwrap();

	values.push((c"tf_long".into(), long));
	values.extend(names.iter().map(|name| (name.clone(), c"1".to_owned())));

	let messages = decode(&restore_bits(&values));

	assert_eq!(messages.len(), 2);

	let (Decoded::SetConVar(first), Decoded::SetConVar(second)) = (&messages[0], &messages[1])
	else {
		panic!("two net_SetConVar");
	};

	assert_eq!(first.len(), 255);
	assert_eq!(second.len(), 46);
	assert_eq!(first[0], (c"sv_cheats".into(), c"0".into()));
	assert_eq!(first[1].0.as_c_str(), c"tf_var_0");
	assert_eq!(second.last().unwrap().0.as_c_str(), c"tf_var_299");
}

#[test]
fn server_cheats_need_no_lease() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);

	link.server_cheats = true;

	assert_eq!(
		cheats.begin_with(&mut link, user(2), Purpose::Observe, now),
		Ok(Begun::ServerCheats)
	);
	assert!(link.take_sent().is_empty());

	// Commands are sent on their own.
	let commands = Purpose::Commands(vec![c"cl_soundscape_flush".into()]);

	assert_eq!(
		cheats.begin_with(&mut link, user(2), commands, now),
		Ok(Begun::ServerCheats)
	);
	assert_eq!(
		link.take_sent(),
		[(
			user(2),
			vec![Decoded::StringCmd(c"cl_soundscape_flush".into())]
		)]
	);
	assert_eq!(cheats.client(user(2)), None);
}

/// Begins a lease for `user_id`, returning its ID and the cookie it sent.
fn start(
	cheats: &mut ClientCheats,
	link: &mut FakeLink,
	user_id: UserId,
	purpose: Purpose,
	now: Instant,
) -> (LeaseId, c_int) {
	let Begun::Lease(lease) = cheats.begin_with(link, user_id, purpose, now).unwrap() else {
		panic!("a lease");
	};
	let sent = link.take_sent();

	assert_eq!(sent.len(), 1);
	assert_eq!(sent[0].0, user_id);

	(lease, query_cookie(&sent[0].1))
}

#[test]
fn the_server_turning_cheats_off_voids_answers_it_may_have_overtaken() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, first) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	// The server's own sv_cheats turning on leaves the client seeing it set,
	// so nothing changes.
	link.server_cheats = true;
	assert_eq!(cheats.on_game_frame_with(&mut link, now), []);
	assert!(link.take_sent().is_empty());
	assert_eq!(confirm(&mut cheats, user(2), first), Verdict::Block);

	// Turning off, the engine sent the client sv_cheats 0, which may have
	// followed the query in the same frame of the client: the answer proves
	// nothing, and the lease is sent again without counting as a failure.
	link.server_cheats = false;
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 1)), []);

	let sent = link.take_sent();
	let second = query_cookie(&sent[1].1);

	assert_eq!(sent, [(user(2), restore()), (user(2), spoof(second, &[]))]);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));
	assert!(!cheats.client(user(2)).unwrap().confirmed);

	// The earlier answer, repeated, is blocked and ignored.
	assert_eq!(confirm(&mut cheats, user(2), first), Verdict::Block);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Sent));

	assert_eq!(confirm(&mut cheats, user(2), second), Verdict::Block);
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 1)),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Confirmed,
		}]
	);
}

#[test]
fn timeouts_restore_then_retry_with_backoff_until_they_fail() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[user(2)]);
	let (lease, first) = start(&mut cheats, &mut link, user(2), Purpose::Observe, now);

	// Not yet timed out.
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 2)), []);

	// Timed out: restored at once, and retried once the delay passes.
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 3)),
		[CheatsEvent::Retrying {
			user_id: user(2),
			lease,
			attempt: 2,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Queued));
	assert!(!cheats.client(user(2)).unwrap().restore_pending);

	// The answer to the first query arrived after the restore was sent, so
	// the client may have processed both in one frame: blocked, but ignored.
	assert_eq!(confirm(&mut cheats, user(2), first), Verdict::Block);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Queued));

	// The first retry waits a second.
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 3)), []);
	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 4)), []);

	let sent = link.take_sent();
	let second = query_cookie(&sent[0].1);

	assert_ne!(second, first);
	assert_eq!(sent, [(user(2), spoof(second, &[]))]);

	// The second retry waits two seconds.
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 7)),
		[CheatsEvent::Retrying {
			user_id: user(2),
			lease,
			attempt: 3,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 8)), []);
	assert!(link.take_sent().is_empty());
	assert_eq!(cheats.on_game_frame_with(&mut link, at(now, 9)), []);

	let third = query_cookie(&link.take_sent()[0].1);

	assert!(![first, second].contains(&third));
	assert_eq!(
		cheats.on_game_frame_with(&mut link, at(now, 12)),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::TimedOut,
		}]
	);
	assert_eq!(link.take_sent(), [(user(2), restore())]);
	assert_eq!(cheats.lease(lease), None);
	assert!(!cheats.client(user(2)).unwrap().confirmed);

	// Late answers stay blocked.
	for cookie in [first, second, third] {
		assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	}
}

#[test]
fn unreadable_answers_end_waiting_leases_unverified_at_once() {
	let mock = MockEngine::new(&[MockClient::active(2), MockClient::active(3)], c"0", &[]);
	let mut cheats = cheats();
	let client = mock.game_client(0);
	let Ok(Begun::Lease(lease)) = cheats.begin(mock.server(), user(2), Purpose::Observe) else {
		panic!("a lease");
	};
	let Ok(Begun::Lease(other)) = cheats.begin(mock.server(), user(3), Purpose::Observe) else {
		panic!("a lease");
	};
	let cookie = query_cookie(&mock.take_sent(0)[0]);

	assert_eq!(mock.take_sent(1).len(), 1);

	// SAFETY: The mock message is leaked, and answers the only virtual call
	// `on_incoming` makes of it.
	let garbled =
		unsafe { mock_incoming_message(IncomingKind::RespondCvarValue, unreadable(), client) };

	// It may answer another query, so it passes, but the client's waiting
	// lease can no longer be confirmed.
	assert_eq!(cheats.on_incoming(garbled), Verdict::Continue);
	assert_eq!(cheats.lease(lease), Some(LeaseStatus::Answered));
	assert_eq!(cheats.lease(other), Some(LeaseStatus::Sent));

	// Restored on the next frame, before the timeout, and not sent again.
	assert_eq!(
		cheats.on_game_frame(mock.server()),
		Ok(vec![CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::Unverified,
		}])
	);
	assert_eq!(
		mock.take_sent(0),
		[vec![Decoded::SetConVar(vec![(
			c"sv_cheats".into(),
			c"0".into()
		)])]]
	);
	assert!(mock.take_sent(1).is_empty());
	assert_eq!(
		cheats.client(user(2)),
		Some(ClientStatus {
			confirmed: false,
			pending: 0,
			refused: false,
			restore_pending: false,
		})
	);

	// A readable answer to its query, late, is blocked and changes nothing.
	assert_eq!(confirm(&mut cheats, user(2), cookie), Verdict::Block);
	assert!(!cheats.client(user(2)).unwrap().confirmed);
}

#[test]
fn values_count_as_set_as_the_engine_reads_them() {
	for (value, set) in [
		(c"1", true),
		(c"2", true),
		(c"-1", true),
		(c"1.5", true),
		(c" 1", true),
		(c"0", false),
		(c"0.9", false),
		(c"", false),
		(c"on", false),
	] {
		assert_eq!(is_set(value), set, "{value:?}");
	}
}

#[test]
fn waiting_leases_end_when_the_server_turns_cheats_on() {
	let now = Instant::now();
	let mut cheats = cheats();
	let mut link = FakeLink::new(&[]);

	link.clients.push((user(2), false));

	let Ok(Begun::Lease(lease)) = cheats.begin_with(&mut link, user(2), Purpose::Observe, now)
	else {
		panic!("a lease");
	};

	link.clients[0].1 = true;
	link.server_cheats = true;

	assert_eq!(
		cheats.on_game_frame_with(&mut link, now),
		[CheatsEvent::Finished {
			user_id: user(2),
			lease,
			outcome: LeaseOutcome::ServerCheats,
		}]
	);
	assert!(link.take_sent().is_empty());

	// The server's own sv_cheats may be off again before the client's frame,
	// so it confirms nothing, as when a lease begins under it.
	assert!(!cheats.client(user(2)).unwrap().confirmed);
}
