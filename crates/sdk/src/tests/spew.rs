use super::*;
use std::ffi::c_int;
use std::sync::{Mutex, MutexGuard};

/// One line a callback saw: kind, text, group, level and colour.
type Seen = (SpewKind, String, String, i32, Option<SpewColor>);

static COLOR: SpewColor = SpewColor {
	r: 1,
	g: 2,
	b: 3,
	a: 4,
};

/// The fake tier0's spew function.
static CURRENT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// Whether a thread is inside [`hold`].
static HELD: AtomicBool = AtomicBool::new(false);

/// The function another plugin replaced, which it calls.
static OTHER_PREVIOUS: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// What the fake engine's and default spew functions printed.
static PRINTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether [`hold`] lets its thread go.
static RELEASED: AtomicBool = AtomicBool::new(false);

/// What the callbacks saw.
static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Watches share statics, so tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn a_busy_stop_can_be_retried() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(hold);
	let printing = std::thread::spawn(|| spew(SpewType::MESSAGE, c"held"));

	while !HELD.load(Ordering::SeqCst) {
		std::thread::yield_now();
	}

	// The watch leaves the chain, but a thread is still inside it.
	assert_eq!(watch.stop(), Stopped::Busy);
	assert_eq!(
		output_function().map(function_address),
		Some(function_address(engine_output))
	);

	// SAFETY: As for `start`.
	let second = unsafe { watch_with(fake_api(), record) };

	assert!(matches!(second, Err(WatchError::AlreadyWatching)));

	RELEASED.store(true, Ordering::SeqCst);
	printing.join().unwrap();

	assert_eq!(watch.stop(), Stopped::Restored);
	assert_eq!(watch.stop(), Stopped::Restored);
	assert_eq!(printed(), ["engine: held"]);
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn a_filter_does_not_hide_recursive_observer_warnings() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(|_| {
		spew(SpewType::WARNING, c"nested\n");
	});
	watch.filter_warnings(Some(|_| false));

	spew(SpewType::WARNING, c"outer\n");
	assert_eq!(printed(), ["engine: nested\n"]);
	assert_eq!(watch.stop(), Stopped::Restored);
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn a_null_function_means_the_default() {
	let _serial = setup(None);
	let mut watch = start(record);

	spew(SpewType::MESSAGE, c"plain");

	assert_eq!(printed(), ["default: plain"]);
	assert_eq!(watch.stop(), Stopped::Restored);
	assert!(output_function().is_none());
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn a_warning_filter_panic_passes_the_warning_on() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);
	watch.filter_warnings(Some(|_| panic!("filter panic")));

	assert_eq!(spew(SpewType::WARNING, c"keep\n"), SpewRetval::CONTINUE);
	assert_eq!(
		spew(SpewType::WARNING, c"keep again\n"),
		SpewRetval::CONTINUE
	);
	assert_eq!(printed(), ["engine: keep\n", "engine: keep again\n"]);
	assert_eq!(watch.stop(), Stopped::Restored);
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn cleared_and_stopped_filters_do_not_affect_the_next_watch() {
	let _serial = setup(Some(engine_output));
	let mut old = start(record);
	old.filter_warnings(Some(|_| false));
	spew(SpewType::WARNING, c"hidden\n");
	old.filter_warnings(None);
	spew(SpewType::WARNING, c"cleared\n");
	old.filter_warnings(Some(|_| false));
	assert_eq!(old.stop(), Stopped::Restored);

	let mut next = start(record);
	old.filter_warnings(Some(|_| false));
	spew(SpewType::WARNING, c"new watch\n");
	assert_eq!(printed(), ["engine: cleared\n", "engine: new watch\n"]);
	assert_eq!(next.stop(), Stopped::Restored);
}

extern "C" fn color() -> *const SpewColor {
	&COLOR
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn contains_panics() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(panic_on_output);

	assert_eq!(spew(SpewType::MESSAGE, c"first"), SpewRetval::CONTINUE);
	assert_eq!(spew(SpewType::MESSAGE, c"second"), SpewRetval::CONTINUE);
	assert_eq!(printed(), ["engine: first", "engine: second"]);
	assert_eq!(watch.stop(), Stopped::Restored);
}

extern "C" fn default_output(_: SpewType, message: *const c_char) -> SpewRetval {
	PRINTED
		.lock()
		.unwrap()
		.push(format!("default: {}", text(message)));
	SpewRetval::CONTINUE
}

/// The engine's spew function, which asks for the debugger on assertions.
extern "C" fn engine_output(kind: SpewType, message: *const c_char) -> SpewRetval {
	PRINTED
		.lock()
		.unwrap()
		.push(format!("engine: {}", text(message)));

	if kind == SpewType::ASSERT {
		SpewRetval::DEBUGGER
	} else if kind == SpewType::ERROR {
		SpewRetval::ABORT
	} else {
		SpewRetval::CONTINUE
	}
}

