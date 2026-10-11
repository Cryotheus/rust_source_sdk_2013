//! Finite, one-shot sparks through the generated ITempEntsSystem ABI.
use super::TempEntities;
use crate::{math::Vector, user_messages::Recipients};
use crate::{raw::vcall, sys};

/// A finite spark burst; magnitude and trail length follow TE_Sparks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SparkBurst {
	/// World position of the burst.
	pub origin: Vector,
	/// Direction used by the engine spark effect.
	pub direction: Vector,
	/// Burst magnitude, from 1 through 8.
	pub magnitude: u8,
	/// Spark trail length, from 1 through 8.
	pub trail_length: u8,
}

/// Invalid finite vectors or unsupported spark magnitude/trail length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("spark vectors must be finite and magnitude/trail length must be 1 through 8")]
pub struct SparkError;

impl TempEntities<'_> {
	/// Emits one short burst. No particle attachment or persistent entity is created.
	/// Prediction filtering is disabled for this call and restored before returning.
	pub fn sparks(self, recipients: &Recipients, burst: SparkBurst) -> Result<(), SparkError> {
		if !burst.origin.is_finite()
			|| !burst.direction.is_finite()
			|| !(1..=8).contains(&burst.magnitude)
			|| !(1..=8).contains(&burst.trail_length)
		{
			return Err(SparkError);
		}
		if recipients.is_empty() {
			return Ok(());
		}
		let origin = sys::Vector::from(burst.origin);
		let direction = sys::Vector::from(burst.direction);
		let filter = recipients.filter();
		let this = self.as_ptr();
		// SAFETY: The generated vtable defines this exact pointer/scalar signature.
		// All arguments and the callback-scoped system outlive the synchronous call.
		// Raising the prediction depth follows CDisablePredictionFiltering, avoiding
		// the game's cast of our recipient filter to its own CRecipientFilter.
		unsafe {
			let pushed = &raw mut (*this)._base.m_nStatusPushed;
			pushed.write(pushed.read() + 1);
			vcall!(this as sys::ITempEntsSystem__bindgen_vtable => ITempEntsSystem_Sparks(
				filter.as_raw(), 0.0, &raw const origin, i32::from(burst.magnitude),
				i32::from(burst.trail_length), &raw const direction,
			));
			pushed.write(pushed.read() - 1);
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::user_messages::recipients;
	use sdk_raw::test_support::{mock_vtable, unexpected_call};

	use std::{
		cell::RefCell,
		ptr::{NonNull, null_mut},
	};

	thread_local! { static RECEIVED: RefCell<Vec<(Vector, Vector, i32, i32, i32, i32)>> = const { RefCell::new(Vec::new()) }; }

	unsafe extern "C" fn capture(
		this: *mut sys::ITempEntsSystem,
		filter: *mut sys::IRecipientFilter,
		delay: f32,
		origin: *const sys::Vector,
		magnitude: i32,
		trail: i32,
		direction: *const sys::Vector,
	) {
		assert_eq!(delay, 0.0);
		// SAFETY: sparks supplies a live generated system and filter, and finite vectors.
		unsafe {
			let count = ((*(*filter).vtable_).IRecipientFilter_GetRecipientCount)(filter);
			let recipient = ((*(*filter).vtable_).IRecipientFilter_GetRecipientIndex)(filter, 0);
			assert_eq!(count, 1);
			RECEIVED.with_borrow_mut(|v| {
				v.push((
					(*origin).into(),
					(*direction).into(),
					magnitude,
					trail,
					recipient,
					(*this)._base.m_nStatusPushed,
				))
			});
		}
	}

	#[test]
	fn sparks_dispatch_typed_vectors_filter_and_restore_prediction() {
		RECEIVED.take();
		// SAFETY: This mock generated vtable retains only signature-correct used slots.
		let vt = unsafe {
			mock_vtable::<sys::ITempEntsSystem__bindgen_vtable>(
				unexpected_call as *const (),
				|vt| {
					(&raw mut (*vt).ITempEntsSystem_Sparks).write(capture);
				},
			)
		};
		let mut system = sys::ITempEntsSystem {
			_base: sys::IPredictionSystem {
				vtable_: (&raw const *vt).cast(),
				m_pNextSystem: null_mut(),
				m_bSuppressEvent: false,
				m_pSuppressHost: null_mut(),
				m_nStatusPushed: 2,
			},
		};
		let burst = SparkBurst {
			origin: Vector::new(1.0, 2.0, 3.0),
			direction: Vector::new(0.0, 0.0, 1.0),
			magnitude: 1,
			trail_length: 2,
		};
		// SAFETY: stack system and owned vtable outlive this wrapper and its calls.
		let te = unsafe { TempEntities::from_raw(NonNull::from(&mut system)) };
		te.sparks(&recipients(&[5], false), burst).unwrap();
		for invalid in [
			SparkBurst {
				origin: Vector::new(f32::NAN, 0.0, 0.0),
				..burst
			},
			SparkBurst {
				magnitude: 0,
				..burst
			},
			SparkBurst {
				trail_length: 9,
				..burst
			},
		] {
			assert_eq!(
				te.sparks(&recipients(&[5], false), invalid),
				Err(SparkError)
			);
		}
		te.sparks(&Recipients::new(), burst).unwrap();
		assert_eq!(
			RECEIVED.take(),
			[(burst.origin, burst.direction, 1, 2, 5, 3)]
		);
		assert_eq!(system._base.m_nStatusPushed, 2);
	}
}
