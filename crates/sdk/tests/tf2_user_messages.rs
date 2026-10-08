#![cfg(feature = "tf2")]
//! Tests of TF2's own user messages: their payloads as TF2's client reads them.

use source_sdk_2013::math::{QAngle, Vector};
use source_sdk_2013::test_support::user_messages::payload;
use source_sdk_2013::tf2::user_messages::ForcePlayerViewAngles;

#[test]
fn view_angles_are_coordinates() {
	let angles = QAngle {
		pitch: 10.0,
		yaw: -90.0,
		roll: 0.0,
	};
	let bits = payload(&ForcePlayerViewAngles { player: 5, angles });
	let mut reader = bits.reader();

	assert_eq!(reader.read_u8(), Ok(1));
	assert_eq!(reader.read_u8(), Ok(5));
	assert_eq!(
		reader.read_bit_vec3_coord(),
		Ok(Vector::new(10.0, -90.0, 0.0))
	);
}
