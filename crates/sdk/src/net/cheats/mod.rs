//! Shows one client `sv_cheats` set without changing the server's own, for
//! client-side state that depends on it, such as cheat-protected commands.
//!
//! # How it works
//!
//! The server tells clients its replicated console variables with
//! `net_SetConVar`, which a client applies whatever the server's own value is.
//! One client can therefore be told `sv_cheats 1` while the server and every
//! other client keep `sv_cheats 0`. The server's cheat checks, its
//! `FCVAR_NOTIFY` announcement, and its cheat-flagged variables are untouched.
//!
//! [`ClientCheats`] coordinates these *leases*, several per client if need be:
//!
//! 1. [`ClientCheats::begin`] queues, in the client's reliable stream and as
//!    one block, `sv_cheats 1`, the lease's commands if any, and an
//!    `svc_GetCvarValue` query for `sv_cheats` carrying a cookie from
//!    [`COOKIES`].
//! 2. The client processes its reliable stream in order and answers with
//!    `clc_RespondCvarValue`, which shows that it processed everything before
//!    the query with `sv_cheats` set, as long as nothing else sent it
//!    `sv_cheats` meanwhile (see [Limits](#limits)).
//!    [`ClientCheats::on_incoming`], called from the plugin's
//!    [`IncomingHandler`], blocks the answer and records it. It sends nothing.
//! 3. Once every lease of the client is answered,
//!    [`ClientCheats::on_game_frame`] sends the *restore*: `sv_cheats` at the
//!    server's value, then every registered variable marked both
//!    `FCVAR_REPLICATED` and `FCVAR_CHEAT` whose value is not its default.
//!    A client reverts its cheat-flagged variables when its `sv_cheats` turns
//!    off, so without the rest of the restore its copies of those would fall
//!    back to their defaults, and its prediction would no longer match the
//!    server, as with `host_timescale` or the grappling hook's speed.
//!
//! A client that does not answer within [`CheatsOptions::confirm_timeout`] is
//! restored anyway, and its lease is sent again after a delay, up to
//! [`CheatsOptions::max_attempts`] times. An answer that arrives once a
//! restore was sent proves nothing, since the client may have processed both
//! within one of its frames, so it is blocked and ignored. An answer that is
//! late but precedes the restore still counts. An answer that the engine's
//! message layout does not let [`IncomingMessage::decode`] read ends the
//! client's waiting leases as [`LeaseOutcome::Unverified`] instead: the
//! client is restored on the next frame, and the leases are not sent again,
//! since no later answer could be read either.
//!
//! If the server's own `sv_cheats` is seen to turn off while leases wait for
//! their answers or their restore, the engine sent every client `sv_cheats 0`,
//! possibly right after a lease's query, so those answers prove nothing:
//! their leases are sent again, without counting as failed attempts.
//!
//! # Wiring
//!
//! Keep one `ClientCheats` for the whole plugin, in a cell only the server's
//! main thread reaches, and send clients `sv_cheats` only through it (see
//! [Limits](#limits)). Then:
//!
//! - hook clients' messages before the first lease begins, and pass every
//!   message the plugin's [`IncomingHandler`] sees to
//!   [`ClientCheats::on_incoming`], returning [`Verdict::Block`] when it
//!   does. Without the hook no answer is seen, and every attempt of every
//!   lease runs to its timeout;
//! - call [`ClientCheats::on_game_frame`] every server frame;
//! - call [`ClientCheats::restore_all`] before the plugin pauses or unloads,
//!   and at level shutdown, followed there by [`ClientCheats::clear`]: clients
//!   keep their user IDs across a level change, but not their client-side
//!   state. The coordinator keeps sending the restore of each client shown
//!   `sv_cheats` set during the level until the client is active on the next
//!   one;
//! - call [`ClientCheats::forget`] once a client disconnects.
//!
//! # Limits
//!
//! - For about one round trip per lease, the client may change its own
//!   cheat-flagged variables, which the restore reverts, and run client-side
//!   cheat commands, whose lasting effects remain. The server's own cheat
//!   checks stay closed.
//! - When no answer is seen, as without the hook, each attempt keeps the
//!   client at `sv_cheats 1` for [`CheatsOptions::confirm_timeout`], and a
//!   lease makes up to [`CheatsOptions::max_attempts`]: nine seconds in all
//!   with [`CheatsOptions::DEFAULT`], with a [`Purpose::Commands`]'s commands
//!   run on each attempt.
//! - A client can lengthen its own window by changing, during it,
//!   cheat-flagged settings of its own network channel, such as
//!   `net_fakelag`, which delay the restore's arrival. This is inferred: the
//!   engine, which declares those settings, is closed source.
//! - An answer only proves something if nothing else sends the client
//!   `sv_cheats` between a lease's block and the client's next frame: another
//!   `ClientCheats`, such as a second plugin's, other code sending
//!   `net_SetConVar` for `sv_cheats`, or a change to the server's own
//!   `sv_cheats`, which the engine sends every client. The client could then
//!   answer `1`, apply `sv_cheats 0`, and only then run its frame, and the
//!   lease would be confirmed although no frame of the client saw `sv_cheats`
//!   set. Let one coordinator per server process send every `sv_cheats` a
//!   client is shown. [`ClientCheats::on_game_frame`] notices the server's own
//!   `sv_cheats` turning off between two frames, but not when it is set and
//!   cleared again within one.
//! - Only cooperative clients are affected: a modified client can ignore the
//!   value and answer anything.
//! - Bots and SourceTV have no client-side `sv_cheats`, and a listen server's
//!   host shares the server's, so leases refuse them.
//! - If the plugin stops without restoring a client, as on a crash, the
//!   client keeps `sv_cheats 1` until the server next sends it the value.
//!
//! The engine's handling of these messages is closed source, and the SDK holds
//! none of it. That clients apply `sv_cheats` from `net_SetConVar`, revert
//! their cheat-flagged variables as soon as it turns off and before the rest
//! of the restore applies, answer queries in order with the rest of their
//! reliable stream, and send the answer only after the frame in which they
//! processed the query, is observed or reported behaviour, not verified
//! against the engine. The SDK only declares the revert itself
//! (`ICvar::RevertFlaggedConVars`, `public/icvar.h:95`), which TF2's client
//! also calls by hand before offline practice
//! (`game/client/tf/vgui/tf_training_ui.cpp:2032-2037`).
//!
//! [`IncomingHandler`]: super::incoming::IncomingHandler

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

