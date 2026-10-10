//! The requested logger policy and native sink boundary regressions.

use super::*;
use crate::api::MetamodVersion;
use crate::sys::api::ISmmApi;
use crate::test_support::smm::MockSmm;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

thread_local! {
	static CAPTURED: RefCell<Vec<(String, String, usize)>> = const { RefCell::new(Vec::new()) };
	static REENTER: RefCell<Option<Arc<queue::LogQueue>>> = const { RefCell::new(None) };
}

use std::sync::atomic::{AtomicUsize, Ordering};

static NATIVE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn capture(
	_this: *mut ISmmApi,
	plugin: *mut c_void,
	format: *const c_char,
	mut arguments: ...
) {
	NATIVE_CALLS.fetch_add(1, Ordering::Relaxed);
	// SAFETY: MetamodApi::log_cstr supplies a NUL-terminated "%s" format and
	// exactly one C string vararg, live for this native call.
	let (format, message) = unsafe {
		(
			CStr::from_ptr(format).to_string_lossy().into_owned(),
			CStr::from_ptr(arguments.next_arg::<*const c_char>())
				.to_string_lossy()
				.into_owned(),
		)
	};
	CAPTURED.with(|captured| {
		captured
			.borrow_mut()
			.push((format, message, plugin as usize))
	});
	REENTER.with(|queue| {
		if let Some(queue) = queue.borrow().as_ref() {
			queue.write(OwnedRecord {
				level: Level::Info,
				text: "during native callback".into(),
			});
		}
	});
}

#[test]
fn closing_old_queue_rejects_late_workers_and_new_load_is_independent() {
	let old = queue::LogQueue::new(8, 1024);
	old.write(OwnedRecord {
		level: Level::Info,
		text: "old pending".into(),
	});
	assert_eq!(old.close().front().unwrap().text, "old pending");
	old.write(OwnedRecord {
		level: Level::Info,
		text: "late old worker".into(),
	});
	assert_eq!(
		old.stats(),
		queue::QueueStats {
			records: 0,
			bytes: 0,
			dropped: 1,
			accepting: false
		}
	);
	let new = queue::LogQueue::new(8, 1024);
	new.write(OwnedRecord {
		level: Level::Info,
		text: "new callback".into(),
	});
	assert_eq!(new.drain().front().unwrap().text, "new callback");
	assert!(old.drain().is_empty());
}

#[cfg(feature = "logger_pretty")]
#[test]
fn concurrent_destinations_keep_independent_color_policies() {
	use pretty::{ColorEnvironment, ColorMode, Destination, PrettyOptions};
	let start = Arc::new(std::sync::Barrier::new(3));
	let mut workers = Vec::new();
	for (destination, color_mode, no_color, clicolor_force, colored, metadata) in [
		(
			Destination::Console,
			ColorMode::Always,
			true,
			false,
			true,
			true,
		),
		(
			Destination::Console,
			ColorMode::Never,
			false,
			true,
			false,
			true,
		),
		(
			Destination::Rcon,
			ColorMode::Always,
			false,
			true,
			false,
			false,
		),
	] {
		let start = start.clone();
		workers.push(std::thread::spawn(move || {
			let logger = Logger::pretty(
				queue::LogQueue::new(32, 8192),
				LevelFilter::Info,
				PrettyOptions {
					root_target: "plugin".into(),
					destination,
					color_mode,
					environment: ColorEnvironment {
						no_color,
						clicolor_force,
						clicolor: None,
					},
					..PrettyOptions::default()
				},
			);
			start.wait();
			for _ in 0..32 {
				logger.log(
					&Record::builder()
						.level(Level::Info)
						.target("plugin")
						.args(format_args!("independent message"))
						.build(),
				);
			}
			let records = logger.sink().drain();
			assert_eq!(records.len(), 32);
			assert_eq!(logger.sink().stats().dropped, 0);
			for record in records {
				assert_eq!(record.text.contains('\u{1b}'), colored);
				if colored {
					assert_eq!(
						record.text,
						"\x1b[38;5;12m[\x1b[38;5;12mINFO\x1b[38;5;12m]\x1b[0m \x1b[38;5;7mindependent message\x1b[0m"
					);
				} else if metadata {
					assert_eq!(record.text, "[INFO] independent message");
				} else {
					assert_eq!(record.text, "independent message");
				}
			}
		}));
	}
	for worker in workers {
		worker.join().unwrap();
	}
}

