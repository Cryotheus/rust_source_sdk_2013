//! The engine's vector, angle, and color types, convertible to and from their
//! `sys` bindings.

use glam::{EulerRot, Quat, Vec3};
use std::ops::{Deref, DerefMut};

/// The math library [`Vector`] wraps and [`QAngle`] converts through.
pub use glam;

/// A color with 8-bit red, green, blue, and alpha components (`color32`).
#[doc(alias("color32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color32 {
	/// The red component.
	pub r: u8,

	/// The green component.
	pub g: u8,

	/// The blue component.
	pub b: u8,

	/// The opacity, where [`u8::MAX`] is opaque.
	pub a: u8,
}

impl Color32 {
	/// A color from its red, green, blue, and alpha components.
	pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
		Self { r, g, b, a }
	}

	/// An opaque color.
	pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
		Self::new(r, g, b, u8::MAX)
	}
}

impl From<sys::color32> for Color32 {
	fn from(value: sys::color32) -> Self {
		Self::new(value.r, value.g, value.b, value.a)
	}
}

/// Euler angles in degrees, as the engine's `QAngle` stores an orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QAngle {
	/// Rotation up or down, in degrees (`PITCH`).
	#[doc(alias("x"))]
	pub pitch: f32,

	/// Rotation left or right, in degrees (`YAW`).
	#[doc(alias("y"))]
	pub yaw: f32,

	/// Rotation about the facing direction, in degrees (`ROLL`).
	#[doc(alias("z"))]
	pub roll: f32,
}

impl QAngle {
	/// The rotation order [`to_quat`](Self::to_quat) and
	/// [`from_quat`](Self::from_quat) use, with the yaw, pitch, and roll as its
	/// first, second, and third angles: yaw about the fixed Y axis, then pitch
	/// about X, then roll about Z.
	///
	/// This is not the engine's own convention: `AngleQuaternion` yaws about
	/// Z, then pitches about the rotated Y, then rolls about the rotated X,
	/// which is glam's intrinsic [`EulerRot::ZYX`] with the yaw, pitch, and
	/// roll as its angles. The axes and the order both differ, so relabelling
	/// the axes alone does not give the engine's rotation.
	pub const EULER_ROT: EulerRot = EulerRot::YXZEx;

	/// The angles of `quat`, in [`EULER_ROT`](Self::EULER_ROT) order.
	pub fn from_quat(quat: Quat) -> Self {
		let (yaw_rad, pitch_rad, roll_rad) = quat.to_euler(Self::EULER_ROT);

		Self {
			pitch: pitch_rad.to_degrees(),
			yaw: yaw_rad.to_degrees(),
			roll: roll_rad.to_degrees(),
		}
	}

	/// Shifts each angle of at least -180 degrees by whole turns into
	/// `[-180, 180)`.
	///
	/// Angles below -180 degrees come out in `(-540, -180]` instead, since `%`
	/// keeps the sign of the dividend.
	pub const fn normalize(self) -> Self {
		Self {
			pitch: (self.pitch + 180.0) % 360.0 - 180.0,
			yaw: (self.yaw + 180.0) % 360.0 - 180.0,
			roll: (self.roll + 180.0) % 360.0 - 180.0,
		}
	}

	/// The rotation these angles describe, in [`EULER_ROT`](Self::EULER_ROT)
	/// order.
	pub fn to_quat(self) -> Quat {
		let pitch_rad = self.pitch.to_radians();
		let yaw_rad = self.yaw.to_radians();
		let roll_rad = self.roll.to_radians();

		Quat::from_euler(Self::EULER_ROT, yaw_rad, pitch_rad, roll_rad)
	}
}

impl From<sys::QAngle> for QAngle {
	fn from(value: sys::QAngle) -> Self {
		Self {
			pitch: value.x,
			yaw: value.y,
			roll: value.z,
		}
	}
}

/// Three floats, as the engine's `Vector` stores positions, directions, and
/// velocities. Dereferences to the [`Vec3`] it wraps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector(pub Vec3);

impl Vector {
	/// A vector from its x, y, and z components.
	pub const fn new(x: f32, y: f32, z: f32) -> Self {
		Self(Vec3::new(x, y, z))
	}
}

impl Deref for Vector {
	type Target = Vec3;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl DerefMut for Vector {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.0
	}
}

impl From<sys::Vector> for Vector {
	fn from(value: sys::Vector) -> Self {
		Self(Vec3 {
			x: value.x,
			y: value.y,
			z: value.z,
		})
	}
}

impl From<QAngle> for sys::QAngle {
	fn from(value: QAngle) -> Self {
		Self {
			x: value.pitch,
			y: value.yaw,
			z: value.roll,
		}
	}
}

impl From<Vector> for sys::Vector {
	fn from(value: Vector) -> Self {
		Self {
			x: value.0.x,
			y: value.0.y,
			z: value.0.z,
		}
	}
}

impl From<Color32> for sys::color32 {
	fn from(value: Color32) -> Self {
		Self {
			r: value.r,
			g: value.g,
			b: value.b,
			a: value.a,
		}
	}
}
