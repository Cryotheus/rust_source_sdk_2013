//! Tests of `crate::hooks::sent_message`: hooks of `UserMessageBegin`,
//! `EntityMessageBegin`, `MessageEnd` and `ClientPrintf` on a mock engine that
//! sends messages from buffers of its own, as TF2's does, through the mock
//! SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::ServerGameDll;
use source_sdk_2013::raw::test_support::edicts::mock_edict;
use source_sdk_2013::raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_binding};
use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::ptr;

thread_local! {
	/// The mock engine's buffer for entity messages.
	static ENTITY_BUFFER: Cell<*mut BfWrite> = const { Cell::new(ptr::null_mut()) };

	/// Whether the mock game DLL is exported on this thread.
	static GAME_DLL_EXPORTED: Cell<bool> = const { Cell::new(false) };

	/// The buffer of the message the mock engine has open, or null.
	static OPEN: Cell<*mut BfWrite> = const { Cell::new(ptr::null_mut()) };

	/// What happened since the last call of the tests' senders, in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// The mock engine's buffer for user messages.
	static USER_BUFFER: Cell<*mut BfWrite> = const { Cell::new(ptr::null_mut()) };
}

/// The type the mock game registered `HintText` at.
const HINT_TEXT: c_int = 6;

/// `HUD_PRINTTALK`, the destination of the tests' `TextMsg`s.
const HUD_PRINTTALK: u8 = 3;

/// Both listeners of the tests.
const LISTENERS: SentMessageListeners = SentMessageListeners {
	user_message: Some(on_user_message),
	client_print: Some(on_client_print),
};

/// The type the mock game registered `TextMsg` at.
const TEXT_MSG: c_int = 5;

/// A type the mock game registered no message at.
const UNREGISTERED: c_int = 60;

/// A recipient filter of the game's, which lists its clients.
#[repr(C)]
struct Filter {
	base: sys::IRecipientFilter,
	clients: Vec<c_int>,
	reliable: bool,
	init: bool,
}

impl Filter {
	fn new(clients: Vec<c_int>, reliable: bool, init: bool) -> Box<Self> {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IRecipientFilter__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IRecipientFilter_IsReliable).write(filter_reliable);
					(&raw mut (*vtable).IRecipientFilter_IsInitMessage).write(filter_init);
					(&raw mut (*vtable).IRecipientFilter_GetRecipientCount).write(filter_count);
					(&raw mut (*vtable).IRecipientFilter_GetRecipientIndex).write(filter_index);
				},
			)
		};

		Box::new(Self {
			base: sys::IRecipientFilter {
				vtable_: Box::leak(vtable),
			},
			clients,
			reliable,
			init,
		})
	}

	fn ptr(&self) -> *mut sys::IRecipientFilter {
		ptr::from_ref(self).cast_mut().cast()
	}
}

/// What a user message listener saw of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
	id: c_int,
	name: Option<CString>,
	recipients: Vec<c_int>,
	reliable: bool,
	init: bool,
	bytes: Vec<u8>,
	bits: usize,

	/// The destination and text of a `TextMsg`, as the message's reader reads
	/// them.
	read: Option<(u8, CString)>,
}

/// Something the engine or a listener noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
	/// The client print listener saw text to the client of the edict index.
	ClientPrint(c_int, CString),

	/// The engine dropped a message whose payload overflowed.
	Dropped,

	/// The engine printed text to the client of the edict index.
	Printed(c_int, CString),

	/// The engine sent a message with this payload.
	Sent(Vec<u8>),

	/// The user message listener saw a message.
	UserMessage(Observed),
}

/// Opens a message in `buffer`, emptied, as the engine does, and returns the
/// buffer for the game to write to.
fn begin(buffer: *mut BfWrite) -> *mut sys::bf_write {
	expect(
		OPEN.get().is_null(),
		"a message began while another was open",
	);

	// SAFETY: The mock engine's buffers are leaked.
	unsafe {
		(*buffer).cur_bit = 0;
		(*buffer).overflow = 0;
	}

	OPEN.set(buffer);
	buffer.cast()
}

