//! Tests of the ABI details each game decides.

use super::*;
use sdk_raw::entities::{SDK2013_NEXT_BOT_TELEPORT_SLOT, SDK2013_TELEPORT_SLOT, TF2_TELEPORT_SLOT};

#[test]
fn each_game_teleports_through_its_game_dlls_slot() {
	assert_eq!(
		Game::TeamFortress2.teleport_vtable_slot().index(),
		TF2_TELEPORT_SLOT
	);
	assert_eq!(
		Game::SourceSdk2013.teleport_vtable_slot().index(),
		SDK2013_TELEPORT_SLOT
	);
	assert_eq!(
		Game::SourceSdk2013NextBot.teleport_vtable_slot().index(),
		SDK2013_NEXT_BOT_TELEPORT_SLOT
	);
}
