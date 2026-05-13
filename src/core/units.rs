//! Unit conversion for IDML geometry.

/// PostScript points per inch.
pub const POINTS_PER_INCH: f64 = 72.0;

/// Millimeters per inch.
pub const MILLIMETERS_PER_INCH: f64 = 25.4;

/// A length measured in IDML points.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Points(pub f64);

impl Points {
    /// Creates a point value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the raw point value.
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        self.0
    }

    /// Converts points to inches.
    #[must_use]
    pub fn to_inches(self) -> Inches {
        Inches(self.0 / POINTS_PER_INCH)
    }

    /// Converts points to millimeters.
    #[must_use]
    pub fn to_millimeters(self) -> Millimeters {
        self.to_inches().to_millimeters()
    }
}

/// A length measured in inches.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Inches(pub f64);

impl Inches {
    /// Creates an inch value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the raw inch value.
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        self.0
    }

    /// Converts inches to points.
    #[must_use]
    pub fn to_points(self) -> Points {
        Points(self.0 * POINTS_PER_INCH)
    }

    /// Converts inches to millimeters.
    #[must_use]
    pub fn to_millimeters(self) -> Millimeters {
        Millimeters(self.0 * MILLIMETERS_PER_INCH)
    }
}

/// A length measured in millimeters.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Millimeters(pub f64);

impl Millimeters {
    /// Creates a millimeter value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the raw millimeter value.
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        self.0
    }

    /// Converts millimeters to inches.
    #[must_use]
    pub fn to_inches(self) -> Inches {
        Inches(self.0 / MILLIMETERS_PER_INCH)
    }

    /// Converts millimeters to points.
    #[must_use]
    pub fn to_points(self) -> Points {
        self.to_inches().to_points()
    }
}

#[cfg(test)]
mod tests {
    use super::{Inches, Millimeters, Points};

    #[test]
    fn converts_points_inches_and_millimeters() {
        assert_close(Points::new(72.0).to_inches().as_f64(), 1.0);
        assert_close(Points::new(72.0).to_millimeters().as_f64(), 25.4);
        assert_close(Inches::new(1.0).to_points().as_f64(), 72.0);
        assert_close(Millimeters::new(25.4).to_points().as_f64(), 72.0);
    }

    #[test]
    fn round_trips_without_layout_scale_drift() {
        let points = Points::new(612.345_678_9);
        let round_trip = points.to_millimeters().to_points();

        assert_close(points.as_f64(), round_trip.as_f64());
    }

    fn assert_close(left: f64, right: f64) {
        assert!((left - right).abs() <= 1e-9, "{left} != {right}");
    }
}
