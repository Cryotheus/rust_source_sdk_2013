//! Exact signs for determinants of original finite f32 support planes.
//!
//! A vertex remains three source planes. Its halfspace sign against a fourth
//! plane with rows [nx, ny, nz, distance] is -sign(det4) * sign(det3).
//! Rounded display coordinates must never be passed back into this predicate.
//!
//! BigInt allocations are infallible inside num-bigint. Their number and size
//! here are bounded before use by the work ledger and the fixed input degree.

use super::{Budget, Error};
use num_bigint::{BigInt, Sign as IntegerSign};

const MAX_EXACT_BITS: u64 = 1_200;
const FILTER_FACTOR: f64 = 32.0 * f64::EPSILON;

const PERMUTATIONS_3: [([usize; 3], bool); 6] = [
	([0, 1, 2], false),
	([0, 2, 1], true),
	([1, 0, 2], true),
	([1, 2, 0], false),
	([2, 0, 1], false),
	([2, 1, 0], true),
];

const PERMUTATIONS_4: [([usize; 4], bool); 24] = [
	([0, 1, 2, 3], false),
	([0, 1, 3, 2], true),
	([0, 2, 1, 3], true),
	([0, 2, 3, 1], false),
	([0, 3, 1, 2], false),
	([0, 3, 2, 1], true),
	([1, 0, 2, 3], true),
	([1, 0, 3, 2], false),
	([1, 2, 0, 3], false),
	([1, 2, 3, 0], true),
	([1, 3, 0, 2], true),
	([1, 3, 2, 0], false),
	([2, 0, 1, 3], false),
	([2, 0, 3, 1], true),
	([2, 1, 0, 3], true),
	([2, 1, 3, 0], false),
	([2, 3, 0, 1], false),
	([2, 3, 1, 0], true),
	([3, 0, 1, 2], true),
	([3, 0, 2, 1], false),
	([3, 1, 0, 2], false),
	([3, 1, 2, 0], true),
	([3, 2, 0, 1], true),
	([3, 2, 1, 0], false),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Sign {
	Negative,
	Zero,
	Positive,
}

/// Exact binary32-to-binary64 expansion, including subnormals and signed zero.
/// Integer bit construction avoids conversions that can flush f32 inputs under
/// DAZ. Every finite nonzero input becomes a normal f64, without consulting the
/// rounding mode. Infinity and NaN encodings retain their sign and payload bits.
pub(super) fn widen(value: f32) -> f64 {
	let bits = value.to_bits();
	let sign = u64::from(bits >> 31) << 63;
	let exponent = (bits >> 23) & 0xff;
	let fraction = bits & 0x7f_ffff;
	let widened = match (exponent, fraction) {
		(0, 0) => sign,

		(0, _) => {
			let highest = 31 - fraction.leading_zeros();
			let significand = u64::from(fraction) ^ (1_u64 << highest);
			sign | (u64::from(highest + 874) << 52) | (significand << (52 - highest))
		}

		(0xff, _) => sign | (0x7ff_u64 << 52) | (u64::from(fraction) << 29),
		_ => sign | (u64::from(exponent + 896) << 52) | (u64::from(fraction) << 29),
	};
	f64::from_bits(widened)
}

pub(super) fn det3(rows: [[f32; 3]; 3], budget: &mut Budget) -> Result<Sign, Error> {
	determinant(rows, &PERMUTATIONS_3, budget)
}

pub(super) fn det4(rows: [[f32; 4]; 4], budget: &mut Budget) -> Result<Sign, Error> {
	determinant(rows, &PERMUTATIONS_4, budget)
}

// Only det3 and det4 instantiate this function. Keep the evaluated expression
// as permutation products followed by a sequential sum: the bound below is
// specific to that expression, not elimination or a reassociated determinant.
fn determinant<const N: usize>(
	rows: [[f32; N]; N],
	permutations: &[([usize; N], bool)],
	budget: &mut Budget,
) -> Result<Sign, Error> {
	budget.predicate()?;
	if rows.iter().flatten().any(|value| !value.is_finite()) {
		return Err(Error::InvalidPlane);
	}

	let mut sum = 0.0_f64;
	let mut permanent = 0.0_f64;
	for (columns, odd) in permutations {
		let mut product = widen(rows[0][columns[0]]);
		for row in 1..N {
			product *= widen(rows[row][columns[row]]);
		}
		if *odd {
			sum -= product;
		} else {
			sum += product;
		}
		permanent += product.abs();
	}

	// Bit widening preserves subnormal f32 inputs even under DAZ. Nonzero
	// four-f32 products are normal f64 values (2^-596 through <2^512), sums
	// stay below 2^517, and nonzero cancellation is at least 2^-648. Thus
	// f64 FTZ/DAZ cannot affect this expression or its error bound.
	// In all four IEEE rounding modes, each operation has relative error
	// at most 2u, where u=2^-53. Products plus 23 effective additions give
	// gamma=52u/(1-52u) against the true permanent P. The computed permanent
	// is at least (1-gamma)*P, so determinant error is at most
	// 52u/(1-104u)*permanent < 64u*permanent. Scaling by 64u=2^-47 is
	// exact and normal in this range, independently of the rounding mode.
	if permanent == 0.0 {
		return Ok(Sign::Zero);
	}
	let bound = FILTER_FACTOR * permanent;
	if sum > bound {
		return Ok(Sign::Positive);
	}
	if sum < -bound {
		return Ok(Sign::Negative);
	}
	exact_determinant(rows, permutations, budget)
}

fn exact_determinant<const N: usize>(
	rows: [[f32; N]; N],
	permutations: &[([usize; N], bool)],
	budget: &mut Budget,
) -> Result<Sign, Error> {
	// Charge all integer conversions, N-1 multiplications per term, and its
	// accumulator addition before any BigInt allocation: 27 / 112 units.
	budget.exact(N * N + permutations.len() * N)?;
	let mut integers: [[BigInt; N]; N] =
		std::array::from_fn(|_| std::array::from_fn(|_| BigInt::from(0_u8)));
	for (row_index, row) in rows.into_iter().enumerate() {
		let parts = row.map(dyadic_parts);
		let common_exponent = parts
			.iter()
			.filter(|(mantissa, _)| *mantissa != 0)
			.map(|(_, exponent)| *exponent)
			.min()
			.unwrap_or(0);
		for (column, (mantissa, exponent)) in parts.into_iter().enumerate() {
			// The row is multiplied by one positive power of two, preserving
			// its orientation. Signed zero canonicalizes to integer zero.
			let value = if mantissa == 0 {
				BigInt::from(0_u8)
			} else {
				BigInt::from(mantissa) << (exponent - common_exponent) as usize
			};
			check_width(&value)?;
			integers[row_index][column] = value;
		}
	}

	let mut sum = BigInt::from(0_u8);
	for (columns, odd) in permutations {
		let mut product = &integers[0][columns[0]] * &integers[1][columns[1]];
		check_width(&product)?;
		for row in 2..N {
			product *= &integers[row][columns[row]];
			check_width(&product)?;
		}
		if *odd {
			sum -= product;
		} else {
			sum += product;
		}
		check_width(&sum)?;
	}
	Ok(match sum.sign() {
		IntegerSign::Minus => Sign::Negative,
		IntegerSign::NoSign => Sign::Zero,
		IntegerSign::Plus => Sign::Positive,
	})
}

fn dyadic_parts(value: f32) -> (i32, i32) {
	let bits = value.to_bits();
	let exponent_bits = ((bits >> 23) & 0xff) as i32;
	let fraction = (bits & 0x7f_ffff) as i32;
	let (mantissa, exponent) = if exponent_bits == 0 {
		(fraction, -149)
	} else {
		(fraction | 0x80_0000, exponent_bits - 150)
	};
	let mantissa = if bits >> 31 == 0 { mantissa } else { -mantissa };
	(mantissa, exponent)
}

fn check_width(value: &BigInt) -> Result<(), Error> {
	// A finite-f32 row has at most 277 coefficient bits after exact scaling;
	// summing 24 products of four coefficients needs at most 1,113 bits.
	// No expression ever contains a rounded intersection or increasing degree.
	if value.bits() > MAX_EXACT_BITS {
		return Err(Error::Arithmetic);
	}
	Ok(())
}
