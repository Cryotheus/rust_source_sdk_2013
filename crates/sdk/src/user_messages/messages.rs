//! Typed user messages, written as the game's own senders write them.
//!
//! The payloads follow the SDK's senders: `game/server/util.cpp` for most,
//! and `game/shared/hintmessage.cpp` for hints. TF2's own messages are in
//! `tf2::user_messages`, with the `tf2` feature.

use super::UserMessage;
use crate::bitbuf::BitWriter;
use crate::math::Color32;
use crate::net::EncodeError;
use std::ffi::CStr;

/// Fixed-point fraction bits of fade times (`SCREENFADE_FRACBITS`).
const FADE_FRACTION_BITS: u32 = 9;

/// Fades the screen to or from a color (`Fade`), as `env_fade` does.
#[doc(alias("ScreenFade_t", "UTIL_ScreenFade", "env_fade"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
	/// Seconds the fade takes, up to about 128.
	pub duration: f32,

	/// Seconds the color is held once reached, up to about 128.
	#[doc(alias("holdTime"))]
	pub hold: f32,

	/// How the fade behaves.
	#[doc(alias("fadeFlags"))]
	pub flags: FadeFlags,

	/// The color faded to or from, whose alpha is the fade's greatest
	/// opacity.
	pub color: Color32,
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

/// How a [`Fade`] behaves (`FFADE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FadeFlags(pub u16);

impl FadeFlags {
	/// From the color to clear.
	#[doc(alias("FFADE_IN"))]
	pub const IN: Self = Self(0x1);

	/// Multiplies the screen by the color instead of blending it.
	#[doc(alias("FFADE_MODULATE"))]
	pub const MODULATE: Self = Self(0x4);

	/// From clear to the color.
	#[doc(alias("FFADE_OUT"))]
	pub const OUT: Self = Self(0x2);

	/// Replaces every other fade.
	#[doc(alias("FFADE_PURGE"))]
	pub const PURGE: Self = Self(0x10);

	/// Holds the color until another fade replaces it.
	#[doc(alias("FFADE_STAYOUT"))]
	pub const STAY_OUT: Self = Self(0x8);

	/// The flags set in either.
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

/// A hint in the HUD's hint box (`HintText`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintText<'a> {
	/// The text shown in the hint box.
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

/// Text on the HUD (`HudMsg`), as `game_text` shows it.
#[doc(alias("HudMsg", "game_text", "hudtextparms_t", "UTIL_HudMessage"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudText<'a> {
	/// Replaces the text shown in the same channel, from 0 to 5.
	pub channel: u8,

	/// The horizontal position as a fraction of the screen's width, from the
	/// left, or from the right if negative; -1 centers the text.
	pub x: f32,

	/// The vertical position as a fraction of the screen's height, from the
	/// top, or from the bottom if negative; -1 centers the text.
	pub y: f32,

	/// The text's color.
	pub color: Color32,

	/// The color of the scan-out and flicker effects.
	pub effect_color: Color32,

	/// How the text appears.
	pub effect: HudTextEffect,

	/// Seconds the text takes to fade in, or, for the scan-out effect, between
	/// characters.
	#[doc(alias("fadeinTime"))]
	pub fade_in: f32,

	/// Seconds the text takes to fade out.
	#[doc(alias("fadeoutTime"))]
	pub fade_out: f32,

	/// Seconds the text stays once shown, before it fades out.
	#[doc(alias("holdTime"))]
	pub hold: f32,

	/// Seconds each character takes to change from the effect color to the
	/// text's, for the scan-out effect.
	#[doc(alias("fxTime"))]
	pub effect_time: f32,

	/// The text shown.
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

/// A hint about a key binding (`KeyHintText`), as `env_hudhint` shows it.
/// Empty text hides it.
#[doc(alias("env_hudhint", "UTIL_HudHintText"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHintText<'a> {
	/// The hint's text, or empty to hide the hint.
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

/// A chat message, formatted like a player's (`SayText2`), as
/// `UTIL_SayText2Filter` sends it.
///
/// The message is a localization token, such as `TF_Chat_All`, whose
/// arguments are usually the speaker's name and the text.
#[doc(alias("UTIL_SayText2Filter"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SayText2<'a> {
	/// The speaking player's index, whose team colors the name, or 0 for the
	/// server.
	pub speaker: u8,