fn fake_api() -> SpewApi {
	SpewApi {
		set_output,
		output: output_function,
		default: default_output,
		group,
		level,
		color,
	}
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn filtering_preserves_later_watchers_and_their_saved_chain() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);
	watch.filter_warnings(Some(|spew| spew.text() != c"obsolete\n"));
	OTHER_PREVIOUS.store(CURRENT.load(Ordering::SeqCst), Ordering::SeqCst);
	set_output(Some(other_output));

	assert_eq!(spew(SpewType::WARNING, c"obsolete\n"), SpewRetval::CONTINUE);
	assert_eq!(SEEN.lock().unwrap().len(), 1);
	assert_eq!(printed(), ["other: obsolete\n"]);
	assert_eq!(watch.stop(), Stopped::Bypassed);

	spew(SpewType::WARNING, c"obsolete\n");
	assert_eq!(
		printed(),
		[
			"other: obsolete\n",
			"other: obsolete\n",
			"engine: obsolete\n"
		]
	);
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn filters_only_selected_warnings_and_preserves_other_answers() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);
	watch.filter_warnings(Some(|spew| spew.text() != c"obsolete\n"));

	assert_eq!(spew(SpewType::WARNING, c"obsolete\n"), SpewRetval::CONTINUE);
	assert_eq!(
		spew(SpewType::WARNING, c"real warning\n"),
		SpewRetval::CONTINUE
	);
	assert_eq!(spew(SpewType::MESSAGE, c"obsolete\n"), SpewRetval::CONTINUE);
	assert_eq!(spew(SpewType::LOG, c"obsolete\n"), SpewRetval::CONTINUE);
	assert_eq!(spew(SpewType::ASSERT, c"obsolete\n"), SpewRetval::DEBUGGER);
	assert_eq!(spew(SpewType::ERROR, c"obsolete\n"), SpewRetval::ABORT);
	assert_eq!(spew(SpewType(99), c"obsolete\n"), SpewRetval::CONTINUE);

	assert_eq!(SEEN.lock().unwrap().len(), 7);
	assert_eq!(
		printed(),
		[
			"engine: real warning\n",
			"engine: obsolete\n",
			"engine: obsolete\n",
			"engine: obsolete\n",
			"engine: obsolete\n",
			"engine: obsolete\n",
		]
	);
	assert_eq!(watch.stop(), Stopped::Restored);
}

extern "C" fn group() -> *const c_char {
	c"console".as_ptr()
}

/// Keeps its thread inside the watch's function until [`RELEASED`].
fn hold(_: &Spew<'_>) {
	HELD.store(true, Ordering::SeqCst);

	while !RELEASED.load(Ordering::SeqCst) {
		std::thread::yield_now();
	}
}

#[test]
fn kinds_follow_tier0() {
	assert_eq!(SpewKind::from(SpewType::MESSAGE), SpewKind::Message);
	assert_eq!(SpewKind::from(SpewType::WARNING), SpewKind::Warning);
	assert_eq!(SpewKind::from(SpewType::ASSERT), SpewKind::Assert);
	assert_eq!(SpewKind::from(SpewType::ERROR), SpewKind::Error);
	assert_eq!(SpewKind::from(SpewType::LOG), SpewKind::Log);
	assert_eq!(SpewKind::from(SpewType(9)), SpewKind::Other(9));
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn leaves_a_later_function_working() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);

	// Another plugin chains after the watch, saving its stub.
	OTHER_PREVIOUS.store(CURRENT.load(Ordering::SeqCst), Ordering::SeqCst);
	set_output(Some(other_output));

	spew(SpewType::MESSAGE, c"chained");

	assert_eq!(watch.stop(), Stopped::Bypassed);
	assert_eq!(
		output_function().map(function_address),
		Some(function_address(other_output))
	);

	// The other plugin's saved stub now skips the stopped watch.
	spew(SpewType::MESSAGE, c"bypassed");

	assert_eq!(SEEN.lock().unwrap().len(), 1);
	assert_eq!(
		printed(),
		[
			"other: chained",
			"engine: chained",
			"other: bypassed",
			"engine: bypassed"
		]
	);
}

extern "C" fn level() -> c_int {
	2
}

#[test]
fn needs_tier0() {
	let scope = ();
	let server = crate::test_support::server::null_server(crate::Game::TeamFortress2, &scope);

	// Tests run without the engine's tier0 loaded.
	assert!(matches!(
		watch(server, record),
		Err(WatchError::Unavailable)
	));
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn one_watch_at_a_time() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);

	// SAFETY: As for `start`.
	let second = unsafe { watch_with(fake_api(), record) };

	assert!(matches!(second, Err(WatchError::AlreadyWatching)));
	assert_eq!(watch.stop(), Stopped::Restored);

	let again = start(record);

	spew(SpewType::LOG, c"again");

	assert_eq!(SEEN.lock().unwrap()[0].0, SpewKind::Log);

	drop(again);

	assert_eq!(
		output_function().map(function_address),
		Some(function_address(engine_output))
	);
}

