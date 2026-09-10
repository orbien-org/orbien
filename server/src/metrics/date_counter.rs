use chrono::{Local, NaiveDate};

#[derive(Debug)]
pub struct DateCounter {
    reserve_days: usize,
    counts: Vec<i64>,
    last_date: NaiveDate,
}

impl DateCounter {
    pub fn new(reserve_days: usize) -> Self {
        let reserve_days = reserve_days.max(1);
        Self {
            reserve_days,
            counts: vec![0; reserve_days],
            last_date: Local::now().date_naive(),
        }
    }

    pub fn today_count(&mut self) -> i64 {
        self.rotate(Local::now().date_naive());
        self.counts[0]
    }

    pub fn last_days(&mut self, days: usize) -> Vec<i64> {
        self.rotate(Local::now().date_naive());
        let n = days.min(self.reserve_days);
        self.counts[..n].to_vec()
    }

    pub fn inc(&mut self, delta: i64) {
        self.rotate(Local::now().date_naive());
        self.counts[0] = self.counts[0].saturating_add(delta);
    }

    fn rotate(&mut self, today: NaiveDate) {
        if today <= self.last_date {
            return;
        }
        let days = (today - self.last_date).num_days();
        if days <= 0 {
            return;
        }
        let shift = days as usize;
        if shift >= self.reserve_days {
            self.counts.fill(0);
        } else {
            self.counts.rotate_right(shift);
            for slot in self.counts.iter_mut().take(shift) {
                *slot = 0;
            }
        }
        self.last_date = today;
    }
}
