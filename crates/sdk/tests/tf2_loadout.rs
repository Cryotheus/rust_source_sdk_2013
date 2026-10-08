#![cfg(feature = "tf2_loadout")]

//! Tests of the loadout reapplier against the mock engine's clients.

use source_sdk_2013::players::UserId;
use source_sdk_2013::test_support::net::cheats::{MockClient, MockEngine};
use source_sdk_2013::test_support::players::user;
use source_sdk_2013::test_support::server::mock_server;
use source_sdk_2013::tf2::loadout::{Loadout, LoadoutError, LoadoutReapplier};
use source_sdk_2013::tf2::weapons::ItemDefinitionIndex;

#[test]
fn pruning_keeps_the_loadouts_of_connected_clients() {
	let scope = ();
	let mut reapplier = LoadoutReapplier::new();
	let loadout = Loadout::new()
		.with_wearable(ItemDefinitionIndex::new(106).unwrap())
		.unwrap();
	let kept = |reapplier: &LoadoutReapplier| -> Vec<UserId> {
		reapplier.iter().map(|(user_id, _)| user_id).collect()
	};

	for id in [2, 3, 4, 5, 6] {
		reapplier.set(user(id), loadout.clone());
	}

	// Without the engine's interface, nothing is forgotten.
	assert!(matches!(
		reapplier.retain_connected(mock_server(&scope)),
		Err(LoadoutError::Interface(_))
	));
	assert_eq!(kept(&reapplier), [2, 3, 4, 5, 6].map(user));

	// A player, a client still loading the level, a bot and an empty
	// slot. User IDs 3 and 6 left without `player_disconnect`, as bots
	// are expected to at a level change.
	let loading = MockClient {
		active: false,
		..MockClient::active(4)
	};
	let bot = MockClient {
		fake: true,
		..MockClient::active(5)
	};
	let mock = MockEngine::new(
		&[MockClient::active(2), loading, MockClient::default(), bot],
		c"0",
		&[],
	);

	reapplier.retain_connected(mock.server()).unwrap();
	assert_eq!(kept(&reapplier), [2, 4, 5].map(user));
}
