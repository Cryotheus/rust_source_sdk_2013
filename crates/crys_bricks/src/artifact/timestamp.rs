use crate::config::{Profile, RequireVarError};
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::fmt::{Display, Formatter};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct TimestampFmt {
	pub profile: Profile,
}

impl TimestampFmt {
	#[doc(alias = "new")]
	pub fn from_env() -> Result<Self, RequireVarError> {
		let profile = Profile::from_env()?;

		Ok(Self { profile })
	}
}

impl Display for TimestampFmt {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let time = DateTime::<Utc>::from(SystemTime::now());
		let day = time.day();
		let month = time.month();
		let year = time.year();

		if self.profile.is_release() {
			write!(f, "{month:02}/{day:02}/{year:04}")
		} else {
			write!(
				f,
				"{month:02}/{day:02}/{year:04} at {}:{:02}",
				time.hour(),
				time.minute()
			)
		}
	}
}