/// A binding to a game server exporting the mock game DLL, whose registry has
/// `TextMsg` and `HintText`.
fn binding() -> ServerBinding {
	if !GAME_DLL_EXPORTED.replace(true) {
		// SAFETY: As for `Filter::new`.
		let vtable = unsafe {
			mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerGameDLL_GetUserMessageInfo)
						.write(game_user_message_info);
				},
			)
		};

		export(
			Module::GameServer,
			ServerGameDll::VERSION,
			leak(sys::IServerGameDLL {
				vtable_: Box::leak(vtable),
			}),
		);
	}

	mock_binding()
}

/// A leaked, empty buffer of 256 bytes, room for the 255 bytes a user
/// message's payload can have at most.
fn buffer() -> *mut BfWrite {
	leak(BfWrite::empty(Vec::leak(vec![0; 64])))
}

#[test]
fn client_prints_reach_the_listener_before_the_engine_prints_them() {
	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let client = leak(mock_edict(2, false));
		let hooks = install(api, engine, LISTENERS).unwrap();

		assert_eq!(
			print(harness, engine, client, c"You are on RED.\n"),
			[
				Seen::ClientPrint(2, c"You are on RED.\n".to_owned()),
				Seen::Printed(2, c"You are on RED.\n".to_owned()),
			]
		);

		// The text is passed as is, not as a format.
		assert_eq!(
			print(harness, engine, client, c"100%s"),
			[
				Seen::ClientPrint(2, c"100%s".to_owned()),
				Seen::Printed(2, c"100%s".to_owned()),
			]
		);

		hooks.remove(api);
	});
}

/// The mock engine's `ClientPrintf`, which notes the text.
unsafe extern "C" fn engine_client_printf(
	_this: *mut sys::IVEngineServer,
	client: *mut sys::edict_t,
	message: *const c_char,
) {
	// SAFETY: The tests pass leaked edicts and terminated text.
	let (index, message) = unsafe {
		(
			(*client)._base.m_EdictIndex,
			CStr::from_ptr(message).to_owned(),
		)
	};

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Printed(index.into(), message)));
}

/// The mock engine's `EntityMessageBegin`, which opens a message in its
/// buffer for entity messages.
unsafe extern "C" fn engine_entity_message_begin(
	_this: *mut sys::IVEngineServer,
	_entity_index: c_int,
	_class: *mut sys::ServerClass,
	_reliable: bool,
) -> *mut sys::bf_write {
	begin(ENTITY_BUFFER.get())
}

/// The mock engine's `MessageEnd`, which sends the open message, or drops it
/// if its payload overflowed.
unsafe extern "C" fn engine_message_end(_this: *mut sys::IVEngineServer) {
	let Some(buffer) = NonNull::new(OPEN.replace(ptr::null_mut())) else {
		expect(false, "a message ended without beginning");
		return;
	};

	// SAFETY: The mock engine's buffers are leaked, word-aligned storage.
	let seen = match unsafe { BfWrite::read_back(buffer) } {
		Some(bits) => Seen::Sent(BitWriter::from(bits).to_bytes()),
		None => Seen::Dropped,
	};

	SEEN.with_borrow_mut(|seen_so_far| seen_so_far.push(seen));
}

/// The mock engine's `UserMessageBegin`, which opens a message in its buffer
/// for user messages.
unsafe extern "C" fn engine_user_message_begin(
	_this: *mut sys::IVEngineServer,
	_filter: *mut sys::IRecipientFilter,
	_message_type: c_int,
) -> *mut sys::bf_write {
	begin(USER_BUFFER.get())
}

/// The mock filter's `GetRecipientCount`.
unsafe extern "C" fn filter_count(this: *const sys::IRecipientFilter) -> c_int {
	// SAFETY: Only mock filters have this vtable.
	unsafe { (*this.cast::<Filter>()).clients.len() as c_int }
}

