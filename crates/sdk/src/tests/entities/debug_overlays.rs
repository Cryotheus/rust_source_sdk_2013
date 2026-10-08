//! Tests of entities' debug overlays, read from and written to the member
//! their datamaps declare.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use crate::test_support::sdk_core::change_tracking_engine;

#[test]
fn overlays_are_read_and_written() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();

	set_datamap(state_maps(vec![]));
	mock.state().debug_overlays = OVERLAY_BUDDHA_MODE | OVERLAY_BBOX_BIT;

	assert_eq!(
		mock.entity().debug_overlays(),
		Ok(DebugOverlays::BUDDHA_MODE | DebugOverlays::BOUNDING_BOX)
	);

	// Bits without a constant are kept.
	let overlays = DebugOverlays::BOUNDING_BOX | DebugOverlays::from_bits_retain(1 << 31);

	mock.entity().set_debug_overlays(engine, overlays).unwrap();

	assert_eq!(mock.state().debug_overlays, OVERLAY_BBOX_BIT | 1 << 31);
	assert_eq!(mock.entity().debug_overlays(), Ok(overlays));
}

#[test]
fn buddha_mode_is_the_games_bit() {
	assert_eq!(DebugOverlays::BUDDHA_MODE.bits(), 0x0200_0000);

	// Every bit the header names has a constant, and no two share one.
	assert_eq!(DebugOverlays::all().bits().count_ones(), 29);
	assert_eq!(DebugOverlays::all().iter().count(), 29);
}