#[cfg(feature = "logger_pretty")]
#[test]
fn forced_console_palette_is_independent_of_style_globals_and_resets_after_message() {
	use pretty::{ColorEnvironment, ColorMode, Destination, PrettyOptions};
	let logger = Logger::pretty(
		queue::LogQueue::new(8, 1024),
		LevelFilter::Trace,
		PrettyOptions {
			root_target: "plugin".into(),
			destination: Destination::Console,
			color_mode: ColorMode::Always,
			environment: ColorEnvironment {
				no_color: true,
				..ColorEnvironment::default()
			},
			..PrettyOptions::default()
		},
	);
	for (level, level_color, tag_color, message_color) in [
		(
			Level::Error,
			"\x1b[38;5;15m\x1b[48;5;1m",
			"\x1b[38;5;15m\x1b[48;5;1m",
			"\x1b[38;5;9m\x1b[48;5;0m",
		),
		(Level::Warn, "\x1b[38;5;3m", "\x1b[38;5;3m", "\x1b[38;5;11m"),
		(
			Level::Info,
			"\x1b[38;5;12m",
			"\x1b[38;5;12m",
			"\x1b[38;5;7m",
		),
		(Level::Debug, "\x1b[38;5;7m", "\x1b[38;5;8m", "\x1b[38;5;8m"),
		(Level::Trace, "\x1b[38;5;8m", "\x1b[38;5;8m", "\x1b[38;5;8m"),
	] {
		logger.log(
			&Record::builder()
				.level(level)
				.target("plugin::module")
				.args(format_args!("text"))
				.build(),
		);
		assert_eq!(
			logger.sink().drain().front().unwrap().text,
			format!(
				"{tag_color}[{level_color}{level}{tag_color} module]\x1b[0m {message_color}text\x1b[0m"
			)
		);
	}
}

#[test]
fn native_conversion_rejects_nul_instead_of_truncating_or_panicking() {
	let record = OwnedRecord {
		level: Level::Warn,
		text: "first\0second".into(),
	};
	let error = record.into_c_string().unwrap_err();
	assert_eq!(error.nul_position(), 5);
	assert_eq!(error.into_vec(), b"first\0second");
}

#[test]
fn owned_worker_messages_reach_both_native_log_abis_outside_queue_lock() {
	for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
		CAPTURED.with(|captured| captured.borrow_mut().clear());
		NATIVE_CALLS.store(0, Ordering::Relaxed);
		let queue = Arc::new(queue::LogQueue::new(8, 1024));
		let worker_queue = queue.clone();
		std::thread::spawn(move || {
			let logger = Logger::new(worker_queue, LevelFilter::Info);
			let message = String::from("worker é 100% %n");
			logger.log(
				&Record::builder()
					.level(Level::Info)
					.target("worker")
					.args(format_args!("{message}"))
					.build(),
			);
			// No engine call during flush on the worker.
			logger.flush();
			logger.log(
				&Record::builder()
					.level(Level::Debug)
					.args(format_args!("filtered"))
					.build(),
			);
		})
		.join()
		.unwrap();
		assert_eq!(queue.stats().records, 1);
		assert_eq!(NATIVE_CALLS.load(Ordering::Relaxed), 0);
		CAPTURED.with(|captured| assert!(captured.borrow().is_empty()));

		let smm = MockSmm::new(version, &[(0, capture as *const ())]);
		let api = smm.api();
		let mut plugin_object = 0_u8;
		let plugin = NonNull::from(&mut plugin_object).cast::<c_void>();
		REENTER.with(|current| *current.borrow_mut() = Some(queue.clone()));
		for record in queue.drain() {
			api.log_cstr(plugin, &record.into_c_string().unwrap());
		}
		REENTER.with(|current| current.borrow_mut().take());
		CAPTURED.with(|captured| {
			assert_eq!(
				&*captured.borrow(),
				&[(
					"%s".into(),
					"worker é 100% %n".into(),
					plugin.as_ptr() as usize
				)]
			);
		});
		assert_eq!(NATIVE_CALLS.load(Ordering::Relaxed), 1);
		assert_eq!(
			queue.drain().front().unwrap().text,
			"during native callback"
		);
	}
}

#[test]
fn plain_feature_keeps_only_message_even_with_pretty_feature_enabled() {
	let logger = Logger::new(queue::LogQueue::new(8, 1024), LevelFilter::Info);
	logger.log(
		&Record::builder()
			.level(Level::Warn)
			.target("foreign::target")
			.args(format_args!("message {}", 42))
			.build(),
	);
	let records = logger.sink().drain();
	assert_eq!(records.front().unwrap().text, "message 42");
	assert_eq!(records.front().unwrap().level, Level::Warn);
}

