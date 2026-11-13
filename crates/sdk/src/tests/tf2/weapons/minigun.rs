//! Tests of telling a burning ring of fire, and a delayed one, from what a
//! minigun keeps before and after its frame.

use super::*;

const NOW: f32 = 100.0;

#[test]
fn a_due_ring_burns_while_the_barrel_spins_or_fires() {
	for state in [MinigunState::Firing, MinigunState::Spinning] {
		let before = read(state, NOW - 0.01);
		let after = read(state, NOW + RING_OF_FIRE_INTERVAL);

		assert!(after.burned_since(before, NOW));
		assert_eq!(after.confirms_layout(before, NOW), None);
	}
}

#[test]
fn a_ring_not_yet_due_or_never_started_does_not_burn() {
	let after = read(MinigunState::Firing, NOW + RING_OF_FIRE_INTERVAL);

	assert!(!after.burned_since(read(MinigunState::Firing, NOW + 0.1), NOW));
	assert!(!after.burned_since(read(MinigunState::Firing, 0.0), NOW));
	assert!(
		!read(MinigunState::Firing, NOW - 0.2)
			.burned_since(read(MinigunState::Firing, NOW - 0.2), NOW)
	);
}

fn read(state: MinigunState, next: f32) -> RingOfFire {
	RingOfFire {
		state: Some(state),
		next,
	}
}

#[test]
fn spinning_up_delays_the_ring_without_burning_it() {
	// A stale time from the last spin, then the barrel leaves its wind-up.
	let before = read(MinigunState::StartFiring, NOW - 3.0);
	let after = read(MinigunState::Firing, NOW + RING_OF_FIRE_INTERVAL);

	assert!(!after.burned_since(before, NOW));
	assert_eq!(after.confirms_layout(before, NOW), Some(true));
	assert_eq!(
		read(MinigunState::Firing, NOW - 3.0).confirms_layout(before, NOW),
		Some(false)
	);
}