use crate::bitbuf::BitWriter;
use crate::commands::CommandFlags;
use crate::interfaces::cvar::ConVar;
use crate::interfaces::{Cvar, GameClient};
use crate::net::incoming::{Incoming, IncomingKind, IncomingMessage, Verdict};
use crate::net::messages::{GetCvarValue, MAX_CONVAR_LEN, SetConVar, StringCmd};
use crate::net::{EncodeError, NetChannel, NetMessage, Reliability, SendError};
use crate::players::UserId;
use crate::server::{InterfaceError, Server};
use std::ffi::{CStr, CString, c_int};
use std::hash::{BuildHasher, RandomState};
use std::mem::take;
use std::num::NonZero;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

/// The variable leases show clients set.
const CHEATS: &CStr = c"sv_cheats";

/// The cookies of the queries leases send, all negative.
///
/// The engine numbers the cookies of its own queries, those plugins start
/// (`IServerPluginHelpers::StartQueryCvarValue`,
/// `public/engine/iserverplugin.h:153-160`) and those the game starts
/// (`IVEngineServer::StartQueryCvarValue`, `public/eiface.h:376-383`, as
/// SourceMod's `QueryClientConVar` does), in a way the SDK does not show.
/// This range is far from small positive numbers, but no collision with
/// either is ruled out. Answers are matched by client, cookie, and variable
/// name, so another query is only mistaken for a lease's if it asks the same
/// client for `sv_cheats` with a cookie a lease of that client awaits: its
/// answer is then blocked, and taken for the lease's. Other code sending its
/// own queries should pick cookies outside this range.
pub const COOKIES: RangeInclusive<c_int> = -0x4348_FFFF..=-0x4348_0000;

/// The most cookies remembered per client after a restore made their answers
/// meaningless, so that late answers are still blocked.
const MAX_RETIRED: usize = 16;

/// The most variables one `net_SetConVar` holds.
const MAX_SET_CONVARS: usize = 255;

/// What [`ClientCheats::begin`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Begun {
	/// The lease is under way. Its end is reported by
	/// [`CheatsEvent::Finished`], and [`ClientCheats::lease`] tells its
	/// progress until then.
	Lease(LeaseId),

	/// The server's own `sv_cheats` is set, which every client already sees,
	/// so no lease was needed and no `sv_cheats` was sent. The commands of a
	/// [`Purpose::Commands`] were sent on their own. Nothing is recorded for
	/// the client, and nothing shows that a frame of it saw `sv_cheats` set:
	/// begin again to make sure once the server's `sv_cheats` is off.
	ServerCheats,
}

/// Why a lease could not begin, or a callback could not reach the engine.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheatsError {
	/// The engine returned no game server.
	#[error("the engine has no game server")]
	NoServer,

	/// No connected client has the user ID.
	#[error("no connected client has user ID {0}")]
	NoClient(UserId),

	/// The client is a bot, SourceTV, or another fake client, which has no
	/// client-side `sv_cheats`.
	#[error("bots and SourceTV have no client-side sv_cheats")]
	FakeClient,

	/// The client has no net channel.
	#[error("the client has no net channel")]
	NoChannel,

	/// The client runs in the server's own process, as a listen server's
	/// host does, and shares the server's `sv_cheats`.
	#[error("the client shares the server's process and its sv_cheats")]
	Loopback,

	/// The server does not register `sv_cheats`.
	#[error("the server does not register sv_cheats")]
	MissingCheats,

	/// [`Server::valve_engine`] or [`Server::cvar`] failed.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The lease's messages could not be encoded or queued.
	#[error(transparent)]
	Send(#[from] SendError),
}

/// Something that happened to a lease or a client during a callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheatsEvent {
	/// A lease ended.
	Finished {
		/// The lease's client.
		user_id: UserId,

		/// The lease.
		lease: LeaseId,

		/// How it ended.
		outcome: LeaseOutcome,
	},

	/// A lease's client did not answer in time, or its messages did not fit
	/// in the client's stream. The client was restored, and the lease is sent
	/// again once its delay passes.
	Retrying {
		/// The lease's client.
		user_id: UserId,

		/// The lease.
		lease: LeaseId,

		/// The attempt about to be made, from 2.
		attempt: u8,
	},

	/// The client's restore did not fit in its stream. If the client was
	/// still shown `sv_cheats` set, `sv_cheats` alone was sent if it fit, and
	/// the rest of the restore follows. Either way, what is missing is sent
	/// again on the next frame in which the client is active.
	RestoreFailed {
		/// The client.
		user_id: UserId,

		/// Why the whole restore could not be sent.
		error: SendError,
	},
}

/// How a [`ClientCheats`] waits and retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CheatsOptions {
	/// How long a client may take to answer before it is restored without
	/// proof that it saw `sv_cheats` set. While no answer can be seen, each
	/// attempt shows the client `sv_cheats` set for this long.
	pub confirm_timeout: Duration,

	/// How many times a lease is sent before it fails.
	pub max_attempts: NonZero<u8>,

	/// How long after its first failed attempt a lease is sent again. The
	/// delay doubles after each further failure.
	pub retry_delay: Duration,
}

impl CheatsOptions {
	/// Three seconds to answer, three attempts, and a one second first retry
	/// delay.
	pub const DEFAULT: Self = Self {
		confirm_timeout: Duration::from_secs(3),
		max_attempts: NonZero::new(3).unwrap(),
		retry_delay: Duration::from_secs(1),
	};

	/// When a lease that failed `failures` times is sent again.
	fn retry_at(self, now: Instant, failures: u8) -> Instant {
		let delay = self
			.retry_delay
			.saturating_mul(1 << failures.saturating_sub(1).min(16));

		now.checked_add(delay).unwrap_or(now)
	}
}

impl Default for CheatsOptions {
	fn default() -> Self {
		Self::DEFAULT
	}
}