	/// Whether it plays the chat sound and shows in the chat history.
	pub chat: bool,

	/// The format, as a localization token or plain text.
	pub message: &'a CStr,

	/// The strings substituted for the format's `%s1`…`%s4`, empty when
	/// unused.
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

/// Shakes the screen (`Shake`), as `env_shake` does.
#[doc(alias("ScreenShake_t", "UTIL_ScreenShake", "env_shake"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shake {
	/// What the message does to the client's shakes.
	pub command: ShakeCommand,

	/// Up to 16.
	pub amplitude: f32,

	/// Up to 255.
	pub frequency: f32,

	/// Seconds the shake lasts.
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

/// What a [`Shake`] does (`ShakeCommand_t`).
#[doc(alias("ShakeCommand_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShakeCommand {
	/// Starts a shake, alongside any in progress.
	#[doc(alias("SHAKE_START"))]
	Start = 0,
	/// Stops every shake in progress.
	#[doc(alias("SHAKE_STOP"))]
	Stop = 1,
	/// Changes the amplitude of a shake in progress.
	#[doc(alias("SHAKE_AMPLITUDE"))]
	Amplitude = 2,
	/// Changes the frequency of a shake in progress.
	#[doc(alias("SHAKE_FREQUENCY"))]
	Frequency = 3,
	/// Only rumbles controllers.
	#[doc(alias("SHAKE_START_RUMBLEONLY"))]
	StartRumbleOnly = 4,
	/// Shakes without rumbling controllers.
	#[doc(alias("SHAKE_START_NORUMBLE"))]
	StartNoRumble = 5,
}

/// Where a [`TextMsg`] prints (`HUD_PRINT*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TextDestination {
	/// The top-left notification area.
	#[doc(alias("HUD_PRINTNOTIFY"))]
	Notify = 1,
	/// The console.
	#[doc(alias("HUD_PRINTCONSOLE"))]
	Console = 2,
	/// The chat.
	#[doc(alias("HUD_PRINTTALK"))]
	Chat = 3,
	/// The center of the screen.
	#[doc(alias("HUD_PRINTCENTER"))]
	Center = 4,
}

/// A localizable message, with up to four arguments for its `%s1`…`%s4`
/// (`TextMsg`), as `ClientPrint` sends it.
///
/// A message starting with `#` is a localization token, such as
/// `#TF_Arena_NoRespawning`.
#[doc(alias("ClientPrint", "UTIL_ClientPrintFilter"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMsg<'a> {
	/// Where the message prints.
	pub destination: TextDestination,

	/// The text, or a localization token.
	pub message: &'a CStr,

	/// The strings substituted for the message's `%s1`…`%s4`, empty when
	/// unused.
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

/// Shows or hides a VGUI panel (`VGUIMenu`), such as the MOTD (`info`), with
/// string key values for it.
#[doc(alias("VGUIMenu", "ShowViewPortPanel"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VguiMenu<'a> {
	/// The panel's name, such as `info` or `team`.
	pub name: &'a CStr,

	/// Whether the panel is shown rather than hidden.
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

/// Seconds as the unsigned 7.9 fixed point fades use, clamped to its range,
/// as `FixedUnsigned16` does.
fn fade_time(seconds: f32) -> u16 {
	(seconds * (1 << FADE_FRACTION_BITS) as f32).clamp(0.0, u16::MAX.into()) as u16
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fade_times_clamp_to_their_range() {
		assert_eq!(fade_time(-1.0), 0);
		assert_eq!(fade_time(1000.0), u16::MAX);
		assert_eq!(fade_time(f32::NAN), 0);
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

	fn payload(message: &impl UserMessage) -> BitWriter {
		let mut out = BitWriter::new();

		message.write(&mut out).unwrap();
		out
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
}
