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

/// Foreground RGB sent separately from console text. Native Source colors
/// cannot express the historical error tag's red background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleColor(pub u8, pub u8, pub u8);

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

/// Determines native console styling without requiring ANSI/VT support.
/// Explicit overrides and the original environment precedence still apply.
pub fn native_color_enabled(mode: ColorMode, environment: ColorEnvironment) -> bool {
	match mode {
		ColorMode::Never => false,
		ColorMode::Always => true,
		ColorMode::Auto if environment.no_color => false,
		ColorMode::Auto if environment.clicolor_force => true,
		ColorMode::Auto => environment.clicolor.unwrap_or(true),
	}
}

fn native_palette(level: Level) -> (ConsoleColor, ConsoleColor, ConsoleColor) {
	let grey = ConsoleColor(192, 192, 192);
	let dark_grey = ConsoleColor(128, 128, 128);
	match level {
		Level::Error => (
			ConsoleColor(255, 255, 255),
			ConsoleColor(255, 255, 255),
			ConsoleColor(255, 0, 0),
		),

		Level::Warn => (
			ConsoleColor(128, 128, 0),
			ConsoleColor(128, 128, 0),
			ConsoleColor(255, 255, 0),
		),

		Level::Info => (ConsoleColor(0, 0, 255), ConsoleColor(0, 0, 255), grey),
		Level::Debug => (grey, dark_grey, dark_grey),
		Level::Trace => (dark_grey, dark_grey, dark_grey),
	}
}

/// Renders owned native-console metadata while retaining its exact boundary.
/// Targets equal to the plugin root are omitted, descendants are shortened,
/// and foreign targets retain the historical leading `::`.
pub(super) fn render_native(record: &Record<'_>, root_target: &str) -> (String, usize) {
	let target = record.target();
	let tag = if target == root_target {
		String::new()
	} else if let Some(local) = target.strip_prefix(&format!("{root_target}::")) {
		format!(" {local}")
	} else {
		format!(" ::{target}")
	};
	let mut text = format!("[{}{tag}] ", record.level());
	let prefix_bytes = text.len();
	text.push_str(&record.args().to_string());
	(text, prefix_bytes)
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

/// Emits a native-console record as colored text fragments, followed by one
/// newline. All colors are out-of-band; concatenating the fragments gives the
/// exact ANSI-free line for capture/RCON. Call the supplied writer only within
/// a valid main-thread engine callback and outside the worker queue lock.
/// Malformed public prefix boundaries fall back to one uncolored fragment.
pub fn write_native(
	record: &super::OwnedRecord,
	plugin_tag: &str,
	colored: bool,
	mut write: impl FnMut(Option<ConsoleColor>, &str),
) {
	let prefix = format!("[{plugin_tag}] ");
	let level = record.level.to_string();
	let valid = record.prefix_bytes >= level.len() + 3
		&& record.text.is_char_boundary(record.prefix_bytes)
		&& record.text.starts_with(&format!("[{level}"))
		&& record
			.text
			.get(record.prefix_bytes.saturating_sub(2)..record.prefix_bytes)
			== Some("] ");
	if !colored || !valid {
		write(None, &format!("{prefix}{}\n", record.text));
		return;
	}
	let (level_color, tag_color, message_color) = native_palette(record.level);
	write(Some(tag_color), &format!("{prefix}["));
	write(Some(level_color), &record.text[1..1 + level.len()]);
	write(
		Some(tag_color),
		&record.text[1 + level.len()..record.prefix_bytes],
	);
	write(
		Some(message_color),
		&format!("{}\n", &record.text[record.prefix_bytes..]),
	);
}