/// One client's leases and what it was last sent.
#[derive(Debug)]
struct Client {
	/// Whether a lease of the client was confirmed since it was last cleared.
	confirmed: bool,

	/// Leases that have not ended, in the order they began.
	leases: Vec<Lease>,

	/// Whether a lease of the client was refused since it was last cleared.
	refused: bool,

	/// Cookies whose answers no longer prove anything, oldest first.
	retired: Vec<c_int>,

	/// Whether the client was shown `sv_cheats` set since it was last
	/// cleared.
	spoofed: bool,

	/// The client's user ID.
	user_id: UserId,

	/// What the client was last sent.
	window: Window,
}

impl Client {
	/// A client with nothing recorded.
	const fn new(user_id: UserId) -> Self {
		Self {
			confirmed: false,
			leases: Vec::new(),
			refused: false,
			retired: Vec::new(),
			spoofed: false,
			user_id,
			window: Window::Closed,
		}
	}

	/// Sends again, without counting a failure, every lease whose answer may
	/// have been overtaken by another `sv_cheats`, retiring its query.
	fn invalidate(&mut self) {
		let mut cookies = Vec::new();

		for lease in &mut self.leases {
			if let LeaseState::Answered { cookie, .. } | LeaseState::Sent { cookie, .. } =
				lease.state
			{
				cookies.push(cookie);
				lease.state = LeaseState::Queued { not_before: None };
			}
		}

		for cookie in cookies {
			self.retire(cookie);
		}
	}

	/// Whether the client records nothing a status would tell.
	fn is_blank(&self) -> bool {
		self.is_idle()
			&& !self.confirmed
			&& !self.refused
			&& !self.spoofed
			&& self.retired.is_empty()
	}

	/// Whether the client has nothing to wait for or send.
	fn is_idle(&self) -> bool {
		self.leases.is_empty() && self.window == Window::Closed
	}

	/// Sends the restore, or what is left of it. Returns whether `sv_cheats`
	/// was restored, which ends the client's sent leases.
	fn restore<L: Link>(
		&mut self,
		link: &mut L,
		peer: L::Peer,
		events: &mut Vec<CheatsEvent>,
	) -> bool {
		let (whole, cheats) = {
			let values = link.restore_values();

			(
				restore_bits(values),
				restore_bits(values.get(..1).unwrap_or_default()),
			)
		};

		let error = match link.send(peer, &whole) {
			Ok(()) => {
				self.window = Window::Closed;
				return true;
			}

			Err(error) => error,
		};

		events.push(CheatsEvent::RestoreFailed {
			user_id: self.user_id,
			error,
		});

		if self.window == Window::Open && link.send(peer, &cheats).is_ok() {
			self.window = Window::Resync;
			return true;
		}

		false
	}

	/// Whether the client is due its restore: it was shown `sv_cheats` set
	/// and every lease was answered or one timed out, or its resynchronization
	/// is still missing.
	fn restore_due(&self, now: Instant, timeout: Duration) -> bool {
		match self.window {
			Window::Closed => false,
			Window::Resync => true,

			Window::Open => {
				self.leases.iter().all(|lease| !lease.is_sent())
					|| self
						.leases
						.iter()
						.any(|lease| lease.timed_out(now, timeout))
			}
		}
	}

	/// Remembers that answers carrying `cookie` prove nothing anymore.
	fn retire(&mut self, cookie: c_int) {
		if self.retired.len() >= MAX_RETIRED {
			self.retired.remove(0);
		}

		self.retired.push(cookie);
	}

	/// Ends or requeues leases once the client was restored, or every lease
	/// when `cancel` is set.
	fn settle(
		&mut self,
		now: Instant,
		options: CheatsOptions,
		events: &mut Vec<CheatsEvent>,
		cancel: bool,
	) {
		let user_id = self.user_id;

		for mut lease in take(&mut self.leases) {
			let outcome = match lease.state {
				LeaseState::Answered { cookie, cheats } => {
					// A repeated answer is blocked too.
					self.retire(cookie);

					match cheats {
						true => {
							self.confirmed = true;
							LeaseOutcome::Confirmed
						}

						false => {
							self.refused = true;
							LeaseOutcome::Refused
						}
					}
				}

				LeaseState::Unverified { cookie } => {
					self.retire(cookie);
					LeaseOutcome::Unverified
				}

				LeaseState::Queued { .. } if cancel => LeaseOutcome::Cancelled,

				LeaseState::Queued { .. } => {
					self.leases.push(lease);
					continue;
				}

				LeaseState::Sent { cookie, .. } if cancel => {
					self.retire(cookie);
					LeaseOutcome::Cancelled
				}

				LeaseState::Sent { cookie, .. }
					if !lease.timed_out(now, options.confirm_timeout) =>
				{
					// Another lease's timeout restored the client first, so this
					// one is sent again without counting as a failure.
					self.retire(cookie);
					lease.state = LeaseState::Queued { not_before: None };
					self.leases.push(lease);
					continue;
				}

				LeaseState::Sent { cookie, .. } => {
					self.retire(cookie);
					lease.failures = lease.failures.saturating_add(1);

					if lease.failures >= options.max_attempts.get() {
						LeaseOutcome::TimedOut
					} else {
						lease.requeue(now, options, user_id, events);
						self.leases.push(lease);
						continue;
					}
				}
			};

			events.push(CheatsEvent::Finished {
				user_id,
				lease: lease.id,
				outcome,
			});
		}
	}
}

/// Coordinates leases that show single clients `sv_cheats` set, and restores
/// them afterwards.
///
/// It holds only plain data, such as user IDs, cookies and times, never an
/// engine handle, so a plugin keeps it between callbacks, and passes each
/// callback's [`Server`] to the methods that reach the engine. Clients are
/// found again by user ID in every callback. No method calls back into the
/// plugin.
///
/// See the [module documentation](self) for how leases work, and how a plugin
/// wires this in.
#[doc(alias = "sv_cheats")]
#[derive(Debug)]
pub struct ClientCheats {
	clients: Vec<Client>,
	next_cookie: Option<c_int>,
	next_lease: NonZero<u64>,
	options: CheatsOptions,

	/// Whether the server's own `sv_cheats` was set when last seen.
	server_cheats: bool,
}

