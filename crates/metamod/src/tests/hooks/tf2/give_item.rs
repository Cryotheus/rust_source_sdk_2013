//! Tests of `crate::hooks::tf2::give_item`: hooks refusing the items given to
//! players, on a mock player class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::mem::MaybeUninit;
use std::ptr::NonNull;

thread_local! {
	/// What [`on_give_item`] decides.
	static ACTION: Cell<GiveItemAction> = const { Cell::new(GiveItemAction::Continue) };

	/// What ran during the calls since the last check, in order, with the
	/// item it was asked for.
	static CALLS: RefCell<Vec<(&'static str, Seen)>> = const { RefCell::new(Vec::new()) };
}

/// The item the game creates.
const ITEM: usize = 0x5e7;

/// An item as a call saw it: its classname, definition index, subtype and
/// force flag.
type Seen = (CString, Option<u16>, c_int, bool);

/// The game's `GiveNamedItem`, which notes that it ran, and creates [`ITEM`].
unsafe extern "C" fn game_give_named_item(
	_: *mut sys::CBaseEntity,
	name: *const c_char,
	subtype: c_int,
	item: *const sys::CEconItemView,
	force: bool,
) -> *mut sys::CBaseEntity {
	// SAFETY: The tests pass a classname, and an item or none.
	let (name, item) = unsafe { (CStr::from_ptr(name), item.as_ref()) };
	let definition = item.map(|item| item.m_iItemDefinitionIndex.m_Value);

	CALLS.with_borrow_mut(|calls| {
		calls.push(("game", (name.to_owned(), definition, subtype, force)));
	});

	ptr::without_provenance_mut(ITEM)
}

#[test]
fn items_can_be_refused_before_the_game_creates_them() {
	/// A player of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_give_named_item as GiveNamedItem as *mut c_void;
			GIVE_NAMED_ITEM_SLOT + 1
		]);
		let mut player = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut player).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<TfPlayer>::from_raw(NonNull::new(player.vtable).unwrap()) };
		let hooks = api.hook_give_items(tf2_binding(no_interfaces), on_give_item);

		// SAFETY: All-zero bytes are a value of each field of an item view.
		let mut item: sys::CEconItemView = unsafe { MaybeUninit::zeroed().assume_init() };
		item.m_iItemDefinitionIndex.m_Value = 205;

		let give = |name: &CStr, item: *const sys::CEconItemView| {
			CALLS.take();
			let given = harness.call::<GiveNamedItem>(
				this,
				GIVE_NAMED_ITEM_SLOT,
				(name.as_ptr(), 2, item, true),
			);
			(given.addr(), CALLS.take())
		};

		assert_eq!(hooks.cover(api, target), Ok(true));

		let launcher = (c"tf_weapon_rocketlauncher".to_owned(), Some(205), 2, true);

		ACTION.set(GiveItemAction::Continue);
		assert_eq!(
			give(c"tf_weapon_rocketlauncher", &raw const item),
			(
				ITEM,
				vec![("hook", launcher.clone()), ("game", launcher.clone())]
			)
		);

		// The stock item has no definition.
		let shotgun = (c"tf_weapon_shotgun".to_owned(), None, 2, true);

		ACTION.set(GiveItemAction::Refuse);
		assert_eq!(
			give(c"tf_weapon_shotgun", ptr::null()),
			(0, vec![("hook", shotgun)])
		);
	});
}

/// The callback, which notes the item and decides [`ACTION`].
fn on_give_item(_server: Server<'_>, _player: Entity<'_>, item: &GiveItem<'_>) -> GiveItemAction {
	let seen = (
		item.class_name.to_owned(),
		item.definition.map(ItemDefinitionIndex::get),
		item.subtype,
		item.force,
	);

	CALLS.with_borrow_mut(|calls| calls.push(("hook", seen)));
	ACTION.get()
}
