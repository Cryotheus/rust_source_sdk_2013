//! TF2 player damage hooks with owned, editable damage arguments.
//!
//! Install for each distinct player class (for example on player activation;
//! bots can have a different vtable). Hooks cover that class, including
//! subsequently connected players of the same class, until
//! removed or the plugin unloads. As with other Metamod hooks, they stop
//! calling handlers while the plugin is paused.

use crate::MetamodApi;
use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};
use source_sdk_2013::damage::DamageEvent;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_int;
use std::ptr::NonNull;

type TakeDamage = unsafe extern "C" fn(*mut sys::CBaseEntity, *const sys::CTakeDamageInfo) -> c_int;

/// Where in TF2's damage processing a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageStage {
	/// `CTFPlayer::OnTakeDamage`, before TF2 applies damage rules. Changes to
	/// amount, type and incoming critical classification flow through those
	/// rules, and can affect the rest of the damage and death processing.
	Incoming,
	/// `CTFPlayer::OnTakeDamage_Alive`, after full and mini critical bonuses
	/// were calculated, before health loss and resistance processing. Use
	/// `DamageInfo::apply_critical_policy` here. Earlier visuals and assist
	/// statistics, and the caller's later death processing, are not rewritten.
	Alive,
}

impl DamageStage {
	/// TF2's slots from SourceMod's
	/// `gamedata/sdkhooks.games/engine.ep2v.txt`, the `tf` section.
	const fn function(self) -> VirtualFunction<TakeDamage> {
		let windows_slot = match self {
			Self::Incoming => 64,
			Self::Alive => 283,
		};
		VirtualFunction::new(windows_slot + if cfg!(target_os = "linux") { 1 } else { 0 })
	}
}

/// What to do with the original call after inspecting or editing damage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DamageAction {
	/// Continue normally. Any edits to the owned copy are discarded.
	#[default]
	Continue,
	/// Call the original virtual method with the edited copy and supersede
	/// this invocation. This bypasses other plugins' subsequent hooks on
	/// this method; nested calls (including the Alive stage) run normally.
	/// Other hooks observing the original invocation still see its original
	/// arguments. No plugin's const argument storage is overwritten.
	Apply,
	/// Skip damage and return zero without invoking the original method.
	Block,
}

/// A callback-scoped server and victim with an independently owned record.
/// A panic is contained by the hook dispatcher and lets the game continue
/// with the original damage arguments.
pub type DamageFn = for<'s> fn(Server<'s>, DamageStage, &mut DamageEvent<'s>) -> DamageAction;

