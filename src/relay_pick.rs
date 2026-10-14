//! Pick the relay(s) nearest to both ends of a session (A -> relay -> B), by great-circle distance.
//! Pure logic, no I/O and no dependencies: hbbs feeds it the relays' and the clients' coordinates
//! (relay_geo.rs) and round-robins inside the returned set, so co-located relays still share the load.

pub type LatLon = (f64, f64);

const EARTH_RADIUS_KM: f64 = 6371.0;

pub fn haversine_km(a: LatLon, b: LatLon) -> f64 {
    let (la1, lo1) = (a.0.to_radians(), a.1.to_radians());
    let (la2, lo2) = (b.0.to_radians(), b.1.to_radians());
    let h = ((la2 - la1) / 2.0).sin().powi(2) + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().asin()
}

/// Indices of the relays whose path cost is within `tol_km` of the lowest, in input order.
/// cost(r) = d(A, r) + d(r, B); when only one end is located, cost(r) = 2 * d(known end, r).
/// Relays with no known location never enter a located choice.
/// None = no choice by location is possible (neither end located, or no relay located):
/// the caller must fall back to plain round-robin over every relay.
pub fn best_relays(relays: &[Option<LatLon>], a: Option<LatLon>, b: Option<LatLon>, tol_km: f64) -> Option<Vec<usize>> {
    let (a, b) = match (a, b) {
        (None, None) => return None,
        (Some(a), None) => (a, a),
        (None, Some(b)) => (b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let costs: Vec<(usize, f64)> = relays
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.map(|r| (i, haversine_km(a, r) + haversine_km(r, b))))
        .collect();
    let best = costs.iter().map(|&(_, c)| c).fold(f64::INFINITY, f64::min);
    if !best.is_finite() {
        return None;
    }
    Some(costs.into_iter().filter(|&(_, c)| c <= best + tol_km).map(|(i, _)| i).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SG: LatLon = (1.2897, 103.8501);
    const KL: LatLon = (3.1390, 101.6869);
    const DE: LatLon = (49.4521, 11.0767); // Nuremberg
    const HANOI: LatLon = (21.0285, 105.8542);
    const HCM: LatLon = (10.8231, 106.6297);
    const NEW_YORK: LatLon = (40.7128, -74.0060);
    const LONDON: LatLon = (51.5074, -0.1278);
    const PARIS: LatLon = (48.8566, 2.3522);

    fn three() -> Vec<Option<LatLon>> {
        vec![Some(SG), Some(SG), Some(DE)]
    }

    #[test]
    fn haversine_known_distance() {
        let d = haversine_km(LONDON, PARIS);
        assert!((d - 343.5).abs() < 2.0, "London-Paris {d}");
        assert!(haversine_km(SG, SG) < 1e-9);
    }

    #[test]
    fn vietnam_pair_gets_both_singapore_relays() {
        assert_eq!(best_relays(&three(), Some(HANOI), Some(HCM), 500.0), Some(vec![0, 1]));
    }

    #[test]
    fn europe_pair_gets_the_europe_relay() {
        assert_eq!(best_relays(&three(), Some(DE), Some(PARIS), 500.0), Some(vec![2]));
    }

    #[test]
    fn america_pair_gets_the_nearer_europe_relay() {
        assert_eq!(best_relays(&three(), Some(NEW_YORK), Some(NEW_YORK), 500.0), Some(vec![2]));
    }

    #[test]
    fn cross_continent_pair_minimises_the_whole_path() {
        // Hanoi -> SG -> Nuremberg is longer than Hanoi -> Nuremberg directly.
        assert_eq!(best_relays(&three(), Some(HANOI), Some(DE), 500.0), Some(vec![2]));
    }

    #[test]
    fn one_located_end_still_steers() {
        assert_eq!(best_relays(&three(), Some(HANOI), None, 500.0), Some(vec![0, 1]));
        assert_eq!(best_relays(&three(), None, Some(PARIS), 500.0), Some(vec![2]));
    }

    #[test]
    fn no_location_means_no_choice() {
        assert_eq!(best_relays(&three(), None, None, 500.0), None);
        assert_eq!(best_relays(&[None, None], Some(HANOI), Some(HCM), 500.0), None);
        assert_eq!(best_relays(&[], Some(HANOI), Some(HCM), 500.0), None);
    }

    #[test]
    fn unlocated_relay_is_never_picked_when_others_are_located() {
        assert_eq!(best_relays(&[Some(SG), None, Some(DE)], Some(HANOI), Some(HCM), 500.0), Some(vec![0]));
    }

    #[test]
    fn tolerance_decides_what_counts_as_a_tie() {
        // Hanoi->KL->HCM is ~260 km shorter than via SG: a tie at 500 km, not at 100 km.
        let r = [Some(SG), Some(KL), Some(DE)];
        assert_eq!(best_relays(&r, Some(HANOI), Some(HANOI), 1000.0), Some(vec![0, 1]));
        assert_eq!(best_relays(&r, Some(HANOI), Some(HANOI), 100.0), Some(vec![1]));
    }
}
