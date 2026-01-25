//! Climate and time system for the simulation
//!
//! Handles day/night cycles, seasons, and environmental conditions.

use serde::{Deserialize, Serialize};

/// The four seasons
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Fall,
    Winter,
}

impl Season {
    /// Growth rate modifier for plants
    pub fn growth_modifier(&self) -> f64 {
        match self {
            Season::Spring => 1.2,
            Season::Summer => 1.0,
            Season::Fall => 0.8,
            Season::Winter => 0.1,
        }
    }

    /// Temperature modifier (-1.0 cold to 1.0 hot)
    pub fn temperature_modifier(&self) -> f64 {
        match self {
            Season::Spring => 0.3,
            Season::Summer => 0.8,
            Season::Fall => 0.2,
            Season::Winter => -0.5,
        }
    }
}

/// Climate state for a simulation zone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Climate {
    /// Day of year (1-365)
    pub day_of_year: u16,
    /// Time of day (0.0-24.0)
    pub time_of_day: f64,
    /// Current year
    pub year: u32,
    /// Latitude for day length calculations (-90 to 90)
    pub latitude: f64,
    /// Time scale multiplier (game seconds per real second)
    pub time_scale: f64,
}

impl Default for Climate {
    fn default() -> Self {
        Self {
            day_of_year: 80,     // Late March (early spring)
            time_of_day: 8.0,    // 8 AM
            year: 1,
            latitude: 42.0,      // NY area latitude
            time_scale: 60.0,    // 1 real second = 1 game minute
        }
    }
}

impl Climate {
    /// Create a new climate with specified starting conditions
    pub fn new(day_of_year: u16, time_of_day: f64, latitude: f64, time_scale: f64) -> Self {
        Self {
            day_of_year: day_of_year.clamp(1, 365),
            time_of_day: time_of_day % 24.0,
            year: 1,
            latitude: latitude.clamp(-90.0, 90.0),
            time_scale,
        }
    }

    /// Get the current season based on day of year (Northern Hemisphere)
    pub fn season(&self) -> Season {
        match self.day_of_year {
            1..=79 => Season::Winter,
            80..=171 => Season::Spring,
            172..=265 => Season::Summer,
            266..=354 => Season::Fall,
            355..=365 => Season::Winter,
            _ => Season::Spring,
        }
    }

    /// Approximate hours of daylight based on latitude and day of year
    pub fn day_length(&self) -> f64 {
        // Simplified calculation - more accurate would use solar declination
        let day_angle = 2.0 * std::f64::consts::PI * (self.day_of_year as f64 - 80.0) / 365.0;
        let declination = 23.45_f64.to_radians() * day_angle.sin();
        let lat_rad = self.latitude.to_radians();

        // Hour angle at sunrise/sunset
        let cos_hour_angle = -(lat_rad.tan() * declination.tan());
        let cos_hour_angle = cos_hour_angle.clamp(-1.0, 1.0);
        let hour_angle = cos_hour_angle.acos();

        // Convert to hours of daylight
        (2.0 * hour_angle.to_degrees() / 15.0).clamp(0.0, 24.0)
    }

    /// Check if it's currently night
    pub fn is_night(&self) -> bool {
        let day_len = self.day_length();
        let sunrise = 12.0 - day_len / 2.0;
        let sunset = 12.0 + day_len / 2.0;
        self.time_of_day < sunrise || self.time_of_day > sunset
    }

    /// Advance time by delta_seconds (real time)
    pub fn advance(&mut self, delta_seconds: f64) {
        // Convert real seconds to game hours
        let game_seconds = delta_seconds * self.time_scale;
        let hours_elapsed = game_seconds / 3600.0;

        self.time_of_day += hours_elapsed;

        // Roll over days
        while self.time_of_day >= 24.0 {
            self.time_of_day -= 24.0;
            self.day_of_year += 1;

            // Roll over years
            if self.day_of_year > 365 {
                self.day_of_year = 1;
                self.year += 1;
            }
        }
    }

    /// Get the current temperature modifier (-1.0 to 1.0)
    /// Combines season with time of day
    pub fn temperature(&self) -> f64 {
        let base = self.season().temperature_modifier();

        // Day/night variation
        let day_len = self.day_length();
        let solar_noon = 12.0;
        let hours_from_noon = (self.time_of_day - solar_noon).abs();
        let day_variation = if hours_from_noon < day_len / 2.0 {
            0.2 * (1.0 - hours_from_noon / (day_len / 2.0))
        } else {
            -0.2
        };

        (base + day_variation).clamp(-1.0, 1.0)
    }

    /// Get growth rate modifier for plants (0.0 to 1.5)
    pub fn growth_rate(&self) -> f64 {
        let season_mod = self.season().growth_modifier();

        // Reduced growth at night
        let light_mod = if self.is_night() { 0.3 } else { 1.0 };

        season_mod * light_mod
    }

    /// Format current date/time as string
    pub fn format(&self) -> String {
        let season = self.season();
        let hour = self.time_of_day as u32;
        let minute = ((self.time_of_day % 1.0) * 60.0) as u32;
        format!(
            "Year {} Day {} ({:?}) {:02}:{:02}",
            self.year, self.day_of_year, season, hour, minute
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seasons() {
        let mut climate = Climate::default();

        climate.day_of_year = 1;
        assert_eq!(climate.season(), Season::Winter);

        climate.day_of_year = 100;
        assert_eq!(climate.season(), Season::Spring);

        climate.day_of_year = 200;
        assert_eq!(climate.season(), Season::Summer);

        climate.day_of_year = 300;
        assert_eq!(climate.season(), Season::Fall);
    }

    #[test]
    fn test_time_advance() {
        let mut climate = Climate::new(365, 23.0, 42.0, 3600.0); // 1 sec = 1 hour

        climate.advance(2.0); // 2 real seconds = 2 hours

        assert_eq!(climate.day_of_year, 1); // Rolled to next year
        assert_eq!(climate.year, 2);
        assert!((climate.time_of_day - 1.0).abs() < 0.01);
    }
}