/// The mock filter's `GetRecipientIndex`.
unsafe extern "C" fn filter_index(this: *const sys::IRecipientFilter, slot: c_int) -> c_int {
	// SAFETY: As above. The hooks ask for slots below the count.
	unsafe { (&(*this.cast::<Filter>()).clients)[slot as usize] }
}

/// The mock filter's `IsInitMessage`.
unsafe extern "C" fn filter_init(this: *const sys::IRecipientFilter) -> bool {
	// SAFETY: As above.
	unsafe { (*this.cast::<Filter>()).init }
}

/// The mock filter's `IsReliable`.
unsafe extern "C" fn filter_reliable(this: *const sys::IRecipientFilter) -> bool {
	// SAFETY: As above.
	unsafe { (*this.cast::<Filter>()).reliable }
}

/// The mock game DLL's `GetUserMessageInfo`, which knows `TextMsg` and
/// `HintText`, both of varying sizes.
unsafe extern "C" fn game_user_message_info(
	_this: *mut sys::IServerGameDLL,
	message_type: c_int,
	name: *mut c_char,
	capacity: c_int,
	size: *mut c_int,
) -> bool {
	let registered = match message_type {
		HINT_TEXT => c"HintText",
		TEXT_MSG => c"TextMsg",
		_ => return false,
	}
	.to_bytes_with_nul();

	if usize::try_from(capacity).is_ok_and(|capacity| registered.len() <= capacity) {
		// SAFETY: The wrapper passes a buffer of `capacity` bytes, and the size
		// to write.
		unsafe {
			ptr::copy_nonoverlapping(registered.as_ptr().cast(), name, registered.len());
			size.write(-1);
		}

		true
	} else {
		expect(false, "the name fits the wrapper's buffer");
		false
	}
}

/// Installs the hooks of `listeners` on a mock engine of [`mock_engine`]'s,
/// for a server whose game DLL [`binding`] exports.
fn install(
	api: MetamodApi<'_>,
	engine: NonNull<sys::IVEngineServer>,
	listeners: SentMessageListeners,
) -> Result<SentMessageHooks, HookError> {
	// SAFETY: The mock engine has the four functions at their slots, and is
	// leaked with its vtable.
	unsafe { api.install_sent_messages(engine, binding(), listeners) }
}

#[test]
fn messages_past_the_hooks_leave_no_message_open() {
	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let players = Filter::new(vec![1], true, false);
		let hooks = install(api, engine, LISTENERS).unwrap();

		// A user message whose end skips the hooks, as a plugin calling the engine
		// past them sends it, reaches no listener.
		SEEN.take();

		let buffer = harness.call::<UserMessageBegin>(
			engine.as_ptr(),
			USER_MESSAGE_BEGIN_SLOT,
			(players.ptr(), TEXT_MSG),
		);

		write(buffer, &text_msg(c"unseen"));

		// SAFETY: The mock engine's own function, called on it.
		unsafe { engine_message_end(engine.as_ptr()) };

		assert_eq!(SEEN.take(), [sent(c"unseen")]);

		// Nor does it reach one as the next message, an entity message, ends.
		assert_eq!(
			send_entity_message(harness, engine, &text_msg(c"entity")),
			[sent(c"entity")]
		);

		// A user message whose beginning skips the hooks reaches no listener as
		// it ends.
		// SAFETY: As above.
		let buffer = unsafe { engine_user_message_begin(engine.as_ptr(), players.ptr(), TEXT_MSG) };

		write(buffer, &text_msg(c"unseen"));
		harness.call::<MessageEnd>(engine.as_ptr(), MESSAGE_END_SLOT, ());

		assert_eq!(SEEN.take(), [sent(c"unseen")]);

		hooks.remove(api);
	});
}

