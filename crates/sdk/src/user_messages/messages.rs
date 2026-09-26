//! Typed user messages, written as the game's own senders write them.
//!
//! The payloads follow the SDK's senders: `game/server/util.cpp` for most,
//! `game/shared/hintmessage.cpp` for hints, and TF2's game rules and player
//! for its own.

use super::UserMessage;
use crate::bitbuf::BitWriter;
use crate::math::{Color32, QAngle};
use crate::net::EncodeError;
use std::ffi::CStr;

/// Fixed-point fraction bits of fade times (`SCREENFADE_FRACBITS`).
const FADE_FRACTION_BITS: u32 = 9;

/// Seconds as the unsigned 7.9 fixed point fades use, clamped to its range,
/// as `FixedUnsigned16` does.
fn fade_time(seconds: f32) -> u16 {
	(seconds * (1 << FADE_FRACTION_BITS) as f32).clamp(0.0, u16::MAX.into()) as u16
}

/// Fades the screen to or from a color (`Fade`), as `env_fade` does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
	/// Seconds the fade takes, up to about 128.
	pub duration: f32,

	/// Seconds the color is held once reached, up to about 128.
	pub hold: f32,

	pub flags: FadeFlags,
	pub color: Color32,
}

/// How a [`Fade`] behaves (`FFADE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FadeFlags(pub u16);

impl FadeFlags {
	/// From the color to clear.
	pub const IN: Self = Self(0x1);

	/// From clear to the color.
	pub const OUT: Self = Self(0x2);

	/// Multiplies the screen by the color instead of blending it.
	pub const MODULATE: Self = Self(0x4);

	/// Holds the color until another fade replaces it.
	pub const STAY_OUT: Self = Self(0x8);

	/// Replaces every other fade.
	pub const PURGE: Self = Self(0x10);

	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

impl UserMessage for Fade {
	fn name(&self) -> &CStr {
		c"Fade"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		// The game writes these as signed shorts, dropping the top bit of
		// times over 64 seconds; clients read the same 16 bits either way.
		out.write_u16(fade_time(self.duration));
		out.write_u16(fade_time(self.hold));
		out.write_u16(self.flags.0);
		out.write_u8(self.color.r);
		out.write_u8(self.color.g);
		out.write_u8(self.color.b);
		out.write_u8(self.color.a);
		Ok(())
	}
}

/// What a [`Shake`] does (`ShakeCommand_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShakeCommand {
	Start = 0,
	Stop = 1,
	/// Changes the amplitude of a shake in progress.
	Amplitude = 2,
	/// Changes the frequency of a shake in progress.
	Frequency = 3,
	/// Only rumbles controllers.
	StartRumbleOnly = 4,
	/// Shakes without rumbling controllers.
	StartNoRumble = 5,
}

/// Shakes the screen (`Shake`), as `env_shake` does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shake {
	pub command: ShakeCommand,

	/// Up to 16.
	pub amplitude: f32,

	/// Up to 255.
	pub frequency: f32,

	/// Seconds.
	pub duration: f32,
}

impl UserMessage for Shake {
	fn name(&self) -> &CStr {
		c"Shake"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_u8(self.command as u8);
		out.write_f32(self.amplitude);
		out.write_f32(self.frequency);
		out.write_f32(self.duration);
		Ok(())
	}
}

/// How a [`HudText`] appears (the `effect` of `hudtextparms_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum HudTextEffect {
	/// Fades in and out.
	#[default]
	Fade = 0,
	/// Fades in and out, flickering between the two colors.
	Flicker = 1,
	/// Types out each character in the second color.
	ScanOut = 2,
}

/// Text on the HUD (`HudMsg`), as `game_text` shows it.
#[doc(alias = "HudMsg")]
#[doc(alias = "game_text")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudText<'a> {
	/// Replaces the text shown in the same channel, from 0 to 5.
	pub channel: u8,

	/// The position as a fraction of the screen, or -1 to center.
	pub x: f32,
	pub y: f32,

	pub color: Color32,
	/// The color of the scan-out and flicker effects.
	pub effect_color: Color32,
	pub effect: HudTextEffect,

	/// Seconds.
	pub fade_in: f32,
	pub fade_out: f32,
	pub hold: f32,
	/// Seconds per character for the scan-out effect.
	pub effect_time: f32,

	pub text: &'a CStr,
}

impl UserMessage for HudText<'_> {
	fn name(&self) -> &CStr {
		c"HudMsg"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_u8(self.channel);
		out.write_f32(self.x);
		out.write_f32(self.y);

		for color in [self.color, self.effect_color] {
			out.write_u8(color.r);
			out.write_u8(color.g);
			out.write_u8(color.b);
			out.write_u8(color.a);
		}

		out.write_u8(self.effect as u8);
		out.write_f32(self.fade_in);
		out.write_f32(self.fade_out);
		out.write_f32(self.hold);
		out.write_f32(self.effect_time);
		out.write_cstr(self.text);
		Ok(())
	}
}

/// Where a [`TextMsg`] prints (`HUD_PRINT*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TextDestination {
	/// The top-left notification area.
	Notify = 1,
	Console = 2,
	Chat = 3,
	Center = 4,
}

/// A localizable message, with up to four arguments for its `%s1`…`%s4`
/// (`TextMsg`), as `ClientPrint` sends it.
///
/// A message starting with `#` is a localization token, such as
/// `#TF_Arena_NoRespawning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMsg<'a> {
	pub destination: TextDestination,
	pub message: &'a CStr,
	pub arguments: [&'a CStr; 4],
}

impl UserMessage for TextMsg<'_> {
	fn name(&self) -> &CStr {
		c"TextMsg"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_u8(self.destination as u8);
		out.write_cstr(self.message);

		for argument in self.arguments {
			out.write_cstr(argument);
		}

		Ok(())
	}
}