impl ClientCheats {
	/// A coordinator with no leases, which waits and retries as `options`
	/// says.
	pub const fn new(options: CheatsOptions) -> Self {
		Self {
			clients: Vec::new(),
			next_cookie: None,
			next_lease: NonZero::<u64>::MIN,
			options,
			server_cheats: false,
		}
	}

	/// Whether any query awaits its answer, or a late or repeated answer may
	/// still arrive.
	fn awaits_answers(&self) -> bool {
		self.clients.iter().any(|client| {
			!client.retired.is_empty() || client.leases.iter().any(|lease| lease.cookie().is_some())
		})
	}

	/// Starts a lease, which shows the client with `user_id` that `sv_cheats`
	/// is set, for `purpose`.
	///
	/// The messages are queued at once if the client is active, which is to
	/// say fully in the game, and its last restore was sent whole. Otherwise
	/// the lease waits, and [`Self::on_game_frame`] sends it once that holds.
	///
	/// If the server's own `sv_cheats` is set, every client already sees it:
	/// nothing but a [`Purpose::Commands`]'s commands is sent, and
	/// [`Begun::ServerCheats`] is returned.
	///
	/// Fails for user IDs no connected client has, for bots, SourceTV,
	/// clients without a net channel, and listen servers' hosts, for commands
	/// that cannot be encoded, and when the messages do not fit in the
	/// client's stream. Nothing is sent or recorded then.
	pub fn begin(
		&mut self,
		server: Server<'_>,
		user_id: UserId,
		purpose: Purpose,
	) -> Result<Begun, CheatsError> {
		let mut link = EngineLink::new(server)?;
		let client = link.client(user_id).ok_or(CheatsError::NoClient(user_id))?;

		if client.is_fake() || client.is_hltv() {
			return Err(CheatsError::FakeClient);
		}

		if client
			.net_channel()
			.ok_or(CheatsError::NoChannel)?
			.is_loopback()
		{
			return Err(CheatsError::Loopback);
		}

		self.begin_with(&mut link, user_id, purpose, Instant::now())
	}

	/// [`Self::begin`] once the client was checked.
	fn begin_with<L: Link>(
		&mut self,
		link: &mut L,
		user_id: UserId,
		purpose: Purpose,
		now: Instant,
	) -> Result<Begun, CheatsError> {
		let (peer, active) = link.peer(user_id).ok_or(CheatsError::NoClient(user_id))?;
		let commands = commands_bits(&purpose).map_err(SendError::from)?;
		let server_cheats = link.server_cheats();

		self.see_server_cheats(server_cheats);

		if server_cheats {
			if !commands.is_empty() {
				link.send(peer, &commands)?;
			}

			return Ok(Begun::ServerCheats);
		}

		let id = self.next_lease();
		let index = self.entry(user_id);
		let mut lease = Lease {
			failures: 0,
			id,
			purpose,
			state: LeaseState::Queued { not_before: None },
		};

		if active && self.clients[index].window != Window::Resync {
			let cookie = self.next_cookie();
			let sent = spoof_bits([(cookie, &lease.purpose)])
				.map_err(SendError::from)
				.and_then(|bits| link.send(peer, &bits));

			if let Err(error) = sent {
				if self.clients[index].is_blank() {
					self.clients.remove(index);
				}

				return Err(error.into());
			}

			let client = &mut self.clients[index];

			lease.state = LeaseState::Sent { cookie, at: now };
			client.spoofed = true;
			client.window = Window::Open;
		}

		self.clients[index].leases.push(lease);

		Ok(Begun::Lease(id))
	}

	/// Forgets every lease, and what each client confirmed or refused,
	/// without sending anything.
	///
	/// Call it at level shutdown, after [`Self::restore_all`]: clients keep
	/// their user IDs across a level change, but reload their client-side
	/// state, so what was confirmed no longer holds.
	///
	/// Each client shown `sv_cheats` set since the last clear is kept, and
	/// sent its restore again by [`Self::on_game_frame`] once it is active on
	/// the next level: the restore at level shutdown may not have fit, or may
	/// not reach a client the level change deactivates. Telling a client the
	/// server's values twice is harmless. New leases of such a client wait
	/// for that restore.
	pub fn clear(&mut self) {
		self.clients.retain_mut(|client| {
			let resync = client.spoofed || client.window != Window::Closed;

			*client = Client {
				window: Window::Resync,
				..Client::new(client.user_id)
			};

			resync
		});
	}

	/// What is known of the client with `user_id`, or `None` if nothing is
	/// recorded for it: no lease of it began since it was last forgotten, or
	/// since it was last cleared with no restore left to send.
	pub fn client(&self, user_id: UserId) -> Option<ClientStatus> {
		self.find(user_id).map(|client| ClientStatus {
			confirmed: client.confirmed,
			pending: client.leases.len(),
			refused: client.refused,
			restore_pending: client.window != Window::Closed,
		})
	}

	/// The index of the client's entry, which is added if missing.
	fn entry(&mut self, user_id: UserId) -> usize {
		if let Some(index) = self
			.clients
			.iter()
			.position(|client| client.user_id == user_id)
		{
			return index;
		}

		self.clients.push(Client::new(user_id));
		self.clients.len() - 1
	}

	/// The client's entry.
	fn find(&self, user_id: UserId) -> Option<&Client> {
		self.clients.iter().find(|client| client.user_id == user_id)
	}

	/// Forgets a client, without sending anything. Returns whether it was
	/// known.
	///
	/// Call it once the client disconnected. A client still connected and
	/// shown `sv_cheats` set keeps it until the server next sends it the
	/// value: restore it with [`Self::restore_all`] first.
	pub fn forget(&mut self, user_id: UserId) -> bool {
		let known = self.clients.len();

		self.clients.retain(|client| client.user_id != user_id);
		self.clients.len() != known
	}

	/// What a lease is waiting for, or `None` once it ended, which a
	/// [`CheatsEvent::Finished`] reported, or if it was forgotten or cleared.
	pub fn lease(&self, lease: LeaseId) -> Option<LeaseStatus> {
		self.clients
			.iter()
			.flat_map(|client| &client.leases)
			.find(|candidate| candidate.id == lease)
			.map(|lease| match lease.state {
				LeaseState::Answered { .. } | LeaseState::Unverified { .. } => {
					LeaseStatus::Answered
				}

				LeaseState::Queued { .. } => LeaseStatus::Queued,
				LeaseState::Sent { .. } => LeaseStatus::Sent,
			})
	}

