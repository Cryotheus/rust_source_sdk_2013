//! Tests of `crate::hooks::tf2::vote`: what the dispatch writes into TF2's vote
//! creation outputs.

use super::*;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::tf2::voting::DEDICATED_SERVER;

/// Blocks restarting the game, and allows every other vote.
struct Policy;

impl VoteStartHandler for Policy {
	fn vote_start(&self, _: Server<'_>, request: VoteRequest<'_>) -> VoteDecision {
		assert_eq!(request.caller_entity_index, DEDICATED_SERVER);
		assert!(request.is_server_request());
		assert_eq!(request.details, c"test");
		if request.issue == VoteIssue::RestartGame {
			VoteDecision::Block
		} else {
			VoteDecision::Allow
		}
	}
}

#[test]
fn veto_supersedes_with_failure_and_allow_preserves_game_outputs() {
	let scope = ();
	// SAFETY: The callback only inspects owned request data. No engine
	// interfaces or entity operations are reachable through this fixture.
	let server = unsafe { tf2_binding(no_interfaces).server(&scope) };
	for (issue, expected, expected_outputs) in [
		(
			VoteIssue::RestartGame,
			HookAction::Supersede(false),
			(0, -1),
		),
		(VoteIssue::NextLevel, HookAction::Ignore, (25, 42)),
	] {
		let (mut failure, mut time) = (25, 42);
		let request = VoteRequest {
			issue,
			caller_entity_index: DEDICATED_SERVER,
			details: c"test",
		};
		// SAFETY: These two stack outputs remain allocated for the call.
		assert_eq!(
			unsafe { dispatch(server, request, &Policy, &mut failure, &mut time) },
			expected
		);
		assert_eq!((failure, time), expected_outputs);
	}
}
