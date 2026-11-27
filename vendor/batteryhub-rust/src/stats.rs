//! Statistics of one device from its battery history (see history.rs): discharge and charge
//! sessions, average rates, estimated battery life and time left.
//!
//! Everything comes from real readings. A session never spans a gap in monitoring (the START
//! flag), short or tiny sessions are left out (a 1 % step over a few minutes says nothing about
//! the rate), and a figure is None when there is not enough data for it; the dashboard then says
//! so instead of showing a made-up number.
use crate::history::Point;

/// A discharge session counts from this length on, whatever its drop: a slow device (a mouse
/// losing 1 % an hour) drops only a few percent between two restarts of BatteryHub, so the drops
/// are added up over the sessions instead.
pub const MIN_DISCHARGE_SECS: u64 = 30 * 60;
/// The discharge figures need this total drop over the counted sessions (and MIN_TOTAL_SECS).
pub const MIN_DROP: u8 = 5;
/// A charge session counts for the rate from this rise and this length on.
pub const MIN_RISE: u8 = 5;
pub const MIN_CHARGE_SECS: u64 = 5 * 60;
/// Averages need this much measured time in total.
pub const MIN_TOTAL_SECS: u64 = 3600;

/// One stretch on battery or on power, between two readings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Session {
    pub start: u64,
    pub end: u64,
    pub from: u8,
    pub to: u8,
    /// Charge sessions: reached 100 % or the device said "full".
    pub completed: bool,
}