	/// A cookie no outstanding or retired query carries.
	fn next_cookie(&mut self) -> c_int {
		let span = COOKIES.end() - COOKIES.start() + 1;

		// The first cookie is picked at random, so that the coordinators of
		// several plugins rarely ask the same client with the same cookie.
		let mut next = self.next_cookie.unwrap_or_else(|| {
			(RandomState::new().hash_one(span) % span.unsigned_abs() as u64) as c_int
		});

		for _ in 0..span {
			let cookie = COOKIES.end() - next;

			next = (next + 1) % span;

			let in_use = self.clients.iter().any(|client| {
				client.retired.contains(&cookie)
					|| client
						.leases
						.iter()
						.any(|lease| lease.cookie() == Some(cookie))
			});

			if !in_use {
				self.next_cookie = Some(next);
				return cookie;
			}
		}

		// Only 65,536 outstanding queries use every cookie.
		self.next_cookie = Some(next);
		*COOKIES.end()
	}

	/// A lease ID not used before.
	fn next_lease(&mut self) -> LeaseId {
		let id = LeaseId(self.next_lease);

		self.next_lease = self.next_lease.saturating_add(1);
		id
	}

	/// Restores active clients whose leases were all answered or timed out,
	/// or whose restore is missing, sends leases that are waiting, retries
	/// what failed, and forgets clients that left. Call it once per server
	/// frame.
	///
	/// Messages are queued, so they reach clients at the end of the frame.
	/// Fails only if the engine's interfaces or `sv_cheats` cannot be found,
	/// and then leaves everything as it was.
	pub fn on_game_frame(&mut self, server: Server<'_>) -> Result<Vec<CheatsEvent>, CheatsError> {
		if self.clients.iter().all(Client::is_idle) {
			return Ok(Vec::new());
		}

		Ok(self.on_game_frame_with(&mut EngineLink::new(server)?, Instant::now()))
	}

	/// [`Self::on_game_frame`] at `now`.
	fn on_game_frame_with<L: Link>(&mut self, link: &mut L, now: Instant) -> Vec<CheatsEvent> {
		let mut events = Vec::new();
		let mut index = 0;

		self.see_server_cheats(link.server_cheats());

		while index < self.clients.len() {
			let Some((peer, active)) = link.peer(self.clients[index].user_id) else {
				let mut client = self.clients.remove(index);

				client.settle(now, self.options, &mut events, true);
				continue;
			};

			// A client that is not active may be changing levels, which can
			// drop what it is sent.
			if active {
				let options = self.options;
				let client = &mut self.clients[index];

				if client.restore_due(now, options.confirm_timeout)
					&& client.restore(link, peer, &mut events)
				{
					client.settle(now, options, &mut events, false);
				}

				self.send_waiting(link, index, peer, now, &mut events);
			}

			index += 1;
		}

		events
	}

	/// Passes a client's message to [`Self::on_response`] if it may answer a
	/// lease's query, and returns its verdict. Call it from the plugin's
	/// [`IncomingHandler`](super::incoming::IncomingHandler) for every message.
	///
	/// Other kinds of messages, and every message while no answer is
	/// expected, return [`Verdict::Continue`] at once, without calling into
	/// the engine. An answer the engine's layout does not let
	/// [`IncomingMessage::decode`] read is let through, since it may answer
	/// another query, and ends the client's leases still waiting for an answer
	/// as [`LeaseOutcome::Unverified`], so that the next
	/// [`Self::on_game_frame`] restores the client instead of waiting for
	/// their timeout.
	#[doc(alias = "ProcessRespondCvarValue")]
	pub fn on_incoming(&mut self, message: IncomingMessage<'_>) -> Verdict {
		if message.kind() != IncomingKind::RespondCvarValue || !self.awaits_answers() {
			return Verdict::Continue;
		}

		let Some(user_id) = message.client().user_id() else {
			return Verdict::Continue;
		};

		match message.decode() {
			Some(response) => self.on_response(user_id, &response),

			None => {
				self.on_unreadable(user_id);
				Verdict::Continue
			}
		}
	}

	/// Records a client's answer to a lease's query, returning
	/// [`Verdict::Block`] for answers to this coordinator's queries, which no
	/// one else asked for, and [`Verdict::Continue`] for everything else.
	///
	/// [`Self::on_incoming`] calls this with the decoded message; call it
	/// directly when the message is already decoded. It only updates state:
	/// the lease is confirmed or refused once the client's restore is sent.
	#[doc(alias = "CLC_RespondCvarValue")]
	pub fn on_response(&mut self, user_id: UserId, response: &Incoming) -> Verdict {
		let Incoming::RespondCvarValue {
			cookie,
			status,
			name,
			value,
		} = response
		else {
			return Verdict::Continue;
		};

		if !COOKIES.contains(cookie) || !name.as_bytes().eq_ignore_ascii_case(CHEATS.to_bytes()) {
			return Verdict::Continue;
		}

		let Some(client) = self
			.clients
			.iter_mut()
			.find(|client| client.user_id == user_id)
		else {
			return Verdict::Continue;
		};

		let lease = client
			.leases
			.iter_mut()
			.find(|lease| lease.cookie() == Some(*cookie));

		match lease {
			Some(lease) if lease.is_sent() => {
				// Only a found value can be checked. A variable the client hides
				// or lacks still answers after everything sent before the query.
				let cheats = *status != 0 || is_set(value);

				lease.state = LeaseState::Answered {
					cookie: *cookie,
					cheats,
				};

				Verdict::Block
			}

			// A repeated answer.
			Some(_) => Verdict::Block,

			None if client.retired.contains(cookie) => Verdict::Block,
			None => Verdict::Continue,
		}
	}

