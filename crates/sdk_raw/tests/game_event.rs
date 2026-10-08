//! Tests of the game event listener and visitor objects Rust implements for
//! the engine.

use source_sdk_2013_raw::abi::WChar;
use source_sdk_2013_raw::interfaces::game_event::{
	EventValue, GameEventListenerObject, OnFireGameEvent, for_event_data,
};
use source_sdk_2013_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_int};
use std::ptr::{NonNull, null, null_mut};

/// The address `VisitLocal` passes.
static LOCAL: u8 = 0;

/// A key's value, copied out of the visit.
#[derive(Debug, PartialEq)]
enum Copied {
	Local(Option<usize>),
	String(Option<CString>),
	Float(f32),
	Int(c_int),
	UInt64(u64),
	WString(Option<Vec<WChar>>),
	Bool(bool),
}

impl From<EventValue<'_>> for Copied {
	fn from(value: EventValue<'_>) -> Self {
		match value {
			EventValue::Local(local) => Self::Local(local.map(NonNull::addr).map(Into::into)),
			EventValue::String(string) => Self::String(string.map(CStr::to_owned)),
			EventValue::Float(float) => Self::Float(float),
			EventValue::Int(int) => Self::Int(int),
			EventValue::UInt64(uint) => Self::UInt64(uint),
			EventValue::WString(wide) => Self::WString(wide.map(<[WChar]>::to_vec)),
			EventValue::Bool(bool) => Self::Bool(bool),
		}
	}
}

/// Records the events it is fired.
#[derive(Debug, Default)]
struct Recorder(RefCell<Vec<*mut sys::IGameEvent>>);

impl OnFireGameEvent for Recorder {
	unsafe fn fire_game_event(&self, event: NonNull<sys::IGameEvent>) {
		self.0.borrow_mut().push(event.as_ptr());
	}
}

#[test]
fn event_data_is_visited_until_the_callback_stops() {
	// SAFETY: The vtable holds only function pointers, and `unexpected_call`
	// aborts the test if any slot but `ForEventData` is called.
	let vtable = unsafe {
		mock_vtable::<sys::IGameEvent__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IGameEvent_ForEventData).write(for_event_data_mock);
		})
	};
	let mut event = sys::IGameEvent {
		vtable_: &raw const *vtable,
	};
	let event = NonNull::from(&mut event);
	let visit = |stop: &CStr| {
		let mut visited = Vec::new();

		// SAFETY: The mock event lives for the call.
		let finished = unsafe {
			for_event_data(event, &mut |name, value| {
				visited.push((name.to_owned(), Copied::from(value)));
				name != stop
			})
		};

		(finished, visited)
	};

	let (finished, visited) = visit(c"");

	assert!(finished);
	assert_eq!(
		visited,
		[
			(
				c"local".to_owned(),
				Copied::Local(Some((&raw const LOCAL).addr()))
			),
			(c"no_local".to_owned(), Copied::Local(None)),
			(
				c"string".to_owned(),
				Copied::String(Some(c"scout".to_owned()))
			),
			(c"no_string".to_owned(), Copied::String(None)),
			(c"float".to_owned(), Copied::Float(2.5)),
			(c"int".to_owned(), Copied::Int(-7)),
			(c"uint64".to_owned(), Copied::UInt64(u64::MAX)),
			(
				c"wstring".to_owned(),
				Copied::WString(Some(vec![b'h'.into(), b'i'.into()]))
			),
			(c"no_wstring".to_owned(), Copied::WString(None)),
			(c"bool".to_owned(), Copied::Bool(true)),
		]
	);

	let (finished, visited) = visit(c"float");

	assert!(!finished);
	assert_eq!(
		visited.last(),
		Some(&(c"float".to_owned(), Copied::Float(2.5)))
	);
	assert_eq!(visited.len(), 5);
}

/// Visits keys of each type, with null and non-null pointers, in order, as
/// the engine does: until the visitor returns `false`.
unsafe extern "C" fn for_event_data_mock(
	_: *const sys::IGameEvent,
	visitor: *mut sys::IGameEventVisitor2,
) -> bool {
	let wide: [WChar; 3] = [b'h'.into(), b'i'.into(), 0];

	// SAFETY: `for_event_data` passes its live visitor.
	unsafe {
		let vtable = (*visitor).vtable_;

		((*vtable).IGameEventVisitor2_VisitLocal)(
			visitor,
			c"local".as_ptr(),
			(&raw const LOCAL).cast(),
		) && ((*vtable).IGameEventVisitor2_VisitLocal)(visitor, c"no_local".as_ptr(), null())
			&& ((*vtable).IGameEventVisitor2_VisitString)(
				visitor,
				c"string".as_ptr(),
				c"scout".as_ptr(),
			)
			&& ((*vtable).IGameEventVisitor2_VisitString)(visitor, c"no_string".as_ptr(), null())
			&& ((*vtable).IGameEventVisitor2_VisitFloat)(visitor, c"float".as_ptr(), 2.5)
			&& ((*vtable).IGameEventVisitor2_VisitInt)(visitor, c"int".as_ptr(), -7)
			&& ((*vtable).IGameEventVisitor2_VisitUint64)(visitor, c"uint64".as_ptr(), u64::MAX)
			&& ((*vtable).IGameEventVisitor2_VisitWString)(
				visitor,
				c"wstring".as_ptr(),
				wide.as_ptr(),
			)
			&& ((*vtable).IGameEventVisitor2_VisitWString)(visitor, c"no_wstring".as_ptr(), null())
			&& ((*vtable).IGameEventVisitor2_VisitBool)(visitor, c"bool".as_ptr(), true)
	}
}

#[test]
fn listeners_pass_fired_events_and_outlive_their_destructors() {
	let listener = GameEventListenerObject::new(Recorder::default());
	let raw = listener.as_raw();
	let mut event = sys::IGameEvent { vtable_: null() };
	let event = &raw mut event;

	// SAFETY: `raw` is the live listener, called through the bindings'
	// vtable as the manager calls it. The recorder never reads the event.
	unsafe {
		let vtable = (*raw).vtable_;

		((*vtable).IGameEventListener2_FireGameEvent)(raw, event);
		((*vtable).IGameEventListener2_FireGameEvent)(raw, null_mut());

		cfg_select! {
			target_os = "windows" => {
				assert_eq!(
					((*vtable).IGameEventListener2_destructor)(raw, 1),
					raw.cast()
				);
			}

			target_os = "linux" => {
				((*vtable).IGameEventListener2_complete_destructor)(raw);
				((*vtable).IGameEventListener2_deleting_destructor)(raw);
			}
		}

		((*vtable).IGameEventListener2_FireGameEvent)(raw, event);
	}

	assert_eq!(*listener.inner().0.borrow(), [event, event]);
}