/// A mock engine, which sends messages from buffers of its own, emptied as
/// each message begins.
fn mock_engine() -> NonNull<sys::IVEngineServer> {
	USER_BUFFER.set(buffer());
	ENTITY_BUFFER.set(buffer());
	OPEN.set(ptr::null_mut());

	// SAFETY: As for `Filter::new`.
	let vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_ClientPrintf).write(engine_client_printf);
			(&raw mut (*vtable).IVEngineServer_EntityMessageBegin)
				.write(engine_entity_message_begin);
			(&raw mut (*vtable).IVEngineServer_MessageEnd).write(engine_message_end);
			(&raw mut (*vtable).IVEngineServer_UserMessageBegin).write(engine_user_message_begin);
		})
	};

	NonNull::new(leak(sys::IVEngineServer {
		vtable_: Box::leak(vtable),
	}))
	.unwrap()
}

/// What the user message listener sees of a `TextMsg` of type `id`, named
/// `name`, to `filter`'s clients, with `text`.
fn observed(id: c_int, name: Option<&CStr>, filter: &Filter, text: &CStr) -> Seen {
	let payload = text_msg(text);

	Seen::UserMessage(Observed {
		id,
		name: name.map(CStr::to_owned),
		recipients: filter.clients.clone(),
		reliable: filter.reliable,
		init: filter.init,
		bytes: payload.to_bytes(),
		bits: payload.len(),
		read: Some((HUD_PRINTTALK, text.to_owned())),
	})
}

/// Notes the client and the text, and panics for the text `panic`.
fn on_client_print(_server: Server<'_>, client: Edict<'_>, text: &CStr) {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::ClientPrint(client.index(), text.to_owned())));

	if text == c"panic" {
		panic!("the listener panicked, as this test means it to");
	}
}

/// Notes the message, and panics for a `TextMsg` with the text `panic`.
fn on_user_message(_server: Server<'_>, message: &SentUserMessage<'_>) {
	let mut reader = message.reader();
	let read = (|| Some((reader.read_u8().ok()?, reader.read_cstring().ok()?)))();

	expect(
		message.data().to_bytes() == message.bytes() && message.data().len() == message.bit_len(),
		"the payload's views agree",
	);

	SEEN.with_borrow_mut(|seen| {
		seen.push(Seen::UserMessage(Observed {
			id: message.id(),
			name: message.name().map(CStr::to_owned),
			recipients: message.recipients().to_vec(),
			reliable: message.is_reliable(),
			init: message.is_init_message(),
			bytes: message.bytes().to_vec(),
			bits: message.bit_len(),
			read: read.clone(),
		}));
	});

	if read.is_some_and(|(_, text)| text.as_c_str() == c"panic") {
		panic!("the listener panicked, as this test means it to");
	}
}

#[test]
fn panicking_listeners_leave_the_messages_and_later_ones_alone() {
	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let players = Filter::new(vec![1, 2], false, false);
		let client = leak(mock_edict(1, false));
		let hooks = install(api, engine, LISTENERS).unwrap();

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &text_msg(c"panic")),
			[
				observed(TEXT_MSG, Some(c"TextMsg"), &players, c"panic"),
				sent(c"panic"),
			]
		);

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &text_msg(c"calm")),
			[
				observed(TEXT_MSG, Some(c"TextMsg"), &players, c"calm"),
				sent(c"calm"),
			]
		);

		assert_eq!(
			print(harness, engine, client, c"panic"),
			[
				Seen::ClientPrint(1, c"panic".to_owned()),
				Seen::Printed(1, c"panic".to_owned()),
			]
		);

		assert_eq!(
			print(harness, engine, client, c"calm"),
			[
				Seen::ClientPrint(1, c"calm".to_owned()),
				Seen::Printed(1, c"calm".to_owned()),
			]
		);

		hooks.remove(api);
	});
}