	/// Ends the client's leases waiting for an answer as unverified, once an
	/// answer it sent could not be read.
	fn on_unreadable(&mut self, user_id: UserId) {
		let Some(client) = self
			.clients
			.iter_mut()
			.find(|client| client.user_id == user_id)
		else {
			return;
		};

		for lease in &mut client.leases {
			if let LeaseState::Sent { cookie, .. } = lease.state {
				lease.state = LeaseState::Unverified { cookie };
			}
		}
	}

	/// How the coordinator waits and retries.
	pub const fn options(&self) -> CheatsOptions {
		self.options
	}

	/// Restores every connected client still shown `sv_cheats` set, or
	/// missing its restore, and ends every lease, as
	/// [`LeaseOutcome::Cancelled`] unless it was already answered.
	///
	/// Call it before the plugin pauses or unloads, and at level shutdown,
	/// followed there by [`Self::clear`]. Clients that left are forgotten. A
	/// restore that does not fit is reported with
	/// [`CheatsEvent::RestoreFailed`], and is sent again by
	/// [`Self::on_game_frame`] once the client is active, even after
	/// [`Self::clear`]. Fails only if the engine's interfaces or `sv_cheats`
	/// cannot be found, and then leaves everything as it was.
	pub fn restore_all(&mut self, server: Server<'_>) -> Result<Vec<CheatsEvent>, CheatsError> {
		if self.clients.iter().all(Client::is_idle) {
			return Ok(Vec::new());
		}

		Ok(self.restore_all_with(&mut EngineLink::new(server)?, Instant::now()))
	}

	/// [`Self::restore_all`] at `now`.
	fn restore_all_with<L: Link>(&mut self, link: &mut L, now: Instant) -> Vec<CheatsEvent> {
		let mut events = Vec::new();
		let options = self.options;

		self.clients.retain_mut(|client| {
			let peer = link.peer(client.user_id);

			if let Some((peer, _)) = peer
				&& client.window != Window::Closed
			{
				client.restore(link, peer, &mut events);
			}

			client.settle(now, options, &mut events, true);
			peer.is_some()
		});

		events
	}

	/// Notes the server's own `sv_cheats`. Once it turns off, the engine
	/// sends every client `sv_cheats 0`, possibly right after a lease's query,
	/// so answers that may have preceded it prove nothing.
	fn see_server_cheats(&mut self, set: bool) {
		if self.server_cheats && !set {
			for client in &mut self.clients {
				client.invalidate();
			}
		}

		self.server_cheats = set;
	}

	/// Sends the client's leases that are waiting and due, in one block.
	fn send_waiting<L: Link>(
		&mut self,
		link: &mut L,
		index: usize,
		peer: L::Peer,
		now: Instant,
		events: &mut Vec<CheatsEvent>,
	) {
		let client = &self.clients[index];
		let due = client
			.leases
			.iter()
			.filter(|lease| lease.is_due(now))
			.count();

		if due == 0 || client.window == Window::Resync {
			return;
		}

		let options = self.options;

		if link.server_cheats() {
			let client = &mut self.clients[index];

			// The engine shows every client the server's own `sv_cheats`, but
			// nothing shows that a frame of the client saw it set, so the
			// client is not confirmed.
			for lease in take(&mut client.leases) {
				if !lease.is_due(now) {
					client.leases.push(lease);
					continue;
				}

				let sent = commands_bits(&lease.purpose)
					.map_err(SendError::from)
					.and_then(|bits| match bits.is_empty() {
						true => Ok(()),
						false => link.send(peer, &bits),
					});

				events.push(CheatsEvent::Finished {
					user_id: client.user_id,
					lease: lease.id,
					outcome: match sent {
						Ok(()) => LeaseOutcome::ServerCheats,
						Err(error) => LeaseOutcome::SendFailed(error),
					},
				});
			}

			return;
		}

		let cookies: Vec<c_int> = (0..due).map(|_| self.next_cookie()).collect();
		let client = &mut self.clients[index];
		let sent = spoof_bits(
			client
				.leases
				.iter()
				.filter(|lease| lease.is_due(now))
				.zip(cookies.iter().copied())
				.map(|(lease, cookie)| (cookie, &lease.purpose)),
		)
		.map_err(SendError::from)
		.and_then(|bits| link.send(peer, &bits));

		match sent {
			Ok(()) => {
				let waiting = client.leases.iter_mut().filter(|lease| lease.is_due(now));

				for (lease, cookie) in waiting.zip(cookies) {
					lease.state = LeaseState::Sent { cookie, at: now };
				}

				client.spoofed = true;
				client.window = Window::Open;
			}

			Err(error) => {
				let user_id = client.user_id;

				for mut lease in take(&mut client.leases) {
					if !lease.is_due(now) {
						client.leases.push(lease);
						continue;
					}

					lease.failures = lease.failures.saturating_add(1);

					if lease.failures >= options.max_attempts.get() {
						events.push(CheatsEvent::Finished {
							user_id,
							lease: lease.id,
							outcome: LeaseOutcome::SendFailed(error.clone()),
						});
					} else {
						lease.requeue(now, options, user_id, events);
						client.leases.push(lease);
					}
				}
			}
		}
	}
}

impl Default for ClientCheats {
	fn default() -> Self {
		Self::new(CheatsOptions::DEFAULT)
	}
}

/// What [`ClientCheats::client`] knows of a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientStatus {
	/// Whether a lease of the client ended [`LeaseOutcome::Confirmed`] since
	/// the client was last forgotten or cleared: it answered that it saw
	/// `sv_cheats` set, and was restored after. The server's own `sv_cheats`
	/// does not count, since nothing shows that a frame of the client saw it.
	pub confirmed: bool,

	/// The leases that have not ended.
	pub pending: usize,

	/// Whether a lease of the client ended [`LeaseOutcome::Refused`] since the
	/// client was last forgotten or cleared.
	pub refused: bool,

	/// Whether the client may still see `sv_cheats` set, or its cheat-flagged
	/// variables at their defaults, until a restore reaches it.
	pub restore_pending: bool,
}

/// The engine, as one callback's [`Server`] reaches it.
struct EngineLink<'s> {
	/// The server's own `sv_cheats`.
	cheats: ConVar<'s>,

	/// Every connected client with a user ID.
	clients: Vec<(UserId, GameClient<'s>)>,

	/// The registry.
	cvar: Cvar<'s>,

	/// What a restore sends, once a restore needs it.
	restore: Option<Vec<(CString, CString)>>,
}

