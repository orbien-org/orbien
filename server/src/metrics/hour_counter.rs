use chrono::{DateTime, Local, Timelike};

#[derive(Debug)]
pub struct HourCounter {
    reserve_hours: usize,
    counts: Vec<i64>,
    last_hour: DateTime<Local>,
}

impl HourCounter {
    pub fn new(reserve_hours: usize) -> Self {
        let reserve_hours = reserve_hours.max(1);
        Self {
            reserve_hours,
            counts: vec![0; reserve_hours],
            last_hour: truncate_hour(Local::now()),
        }
    }

    pub fn last_hours(&mut self, hours: usize) -> Vec<i64> {
        self.rotate(Local::now());
        let n = hours.min(self.reserve_hours);
        self.counts[..n].to_vec()
    }

    pub fn inc(&mut self, delta: i64) {
        self.rotate(Local::now());
        self.counts[0] = self.counts[0].saturating_add(delta);
    }

    fn rotate(&mut self, now: DateTime<Local>) {
        let current = truncate_hour(now);
        if current <= self.last_hour {
            return;
        }
        let elapsed = current.signed_duration_since(self.last_hour);
        let hours = elapsed.num_hours();
        if hours <= 0 {
            return;
        }
        let shift = hours as usize;
        if shift >= self.reserve_hours {
            self.counts.fill(0);
        } else {
            self.counts.rotate_right(shift);
            for slot in self.counts.iter_mut().take(shift) {
                *slot = 0;
            }
        }
        self.last_hour = current;
    }
}

fn truncate_hour(t: DateTime<Local>) -> DateTime<Local> {
    t.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}