/// Has the game print `text` to the client of `client` through the engine's
/// hooked vtable, and returns what happened.
fn print(
	harness: &Harness,
	engine: NonNull<sys::IVEngineServer>,
	client: *mut sys::edict_t,
	text: &CStr,
) -> Vec<Seen> {
	SEEN.take();
	harness.call::<ClientPrintf>(engine.as_ptr(), CLIENT_PRINTF_SLOT, (client, text.as_ptr()));
	SEEN.take()
}

#[test]
fn removed_hooks_pass_nothing_on_and_can_be_replaced() {
	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let players = Filter::new(vec![3], true, false);
		let client = leak(mock_edict(3, false));
		let hooks = install(api, engine, LISTENERS).unwrap();

		hooks.remove(api);

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &text_msg(c"gone")),
			[sent(c"gone")]
		);

		assert_eq!(
			print(harness, engine, client, c"gone"),
			[Seen::Printed(3, c"gone".to_owned())]
		);

		// Only the hooks of the listeners given are installed.
		let prints_only = SentMessageListeners {
			user_message: None,
			client_print: Some(on_client_print),
		};

		let replaced = install(api, engine, prints_only).unwrap();

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &text_msg(c"unheard")),
			[sent(c"unheard")]
		);

		// Removing the old hooks again leaves the new ones.
		hooks.remove(api);

		assert_eq!(
			print(harness, engine, client, c"back"),
			[
				Seen::ClientPrint(3, c"back".to_owned()),
				Seen::Printed(3, c"back".to_owned()),
			]
		);

		replaced.remove(api);
	});
}

/// Has the game send an entity message with `payload` through the engine's
/// hooked vtable, and returns what happened.
fn send_entity_message(
	harness: &Harness,
	engine: NonNull<sys::IVEngineServer>,
	payload: &BitWriter,
) -> Vec<Seen> {
	SEEN.take();

	let buffer = harness.call::<EntityMessageBegin>(
		engine.as_ptr(),
		ENTITY_MESSAGE_BEGIN_SLOT,
		(7, ptr::null_mut(), true),
	);

	write(buffer, payload);
	harness.call::<MessageEnd>(engine.as_ptr(), MESSAGE_END_SLOT, ());
	SEEN.take()
}

/// Has the game send a user message of type `id` to `filter`'s clients with
/// `payload` through the engine's hooked vtable, and returns what happened.
fn send_user_message(
	harness: &Harness,
	engine: NonNull<sys::IVEngineServer>,
	filter: &Filter,
	id: c_int,
	payload: &BitWriter,
) -> Vec<Seen> {
	SEEN.take();

	let buffer = harness.call::<UserMessageBegin>(
		engine.as_ptr(),
		USER_MESSAGE_BEGIN_SLOT,
		(filter.ptr(), id),
	);

	write(buffer, payload);
	harness.call::<MessageEnd>(engine.as_ptr(), MESSAGE_END_SLOT, ());
	SEEN.take()
}

/// What the engine sends of a `TextMsg` with `text`.
fn sent(text: &CStr) -> Seen {
	Seen::Sent(text_msg(text).to_bytes())
}

/// The payload of a `TextMsg` with `text` to the chat, as the game writes it.
fn text_msg(text: &CStr) -> BitWriter {
	let mut payload = BitWriter::new();

	payload.write_u8(HUD_PRINTTALK);
	payload.write_cstr(text);
	payload
}

