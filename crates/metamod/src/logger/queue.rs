//! Bounded storage for owned logs, without engine access or callback pointers.

use super::{OwnedRecord, Sink};
use std::collections::VecDeque;
use std::sync::Mutex;

/// A worker-safe queue bounded by message count and UTF-8 text bytes.
///
/// Overflow rejects the newest record, preserving queued order. Text is already
/// owned when offered; these limits bound retained data, not a producer's
/// temporary formatting allocation. Zero limits disable queueing.
/// `flush` deliberately does not invoke engine code: drain at a current callback.
#[derive(Debug)]
pub struct LogQueue {
	max_records: usize,
	max_bytes: usize,
	state: Mutex<State>,
}

impl LogQueue {
	/// Creates an empty queue for one loaded host generation.
	pub const fn new(max_records: usize, max_bytes: usize) -> Self {
		Self {
			max_records,
			max_bytes,
			state: Mutex::new(State {
				records: VecDeque::new(),
				bytes: 0,
				dropped: 0,
				accepting: true,
			}),
		}
	}

	/// Closes this generation and returns all pending records in order.
	///
	/// Later writes are counted as dropped. The returned records are outside
	/// the lock, so the host can forward them before releasing callback access.
	/// Do not reopen or share the queue with a later plugin generation.
	pub fn close(&self) -> VecDeque<OwnedRecord> {
		let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
		state.accepting = false;
		state.bytes = 0;
		std::mem::take(&mut state.records)
	}

	/// Takes the current batch in order and releases the lock before returning.
	///
	/// Records written reentrantly while the host forwards this batch belong to
	/// the next batch. There is no recursive engine call under a queue lock.
	pub fn drain(&self) -> VecDeque<OwnedRecord> {
		let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
		state.bytes = 0;
		std::mem::take(&mut state.records)
	}

	/// Returns a consistent snapshot of pending and rejected records.
	pub fn stats(&self) -> QueueStats {
		let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
		QueueStats {
			records: state.records.len(),
			bytes: state.bytes,
			dropped: state.dropped,
			accepting: state.accepting,
		}
	}
}

impl Sink for LogQueue {
	fn write(&self, record: OwnedRecord) {
		let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
		if !state.accepting
			|| self.max_bytes == 0
			|| state.records.len() >= self.max_records
			|| record.text.len() > self.max_bytes.saturating_sub(state.bytes)
		{
			state.dropped = state.dropped.saturating_add(1);
			return;
		}

		// Drop excess String capacity before retaining the accepted message.
		// A short message offered with a large reserve must not bypass the
		// queue's byte bound through unused allocation capacity.
		let record = OwnedRecord {
			level: record.level,
			text: record.text.into_boxed_str().into_string(),
		};
		state.bytes += record.text.len();
		state.records.push_back(record);
	}
}

/// A queue snapshot. Byte limits cover UTF-8 message text; count limits also
/// bound per-record/container overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueStats {
	/// Number of pending records.
	pub records: usize,
	/// Total bytes in pending message text.
	pub bytes: usize,
	/// Number of overflow or closed-generation records rejected.
	pub dropped: u64,
	/// Whether this generation remains open. Capacity limits may still reject records.
	pub accepting: bool,
}

#[derive(Debug)]
struct State {
	records: VecDeque<OwnedRecord>,
	bytes: usize,
	dropped: u64,
	accepting: bool,
}