impl<'s> EngineLink<'s> {
	/// Finds the server's clients and `sv_cheats`.
	fn new(server: Server<'s>) -> Result<Self, CheatsError> {
		let game_server = server
			.valve_engine()?
			.game_server()
			.ok_or(CheatsError::NoServer)?;

		let cvar = server.cvar()?;
		let cheats = cvar.find_var(CHEATS).ok_or(CheatsError::MissingCheats)?;
		let clients = game_server
			.clients()
			.filter(|client| client.is_connected())
			.filter_map(|client| Some((client.user_id()?, client)))
			.collect();

		Ok(Self {
			cheats,
			clients,
			cvar,
			restore: None,
		})
	}

	/// The connected client with `user_id`.
	fn client(&self, user_id: UserId) -> Option<GameClient<'s>> {
		self.clients
			.iter()
			.find(|&&(id, _)| id == user_id)
			.map(|&(_, client)| client)
	}
}

impl<'s> Link for EngineLink<'s> {
	type Peer = NetChannel<'s>;

	fn peer(&mut self, user_id: UserId) -> Option<(Self::Peer, bool)> {
		let client = self.client(user_id)?;

		Some((client.net_channel()?, client.is_active()))
	}

	fn restore_values(&mut self) -> &[(CString, CString)] {
		let (cvar, cheats) = (self.cvar, self.cheats);

		self.restore
			.get_or_insert_with(|| restore_values_of(cvar, cheats))
	}

	fn send(&mut self, peer: Self::Peer, bits: &BitWriter) -> Result<(), SendError> {
		peer.send_encoded(bits, Reliability::Reliable)
	}

	fn server_cheats(&mut self) -> bool {
		self.cheats.int() != 0
	}
}

/// One request to show a client `sv_cheats` set.
#[derive(Debug)]
struct Lease {
	/// Attempts that timed out or could not be sent.
	failures: u8,

	/// The lease's ID.
	id: LeaseId,

	/// What it is for.
	purpose: Purpose,

	/// What it waits for.
	state: LeaseState,
}

impl Lease {
	/// The cookie of the lease's latest query, if one was sent.
	const fn cookie(&self) -> Option<c_int> {
		match self.state {
			LeaseState::Answered { cookie, .. }
			| LeaseState::Sent { cookie, .. }
			| LeaseState::Unverified { cookie } => Some(cookie),

			LeaseState::Queued { .. } => None,
		}
	}

	/// Whether the lease waits to be sent, and may be now.
	fn is_due(&self, now: Instant) -> bool {
		matches!(self.state, LeaseState::Queued { not_before } if not_before.is_none_or(|at| now >= at))
	}

	/// Whether the lease's query awaits its answer.
	const fn is_sent(&self) -> bool {
		matches!(self.state, LeaseState::Sent { .. })
	}

	/// Queues the lease to be sent again after its delay, reporting it.
	fn requeue(
		&mut self,
		now: Instant,
		options: CheatsOptions,
		user_id: UserId,
		events: &mut Vec<CheatsEvent>,
	) {
		self.state = LeaseState::Queued {
			not_before: Some(options.retry_at(now, self.failures)),
		};

		events.push(CheatsEvent::Retrying {
			user_id,
			lease: self.id,
			attempt: self.failures.saturating_add(1),
		});
	}

	/// Whether the lease's query has waited at least `timeout`.
	fn timed_out(&self, now: Instant, timeout: Duration) -> bool {
		matches!(self.state, LeaseState::Sent { at, .. } if now.saturating_duration_since(at) >= timeout)
	}
}

/// Identifies a lease of a [`ClientCheats`], which never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(NonZero<u64>);

impl LeaseId {
	/// The ID as a number, from 1.
	pub const fn get(self) -> u64 {
		self.0.get()
	}
}

/// How a lease ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseOutcome {
	/// The client answered after processing the lease's messages with
	/// `sv_cheats` set, and was then restored.
	Confirmed,

	/// The server's own `sv_cheats` was set when the lease was due, which
	/// every client already sees. Its commands were sent on their own. Nothing
	/// shows that a frame of the client saw `sv_cheats` set.
	ServerCheats,

	/// The client answered that its `sv_cheats` was not set: it ignored the
	/// value, as a modified client may. Trying again would not help.
	Refused,

	/// The client did not answer in time on any of
	/// [`CheatsOptions::max_attempts`] attempts.
	TimedOut,

	/// The client answered, but the engine's message layout did not let
	/// [`IncomingMessage::decode`] read the answer, so the client was
	/// restored without proof that it saw `sv_cheats` set. The lease is not
	/// sent again, since no later answer could be read either.
	Unverified,

	/// The lease's messages did not fit in the client's stream on its last
	/// attempt.
	SendFailed(SendError),

	/// [`ClientCheats::restore_all`] ended the lease before the client
	/// answered, or the client left.
	Cancelled,
}

/// What a lease waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseState {
	/// The client answered the query carrying `cookie`, and whether its
	/// `sv_cheats` was set.
	Answered { cookie: c_int, cheats: bool },

	/// To be sent, once the client is active and, if set, `not_before` passed.
	Queued { not_before: Option<Instant> },

	/// Sent with a query carrying `cookie` at `at`.
	Sent { cookie: c_int, at: Instant },

	/// Sent with a query carrying `cookie`, when an answer of the client's
	/// arrived that could not be read.
	Unverified { cookie: c_int },
}

/// What a lease is waiting for, from [`ClientCheats::lease`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaseStatus {
	/// To be sent: the client is not yet active, its restore is not complete,
	/// or the delay before a retry has not passed.
	Queued,

	/// Sent, and waiting for the client's answer.
	Sent,

	/// Answered, or an answer that could not be read arrived. The lease ends
	/// once the client's restore is sent, after every other lease of the
	/// client was answered too.
	Answered,
}

/// What a coordinator needs from the engine during one callback, so its
/// decisions can be tested without one.
trait Link {
	/// Where messages are sent: a client's channel.
	type Peer: Copy;