#[test]
fn the_buffer_another_hook_supplies_is_the_one_read() {
	thread_local! {
		/// The buffer the other hook supplies, as SourceMod does for messages it
		/// intercepts.
		static INTERCEPT: Cell<*mut BfWrite> = const { Cell::new(ptr::null_mut()) };
	}

	/// Supplies the other hook's buffer, emptied, in place of the engine's.
	fn intercept(_call: &HookCall<'_, UserMessageBegin>) -> HookAction<*mut sys::bf_write> {
		let buffer = INTERCEPT.get();

		// SAFETY: The test leaks the buffer.
		unsafe { (*buffer).cur_bit = 0 };

		HookAction::Supersede(buffer.cast())
	}

	/// Keeps the engine from ending a message it never began.
	fn supersede_end(_call: &HookCall<'_, MessageEnd>) -> HookAction<()> {
		HookAction::Supersede(())
	}

	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let target = HookTarget::instance(engine);
		let players = Filter::new(vec![1, 2], true, false);

		INTERCEPT.set(buffer());

		// SAFETY: The mock engine has both functions at their slots, and is
		// leaked with its vtable. The other hook's handlers run first, as they
		// are added first.
		unsafe {
			api.add_hook(USER_MESSAGE_BEGIN, target, HookTiming::Pre, &intercept)
				.unwrap();
			api.add_hook(MESSAGE_END, target, HookTiming::Pre, &supersede_end)
				.unwrap();
		}

		let hooks = install(api, engine, LISTENERS).unwrap();
		let intercepted = text_msg(c"intercepted");

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &intercepted),
			[observed(
				TEXT_MSG,
				Some(c"TextMsg"),
				&players,
				c"intercepted"
			)]
		);

		hooks.remove(api);
	});
}

#[test]
fn user_messages_reach_the_listener_before_the_engine_sends_them() {
	on_both(|harness| {
		let api = harness.api();
		let engine = mock_engine();
		let players = Filter::new(vec![1, 3], true, false);
		let hooks = install(api, engine, LISTENERS).unwrap();

		assert!(matches!(
			install(api, engine, LISTENERS),
			Err(HookError::AlreadyInstalled)
		));

		let welcome = text_msg(c"#TF_Welcome");

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &welcome),
			[
				observed(TEXT_MSG, Some(c"TextMsg"), &players, c"#TF_Welcome"),
				sent(c"#TF_Welcome"),
			]
		);

		// A message of a type the game's registry does not know has no name, and
		// an init message to no one still reaches the listener.
		let signon = Filter::new(vec![], false, true);

		assert_eq!(
			send_user_message(harness, engine, &signon, UNREGISTERED, &text_msg(c"init")),
			[
				observed(UNREGISTERED, None, &signon, c"init"),
				sent(c"init"),
			]
		);

		// An empty payload is passed on as such.
		let bots = Filter::new(vec![2, 4, 5], false, false);

		assert_eq!(
			send_user_message(harness, engine, &bots, HINT_TEXT, &BitWriter::new()),
			[
				Seen::UserMessage(Observed {
					id: HINT_TEXT,
					name: Some(c"HintText".to_owned()),
					recipients: vec![2, 4, 5],
					reliable: false,
					init: false,
					bytes: vec![],
					bits: 0,
					read: None,
				}),
				Seen::Sent(vec![]),
			]
		);

		// Entity messages reach no listener.
		assert_eq!(
			send_entity_message(harness, engine, &text_msg(c"entity")),
			[sent(c"entity")]
		);

		// A payload that overflowed the buffer, which the engine drops, reaches no
		// listener.
		let mut overflowing = BitWriter::new();

		overflowing.write_bytes(&[b'x'; 300]);

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &overflowing),
			[Seen::Dropped]
		);

		assert_eq!(
			send_user_message(harness, engine, &players, TEXT_MSG, &text_msg(c"after")),
			[
				observed(TEXT_MSG, Some(c"TextMsg"), &players, c"after"),
				sent(c"after"),
			]
		);

		hooks.remove(api);
	});
}

/// Writes `payload` to the buffer the game got, as the game does.
fn write(buffer: *mut sys::bf_write, payload: &BitWriter) {
	let buffer = NonNull::new(buffer).expect("the message began");

	// SAFETY: The tests' buffers are leaked, word-aligned storage, which only
	// the game writes to while the message is open.
	unsafe { BfWrite::append(BfWrite::from_sys(buffer), payload.as_words(), payload.len()) };
}