#[derive(Debug, thiserror::Error)]
pub enum DamageHookError {
	#[error("damage hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,
	#[error(transparent)]
	Hook(#[from] HookError),
}

#[derive(Clone, Copy)]
struct RoutedDamage {
	binding: ServerBinding,
	callback: DamageFn,
	original: TakeDamage,
	hook: HookId,
	stage: DamageStage,
	vtable: usize,
}

struct DamageRoute {
	state: Cell<Option<RoutedDamage>>,
}

impl DamageRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DamageRoute {}

static ROUTES: [DamageRoute; 32] = [const { DamageRoute::new() }; 32];

impl Handler<TakeDamage> for DamageRoute {
	fn call(&self, call: &HookCall<'_, TakeDamage>) -> HookAction<c_int> {
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (info,) = call.args();
		let (Some(victim), Some(info)) = (NonNull::new(call.this()), NonNull::new(info.cast_mut()))
		else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies a live player, a const damage
		// reference and the matching original virtual method.
		unsafe {
			dispatch(
				server,
				route.stage,
				route.original,
				route.callback,
				victim,
				info,
			)
		}
	}
}

/// # Safety
/// The pointers must be the live arguments of the selected native damage
/// method, and `original` must be its unhooked function of this signature.
unsafe fn dispatch(
	server: Server<'_>,
	stage: DamageStage,
	original: TakeDamage,
	callback: DamageFn,
	victim: NonNull<sys::CBaseEntity>,
	info: NonNull<sys::CTakeDamageInfo>,
) -> HookAction<c_int> {
	// SAFETY: The caller supplies live callback-scoped arguments. Only
	// byte-copying reads the const damage object; all modifications are local.
	let mut event = unsafe { DamageEvent::from_raw(server, victim, info) };
	match callback(server, stage, &mut event) {
		DamageAction::Continue => HookAction::Ignore,
		DamageAction::Block => HookAction::Supersede(0),
		DamageAction::Apply => {
			// SAFETY: Original matches the target method's ABI. Player lifetime
			// is callback-scoped, and the copy lives through the synchronous
			// call. The Server contract guarantees only deferred removal.
			let result = unsafe { original(victim.as_ptr(), event.info.as_ptr()) };
			HookAction::Supersede(result)
		}
	}
}

impl MetamodApi<'_> {
	/// Hooks incoming or already-scaled damage on this TF2 player's class.
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class/stage
	/// returns `HookError::AlreadyInstalled`; removing its ID allows replacement.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that an immediate
	/// synthetic call will be intercepted. Install during player activation
	/// and keep the hook registered; for probes, fire damage from a later
	/// engine callback. KHook polls every 5 ms and can retry while a detour is
	/// busy, so a fixed single-frame delay is not a readiness guarantee.
	///
	/// `Apply` calls the original with a copy, so later hooks on the same
	/// invocation cannot transform that copy. Use `Continue` for observation
	/// and document this ordering when composing multiple plugins.
	pub fn hook_player_damage(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		stage: DamageStage,
		callback: DamageFn,
	) -> Result<HookId, DamageHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(DamageHookError::NotTfPlayer);
		}
		// SAFETY: Every live CBaseEntity starts with its primary vtable pointer.
		let vtable = unsafe { player.as_ptr().cast::<usize>().read() };
		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == vtable && state.stage == stage && self.has_hook(state.hook)
			})
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let route = ROUTES
			.iter()
			.find(|route| {
				route
					.state
					.get()
					.is_none_or(|state| !self.has_hook(state.hook))
			})
			.ok_or(HookError::TooManyFunctions)?;
		let target = HookTarget::class_of(NonNull::new(player.as_ptr()).unwrap());
		// SAFETY: CTFPlayer's primary vtable has these TF2 slots and the
		// signature int (CTakeDamageInfo const &), on both supported ABIs.
		// Its vtable belongs to the server DLL, which outlives this plugin.
		let original = unsafe { self.original_function(stage.function(), target) }?;
		// SAFETY: The same vtable and signature as checked for original above.
		let hook = unsafe { self.add_hook(stage.function(), target, HookTiming::Pre, route) }?;
		route.state.set(Some(RoutedDamage {
			binding,
			callback,
			original,
			hook,
			stage,
			vtable,
		}));
		Ok(hook)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use source_sdk_2013::InterfaceFactory;
	use source_sdk_2013::damage::{DamageInfo, DamageType};
	use std::ffi::{c_char, c_void};
	use std::mem::{MaybeUninit, offset_of, size_of};

	thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }

	unsafe extern "C" fn no_interfaces(_: *const c_char, _: *mut c_int) -> *mut c_void {
		std::ptr::null_mut()
	}

	unsafe extern "C" fn original(
		_: *mut sys::CBaseEntity,
		info: *const sys::CTakeDamageInfo,
	) -> c_int {
		CALLS.with(|calls| calls.set(calls.get() + 1));
		// SAFETY: The test dispatch supplies a constructed local damage record.
		unsafe { (&raw const (*info).m_flDamage).read() as c_int }
	}

	fn probe(callback: DamageFn) -> (HookAction<c_int>, usize) {
		CALLS.with(|calls| calls.set(0));
		let scope = ();
		let factory = InterfaceFactory::new(no_interfaces);
		// SAFETY: These tests exercise no engine interface; their only virtual
		// call is the explicit mock original below, within this stack scope.
		let server = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };
		let mut victim = MaybeUninit::<sys::CBaseEntity>::zeroed();
		let info = DamageInfo::new(11.0, DamageType::BULLET);
		// SAFETY: Both allocations live through dispatch. The mock original
		// and callbacks never dereference the mock entity or call its methods.
		let action = unsafe {
			dispatch(
				server,
				DamageStage::Incoming,
				original,
				callback,
				NonNull::new(victim.as_mut_ptr()).unwrap(),
				NonNull::new(info.as_ptr().cast_mut()).unwrap(),
			)
		};
		assert_eq!(
			info.amount(),
			11.0,
			"a const source record must never be overwritten"
		);
		(action, CALLS.with(Cell::get))
	}

	#[test]
	fn changed_damage_reaches_original_once_without_overwriting_source() {
		let (action, calls) = probe(|_, _, event| {
			event.info.set_amount(37.0);
			DamageAction::Apply
		});
		assert_eq!(action, HookAction::Supersede(37));
		assert_eq!(calls, 1);
	}

	#[test]
	fn ignored_edits_and_blocking_do_not_call_original() {
		let (action, calls) = probe(|_, _, event| {
			event.info.set_amount(37.0);
			DamageAction::Continue
		});
		assert_eq!(action, HookAction::Ignore);
		assert_eq!(calls, 0);
		assert_eq!(
			probe(|_, _, _| DamageAction::Block),
			(HookAction::Supersede(0), 0)
		);
	}

	#[test]
	fn incoming_signature_and_slot_match_generated_base_entity() {
		let _: fn(&sys::CBaseEntity__bindgen_vtable) -> TakeDamage =
			|vtable| vtable.CBaseEntity_OnTakeDamage;
		assert_eq!(
			DamageStage::Incoming.function().index(),
			offset_of!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_OnTakeDamage)
				/ size_of::<usize>()
		);
	}
}