impl Session {
    pub fn secs(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
    /// Percent per hour (positive: charging, negative: discharging).
    pub fn rate(&self) -> f64 {
        let h = self.secs() as f64 / 3600.0;
        if h <= 0.0 { 0.0 } else { (self.to as f64 - self.from as f64) / h }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Every session on battery (oldest first), counted or not.
    pub discharges: Vec<Session>,
    /// Every session on power (oldest first).
    pub charges: Vec<Session>,
    /// Average drop while on battery, % per hour.
    pub discharge_rate: Option<f64>,
    /// Average rise while charging, % per hour.
    pub charge_rate: Option<f64>,
    /// 100 % to 0 % at the average rate, in hours.
    pub battery_life_h: Option<f64>,
    /// From the current level to 0 % at the average rate (only while on battery), in hours.
    pub remaining_h: Option<f64>,
    /// 0 % to 100 % at the average charge rate, in hours.
    pub full_charge_h: Option<f64>,
    /// From the current level to 100 % (only while charging), in hours.
    pub to_full_h: Option<f64>,
    /// Average length of the charge sessions that counted, in hours.
    pub avg_charge_session_h: Option<f64>,
    /// End of the last charge session.
    pub last_charged: Option<u64>,
    pub readings: usize,
    /// Time covered by readings (gaps left out), in seconds.
    pub monitored_secs: u64,
    pub first_reading: Option<u64>,
}

/// Splits a run of points of one state into a session (first point to last point).
fn session(run: &[Point], charge: bool) -> Option<Session> {
    let (first, last) = (run.first()?, run.last()?);
    let to = if charge { run.iter().map(|p| p.level).max()? } else { last.level };
    // A charge session ends when it reached its highest level (the time on the charger after
    // that is "full", not charging).
    let end = if charge { run.iter().find(|p| p.level == to).map(|p| p.t)? } else { last.t };
    let completed = charge && (to >= 100 || run.iter().any(|p| p.full()));
    Some(Session { start: first.t, end, from: first.level, to, completed })
}

/// Whether a discharge session is used for the averages.
pub fn counts_discharge(s: &Session) -> bool {
    s.secs() >= MIN_DISCHARGE_SECS
}

/// Whether a charge session is used for the averages.
pub fn counts_charge(s: &Session) -> bool {
    s.to >= s.from.saturating_add(MIN_RISE) && s.secs() >= MIN_CHARGE_SECS
}

/// Statistics of `pts` (oldest first, as History::series returns them). `current` is the
/// device's level and charging state right now, if it is connected.
pub fn compute(pts: &[Point], current: Option<(i32, bool)>) -> Stats {
    let mut st = Stats { readings: pts.len(), first_reading: pts.first().map(|p| p.t), ..Default::default() };
    // Runs: consecutive points of the same state within one stretch of monitoring.
    // Right after a charge a device keeps reporting the level it reached for a while (the reading
    // settles before it moves), also across times it was off: a discharge only counts from its
    // first change of level after a charge, or those hours would count as hours without any drop.
    let mut settling = false;
    let mut i = 0;
    while i < pts.len() {
        let charging = pts[i].charging();
        let mut j = i + 1;
        while j < pts.len() && !pts[j].starts() && pts[j].charging() == charging {
            j += 1;
        }
        // The next reading in the same stretch shows when this state ended: a discharge run ends
        // at the first reading on power (the level there is still the battery's), so it counts.
        let mut run: Vec<Point> = pts[i..j].to_vec();
        if !charging && j < pts.len() && !pts[j].starts() {
            // By the first reading on power the level may already have gone up: the run ends
            // there with its own last level, never with the charger's.
            let last = run[run.len() - 1].level;
            run.push(Point { flags: 0, level: pts[j].level.min(last), ..pts[j] });
        }
        if charging {
            settling = true;
        } else if settling {
            match run.iter().position(|p| p.level != run[0].level) {
                Some(m) => {
                    settling = false;
                    run.drain(..m);
                }
                None => run.clear(),
            }
        }
        if let Some(s) = session(&run, charging)
            && run.len() > 1
        {
            if charging { st.charges.push(s) } else { st.discharges.push(s) }
        }
        i = j;
    }
    // Monitored time: sum of the stretches.
    let mut seg_start = None;
    let mut prev: Option<&Point> = None;
    for p in pts {
        if p.starts() || seg_start.is_none() {
            if let (Some(s), Some(e)) = (seg_start, prev) {
                st.monitored_secs += e.t.saturating_sub(s);
            }
            seg_start = Some(p.t);
        }
        prev = Some(p);
    }
    if let (Some(s), Some(e)) = (seg_start, prev) {
        st.monitored_secs += e.t.saturating_sub(s);
    }

    // A session that ends a little higher than it started (a reading jumping back by 1 %) counts
    // with that rise, so such steps cancel out instead of only the drops being added up.
    let (drop, secs) = st.discharges.iter().filter(|s| counts_discharge(s)).fold((0i64, 0u64), |(d, t), s| (d + s.from as i64 - s.to as i64, t + s.secs()));
    if secs >= MIN_TOTAL_SECS && drop >= MIN_DROP as i64 {
        let rate = drop as f64 / (secs as f64 / 3600.0);
        st.discharge_rate = Some(rate);
        st.battery_life_h = Some(100.0 / rate);
        if let Some((level, false)) = current
            && level >= 0
        {
            st.remaining_h = Some(level as f64 / rate);
        }
    }
    let counted_chg: Vec<&Session> = st.charges.iter().filter(|s| counts_charge(s)).collect();
    let (rise, secs) = counted_chg.iter().fold((0u64, 0u64), |(r, t), s| (r + (s.to - s.from) as u64, t + s.secs()));
    if !counted_chg.is_empty() && rise >= MIN_RISE as u64 && secs > 0 {
        let rate = rise as f64 / (secs as f64 / 3600.0);
        st.charge_rate = Some(rate);
        st.full_charge_h = Some(100.0 / rate);
        st.avg_charge_session_h = Some(secs as f64 / 3600.0 / counted_chg.len() as f64);
        if let Some((level, true)) = current
            && (0..100).contains(&level)
        {
            st.to_full_h = Some((100 - level) as f64 / rate);
        }
    }
    st.last_charged = st.charges.iter().filter(|s| s.to > s.from || s.completed).map(|s| s.end).max();
    st
}

/// An early estimate needs this much measured time (on battery, or charging) and a change of 1 %.
pub const EARLY_MIN_SECS: u64 = 10 * 60;

/// The rate behind the time left and the card's forecast, in % per hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub rate: f64,
    /// Not the average above (too little data for it yet), but every measured stretch so far:
    /// available from the first 1 % on, and more exact the longer the device is measured.
    pub early: bool,
    /// Measured time it is based on, in seconds.
    pub secs: u64,
}

impl Stats {
    /// The discharge rate to estimate with: the average when there is one, else an early one.
    pub fn discharge_estimate(&self) -> Option<Estimate> {
        if let Some(rate) = self.discharge_rate {
            let secs = self.discharges.iter().filter(|s| counts_discharge(s)).map(|s| s.secs()).sum();
            return Some(Estimate { rate, early: false, secs });
        }
        let (drop, secs) = self.discharges.iter().fold((0i64, 0u64), |(d, t), s| (d + s.from as i64 - s.to as i64, t + s.secs()));
        (drop >= 1 && secs >= EARLY_MIN_SECS).then(|| Estimate { rate: drop as f64 / (secs as f64 / 3600.0), early: true, secs })
    }

