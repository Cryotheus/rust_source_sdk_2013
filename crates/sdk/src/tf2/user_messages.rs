//! TF2's own user messages, which only its client registers.
//!
//! The payloads follow TF2's game rules and player, which send them.

use crate::bitbuf::BitWriter;
use crate::math::QAngle;
use crate::net::EncodeError;
use crate::user_messages::UserMessage;
use std::ffi::CStr;

/// Turns a TF2 player's view (`ForcePlayerViewAngles`), as teleporters do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcePlayerViewAngles {
	/// The player's index.
	pub player: u8,

	/// The angles the player's view turns to.
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

/// TF2's notification with an icon (`HudNotifyCustom`), as
/// `CTFGameRules::SendHudNotification` sends it.
#[doc(alias("HudNotifyCustom", "SendHudNotification", "game_text_tf"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudNotification<'a> {
	/// The text shown with the icon.
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
