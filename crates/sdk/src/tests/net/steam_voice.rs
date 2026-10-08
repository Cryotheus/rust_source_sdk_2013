//! Tests of `crate::net::steam_voice`: reading and writing packets of voice.

use super::*;

const SPEAKER: u64 = 0x0110_0001_0000_0042;

#[test]
fn the_crc_is_the_standard_crc_32() {
	assert_eq!(crc32(b""), 0);
	assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn packets_are_the_steam_id_then_sections_then_their_crc() {
	let frames = encode_opus_frames(&[
		OpusFrame::Audio {
			sequence: 7,
			data: &[1, 2, 3],
		},
		OpusFrame::Reset { sequence: 8 },
	])
	.unwrap();

	let packet = encode(
		SPEAKER,
		&[
			Section::SampleRate(24000),
			Section::Opus(&frames),
			Section::Silence(480),
		],
	)
	.unwrap();

	let (contents, crc) = packet.split_at(packet.len() - 4);

	assert_eq!(contents[..8], SPEAKER.to_le_bytes());
	assert_eq!(contents[8..11], [11, 0xC0, 0x5D]);
	assert_eq!(contents[11..14], [6, 11, 0]);
	assert_eq!(contents[14..21], [3, 0, 7, 0, 1, 2, 3]);
	assert_eq!(contents[21..25], [0xFF, 0xFF, 8, 0]);
	assert_eq!(contents[25..], [0, 0xE0, 0x01]);
	assert_eq!(crc, crc32(contents).to_le_bytes());

	let voice = SteamVoice::parse(&packet).unwrap();
	let sections = voice.sections().collect::<Result<Vec<_>, _>>().unwrap();

	assert_eq!(voice.steam_id(), SPEAKER);
	assert_eq!(
		sections,
		[
			Section::SampleRate(24000),
			Section::Opus(&frames),
			Section::Silence(480)
		]
	);
	assert_eq!(
		opus_frames(&frames).collect::<Result<Vec<_>, _>>().unwrap(),
		[
			OpusFrame::Audio {
				sequence: 7,
				data: &[1, 2, 3]
			},
			OpusFrame::Reset { sequence: 8 }
		]
	);
}

#[test]
fn another_steam_id_keeps_the_sections_and_a_valid_crc() {
	let packet = encode(SPEAKER, &[Section::SampleRate(24000), Section::Silence(1)]).unwrap();
	let moved = SteamVoice::parse(&packet).unwrap().with_steam_id(0);
	let voice = SteamVoice::parse(&moved).unwrap();

	assert_eq!(voice.steam_id(), 0);
	assert_eq!(moved[8..moved.len() - 4], packet[8..packet.len() - 4]);
	assert_eq!(
		voice.sections().collect::<Result<Vec<_>, _>>().unwrap(),
		[Section::SampleRate(24000), Section::Silence(1)]
	);
}

#[test]
fn damaged_packets_are_refused() {
	let mut packet = encode(SPEAKER, &[Section::Silence(1)]).unwrap();

	assert_eq!(
		SteamVoice::parse(&packet[..11]),
		Err(SteamVoiceError::TooShort { len: 11 })
	);

	packet[9] ^= 1;
	assert!(matches!(
		SteamVoice::parse(&packet),
		Err(SteamVoiceError::Crc { .. })
	));
}

#[test]
fn reading_stops_at_the_first_unreadable_section_or_frame() {
	let packet = encode(SPEAKER, &[Section::Silence(1)]).unwrap();
	let mut body = packet[8..packet.len() - 4].to_vec();

	body.extend_from_slice(&[9, 0, 0]);
	body.extend_from_slice(&[0, 1, 0]);

	let mut unknown = SPEAKER.to_le_bytes().to_vec();

	unknown.extend_from_slice(&body);
	unknown.extend_from_slice(&crc32(&unknown).to_le_bytes());

	assert_eq!(
		SteamVoice::parse(&unknown)
			.unwrap()
			.sections()
			.collect::<Vec<_>>(),
		[
			Ok(Section::Silence(1)),
			Err(SteamVoiceError::UnknownSection { kind: 9 })
		]
	);

	// A frame of 5 bytes, of which 1 is there.
	assert_eq!(
		opus_frames(&[5, 0, 1, 0, 42]).collect::<Vec<_>>(),
		[Err(SteamVoiceError::Truncated)]
	);
}

#[test]
fn what_lengths_cannot_say_is_not_encoded() {
	let long = vec![0; 65536];

	assert_eq!(
		encode(SPEAKER, &[Section::Opus(&long)]),
		Err(SteamVoiceError::TooLong { len: 65536 })
	);
	assert_eq!(
		encode_opus_frames(&[OpusFrame::Audio {
			sequence: 0,
			data: &long[..65535]
		}]),
		Err(SteamVoiceError::TooLong { len: 65535 })
	);
}