    /// The charge rate to estimate with: the average when there is one, else an early one.
    pub fn charge_estimate(&self) -> Option<Estimate> {
        if let Some(rate) = self.charge_rate {
            let secs = self.charges.iter().filter(|s| counts_charge(s)).map(|s| s.secs()).sum();
            return Some(Estimate { rate, early: false, secs });
        }
        let (rise, secs) = self.charges.iter().fold((0u64, 0u64), |(r, t), s| (r + s.to.saturating_sub(s.from) as u64, t + s.secs()));
        (rise >= 1 && secs >= EARLY_MIN_SECS).then(|| Estimate { rate: rise as f64 / (secs as f64 / 3600.0), early: true, secs })
    }

    /// Hours until empty (on battery) or until full (charging) from `level`, and the estimate used.
    pub fn time_left(&self, level: i32, charging: bool) -> Option<(f64, Estimate)> {
        if !(0..=100).contains(&level) || (charging && level >= 100) {
            return None;
        }
        let e = if charging { self.charge_estimate()? } else { self.discharge_estimate()? };
        let left = if charging { 100 - level } else { level };
        (e.rate > 0.0).then(|| (left as f64 / e.rate, e))
    }
}

/// The level expected `dt` seconds after `level`, going down (or up while charging) at `rate`
/// % per hour, within 0..100.
pub fn project(level: f64, rate: f64, charging: bool, dt: u64) -> f64 {
    let d = rate * dt as f64 / 3600.0;
    if charging { (level + d).min(100.0) } else { (level - d).max(0.0) }
}

/// The reading that holds at `t`: the last one at or before it, if `t` lies inside a measured
/// stretch. The last stretch counts up to `open_until` (now, while the device is read).
pub fn level_at(pts: &[Point], t: u64, open_until: u64) -> Option<Point> {
    let i = pts.partition_point(|p| p.t <= t).checked_sub(1)?;
    let p = pts[i];
    match pts.get(i + 1) {
        Some(next) if next.starts() && t > p.t => None,
        Some(_) => Some(p),
        None => (t == p.t || t <= open_until).then_some(p),
    }
}

/// "3 h 20 min", "45 min", "2 d 4 h" - rounded to what the data supports.
pub fn fmt_hours(h: f64) -> String {
    use crate::i18n::tr;
    let mins = (h * 60.0).round() as i64;
    if mins < 1 {
        tr("under 1 min")
    } else if mins < 60 {
        format!("{mins} {}", tr("min"))
    } else if mins < 48 * 60 {
        let (hh, mm) = (mins / 60, mins % 60);
        // Over 10 h, minutes are noise: round to the hour.
        if hh >= 10 || mm == 0 { format!("{} {}", (mins + 30) / 60, tr("h")) } else { format!("{hh} {} {mm} {}", tr("h"), tr("min")) }
    } else {
        let hh = (mins + 30) / 60;
        if hh % 24 == 0 { format!("{} {}", hh / 24, tr("d")) } else { format!("{} {} {} {}", hh / 24, tr("d"), hh % 24, tr("h")) }
    }
}

/// "12.5 %/h", "0.8 %/h".
pub fn fmt_rate(r: f64) -> String {
    if r >= 10.0 { format!("{r:.0} %/h") } else { format!("{} %/h", crate::i18n::decimal(r, 1)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{CHARGING, FULL, START};

    fn p(t_min: u64, level: u8, flags: u8) -> Point {
        Point { t: t_min * 60, level, flags }
    }

    #[test]
    fn not_enough_data_means_no_figures() {
        let st = compute(&[p(0, 80, START), p(20, 79, 0)], Some((79, false)));
        assert_eq!(st.discharge_rate, None);
        assert_eq!(st.remaining_h, None);
        assert_eq!(st.charge_rate, None);
        assert_eq!(compute(&[], None), Stats::default());
    }

    #[test]
    fn charge_start_does_not_turn_a_drop_into_a_rise() {
        // 73 -> 70 % over 3 h, then on the charger: the first reading on power already says 74 %.
        let pts = [p(0, 73, START), p(60, 72, 0), p(120, 71, 0), p(180, 70, 0), p(185, 74, CHARGING), p(200, 82, CHARGING)];
        let st = compute(&pts, None);
        let s = st.discharges[0];
        assert_eq!((s.from, s.to), (73, 70));
        assert!((st.discharge_estimate().unwrap().rate - 3.0 / (185.0 / 60.0)).abs() < 1e-9);
    }

    #[test]
    fn level_held_after_a_charge_is_not_counted_as_no_drop() {
        // 4 h at 1 %/h, a charge to 82 %, then 82 % for 3 h in two sessions (switched off in
        // between) before the level starts to move again: 81 % one hour later.
        let pts = [
            p(0, 80, START),
            p(60, 79, 0),
            p(120, 78, 0),
            p(180, 77, 0),
            p(240, 76, 0),
            p(250, 82, CHARGING),
            p(260, 82, 0),
            p(320, 82, 0),
            p(900, 82, START),
            p(960, 82, 0),
            p(1020, 82, 0),
            p(1080, 81, 0),
            p(1140, 81, 0),
        ];
        let st = compute(&pts, Some((81, false)));
        // Only 0 -> 250 min (4 % drop) and 1080 -> 1140 min (no drop) count: 4 % in 310 min.
        assert!((st.discharge_estimate().unwrap().rate - 4.0 / (310.0 / 60.0)).abs() < 1e-9);
    }

    #[test]
    fn discharge_rate_life_and_time_left() {
        // 100 % -> 80 % in 4 hours: 5 %/h, 20 h of battery life, 16 h left at 80 %.
        let pts = [p(0, 100, START), p(60, 95, 0), p(120, 90, 0), p(180, 85, 0), p(240, 80, 0)];
        let st = compute(&pts, Some((80, false)));
        assert!((st.discharge_rate.unwrap() - 5.0).abs() < 1e-9);
        assert!((st.battery_life_h.unwrap() - 20.0).abs() < 1e-9);
        assert!((st.remaining_h.unwrap() - 16.0).abs() < 1e-9);
        assert_eq!(st.monitored_secs, 240 * 60);
        // While charging there is no "time left on battery".
        assert_eq!(compute(&pts, Some((80, true))).remaining_h, None);
    }

    #[test]
    fn slow_drops_add_up_over_restarts() {
        // A mouse losing about 1 %/h, with BatteryHub restarted every 2 hours: no session drops
        // 5 %, but together they do (80 -> 78, 78 -> 76, 75 -> 74 after a jump back from 73: 5 % in
        // 365 min). The 20-minute stretch is too short and left out.
        let pts = [
            p(0, 80, START),
            p(60, 79, 0),
            p(120, 78, 0),
            p(130, 78, START),
            p(190, 77, 0),
            p(250, 76, 0),
            p(260, 76, START),
            p(280, 75, 0),
            p(300, 75, START),
            p(360, 74, 0),
            p(420, 73, 0),
            p(425, 74, 0),
        ];
        let st = compute(&pts, Some((74, false)));
        assert!((st.discharge_rate.unwrap() - 5.0 / (365.0 / 60.0)).abs() < 1e-9);
        assert!(st.remaining_h.is_some());
        // Below 5 % in total there is still no figure.
        assert_eq!(compute(&pts[..8], Some((75, false))).discharge_rate, None);
    }

    #[test]
    fn gaps_split_sessions_and_are_not_counted() {
        // 50 -> 45 over 1 h, then the PC was off for 10 h and the level is 20 afterwards: the
        // drop during the gap is not measured, so it is not part of any session.
        let pts = [p(0, 50, START), p(60, 45, 0), p(660, 20, START), p(720, 15, 0)];
        let st = compute(&pts, None);
        assert_eq!(st.discharges.len(), 2);
        assert!((st.discharge_rate.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(st.monitored_secs, 120 * 60);
    }

    #[test]
    fn charge_sessions_rates_and_completion() {
        let pts = [
            p(0, 60, START),
            p(60, 50, 0),
            p(61, 50, CHARGING),
            p(91, 80, CHARGING),
            p(121, 100, CHARGING | FULL),
            p(200, 100, CHARGING | FULL), // on the charger, already full
            p(201, 100, 0),
        ];
        let st = compute(&pts, Some((60, true)));
        assert_eq!(st.charges.len(), 1);
        let c = st.charges[0];
        assert!(c.completed);
        assert_eq!((c.from, c.to, c.secs()), (50, 100, 60 * 60)); // ends when it reached 100
        assert!((st.charge_rate.unwrap() - 50.0).abs() < 1e-9);
        assert!((st.full_charge_h.unwrap() - 2.0).abs() < 1e-9);
        assert!((st.to_full_h.unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(st.last_charged, Some(121 * 60));
        // The discharge before it ends at the first reading on power.
        assert_eq!(st.discharges[0].end, 61 * 60);
    }

    #[test]
    fn early_estimate_from_the_first_percent() {
        // 20 minutes, 80 -> 79: too little for the average, enough for an early 3 %/h.
        let pts = [p(0, 80, START), p(5, 80, 0), p(20, 79, 0)];
        let st = compute(&pts, Some((79, false)));
        assert_eq!(st.discharge_rate, None);
        let e = st.discharge_estimate().unwrap();
        assert!(e.early && (e.rate - 3.0).abs() < 1e-9 && e.secs == 20 * 60);
        let (h, _) = st.time_left(79, false).unwrap();
        assert!((h - 79.0 / 3.0).abs() < 1e-9);
        // Under 10 minutes, or no drop yet: nothing to estimate with.
        assert_eq!(compute(&pts[..2], None).discharge_estimate(), None);
        assert_eq!(compute(&[p(0, 80, START), p(60, 80, 0)], None).discharge_estimate(), None);
        // With enough data the average is used, not an early one.
        let long = [p(0, 100, START), p(60, 95, 0), p(120, 90, 0)];
        assert!(!compute(&long, None).discharge_estimate().unwrap().early);
        // Charging: 40 -> 50 in 20 minutes, 30 %/h, 1 h 40 min to full from 50.
        let chg = [p(0, 40, START | CHARGING), p(20, 50, CHARGING)];
        let (h, e) = compute(&chg, Some((50, true))).time_left(50, true).unwrap();
        assert!((e.rate - 30.0).abs() < 1e-9 && (h - 50.0 / 30.0).abs() < 1e-9);
        assert_eq!(compute(&chg, None).time_left(100, true), None);
    }

    #[test]
    fn projection_and_level_at() {
        assert!((project(50.0, 2.0, false, 3 * 3600) - 44.0).abs() < 1e-9);
        assert_eq!(project(3.0, 2.0, false, 12 * 3600), 0.0);
        assert_eq!(project(90.0, 30.0, true, 3600), 100.0);
        let pts = [p(0, 80, START), p(30, 79, 0), p(90, 78, START), p(100, 77, 0)];
        assert_eq!(level_at(&pts, 10 * 60, 0).map(|q| q.level), Some(80));
        assert_eq!(level_at(&pts, 30 * 60, 0).map(|q| q.level), Some(79));
        // Between the stretches (not measured) and before the first reading: nothing.
        assert_eq!(level_at(&pts, 60 * 60, 0), None);
        assert_eq!(level_at(&[p(10, 80, START)], 0, 0), None);
        // After the last reading only while the device is still read.
        assert_eq!(level_at(&pts, 110 * 60, 0), None);
        assert_eq!(level_at(&pts, 110 * 60, 120 * 60).map(|q| q.level), Some(77));
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_hours(0.5), "30 min");
        assert_eq!(fmt_hours(3.34), "3 h 20 min");
        assert_eq!(fmt_hours(12.4), "12 h");
        assert_eq!(fmt_hours(52.0), "2 d 4 h");
        assert_eq!(fmt_rate(4.26), "4.3 %/h");
        assert_eq!(fmt_rate(12.4), "12 %/h");
    }
}