	/// The connected client with `user_id`, and whether it is active, or
	/// `None` if no connected client with a channel has it.
	fn peer(&mut self, user_id: UserId) -> Option<(Self::Peer, bool)>;

	/// What a restore sends, `sv_cheats` first.
	fn restore_values(&mut self) -> &[(CString, CString)];

	/// Queues encoded messages in the client's reliable stream.
	fn send(&mut self, peer: Self::Peer, bits: &BitWriter) -> Result<(), SendError>;

	/// Whether the server's own `sv_cheats` is set.
	fn server_cheats(&mut self) -> bool;
}

/// What a lease shows a client `sv_cheats` set for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Purpose {
	/// For the client to see `sv_cheats` set during at least one of its
	/// frames, for state it latches from it, such as the achievement manager's
	/// cheat flag. The answer to the query shows the client processed
	/// `sv_cheats 1` before it, and should only leave the client after it
	/// finished the frame in which it processed it.
	#[default]
	Observe,

	/// To run commands on the client while it sees `sv_cheats` set, each sent
	/// as a [`StringCmd`] of at most
	/// [`MAX_COMMAND_LEN`](super::messages::MAX_COMMAND_LEN) bytes.
	///
	/// Clients run commands the server sends from their command buffer on
	/// their next frame, and only those marked `FCVAR_SERVER_CAN_EXECUTE`.
	/// Clients should only send the answer to the query with a later frame's
	/// packet, after running their command buffer, so it should also show
	/// that the commands ran, as the acknowledgement of the same messages
	/// would. A retried lease runs its commands again, so they should not
	/// mind running twice.
	Commands(Vec<CString>),
}

impl Purpose {
	/// The commands the lease runs, if any.
	pub fn commands(&self) -> &[CString] {
		match self {
			Self::Observe => &[],
			Self::Commands(commands) => commands,
		}
	}
}

/// What a client was last sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Window {
	/// The server's values, as far as this coordinator knows.
	Closed,

	/// `sv_cheats 1`, not yet restored.
	Open,

	/// The server's `sv_cheats` without the rest of the restore, or anything
	/// before [`ClientCheats::clear`]: the whole restore is still to be sent.
	Resync,
}

/// Encodes a lease's commands, each a [`StringCmd`].
fn commands_bits(purpose: &Purpose) -> Result<BitWriter, EncodeError> {
	let mut bits = BitWriter::new();

	for command in purpose.commands() {
		bits.write_bits(&StringCmd { command }.encode()?);
	}

	Ok(bits)
}

/// Whether a `net_SetConVar` can hold the variable.
fn fits(name: &CStr, value: &CStr) -> bool {
	name.count_bytes() <= MAX_CONVAR_LEN && value.count_bytes() <= MAX_CONVAR_LEN
}

/// Whether a console variable's value turns it on, as `ConVar::GetBool` reads
/// it: as a number, truncated to an integer, that is not zero.
fn is_set(value: &CStr) -> bool {
	value
		.to_str()
		.ok()
		.and_then(|value| value.trim().parse::<f64>().ok())
		.is_some_and(|value| value.trunc() != 0.0)
}

/// Encodes a restore: `net_SetConVar` messages of at most 255 variables each,
/// in order. Variables too long for the message are left out.
fn restore_bits(values: &[(CString, CString)]) -> BitWriter {
	let values: Vec<(&CStr, &CStr)> = values
		.iter()
		.filter(|(name, value)| fits(name, value))
		.map(|(name, value)| (name.as_c_str(), value.as_c_str()))
		.collect();

	let mut bits = BitWriter::new();

	for chunk in values.chunks(MAX_SET_CONVARS) {
		// Every variable fits, and no chunk holds more than a message does, so
		// encoding cannot fail.
		if let Ok(message) = (SetConVar { convars: chunk }).encode() {
			bits.write_bits(&message);
		}
	}

	bits
}

/// What a client shown `sv_cheats` set must be sent to match the server
/// again: `sv_cheats` at the server's value, then every registered variable
/// marked both `FCVAR_REPLICATED` and `FCVAR_CHEAT` whose value is not its
/// default, in the registry's order, as [`ClientCheats`] sends them.
///
/// Clients revert their cheat-flagged variables when their `sv_cheats` turns
/// off, which would leave their copies of these at their defaults. Variables
/// too long for a `net_SetConVar` are left out, and the values are copied.
#[doc(alias = "FCVAR_REPLICATED")]
pub fn restore_values(cvar: Cvar<'_>) -> Result<Vec<(CString, CString)>, CheatsError> {
	let cheats = cvar.find_var(CHEATS).ok_or(CheatsError::MissingCheats)?;

	Ok(restore_values_of(cvar, cheats))
}

/// [`restore_values`] with `sv_cheats` already found.
fn restore_values_of(cvar: Cvar<'_>, cheats: ConVar<'_>) -> Vec<(CString, CString)> {
	let mut value = cheats.string();

	if value.count_bytes() > MAX_CONVAR_LEN {
		value = CString::new(cheats.int().to_string()).unwrap_or_default();
	}

	let mut values = vec![(CHEATS.to_owned(), value)];
	let replicated_cheat = CommandFlags::REPLICATED | CommandFlags::CHEAT;

	for var in cvar.vars() {
		if !var.flags().contains(replicated_cheat) || var.is_default() {
			continue;
		}

		let name = var.name();
		let listed = values
			.iter()
			.any(|(listed, _)| listed.as_bytes().eq_ignore_ascii_case(name.to_bytes()));

		if listed {
			continue;
		}

		let value = var.string();

		if fits(name, &value) {
			values.push((name.to_owned(), value));
		}
	}

	values
}

/// Encodes a spoof: `sv_cheats 1`, then each lease's commands and query.
fn spoof_bits<'a>(
	leases: impl IntoIterator<Item = (c_int, &'a Purpose)>,
) -> Result<BitWriter, EncodeError> {
	let mut bits = SetConVar {
		convars: &[(CHEATS, c"1")],
	}
	.encode()?;

	for (cookie, purpose) in leases {
		bits.write_bits(&commands_bits(purpose)?);
		bits.write_bits(
			&GetCvarValue {
				cookie,
				name: CHEATS,
			}
			.encode()?,
		);
	}

	Ok(bits)
}