/// A chat message, formatted like a player's (`SayText2`), as
/// `UTIL_SayText2Filter` sends it.
///
/// The message is a localization token, such as `TF_Chat_All`, whose
/// arguments are usually the speaker's name and the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SayText2<'a> {
	/// The speaking player's index, whose team colors the name, or 0 for the
	/// server.
	pub speaker: u8,

	/// Whether it plays the chat sound and shows in the chat history.
	pub chat: bool,

	pub message: &'a CStr,
	pub arguments: [&'a CStr; 4],
}

impl UserMessage for SayText2<'_> {
	fn name(&self) -> &CStr {
		c"SayText2"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_u8(self.speaker);
		out.write_u8(self.chat.into());
		out.write_cstr(self.message);

		for argument in self.arguments {
			out.write_cstr(argument);
		}

		Ok(())
	}
}

/// A hint in the HUD's hint box (`HintText`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintText<'a> {
	pub text: &'a CStr,
}

impl UserMessage for HintText<'_> {
	fn name(&self) -> &CStr {
		c"HintText"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_cstr(self.text);
		Ok(())
	}
}

/// A hint about a key binding (`KeyHintText`), as `env_hudhint` shows it.
/// Empty text hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHintText<'a> {
	pub text: &'a CStr,
}

impl UserMessage for KeyHintText<'_> {
	fn name(&self) -> &CStr {
		c"KeyHintText"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		// Clients read one string.
		out.write_u8(1);
		out.write_cstr(self.text);
		Ok(())
	}
}

/// TF2's notification with an icon (`HudNotifyCustom`), as
/// `CTFGameRules::SendHudNotification` sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudNotification<'a> {
	pub text: &'a CStr,

	/// The icon's name, such as `ico_notify_flag_moving`.
	pub icon: &'a CStr,

	/// The team whose color the notification takes, or 0 for none.
	pub team: u8,
}

impl UserMessage for HudNotification<'_> {
	fn name(&self) -> &CStr {
		c"HudNotifyCustom"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_cstr(self.text);
		out.write_cstr(self.icon);
		out.write_u8(self.team);
		Ok(())
	}
}

/// Turns a TF2 player's view (`ForcePlayerViewAngles`), as teleporters do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcePlayerViewAngles {
	/// The player's index.
	pub player: u8,
	pub angles: QAngle,
}

impl UserMessage for ForcePlayerViewAngles {
	fn name(&self) -> &CStr {
		c"ForcePlayerViewAngles"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		// Flags, which the game always sends as 1.
		out.write_u8(1);
		out.write_u8(self.player);
		out.write_bit_angles(self.angles);
		Ok(())
	}
}

/// Shows or hides a VGUI panel (`VGUIMenu`), such as the MOTD (`info`), with
/// string key values for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VguiMenu<'a> {
	/// The panel's name, such as `info` or `team`.
	pub name: &'a CStr,
	pub show: bool,
	/// At most 255, and about 192 bytes in all, as clients' buffers allow.
	pub keys: &'a [(&'a CStr, &'a CStr)],
}

impl UserMessage for VguiMenu<'_> {
	fn name(&self) -> &CStr {
		c"VGUIMenu"
	}

	fn write(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		let count = u8::try_from(self.keys.len()).map_err(|_| EncodeError::OutOfRange {
			field: "key count",
			value: self.keys.len() as u64,
			max: u8::MAX.into(),
		})?;

		out.write_cstr(self.name);
		out.write_u8(self.show.into());
		out.write_u8(count);

		for &(key, value) in self.keys {
			out.write_cstr(key);
			out.write_cstr(value);
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn payload(message: &impl UserMessage) -> BitWriter {
		let mut out = BitWriter::new();

		message.write(&mut out).unwrap();
		out
	}

	#[test]
	fn fixed_size_messages_have_their_registered_sizes() {
		let fade = Fade {
			duration: 1.5,
			hold: 100.0,
			flags: FadeFlags::OUT.union(FadeFlags::STAY_OUT),
			color: Color32::rgb(0, 0, 0),
		};
		let bits = payload(&fade);
		let mut reader = bits.reader();

		// `Fade` is registered with 10 bytes, `Shake` with 13.
		assert_eq!(bits.byte_len(), 10);
		assert_eq!(reader.read_u16(), Ok(768));
		assert_eq!(reader.read_u16(), Ok(51200));
		assert_eq!(reader.read_u16(), Ok(0xA));

		let shake = Shake {
			command: ShakeCommand::Start,
			amplitude: 10.0,
			frequency: 150.0,
			duration: 2.0,
		};

		assert_eq!(payload(&shake).byte_len(), 13);
	}

	#[test]
	fn fade_times_clamp_to_their_range() {
		assert_eq!(fade_time(-1.0), 0);
		assert_eq!(fade_time(1000.0), u16::MAX);
		assert_eq!(fade_time(f32::NAN), 0);
	}

	#[test]
	fn text_messages_carry_every_argument() {
		let message = TextMsg {
			destination: TextDestination::Center,
			message: c"Run.",
			arguments: [c"", c"", c"", c""],
		};
		let bits = payload(&message);
		let mut reader = bits.reader();

		assert_eq!(reader.read_u8(), Ok(4));
		assert_eq!(reader.read_cstring().as_deref(), Ok(c"Run."));

		for _ in 0..4 {
			assert_eq!(reader.read_cstring().as_deref(), Ok(c""));
		}

		assert_eq!(reader.remaining(), 0);
	}

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
			Ok(crate::math::Vector::new(10.0, -90.0, 0.0))
		);
	}
}
