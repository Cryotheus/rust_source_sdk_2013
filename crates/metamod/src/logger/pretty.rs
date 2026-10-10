//! Console formatting and explicit, per-destination color inputs.
//!
//! Terminal and environment facts are supplied by the host. Rendering never
//! probes or changes a console, reads environment variables, or executes a
//! terminal command. The fixed console palette is rendered by `anstyle` only
//! after destination policy enables ANSI, without process-global style state.
//! ANSI encoding is identical on Windows and Unix; native WinAPI styling is not
//! used. The host decides whether this destination supports ANSI.
//!
//! Automatic precedence is presence of NO_COLOR, then presence of
//! CLICOLOR_FORCE, then CLICOLOR (zero/nonzero), then the supplied terminal
//! capability. Explicit always/never comes first. Presence follows the original
//! policy literally, including an empty value or CLICOLOR_FORCE=0.

use anstyle::{Ansi256Color, Style};
use log::{Level, Record};

/// Environment facts, injected rather than read or changed during rendering.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ColorEnvironment {
	/// Whether NO_COLOR is present, including an empty value.
	pub no_color: bool,
	/// Whether CLICOLOR_FORCE is present, including empty or zero.
	pub clicolor_force: bool,
	/// CLICOLOR's zero/nonzero choice, or None when unset.
	pub clicolor: Option<bool>,
}

/// Explicit console color override.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
	/// Apply environment precedence, then terminal capability.
	#[default]
	Auto,
	/// Emit no styling commands.
	Never,
	/// Emit console styling even without automatic capability detection.
	Always,
}

/// Identity of a separately routed output destination.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
	/// Console output whose capability the host independently knows.
	Console,
	/// RCON output without a verified per-socket color adapter.
	Rcon,
	/// A file, replicated engine sink or other unidentified destination.
	#[default]
	Unknown,
}

/// Formatting inputs for one separately routed sink.
#[derive(Debug, Default, Clone)]
pub struct PrettyOptions {
	/// Root module name omitted from the metadata tag. Descendant module names
	/// are shortened; foreign targets keep their leading double colon.
	pub root_target: String,
	/// The destination, independent of the server's stdout.
	pub destination: Destination,
	/// Explicit console override.
	pub color_mode: ColorMode,
	/// Injected environment policy.
	pub environment: ColorEnvironment,
	/// Capability of this console, not any replicated destination.
	pub terminal: TerminalColor,
}

impl PrettyOptions {
	/// Resolves styling for a known console. RCON and unknown destinations
	/// remain plain even with Always; a console override is not client consent.
	pub fn color_enabled(&self) -> bool {
		if self.destination != Destination::Console {
			return false;
		}

		match self.color_mode {
			ColorMode::Never => false,
			ColorMode::Always => true,
			ColorMode::Auto if self.environment.no_color => false,
			ColorMode::Auto if self.environment.clicolor_force => true,

			ColorMode::Auto => self.environment.clicolor.unwrap_or(match self.terminal {
				TerminalColor::Windows {
					virtual_terminal_processing,
				} => virtual_terminal_processing,

				TerminalColor::Unix { is_terminal } => is_terminal,
				TerminalColor::Unknown => false,
			}),
		}
	}

	pub(super) fn render(&self, record: &Record<'_>) -> String {
		if self.destination != Destination::Console {
			return record.args().to_string();
		}

		let target = record.target();
		let tag = if target == self.root_target {
			String::new()
		} else if let Some(local) = target.strip_prefix(&format!("{}::", self.root_target)) {
			format!(" {local}")
		} else {
			format!(" ::{target}")
		};

		if !self.color_enabled() {
			return format!("[{}{tag}] {}", record.level(), record.args());
		}

		let (level_style, tag_style, message_style) = styles(record.level());
		format!(
			"{tag_style}[{level_style}{}{tag_style}{tag}]{tag_style:#} {message_style}{}{message_style:#}",
			record.level(),
			record.args(),
		)
	}
}

/// The already-known console capability, without changing terminal modes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColor {
	/// Existing Windows console mode; no attempt is made to enable VT.
	Windows {
		/// ENABLE_VIRTUAL_TERMINAL_PROCESSING was already set.
		virtual_terminal_processing: bool,
	},
	/// Unix stdout for this independently routed console.
	Unix {
		/// Whether this stdout is a terminal.
		is_terminal: bool,
	},
	/// No proven terminal capability.
	#[default]
	Unknown,
}

// Fixed indexed equivalents of the console's existing severity palette.
// The caller has already selected a known console and enabled styling.
fn styles(level: Level) -> (Style, Style, Style) {
	let foreground = |color| Style::new().fg_color(Some(Ansi256Color(color).into()));
	match level {
		Level::Error => (
			foreground(15).bg_color(Some(Ansi256Color(1).into())),
			foreground(15).bg_color(Some(Ansi256Color(1).into())),
			foreground(9).bg_color(Some(Ansi256Color(0).into())),
		),

		Level::Warn => (foreground(3), foreground(3), foreground(11)),
		Level::Info => (foreground(12), foreground(12), foreground(7)),
		Level::Debug => (foreground(7), foreground(8), foreground(8)),
		Level::Trace => (foreground(8), foreground(8), foreground(8)),
	}
}