/// Another plugin's spew function, chained after the watch's.
extern "C" fn other_output(kind: SpewType, message: *const c_char) -> SpewRetval {
	PRINTED
		.lock()
		.unwrap()
		.push(format!("other: {}", text(message)));

	let previous = to_function(OTHER_PREVIOUS.load(Ordering::SeqCst)).expect("it chained");

	// SAFETY: The function it replaced, called as it was called.
	unsafe { previous(kind, message) }
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn output_from_the_callback_passes_straight_on() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record_and_print);

	spew(SpewType::MESSAGE, c"outer");

	let seen = SEEN.lock().unwrap().clone();

	assert_eq!(seen.len(), 1);
	assert_eq!(seen[0].1, "outer");
	assert_eq!(printed(), ["engine: nested", "engine: outer"]);
	assert_eq!(watch.stop(), Stopped::Restored);
}

extern "C" fn output_function() -> Option<SpewOutputFn> {
	to_function(CURRENT.load(Ordering::SeqCst))
}

fn panic_on_output(_: &Spew<'_>) {
	panic!("the callback panics");
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn passes_answers_back() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);

	assert_eq!(spew(SpewType::ASSERT, c"failed"), SpewRetval::DEBUGGER);
	assert_eq!(spew(SpewType::MESSAGE, c"fine"), SpewRetval::CONTINUE);
	assert_eq!(watch.stop(), Stopped::Restored);
}

fn printed() -> Vec<String> {
	PRINTED.lock().unwrap().clone()
}

fn record(spew: &Spew<'_>) {
	SEEN.lock().unwrap().push((
		spew.kind(),
		spew.text().to_string_lossy().into_owned(),
		spew.group().to_string_lossy().into_owned(),
		spew.level(),
		spew.color(),
	));
}

fn record_and_print(spew_seen: &Spew<'_>) {
	record(spew_seen);
	spew(SpewType::MESSAGE, c"nested");
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn sees_output_and_passes_it_on() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);

	assert_ne!(
		output_function().map(function_address),
		Some(function_address(engine_output))
	);
	assert_eq!(spew(SpewType::WARNING, c"hello\n"), SpewRetval::CONTINUE);

	assert_eq!(
		*SEEN.lock().unwrap(),
		[(
			SpewKind::Warning,
			"hello\n".to_owned(),
			"console".to_owned(),
			2,
			Some(COLOR)
		)]
	);

	assert_eq!(printed(), ["engine: hello\n"]);
	assert_eq!(watch.stop(), Stopped::Restored);
	assert_eq!(
		output_function().map(function_address),
		Some(function_address(engine_output))
	);

	spew(SpewType::MESSAGE, c"after");

	assert_eq!(SEEN.lock().unwrap().len(), 1);
	assert_eq!(printed(), ["engine: hello\n", "engine: after"]);
}

extern "C" fn set_output(function: Option<SpewOutputFn>) {
	CURRENT.store(
		function.map_or(ptr::null_mut(), |function| function as *mut c_void),
		Ordering::SeqCst,
	);
}

/// Takes the test lock and resets the fake tier0 to the engine's function.
fn setup(current: Option<SpewOutputFn>) -> MutexGuard<'static, ()> {
	let guard = SERIAL
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

	set_output(current);
	PRINTED.lock().unwrap().clear();
	SEEN.lock().unwrap().clear();
	OTHER_PREVIOUS.store(ptr::null_mut(), Ordering::SeqCst);
	HELD.store(false, Ordering::SeqCst);
	RELEASED.store(false, Ordering::SeqCst);
	guard
}

/// Prints `message` as tier0 would: through its current spew function.
fn spew(kind: SpewType, message: &CStr) -> SpewRetval {
	let function = output_function().unwrap_or(default_output);

	// SAFETY: A spew function, given a C string.
	unsafe { function(kind, message.as_ptr()) }
}

fn start(callback: SpewFn) -> SpewWatch {
	// SAFETY: The fake tier0's functions are statics of this module, callable
	// from any thread, and the tests change the spew function on one thread
	// at a time.
	unsafe { watch_with(fake_api(), callback) }.expect("the watch starts")
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot run machine code")]
fn stopping_waits_for_warning_filters_in_flight() {
	let _serial = setup(Some(engine_output));
	let mut watch = start(record);
	watch.filter_warnings(Some(|spew| {
		hold(spew);
		true
	}));
	let thread = std::thread::spawn(|| spew(SpewType::WARNING, c"held\n"));

	while !HELD.load(Ordering::SeqCst) {
		std::thread::yield_now();
	}

	assert_eq!(watch.stop(), Stopped::Busy);
	assert_eq!(
		output_function().map(function_address),
		Some(function_address(engine_output))
	);
	RELEASED.store(true, Ordering::SeqCst);
	assert_eq!(thread.join().unwrap(), SpewRetval::CONTINUE);
	assert_eq!(watch.stop(), Stopped::Restored);
	assert_eq!(printed(), ["engine: held\n"]);
}

fn text(message: *const c_char) -> String {
	// SAFETY: The tests spew C strings.
	unsafe { CStr::from_ptr(message) }
		.to_string_lossy()
		.into_owned()
}

fn to_function(address: *mut c_void) -> Option<SpewOutputFn> {
	// SAFETY: Only spew functions are stored.
	(!address.is_null())
		.then(|| unsafe { std::mem::transmute::<*mut c_void, SpewOutputFn>(address) })
}
