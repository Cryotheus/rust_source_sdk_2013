//! Tests of passing a client's handler a command as though the client sent
//! it, as far as they need none of the engine's own objects.

use super::*;
use crate::test_support::net::cheats::{MockClient, MockEngine};

/// A remote player in slot 0, a bot in slot 1, SourceTV in slot 2, a remote
/// player without a channel in slot 3, and an empty slot 4.
const CLIENTS: [MockClient; 5] = [
	MockClient::active(2),
	MockClient {
		fake: true,
		..MockClient::active(3)
	},
	MockClient {
		hltv: true,
		..MockClient::active(4)
	},
	MockClient {
		no_channel: true,
		..MockClient::active(5)
	},
	MockClient {
		user_id: 0,
		active: false,
		fake: false,
		hltv: false,
		loopback: false,
		no_channel: false,
	},
];

/// Passes `command` to the client in `slot` of `engine`.
fn process(engine: &MockEngine, slot: usize, command: &CStr) -> Result<bool, StringCommandError> {
	// SAFETY: Each test's command is refused before any handler runs it.
	unsafe {
		engine
			.game_client(slot)
			.process_string_command(engine.server(), command)
	}
}

#[test]
fn commands_that_do_not_fit_are_refused_first() {
	let engine = MockEngine::new(&CLIENTS, c"0", &[]);
	let longest = CString::new(vec![b'a'; raw::MAX_STRING_CMD_LEN]).unwrap();
	let too_long = CString::new(vec![b'a'; raw::MAX_STRING_CMD_LEN + 1]).unwrap();

	for slot in 0..CLIENTS.len() {
		assert!(
			matches!(
				process(&engine, slot, &too_long),
				Err(StringCommandError::TooLong)
			),
			"slot {slot}"
		);
	}

	// One that fits is refused for the bot as any other command is.
	assert!(matches!(
		process(&engine, 1, &longest),
		Err(StringCommandError::FakeClient)
	));
}

#[test]
fn only_clients_with_a_channel_are_passed_commands() {
	let engine = MockEngine::new(&CLIENTS, c"0", &[]);

	for slot in [1, 2] {
		assert!(
			matches!(
				process(&engine, slot, c"say hi"),
				Err(StringCommandError::FakeClient)
			),
			"slot {slot}"
		);
	}

	for slot in [3, 4] {
		assert!(
			matches!(
				process(&engine, slot, c"say hi"),
				Err(StringCommandError::NoChannel)
			),
			"slot {slot}"
		);
	}
}

#[test]
fn refusals_map_to_their_reasons() {
	assert!(matches!(
		StringCommandError::from(StringCmdError::TooLong),
		StringCommandError::TooLong
	));
	assert!(matches!(
		StringCommandError::from(StringCmdError::UnexpectedLayout),
		StringCommandError::UnexpectedLayout
	));
	assert!(matches!(
		StringCommandError::from(ClientLayoutError),
		StringCommandError::UnexpectedLayout
	));
}