#[cfg(feature = "logger_pretty")]
#[test]
fn pretty_policy_obeys_explicit_override_environment_then_terminal() {
	use pretty::{
		ColorEnvironment as Env, ColorMode as Mode, Destination, PrettyOptions, TerminalColor,
	};
	let console = PrettyOptions {
		destination: Destination::Console,
		..PrettyOptions::default()
	};
	let cases = [
		(
			Mode::Always,
			Env {
				no_color: true,
				clicolor_force: false,
				clicolor: Some(false),
			},
			TerminalColor::Unknown,
			true,
		),
		(
			Mode::Never,
			Env {
				no_color: false,
				clicolor_force: true,
				clicolor: Some(true),
			},
			TerminalColor::Unix { is_terminal: true },
			false,
		),
		(
			Mode::Auto,
			Env {
				no_color: true,
				clicolor_force: true,
				clicolor: Some(true),
			},
			TerminalColor::Unix { is_terminal: true },
			false,
		),
		(
			Mode::Auto,
			Env {
				no_color: false,
				clicolor_force: true,
				clicolor: Some(false),
			},
			TerminalColor::Unknown,
			true,
		),
		(
			Mode::Auto,
			Env {
				no_color: false,
				clicolor_force: false,
				clicolor: Some(false),
			},
			TerminalColor::Windows {
				virtual_terminal_processing: true,
			},
			false,
		),
		(
			Mode::Auto,
			Env {
				no_color: false,
				clicolor_force: false,
				clicolor: Some(true),
			},
			TerminalColor::Unknown,
			true,
		),
		(
			Mode::Auto,
			Env::default(),
			TerminalColor::Windows {
				virtual_terminal_processing: true,
			},
			true,
		),
		(
			Mode::Auto,
			Env::default(),
			TerminalColor::Windows {
				virtual_terminal_processing: false,
			},
			false,
		),
		(
			Mode::Auto,
			Env::default(),
			TerminalColor::Unix { is_terminal: true },
			true,
		),
		(
			Mode::Auto,
			Env::default(),
			TerminalColor::Unix { is_terminal: false },
			false,
		),
		(Mode::Auto, Env::default(), TerminalColor::Unknown, false),
	];
	for (color_mode, environment, terminal, expected) in cases {
		let options = PrettyOptions {
			color_mode,
			environment,
			terminal,
			..console.clone()
		};
		assert_eq!(options.color_enabled(), expected, "{options:?}");
	}
}

#[test]
fn queue_limits_bound_utf8_bytes_and_count_without_losing_accepted_order() {
	let queue = queue::LogQueue::new(3, 4);
	for text in ["é", "é", "x"] {
		queue.write(OwnedRecord {
			level: Level::Info,
			text: text.into(),
		});
	}
	assert_eq!(
		queue.stats(),
		queue::QueueStats {
			records: 2,
			bytes: 4,
			dropped: 1,
			accepting: true
		}
	);
	assert_eq!(
		queue
			.drain()
			.into_iter()
			.map(|record| record.text)
			.collect::<Vec<_>>(),
		["é", "é"]
	);
	queue.write(OwnedRecord {
		level: Level::Warn,
		text: "longer".into(),
	});
	assert_eq!(queue.stats().dropped, 2);
	for text in ["", "", "", ""] {
		queue.write(OwnedRecord {
			level: Level::Info,
			text: text.into(),
		});
	}
	assert_eq!(
		queue.stats(),
		queue::QueueStats {
			records: 3,
			bytes: 0,
			dropped: 3,
			accepting: true
		}
	);
}

#[cfg(feature = "logger_pretty")]
#[test]
fn rcon_unknown_and_console_rendering_are_independent_even_when_forced() {
	use pretty::{ColorEnvironment, ColorMode, Destination, PrettyOptions, TerminalColor};
	for destination in [Destination::Rcon, Destination::Unknown] {
		let options = PrettyOptions {
			root_target: "plugin".into(),
			destination,
			color_mode: ColorMode::Always,
			environment: ColorEnvironment {
				clicolor_force: true,
				..ColorEnvironment::default()
			},
			terminal: TerminalColor::Windows {
				virtual_terminal_processing: true,
			},
		};
		assert!(!options.color_enabled());
		let logger = Logger::pretty(queue::LogQueue::new(8, 1024), LevelFilter::Trace, options);
		logger.log(
			&Record::builder()
				.level(Level::Error)
				.target("plugin::module")
				.args(format_args!("message"))
				.build(),
		);
		assert_eq!(logger.sink().drain().front().unwrap().text, "message");
	}

	let options = PrettyOptions {
		root_target: "plugin".into(),
		destination: Destination::Console,
		color_mode: ColorMode::Never,
		..PrettyOptions::default()
	};
	let logger = Logger::pretty(queue::LogQueue::new(8, 1024), LevelFilter::Trace, options);
	for (target, expected) in [
		("plugin", "[INFO] text"),
		("plugin::module", "[INFO module] text"),
		("foreign", "[INFO ::foreign] text"),
	] {
		logger.log(
			&Record::builder()
				.level(Level::Info)
				.target(target)
				.args(format_args!("text"))
				.build(),
		);
		assert_eq!(logger.sink().drain().front().unwrap().text, expected);
	}
}

#[test]
fn short_message_cannot_retain_an_oversized_string_reservation() {
	let queue = queue::LogQueue::new(1, 1);
	let mut text = String::with_capacity(65536);
	text.push('x');
	queue.write(OwnedRecord {
		level: Level::Info,
		text,
	});
	let records = queue.drain();
	assert_eq!(records.front().unwrap().text.capacity(), 1);
	assert_eq!(records.front().unwrap().text, "x");
}

#[test]
fn zero_limits_reject_even_empty_records() {
	for (max_records, max_bytes) in [(0, 1024), (8, 0), (0, 0)] {
		let queue = queue::LogQueue::new(max_records, max_bytes);
		queue.write(OwnedRecord {
			level: Level::Info,
			text: String::new(),
		});
		assert_eq!(
			queue.stats(),
			queue::QueueStats {
				records: 0,
				bytes: 0,
				dropped: 1,
				accepting: true
			}
		);
		assert!(queue.drain().is_empty());
	}
}
